// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — Theme ("Cosmic Dawn")
// ═══════════════════════════════════════════════════════════════════════════
//
// Converts the hex color strings from the user's config into an iced
// custom Theme built on Palette + Extended.

use iced::Color;
use iced::Theme;
use iced::theme::Palette;

use warpstash_common::config::ThemeConfig;

// ─── Hex Parsing ─────────────────────────────────────────────────────────

/// Parse a CSS-style hex color string into an iced `Color`.
///
/// Supports:
///   "#RRGGBB"   → opaque
///   "#RRGGBBAA" → with alpha
///
/// Returns a magenta fallback on parse failure, making bad colors obvious.
pub fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');

    let parse = |s: &str| u8::from_str_radix(s, 16).unwrap_or(0);

    match hex.len() {
        6 => {
            let r = parse(&hex[0..2]);
            let g = parse(&hex[2..4]);
            let b = parse(&hex[4..6]);
            Color::from_rgb8(r, g, b)
        }
        8 => {
            let r = parse(&hex[0..2]);
            let g = parse(&hex[2..4]);
            let b = parse(&hex[4..6]);
            let a = parse(&hex[6..8]);
            Color::from_rgba8(r, g, b, a as f32 / 255.0)
        }
        _ => Color::from_rgb8(0xFF, 0x00, 0xFF), // magenta = obviously wrong
    }
}

// ─── Theme Construction ──────────────────────────────────────────────────

/// Build a custom iced `Theme` from the Cosmic Dawn config palette.
///
/// Maps our config colors onto iced's `Palette`:
///   background → Palette::background
///   text       → Palette::text
///   primary    → Palette::primary  (cyan #00E5CC)
///   accent     → Palette::danger   (dawn red #FF6B4A)
///   secondary  → Palette::success  (purple #7C3AED — reused)
///   border     → Palette::warning  (mapped loosely)
pub fn build_theme(cfg: &ThemeConfig) -> Theme {
    let palette = Palette {
        background: hex_to_color(&cfg.background),
        text: hex_to_color(&cfg.text),
        primary: hex_to_color(&cfg.primary),
        success: hex_to_color(&cfg.secondary),
        warning: hex_to_color(&cfg.border),
        danger: hex_to_color(&cfg.accent),
    };

    Theme::custom("Cosmic Dawn".to_string(), palette)
}

// ─── Convenience Accessors ───────────────────────────────────────────────

/// A resolved color set that the view layer can use directly, without
/// re-parsing hex on every frame.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub text: Color,
    pub text_dim: Color,
    pub border: Color,
    pub selected: Color,
}

impl Colors {
    /// Resolve all hex strings from the config once.
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        Self {
            background: hex_to_color(&cfg.background),
            surface: hex_to_color(&cfg.surface),
            surface_hover: hex_to_color(&cfg.surface_hover),
            primary: hex_to_color(&cfg.primary),
            secondary: hex_to_color(&cfg.secondary),
            accent: hex_to_color(&cfg.accent),
            text: hex_to_color(&cfg.text),
            text_dim: hex_to_color(&cfg.text_dim),
            border: hex_to_color(&cfg.border),
            selected: hex_to_color(&cfg.selected),
        }
    }
}
