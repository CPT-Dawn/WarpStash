// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — Shared Types
// ═══════════════════════════════════════════════════════════════════════════
//
// Domain types shared between the daemon (writer) and the UI (reader).

use serde::{Deserialize, Serialize};

/// The type of content stored in a clipboard entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
    Image,
}

impl ContentType {
    /// Convert to the string stored in SQLite.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Text => "text",
            ContentType::Image => "image",
        }
    }

    /// Parse from the string stored in SQLite.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "text" => Some(ContentType::Text),
            "image" => Some(ContentType::Image),
            _ => None,
        }
    }
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A clipboard entry as stored in the database and displayed in the UI.
///
/// Text entries populate `text_content`; image entries populate `blob_content`.
/// Both always have `preview` — a short human-readable summary.
#[derive(Debug, Clone)]
pub struct ClipboardEntry {
    /// Auto-incremented row ID.
    pub id: i64,

    /// Blake3 hex digest of the content bytes. Used for deduplication.
    pub content_hash: String,

    /// Whether this is a text or image entry.
    pub content_type: ContentType,

    /// MIME type string, e.g. "text/plain", "image/png".
    pub mime_type: String,

    /// Full text content (only for text entries).
    pub text_content: Option<String>,

    /// Raw image bytes (only for image entries).
    pub blob_content: Option<Vec<u8>>,

    /// Short preview string:
    /// - Text: first N lines / 200 chars
    /// - Image: "[image 800×600 PNG]"
    pub preview: String,

    /// Content size in bytes.
    pub byte_size: usize,

    /// Whether the entry is pinned (immune to eviction).
    pub pinned: bool,

    /// Unix epoch milliseconds when the entry was first captured.
    pub created_at: i64,
}

/// A lightweight version of `ClipboardEntry` used for UI list rendering.
/// Does not carry the full `blob_content` to avoid loading megabytes of
/// images into memory just for the list view.
#[derive(Debug, Clone)]
pub struct EntryPreview {
    pub id: i64,
    pub content_type: ContentType,
    pub mime_type: String,
    pub preview: String,
    pub byte_size: usize,
    pub pinned: bool,
    pub created_at: i64,
}

/// Data needed to insert a new clipboard capture into the database.
/// Produced by the daemon's Wayland watcher, consumed by db::insert_entry.
#[derive(Debug, Clone)]
pub struct NewEntry {
    /// Blake3 hex digest.
    pub content_hash: String,

    /// Text or image.
    pub content_type: ContentType,

    /// MIME type string.
    pub mime_type: String,

    /// Full text content (text entries only).
    pub text_content: Option<String>,

    /// Raw bytes (image entries only).
    pub blob_content: Option<Vec<u8>>,

    /// Human-readable preview.
    pub preview: String,

    /// Size of the content in bytes.
    pub byte_size: usize,
}

/// Generate a preview string from raw clipboard content.
///
/// For text: first `max_lines` lines, truncated to 200 characters.
/// For images: a descriptive string like "[image 1920×1080 PNG]".
pub fn make_preview(
    content_type: ContentType,
    data: &[u8],
    mime_type: &str,
    max_lines: usize,
) -> String {
    match content_type {
        ContentType::Text => {
            let text = String::from_utf8_lossy(data);
            let preview: String = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");

            if preview.len() > 200 {
                format!("{}…", &preview[..200])
            } else {
                preview
            }
        }
        ContentType::Image => {
            // Try to read image dimensions.
            match image::guess_format(data) {
                Ok(format) => {
                    let format_name = format!("{:?}", format);
                    match image::load_from_memory(data) {
                        Ok(img) => {
                            format!("[image {}×{} {}]", img.width(), img.height(), format_name)
                        }
                        Err(_) => format!("[image {} {}B]", format_name, data.len()),
                    }
                }
                Err(_) => {
                    // Fall back to MIME type.
                    let ext = mime_type.rsplit('/').next().unwrap_or("unknown");
                    format!("[image {} {}B]", ext, data.len())
                }
            }
        }
    }
}
