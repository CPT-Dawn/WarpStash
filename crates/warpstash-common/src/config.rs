// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — Configuration
// ═══════════════════════════════════════════════════════════════════════════
//
// Loads config with a three-tier cascade:
//   1. Compiled-in default (assets/default_config.toml via include_str!)
//   2. User file at ~/.config/warpstash/config.toml (merged on top)
//   3. (Future) CLI overrides
//
// Missing keys in the user file gracefully fall back to defaults.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// The default configuration, baked into the binary at compile time.
const DEFAULT_CONFIG_TOML: &str = include_str!("../../../assets/default_config.toml");

// ─── Top-Level Config ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub keys: KeysConfig,
}

// ─── Section: General ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_max_history")]
    pub max_history: usize,
    #[serde(default = "default_true")]
    pub deduplicate: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            max_history: default_max_history(),
            deduplicate: true,
        }
    }
}

// ─── Section: Daemon ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub watch_primary_selection: bool,
    #[serde(default = "default_true")]
    pub store_images: bool,
    #[serde(default = "default_max_image_bytes")]
    pub max_image_bytes: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            watch_primary_selection: false,
            store_images: true,
            max_image_bytes: default_max_image_bytes(),
        }
    }
}

// ─── Section: UI ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_border_radius")]
    pub border_radius: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_preview_lines")]
    pub preview_lines: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            width: default_width(),
            height: default_height(),
            border_radius: default_border_radius(),
            opacity: default_opacity(),
            preview_lines: default_preview_lines(),
        }
    }
}

// ─── Section: Theme ("Cosmic Dawn") ──────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_surface")]
    pub surface: String,
    #[serde(default = "default_surface_hover")]
    pub surface_hover: String,
    #[serde(default = "default_primary")]
    pub primary: String,
    #[serde(default = "default_secondary")]
    pub secondary: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_text")]
    pub text: String,
    #[serde(default = "default_text_dim")]
    pub text_dim: String,
    #[serde(default = "default_border")]
    pub border: String,
    #[serde(default = "default_selected")]
    pub selected: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: default_background(),
            surface: default_surface(),
            surface_hover: default_surface_hover(),
            primary: default_primary(),
            secondary: default_secondary(),
            accent: default_accent(),
            text: default_text(),
            text_dim: default_text_dim(),
            border: default_border(),
            selected: default_selected(),
        }
    }
}

// ─── Section: Keybindings ────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct KeysConfig {
    #[serde(default = "default_key_up")]
    pub scroll_up: String,
    #[serde(default = "default_key_down")]
    pub scroll_down: String,
    #[serde(default = "default_key_select")]
    pub select: String,
    #[serde(default = "default_key_delete")]
    pub delete: String,
    #[serde(default = "default_key_search")]
    pub search: String,
    #[serde(default = "default_key_quit")]
    pub quit: String,
    #[serde(default = "default_key_pin")]
    pub pin: String,
}

impl Default for KeysConfig {
    fn default() -> Self {
        Self {
            scroll_up: default_key_up(),
            scroll_down: default_key_down(),
            select: default_key_select(),
            delete: default_key_delete(),
            search: default_key_search(),
            quit: default_key_quit(),
            pin: default_key_pin(),
        }
    }
}

// ─── Default Value Helpers ───────────────────────────────────────────────

fn default_true() -> bool {
    true
}
fn default_max_history() -> usize {
    1000
}
fn default_max_image_bytes() -> usize {
    10_485_760
}
fn default_width() -> u32 {
    680
}
fn default_height() -> u32 {
    520
}
fn default_border_radius() -> f32 {
    12.0
}
fn default_opacity() -> f32 {
    0.92
}
fn default_preview_lines() -> usize {
    3
}

fn default_background() -> String {
    "#0D0B1A".into()
}
fn default_surface() -> String {
    "#1A1730".into()
}
fn default_surface_hover() -> String {
    "#2A2545".into()
}
fn default_primary() -> String {
    "#00E5CC".into()
}
fn default_secondary() -> String {
    "#7C3AED".into()
}
fn default_accent() -> String {
    "#FF6B4A".into()
}
fn default_text() -> String {
    "#E8E6F0".into()
}
fn default_text_dim() -> String {
    "#8B87A3".into()
}
fn default_border() -> String {
    "#2E2A45".into()
}
fn default_selected() -> String {
    "#00E5CC22".into()
}

fn default_key_up() -> String {
    "k".into()
}
fn default_key_down() -> String {
    "j".into()
}
fn default_key_select() -> String {
    "Return".into()
}
fn default_key_delete() -> String {
    "d".into()
}
fn default_key_search() -> String {
    "/".into()
}
fn default_key_quit() -> String {
    "Escape".into()
}
fn default_key_pin() -> String {
    "p".into()
}

// ─── XDG Path Resolution ────────────────────────────────────────────────

/// Returns the XDG-compliant project directories for WarpStash.
/// Config : ~/.config/warpstash/
/// Data   : ~/.local/share/warpstash/
pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "warpstash")
        .context("Failed to resolve XDG base directories — is $HOME set?")
}

/// Returns the path to the user config file:
/// ~/.config/warpstash/config.toml
pub fn config_path() -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("config.toml"))
}

/// Returns the path to the database file:
/// ~/.local/share/warpstash/clipboard.db
pub fn db_path() -> Result<PathBuf> {
    let dirs = project_dirs()?;
    let data_dir = dirs.data_dir();
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;
    Ok(data_dir.join("clipboard.db"))
}

// ─── Config Loading ─────────────────────────────────────────────────────

/// Load and return the merged configuration.
///
/// 1. Parse the compiled-in default config.
/// 2. If a user config file exists at the XDG path, parse it and let
///    serde's defaults fill in any missing keys.
/// 3. Return the final merged config.
pub fn load_config() -> Result<Config> {
    // Step 1: parse compiled-in defaults (guaranteed to be valid TOML).
    let default_config: Config = toml::from_str(DEFAULT_CONFIG_TOML)
        .context("BUG: embedded default_config.toml is invalid")?;

    // Step 2: check for a user override file.
    let user_path = config_path()?;
    if user_path.exists() {
        info!("Loading user config from {}", user_path.display());
        let user_toml = std::fs::read_to_string(&user_path)
            .with_context(|| format!("Failed to read config file: {}", user_path.display()))?;

        let user_config: Config = toml::from_str(&user_toml).with_context(|| {
            format!(
                "Failed to parse config file: {}\nFix the syntax or delete the file to use defaults.",
                user_path.display()
            )
        })?;

        debug!("User config loaded successfully");
        Ok(user_config)
    } else {
        debug!(
            "No user config found at {} — using compiled-in defaults",
            user_path.display()
        );
        Ok(default_config)
    }
}

/// Ensure the config directory exists. Called by the daemon on first start
/// so users have a clear place to drop their config.toml.
pub fn ensure_config_dir() -> Result<()> {
    let dirs = project_dirs()?;
    let config_dir = dirs.config_dir();
    if !config_dir.exists() {
        std::fs::create_dir_all(config_dir).with_context(|| {
            format!(
                "Failed to create config directory: {}",
                config_dir.display()
            )
        })?;
        info!("Created config directory: {}", config_dir.display());
    }
    Ok(())
}

/// Write the default config to the XDG config path if it doesn't already
/// exist, so the user has a starting point to customize.
pub fn write_default_config_if_missing() -> Result<()> {
    ensure_config_dir()?;
    let path = config_path()?;
    if !path.exists() {
        std::fs::write(&path, DEFAULT_CONFIG_TOML)
            .with_context(|| format!("Failed to write default config to {}", path.display()))?;
        info!("Wrote default config to {}", path.display());
    } else {
        warn!(
            "Config file already exists at {} — not overwriting",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let config: Config = toml::from_str(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(config.general.max_history, 1000);
        assert!(config.general.deduplicate);
        assert_eq!(config.theme.background, "#0D0B1A");
        assert_eq!(config.keys.scroll_up, "k");
    }
}
