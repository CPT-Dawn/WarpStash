// ═══════════════════════════════════════════════════════════════════════════
// WarpStash Daemon — Wayland Data-Control Clipboard Watcher
// ═══════════════════════════════════════════════════════════════════════════
//
// Connects to the Wayland compositor, binds zwlr_data_control_manager_v1,
// and listens for every clipboard (and optionally primary selection) change.
//
// Architecture:
//   • The Wayland event loop runs in a dedicated std::thread (Wayland
//     protocol is not async-friendly — it uses blocking roundtrips).
//   • Each clipboard change is read via a pipe, hashed, and sent to the
//     main tokio runtime via an unbounded channel for database insertion.
//
// Protocol flow:
//   1. Connect → bind wl_seat + zwlr_data_control_manager_v1
//   2. manager.get_data_device(seat) → data_device
//   3. data_device emits `data_offer` → new offer object created
//   4. offer emits `offer(mime_type)` for each supported MIME type
//   5. data_device emits `selection` → the active offer is ready to read
//   6. We call `offer.receive(mime, fd)` and read from the pipe
//   7. Send the captured data through the channel; repeat from step 3

use std::io::Read;
use std::os::fd::{AsFd, FromRawFd, IntoRawFd, OwnedFd};

use anyhow::{bail, Context, Result};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
};

use crate::CapturedClip;

// ─── MIME type priority ──────────────────────────────────────────────────

/// Text MIME types in preference order.
const TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// Image MIME types in preference order.
const IMAGE_MIMES: &[&str] = &["image/png", "image/jpeg", "image/bmp", "image/gif"];

// ─── Wayland State ───────────────────────────────────────────────────────

/// The global Wayland state for our data-control listener.
///
/// Instead of per-object user data (complex in wayland-client 0.31), we
/// track all offer state centrally here. The protocol guarantees that
/// offers arrive sequentially: `data_offer` → N × `offer(mime)` →
/// `selection`.
struct WaylandState {
    /// Channel sender to push captured clips to the tokio runtime.
    tx: mpsc::UnboundedSender<CapturedClip>,

    /// The data-control manager global (bound from registry).
    manager: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,

    /// The wl_seat global (bound from registry).
    seat: Option<wl_seat::WlSeat>,

    /// Whether to also watch primary selection.
    watch_primary: bool,

    /// Whether to capture images.
    store_images: bool,

    /// Maximum image size in bytes.
    max_image_bytes: usize,

    /// The current pending offer object and its accumulated MIME types.
    /// Reset each time a `data_offer` event creates a new offer.
    current_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,

    /// MIME types advertised by the current offer.
    current_mimes: Vec<String>,
}

// ─── wl_registry ─────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" => {
                    debug!(name, version, "Binding wl_seat");
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                "zwlr_data_control_manager_v1" => {
                    debug!(name, version, "Binding zwlr_data_control_manager_v1");
                    state.manager = Some(
                        registry
                            .bind::<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, _, _>(
                                name,
                                version.min(2),
                                qh,
                                (),
                            ),
                    );
                }
                _ => {}
            }
        }
    }
}

// ─── wl_seat (no-op — we just need the object handle) ───────────────────

delegate_noop!(WaylandState: ignore wl_seat::WlSeat);

// ─── zwlr_data_control_manager_v1 (no client-side events) ───────────────

delegate_noop!(WaylandState: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

// ─── zwlr_data_control_offer_v1 ─────────────────────────────────────────

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _offer: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            trace!(mime = %mime_type, "Offer advertises MIME type");
            state.current_mimes.push(mime_type);
        }
    }
}

// ─── zwlr_data_control_device_v1 ────────────────────────────────────────

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _device: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            // A new offer has been created by the compositor. Store it and
            // reset the MIME list — subsequent `Offer` events will fill it.
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                trace!("New data offer received");
                // Destroy the previous offer if still around.
                if let Some(old) = state.current_offer.take() {
                    old.destroy();
                }
                state.current_mimes.clear();
                state.current_offer = Some(id);
            }

            // The clipboard selection has changed.
            zwlr_data_control_device_v1::Event::Selection { id } => {
                debug!("Clipboard selection changed");
                if let Some(offer) = id {
                    handle_selection(state, &offer, false);
                } else {
                    debug!("Selection cleared");
                }
            }

            // The primary selection has changed.
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                if state.watch_primary {
                    debug!("Primary selection changed");
                    if let Some(offer) = id {
                        handle_selection(state, &offer, true);
                    }
                }
            }

            zwlr_data_control_device_v1::Event::Finished => {
                warn!("Data control device finished — compositor may have restarted");
            }

            _ => {}
        }
    }

    // When the compositor creates a new offer object via `data_offer`,
    // wayland-client 0.31 needs to know what user data to assign.
    // We provide `()` since we track state centrally.
    fn event_created_child(
        opcode: u16,
        _qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        // opcode 0 = data_offer
        if opcode == 0 {
            _qh.make_data::<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, _>(())
        } else {
            panic!("unexpected event_created_child opcode: {opcode}");
        }
    }
}

// ─── Selection Handler ───────────────────────────────────────────────────

fn handle_selection(
    state: &mut WaylandState,
    offer: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    is_primary: bool,
) {
    let mimes = std::mem::take(&mut state.current_mimes);

    if mimes.is_empty() {
        debug!("Offer has no MIME types");
        return;
    }

    trace!(mimes = ?mimes, "Choosing best MIME from offer");

    // Pick the best MIME type we can handle.
    let chosen_mime = match select_best_mime(&mimes, state.store_images) {
        Some(m) => m,
        None => {
            debug!(offered = ?mimes, "No supported MIME type in offer");
            return;
        }
    };

    // Read the data from the offer via a pipe.
    let data = match read_offer_data(offer, &chosen_mime) {
        Ok(d) => d,
        Err(e) => {
            error!(error = %e, mime = %chosen_mime, "Failed to read offer data");
            return;
        }
    };

    if data.is_empty() {
        debug!("Offer data is empty, skipping");
        return;
    }

    // Check image size limit.
    let is_image = IMAGE_MIMES.contains(&chosen_mime.as_str());
    if is_image && data.len() > state.max_image_bytes {
        warn!(
            size = data.len(),
            max = state.max_image_bytes,
            "Image exceeds max_image_bytes, discarding"
        );
        return;
    }

    let clip = CapturedClip {
        data,
        mime_type: chosen_mime,
        is_primary,
    };

    if let Err(e) = state.tx.send(clip) {
        error!(error = %e, "Failed to send captured clip through channel");
    }

    // Clean up.
    offer.destroy();
    state.current_offer = None;
}

/// Select the best MIME type from the offered list.
fn select_best_mime(offered: &[String], store_images: bool) -> Option<String> {
    // Prefer text.
    for &text_mime in TEXT_MIMES {
        if offered.iter().any(|m| m == text_mime) {
            return Some(text_mime.to_string());
        }
    }

    // Then images (if enabled).
    if store_images {
        for &img_mime in IMAGE_MIMES {
            if offered.iter().any(|m| m == img_mime) {
                return Some(img_mime.to_string());
            }
        }
    }

    None
}

/// Read data from a data-control offer via a Unix pipe.
fn read_offer_data(
    offer: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    mime_type: &str,
) -> Result<Vec<u8>> {
    let (read_fd, write_fd) = make_pipe()?;

    // Tell the compositor to write the clipboard data to our pipe.
    offer.receive(mime_type.to_string(), write_fd.as_fd());

    // Drop the write end so we get EOF after the compositor finishes writing.
    drop(write_fd);

    // Read all data from the pipe.
    let mut buf = Vec::new();
    let mut file = unsafe { std::fs::File::from_raw_fd(read_fd.into_raw_fd()) };
    file.read_to_end(&mut buf)
        .context("Failed to read clipboard data from pipe")?;

    debug!(
        size = buf.len(),
        mime = mime_type,
        "Read clipboard data from offer"
    );
    Ok(buf)
}

/// Create a pipe with O_CLOEXEC via pipe2(2).
fn make_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if ret != 0 {
        bail!("pipe2() failed: {}", std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

// ─── Public API ──────────────────────────────────────────────────────────

/// Spawn the Wayland clipboard watcher in a dedicated thread.
///
/// Returns an unbounded receiver that yields `CapturedClip` for every
/// clipboard change detected by the compositor.
pub fn spawn_watcher(
    watch_primary: bool,
    store_images: bool,
    max_image_bytes: usize,
) -> Result<mpsc::UnboundedReceiver<CapturedClip>> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
        .name("wayland-watcher".into())
        .spawn(move || {
            if let Err(e) = run_watcher(tx, watch_primary, store_images, max_image_bytes) {
                error!(error = %e, "Wayland watcher thread crashed");
            }
        })
        .context("Failed to spawn Wayland watcher thread")?;

    Ok(rx)
}

/// The main Wayland event loop — runs forever in a dedicated thread.
fn run_watcher(
    tx: mpsc::UnboundedSender<CapturedClip>,
    watch_primary: bool,
    store_images: bool,
    max_image_bytes: usize,
) -> Result<()> {
    info!("Connecting to Wayland display...");
    let conn = Connection::connect_to_env()
        .context("Failed to connect to Wayland display — is a compositor running?")?;

    let display = conn.display();

    let mut state = WaylandState {
        tx,
        manager: None,
        seat: None,
        watch_primary,
        store_images,
        max_image_bytes,
        current_offer: None,
        current_mimes: Vec::new(),
    };

    let mut event_queue: EventQueue<WaylandState> = conn.new_event_queue();
    let qh = event_queue.handle();

    // Trigger registry enumeration.
    display.get_registry(&qh, ());

    // First roundtrip: discover globals.
    event_queue
        .roundtrip(&mut state)
        .context("Initial Wayland roundtrip failed")?;

    // Validate we got the necessary globals.
    let manager = state.manager.as_ref().context(
        "Compositor does not support wlr-data-control-unstable-v1. \
             Is your compositor (Hyprland/Sway) running with the protocol enabled?",
    )?;
    let seat = state
        .seat
        .as_ref()
        .context("No wl_seat found — is a seat available?")?;

    info!("Wayland globals bound — creating data control device");

    // Create the data control device for this seat.
    let _device = manager.get_data_device(seat, &qh, ());

    // Second roundtrip to register.
    event_queue
        .roundtrip(&mut state)
        .context("Wayland roundtrip failed after creating data device")?;

    info!("Clipboard watcher active — listening for copy events");

    // Main event loop: dispatch events forever.
    loop {
        event_queue
            .blocking_dispatch(&mut state)
            .context("Wayland event dispatch error")?;
    }
}
