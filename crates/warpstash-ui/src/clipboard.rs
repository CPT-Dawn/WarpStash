// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — Clipboard Paste-Back
// ═══════════════════════════════════════════════════════════════════════════
//
// When the user selects an entry from the picker, we "paste it back" to
// the Wayland clipboard using wl-clipboard-rs so it becomes the active
// selection ready to Ctrl+V.

use anyhow::{Context, Result};
use tracing::info;
use wl_clipboard_rs::copy::{MimeType, Options, Source};

use warpstash_common::types::ContentType;

/// Copy content back to the Wayland clipboard.
///
/// For text entries, sets the clipboard with `MimeType::Text`.
/// For image entries, sets it with the original MIME type.
pub fn paste_to_clipboard(
    content_type: ContentType,
    mime_type: &str,
    text_content: Option<&str>,
    blob_content: Option<&[u8]>,
) -> Result<()> {
    match content_type {
        ContentType::Text => {
            let text = text_content.context("Text entry missing text_content")?;
            info!(len = text.len(), "Pasting text to clipboard");

            Options::new()
                .copy(
                    Source::Bytes(text.as_bytes().into()),
                    MimeType::Text,
                )
                .context("wl-clipboard-rs: failed to copy text")?;
        }
        ContentType::Image => {
            let blob = blob_content.context("Image entry missing blob_content")?;
            info!(
                len = blob.len(),
                mime = mime_type,
                "Pasting image to clipboard"
            );

            Options::new()
                .copy(
                    Source::Bytes(blob.into()),
                    MimeType::Specific(mime_type.to_string()),
                )
                .context("wl-clipboard-rs: failed to copy image")?;
        }
    }

    Ok(())
}
