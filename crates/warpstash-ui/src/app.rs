// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — UI Application (Elm Architecture)
// ═══════════════════════════════════════════════════════════════════════════
//
// Rofi-style clipboard picker implemented with iced + iced_layershell.
//
// State machine:
//   Normal mode  → j/k navigate, Enter paste, d delete, p pin, / search, Esc quit
//   Search mode  → text input captures keys, Esc exits search, Enter searches

use std::cell::RefCell;

use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::widget::operation::focus;
use iced::widget::{column, container, row, scrollable, text, text_input, Column};
use iced::{event, Color, Element, Length, Subscription, Task, Theme};

use iced_layershell::to_layer_message;

use tracing::{debug, error, warn};

use warpstash_common::config::Config;
use warpstash_common::db::Database;
use warpstash_common::types::{ContentType, EntryPreview};

use crate::clipboard::paste_to_clipboard;
use crate::theme::{self, Colors};

// ─── State ───────────────────────────────────────────────────────────────

/// Application state held across the entire lifetime of the picker.
pub struct WarpStash {
    /// Lightweight entry previews displayed in the list.
    pub entries: Vec<EntryPreview>,

    /// Index of the currently highlighted entry.
    pub selected: usize,

    /// Current search query string (bound to the text input).
    pub search_query: String,

    /// Whether the search text input is focused / active.
    pub search_mode: bool,

    /// Handle to the SQLite database (read-write for pin/delete).
    pub db: Database,

    /// Resolved theme colors for the view layer.
    pub colors: Colors,

    /// The iced custom theme (Cosmic Dawn).
    pub theme: Theme,

    /// User configuration.
    pub config: Config,
}

// ─── Message ─────────────────────────────────────────────────────────────

/// All events that drive the UI state machine.
///
/// The `#[to_layer_message]` macro adds the layershell action variants
/// (AnchorChange, SizeChange, etc.) and implements `TryInto<LayershellCustomActionWithId>`.
#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    // ── Navigation ───────────────────────────────────────────────────
    MoveUp,
    MoveDown,

    // ── Actions ──────────────────────────────────────────────────────
    /// Paste the selected entry to the clipboard and exit.
    Select,
    /// Delete the selected entry.
    Delete,
    /// Toggle pin on the selected entry.
    Pin,
    /// Quit the picker.
    Quit,

    // ── Search ───────────────────────────────────────────────────────
    /// Focus the search input.
    EnterSearch,
    /// The search text changed (from the text input widget).
    SearchChanged(String),
    /// User pressed Enter in the search input.
    SearchSubmit,
    /// Escape was pressed — exit search mode or quit.
    EscapePressed,

    // ── Entry selection by click ─────────────────────────────────────
    EntryClicked(usize),
}

// ─── Boot ────────────────────────────────────────────────────────────────

/// Initialize the application state.
///
/// Called once by the iced_layershell runtime before the first frame.
pub fn boot(config: Config, db: Database) -> impl Fn() -> (WarpStash, Task<Message>) {
    // Wrap in RefCell<Option<…>> so we can .take() once from an Fn closure.
    // The iced_layershell runtime only calls boot once in practice.
    let db_cell = RefCell::new(Some(db));

    move || {
        let db = db_cell
            .borrow_mut()
            .take()
            .expect("boot() must only be called once");

        let colors = Colors::from_config(&config.theme);
        let theme = theme::build_theme(&config.theme);

        // Load the initial list of entries.
        let entries = db.list_previews(100).unwrap_or_else(|e| {
            error!("Failed to load clipboard entries: {e}");
            Vec::new()
        });

        let state = WarpStash {
            entries,
            selected: 0,
            search_query: String::new(),
            search_mode: false,
            db,
            colors,
            theme,
            config: config.clone(),
        };

        (state, Task::none())
    }
}

// ─── Update ──────────────────────────────────────────────────────────────

/// Process a message and mutate application state.
pub fn update(state: &mut WarpStash, message: Message) -> Task<Message> {
    match message {
        // ── Navigation ───────────────────────────────────────────────
        Message::MoveUp => {
            if !state.entries.is_empty() && state.selected > 0 {
                state.selected -= 1;
            }
            Task::none()
        }
        Message::MoveDown => {
            if !state.entries.is_empty() && state.selected < state.entries.len() - 1 {
                state.selected += 1;
            }
            Task::none()
        }

        // ── Actions ──────────────────────────────────────────────────
        Message::Select | Message::SearchSubmit => {
            if let Some(entry) = state.entries.get(state.selected) {
                let entry_id = entry.id;
                match state.db.get_entry(entry_id) {
                    Ok(Some(full_entry)) => {
                        if let Err(e) = paste_to_clipboard(
                            full_entry.content_type,
                            &full_entry.mime_type,
                            full_entry.text_content.as_deref(),
                            full_entry.blob_content.as_deref(),
                        ) {
                            error!("Paste failed: {e}");
                            return Task::none();
                        }
                        debug!(id = entry_id, "Pasted entry to clipboard");
                        // Graceful exit.
                        return iced::exit();
                    }
                    Ok(None) => {
                        warn!(id = entry_id, "Entry not found in database");
                    }
                    Err(e) => {
                        error!(id = entry_id, "Failed to load entry: {e}");
                    }
                }
            }
            Task::none()
        }

        Message::Delete => {
            if let Some(entry) = state.entries.get(state.selected) {
                let entry_id = entry.id;
                match state.db.delete_entry(entry_id) {
                    Ok(_) => {
                        debug!(id = entry_id, "Deleted entry");
                        reload_entries(state);
                        // Adjust selection if it's past the end.
                        if state.selected >= state.entries.len() && state.selected > 0 {
                            state.selected -= 1;
                        }
                    }
                    Err(e) => error!(id = entry_id, "Delete failed: {e}"),
                }
            }
            Task::none()
        }

        Message::Pin => {
            if let Some(entry) = state.entries.get(state.selected) {
                let entry_id = entry.id;
                match state.db.toggle_pin(entry_id) {
                    Ok(_) => {
                        debug!(id = entry_id, "Toggled pin");
                        reload_entries(state);
                    }
                    Err(e) => error!(id = entry_id, "Pin toggle failed: {e}"),
                }
            }
            Task::none()
        }

        Message::Quit => iced::exit(),

        // ── Search ───────────────────────────────────────────────────
        Message::EnterSearch => {
            state.search_mode = true;
            focus(iced::widget::Id::new("search"))
        }

        Message::SearchChanged(query) => {
            state.search_query = query;
            state.selected = 0;
            if state.search_query.is_empty() {
                reload_entries(state);
            } else {
                match state.db.search_previews(&state.search_query, 100) {
                    Ok(results) => state.entries = results,
                    Err(e) => {
                        warn!("Search failed: {e}");
                        reload_entries(state);
                    }
                }
            }
            Task::none()
        }

        Message::EscapePressed => {
            if state.search_mode {
                // Exit search mode, clear query, reload full list.
                state.search_mode = false;
                state.search_query.clear();
                state.selected = 0;
                reload_entries(state);
                Task::none()
            } else {
                // Quit the picker.
                iced::exit()
            }
        }

        // ── Click ────────────────────────────────────────────────────
        Message::EntryClicked(index) => {
            state.selected = index;
            Task::none()
        }

        // ── Layershell actions (handled by the runtime) ──────────────
        _ => Task::none(),
    }
}

// ─── View ────────────────────────────────────────────────────────────────

/// Build the widget tree for the current state.
pub fn view(state: &WarpStash) -> Element<'_, Message> {
    let c = &state.colors;

    // ── Search bar ───────────────────────────────────────────────────
    let search_input = text_input("  Search clipboard…", &state.search_query)
        .id(iced::widget::Id::new("search"))
        .on_input(Message::SearchChanged)
        .on_submit(Message::SearchSubmit)
        .padding(10)
        .size(16);

    let search_bar = container(search_input).width(Length::Fill).padding([8, 12]);

    // ── Entry list ───────────────────────────────────────────────────
    let entry_list: Column<Message> =
        state
            .entries
            .iter()
            .enumerate()
            .fold(Column::new().spacing(2), |col, (i, entry)| {
                let is_selected = i == state.selected;
                col.push(entry_row(entry, i, is_selected, c))
            });

    let scrollable_list =
        scrollable(container(entry_list).width(Length::Fill).padding([0, 8])).height(Length::Fill);

    // ── Footer hints ─────────────────────────────────────────────────
    let hints = text("  j↓  k↑  ↵ paste  d delete  p pin  / search  Esc quit")
        .size(12)
        .color(c.text_dim);

    let footer = container(hints).width(Length::Fill).padding([6, 12]);

    // ── Assemble ─────────────────────────────────────────────────────
    let divider_top =
        container(text(""))
            .height(1)
            .width(Length::Fill)
            .style(move |_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(c.border)),
                ..Default::default()
            });
    let divider_bottom =
        container(text(""))
            .height(1)
            .width(Length::Fill)
            .style(move |_theme: &Theme| container::Style {
                background: Some(iced::Background::Color(c.border)),
                ..Default::default()
            });

    let content = column![
        search_bar,
        divider_top,
        scrollable_list,
        divider_bottom,
        footer,
    ];

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(c.background)),
            border: iced::Border {
                color: c.border,
                width: 2.0,
                radius: 12.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Render a single clipboard entry row.
fn entry_row<'a>(
    entry: &'a EntryPreview,
    _index: usize,
    is_selected: bool,
    c: &Colors,
) -> Element<'a, Message> {
    let indicator = if is_selected { "▸ " } else { "  " };

    // First line: indicator + preview text.
    let preview_text = entry.preview.lines().next().unwrap_or("(empty)");
    let first_line = row![
        text(indicator).size(14).color(c.primary),
        text(preview_text)
            .size(14)
            .color(if is_selected { c.text } else { c.text_dim }),
    ];

    // Second line: metadata.
    let type_badge = match entry.content_type {
        ContentType::Text => "txt",
        ContentType::Image => "img",
    };
    let size_str = format_size(entry.byte_size);
    let pin_str = if entry.pinned { " 📌" } else { "" };

    let meta_line = text(format!("  {type_badge}  {size_str}{pin_str}"))
        .size(11)
        .color(c.text_dim);

    let bg_color = if is_selected {
        c.selected
    } else {
        Color::TRANSPARENT
    };

    let entry_content = column![first_line, meta_line].spacing(2);

    container(entry_content)
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(bg_color)),
            border: iced::Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ─── Subscription ────────────────────────────────────────────────────────

/// Wire up keyboard event listening.
///
/// Uses `event::listen_with` to intercept all keyboard events with their
/// capture status, so we can distinguish between widget-consumed events
/// (e.g. text input) and free events (normal mode navigation).
pub fn subscription(_state: &WarpStash) -> Subscription<Message> {
    iced::event::listen_with(|ev, status, _id| {
        let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = ev else {
            return None;
        };

        // Escape is always intercepted, even when a widget has focus.
        if key == Key::Named(Named::Escape) {
            return Some(Message::EscapePressed);
        }

        // All other keys are only handled when no widget captured them.
        if status != event::Status::Ignored {
            return None;
        }

        match key.as_ref() {
            Key::Named(Named::Enter) => Some(Message::Select),
            Key::Named(Named::ArrowDown) | Key::Character("j") => Some(Message::MoveDown),
            Key::Named(Named::ArrowUp) | Key::Character("k") => Some(Message::MoveUp),
            Key::Character("d") => Some(Message::Delete),
            Key::Character("p") => Some(Message::Pin),
            Key::Character("/") => Some(Message::EnterSearch),
            _ => None,
        }
    })
}

// ─── Theme ───────────────────────────────────────────────────────────────

/// Returns the application theme for the layer-shell style callback.
pub fn get_theme(state: &WarpStash) -> Theme {
    state.theme.clone()
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Reload the entry list from the database (full or search-filtered).
fn reload_entries(state: &mut WarpStash) {
    state.entries = state.db.list_previews(100).unwrap_or_else(|e| {
        error!("Failed to reload entries: {e}");
        Vec::new()
    });
}

/// Human-readable byte size.
fn format_size(bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
