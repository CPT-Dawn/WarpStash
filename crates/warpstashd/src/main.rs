// ═══════════════════════════════════════════════════════════════════════════
// WarpStash Daemon — Main Entry Point
// ═══════════════════════════════════════════════════════════════════════════
//
// Launch sequence:
//   1. Initialize tracing (structured logging).
//   2. Load configuration (embedded defaults + optional user config.toml).
//   3. Open/create the SQLite database.
//   4. Spawn the Wayland data-control clipboard watcher thread.
//   5. Enter the tokio event loop: receive captured clips from the watcher,
//      deduplicate via BLAKE3, insert into the database, enforce max_history.
//   6. Graceful shutdown on SIGINT / SIGTERM.
//
// Usage:
//   # In hyprland.conf:
//   exec-once = warpstashd
//
//   # Or manually:
//   $ warpstashd

mod dedup;
mod wayland;

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use warpstash_common::config;
use warpstash_common::db::Database;
use warpstash_common::types::{make_preview, ContentType, NewEntry};

/// A captured clipboard payload sent from the Wayland watcher thread.
#[derive(Debug, Clone)]
pub struct CapturedClip {
    /// Raw clipboard data bytes.
    pub data: Vec<u8>,
    /// MIME type string (e.g. "text/plain;charset=utf-8", "image/png").
    pub mime_type: String,
    /// Whether this came from the primary selection (middle-click).
    pub is_primary: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Tracing ───────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .init();

    info!(version = env!("CARGO_PKG_VERSION"), "warpstashd starting");

    // ── 2. Configuration ─────────────────────────────────────────────────
    config::write_default_config_if_missing().context("Failed to write default config")?;

    let cfg = config::load_config().context("Failed to load configuration")?;

    info!(
        max_history = cfg.general.max_history,
        deduplicate = cfg.general.deduplicate,
        watch_primary = cfg.daemon.watch_primary_selection,
        store_images = cfg.daemon.store_images,
        "Configuration loaded"
    );

    // ── 3. Database ──────────────────────────────────────────────────────
    let db_path = config::db_path().context("Failed to resolve database path")?;
    let db = Database::open(&db_path).context("Failed to open database")?;

    let existing = db.entry_count()?;
    info!(
        path = %db_path.display(),
        entries = existing,
        "Database ready"
    );

    // ── 4. Wayland Watcher ───────────────────────────────────────────────
    let mut rx = wayland::spawn_watcher(
        cfg.daemon.watch_primary_selection,
        cfg.daemon.store_images,
        cfg.daemon.max_image_bytes,
    )
    .context("Failed to start Wayland clipboard watcher")?;

    info!("Clipboard watcher started — entering main loop");

    // ── 5. Main Processing Loop ──────────────────────────────────────────
    let max_history = cfg.general.max_history;
    let deduplicate = cfg.general.deduplicate;
    let preview_lines = cfg.ui.preview_lines;

    loop {
        tokio::select! {
            // Handle incoming clipboard captures.
            Some(clip) = rx.recv() => {
                if let Err(e) = process_clip(&db, &clip, max_history, deduplicate, preview_lines) {
                    error!(error = %e, "Failed to process clipboard entry");
                }
            }

            // Graceful shutdown on Ctrl+C / SIGTERM.
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal — exiting gracefully");
                break;
            }
        }
    }

    info!("warpstashd stopped");
    Ok(())
}

/// Process a single captured clipboard payload:
///   1. Determine content type (text vs image).
///   2. Hash content for dedup.
///   3. Skip if consecutive duplicate.
///   4. Build preview string.
///   5. Insert into database.
///   6. Enforce max_history.
fn process_clip(
    db: &Database,
    clip: &CapturedClip,
    max_history: usize,
    deduplicate: bool,
    preview_lines: usize,
) -> Result<()> {
    // Determine content type from MIME.
    let content_type = if clip.mime_type.starts_with("image/") {
        ContentType::Image
    } else {
        ContentType::Text
    };

    // Hash for dedup.
    let hash = dedup::content_hash(&clip.data);
    debug!(
        hash = %hash,
        mime = %clip.mime_type,
        size = clip.data.len(),
        content_type = %content_type,
        "Processing captured clip"
    );

    // Dedup check: skip if the most recent entry has the same hash.
    if deduplicate {
        let most_recent = db.most_recent_hash()?;
        if dedup::is_consecutive_duplicate(&hash, most_recent.as_deref()) {
            debug!(hash = %hash, "Consecutive duplicate — skipping");
            return Ok(());
        }
    }

    // Build preview.
    let preview = make_preview(content_type, &clip.data, &clip.mime_type, preview_lines);

    // Construct the entry.
    let entry = match content_type {
        ContentType::Text => {
            let text = String::from_utf8_lossy(&clip.data).into_owned();
            NewEntry {
                content_hash: hash,
                content_type,
                mime_type: clip.mime_type.clone(),
                text_content: Some(text),
                blob_content: None,
                preview,
                byte_size: clip.data.len(),
            }
        }
        ContentType::Image => NewEntry {
            content_hash: hash,
            content_type,
            mime_type: clip.mime_type.clone(),
            text_content: None,
            blob_content: Some(clip.data.clone()),
            preview,
            byte_size: clip.data.len(),
        },
    };

    // Insert.
    let id = db.insert_entry(&entry)?;
    info!(
        id,
        preview = %entry.preview,
        size = entry.byte_size,
        "Stored clipboard entry"
    );

    // Evict old entries.
    let evicted = db.enforce_max_history(max_history)?;
    if evicted > 0 {
        warn!(
            evicted,
            "Evicted old entries to enforce max_history={}", max_history
        );
    }

    Ok(())
}
