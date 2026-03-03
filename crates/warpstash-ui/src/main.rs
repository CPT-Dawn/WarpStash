// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — UI Entry Point
// ═══════════════════════════════════════════════════════════════════════════
//
// Launches the Rofi-style clipboard picker as a Wayland layer-shell overlay.
//
// Startup sequence:
//   1. Init tracing
//   2. Load config
//   3. Open database
//   4. Query active Hyprland monitor (optional — graceful fallback)
//   5. Configure layer-shell settings (overlay, exclusive keyboard, centered)
//   6. Run iced_layershell application

mod app;
mod clipboard;
mod theme;

use anyhow::{Context, Result};
use tracing::info;

use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_layershell::settings::{LayerShellSettings, Settings};

use warpstash_common::config::{self, load_config};
use warpstash_common::db::Database;

fn main() -> Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warpstash_ui=info".into()),
        )
        .init();

    info!("WarpStash UI starting");

    // ── Config ───────────────────────────────────────────────────────
    let cfg = load_config().context("Failed to load config")?;
    let ui_width = cfg.ui.width;
    let ui_height = cfg.ui.height;

    // ── Database ─────────────────────────────────────────────────────
    let db_path = config::db_path().context("Failed to resolve database path")?;
    let db = Database::open(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;

    info!(entries = db.entry_count().unwrap_or(0), "Database opened");

    // ── Monitor geometry (Hyprland-aware centering) ──────────────────
    let (margin_top, margin_left) = compute_centered_margins(ui_width, ui_height);

    // ── Layer-shell settings ─────────────────────────────────────────
    let layer_settings = LayerShellSettings {
        anchor: Anchor::Top | Anchor::Left,
        layer: Layer::Overlay,
        exclusive_zone: -1,
        size: Some((ui_width, ui_height)),
        // margin order: (top, right, bottom, left)
        margin: (margin_top, 0, 0, margin_left),
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        events_transparent: false,
        ..Default::default()
    };

    let settings = Settings {
        layer_settings,
        antialiasing: true,
        ..Default::default()
    };

    // ── Run ──────────────────────────────────────────────────────────
    let boot_fn = app::boot(cfg.clone(), db);

    iced_layershell::application(boot_fn, "warpstash", app::update, app::view)
        .settings(settings)
        .subscription(app::subscription)
        .theme(app::get_theme)
        .style(|state: &app::WarpStash, _theme| {
            iced::theme::Style {
                background_color: state.colors.background,
                text_color: state.colors.text,
            }
        })
        .run()
        .context("iced_layershell runtime error")?;

    info!("WarpStash UI exiting");
    Ok(())
}

/// Compute top/left margins to center the popup on the active monitor.
///
/// Tries Hyprland IPC first. Falls back to assuming a 1920×1080 display.
fn compute_centered_margins(width: u32, height: u32) -> (i32, i32) {
    // Try Hyprland IPC at runtime — the crate talks over a Unix socket,
    // so this gracefully fails when not running under Hyprland.
    {
        use hyprland::data::Monitor;
        use hyprland::shared::HyprDataActive;

        match Monitor::get_active() {
            Ok(mon) => {
                let mon_w = mon.width as i32;
                let mon_h = mon.height as i32;
                let top = (mon_h - height as i32) / 2;
                let left = (mon_w - width as i32) / 2;
                info!(monitor = %mon.name, "Centered on Hyprland monitor");
                return (top.max(0), left.max(0));
            }
            Err(_) => {
                info!("Hyprland not available — using fallback centering");
            }
        }
    }

    // Fallback: assume 1920×1080.
    let top = (1080i32 - height as i32) / 2;
    let left = (1920i32 - width as i32) / 2;
    (top.max(0), left.max(0))
}
