# WarpStash (Still 🛠️)

WarpStash is a Wayland-native clipboard history system for Linux, built in Rust.
It is split into two applications:

- `warpstashd`: a background daemon that watches clipboard offers and persists entries.
- `warpstash-ui`: a keyboard-first picker overlay to search, preview, and restore entries.

Clipboard content is stored in SQLite with Full-Text Search (FTS5), deduplicated by
content hash, and managed with configurable history limits.

## Why WarpStash

- Wayland-first architecture using `wlr-data-control` protocol bindings.
- Fast search via SQLite FTS5.
- Persistent history with optional image support.
- Keyboard-driven UI designed for quick recall and paste.
- Clean separation of shared core logic, daemon, and UI.

## Architecture

This workspace contains three crates:

- `crates/warpstash-common`: shared config, storage, and domain types.
- `crates/warpstashd`: clipboard watcher + ingest pipeline.
- `crates/warpstash-ui`: layer-shell picker and paste-back flow.

High-level data flow:

1. `warpstashd` listens for Wayland clipboard offers.
2. Best-supported MIME payload is selected and read.
3. Content is normalized and classified (text, image, binary).
4. Consecutive duplicate checks are applied (BLAKE3-based).
5. Entry is persisted into SQLite; FTS index is updated via triggers.
6. `warpstash-ui` queries recent or searched entries and allows restoration.

## Storage Model

WarpStash uses SQLite in WAL mode for robust concurrent access between daemon and UI.

- Main table: `entries`
	- id, timestamp, content_type, mime_type, textual preview, raw data blob,
		optional image dimensions, optional source app, and pin flag.
- Search index: `entries_fts` virtual table (FTS5).
- Trigger-based synchronization keeps FTS rows in sync on insert/update/delete.

Search results are ranked and recent entries can be listed independently.

## Features

- Clipboard capture for text, image, and generic binary payloads.
- Configurable max history with automatic pruning of oldest unpinned entries.
- Optional deduplication of consecutive identical clipboard content.
- Live query in UI backed by FTS search.
- Pin/unpin to protect entries from history pruning.
- Delete individual entries from the picker.
- Preview-friendly rendering for text and image metadata.

## Requirements

- Linux with Wayland session.
- A compositor and environment exposing data-control support used by
	`wl-clipboard-rs` / `wlr-data-control`.
- Rust toolchain (stable recommended).
- SQLite (system library) available for `rusqlite` with bundled features in this project.

## Build

From repository root:

```bash
cargo build --workspace
```

Run tests:

```bash
cargo test --workspace
```

## Quick Start

1. Start daemon:

```bash
cargo run -p warpstashd
```

2. In a separate terminal, launch UI picker:

```bash
cargo run -p warpstash-ui
```

3. Copy text or images in your Wayland session.

4. Use configured keybindings in the picker to search, select, and paste entries.

## Configuration

On first daemon run, WarpStash writes a default config if missing.

Default locations:

- Config: `${XDG_CONFIG_HOME:-~/.config}/warpstash/config.toml`
- Data: `${XDG_DATA_HOME:-~/.local/share}/warpstash/warpstash.db`

Project defaults live in `assets/default_config.toml` and include:

- History limits and dedup behavior.
- Supported MIME priority.
- UI dimensions, typography scale, and colors.
- Picker keybindings.

## Workspace Layout

```text
.
├── assets/default_config.toml
├── crates/
│   ├── warpstash-common/
│   │   └── src/{config.rs,db.rs,types.rs,lib.rs}
│   ├── warpstashd/
│   │   └── src/{main.rs,wayland.rs,dedup.rs}
│   └── warpstash-ui/
│       └── src/{main.rs,app.rs,clipboard.rs,theme.rs}
└── Cargo.toml
```

## Development

Useful commands:

```bash
# Format
cargo fmt --all

# Lints
cargo clippy --workspace --all-targets -- -D warnings

# Tests
cargo test --workspace
```

## Troubleshooting

- Clipboard events are not captured:
	- Confirm you are on Wayland (not X11).
	- Verify compositor/data-control compatibility.
	- Run daemon in foreground and inspect logs.
- UI opens but no results:
	- Check DB path in config and ensure daemon is running.
	- Confirm entries exist in the database after copying content.
- Build issues on Linux:
	- Ensure required system graphics/Wayland development libraries are installed.

## Status

WarpStash is actively evolving. The current codebase already provides a working
clipboard persistence and recall loop, with room for additional UX and platform
integration improvements.

## License

This project is licensed under the terms in `LICENSE`.