// ═══════════════════════════════════════════════════════════════════════════
// WarpStash — Database Layer
// ═══════════════════════════════════════════════════════════════════════════
//
// SQLite via rusqlite with WAL journal mode. The daemon writes entries,
// the UI reads them — WAL allows concurrent access without locking.
//
// Schema features:
//   • UNIQUE(content_hash) with ON CONFLICT REPLACE for deduplication
//   • FTS5 virtual table for instant text search
//   • Triggers to keep the FTS index in sync
//   • Automatic eviction of oldest unpinned entries when max_history is hit

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use tracing::{debug, info};

use crate::types::{ClipboardEntry, ContentType, EntryPreview, NewEntry};

// ─── Schema ──────────────────────────────────────────────────────────────

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;

CREATE TABLE IF NOT EXISTS entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    content_hash TEXT    NOT NULL,
    content_type TEXT    NOT NULL CHECK(content_type IN ('text', 'image')),
    mime_type    TEXT    NOT NULL,
    text_content TEXT,
    blob_content BLOB,
    preview      TEXT    NOT NULL,
    byte_size    INTEGER NOT NULL,
    pinned       INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

-- Unique on hash: inserting a duplicate hash replaces the old row,
-- effectively "bumping" it to a new created_at.
CREATE UNIQUE INDEX IF NOT EXISTS idx_content_hash ON entries(content_hash);

-- Fast ordering for the UI list.
CREATE INDEX IF NOT EXISTS idx_created ON entries(created_at DESC);

-- FTS5 for instant text search.
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    text_content,
    content='entries',
    content_rowid='id'
);

-- Keep FTS in sync on INSERT.
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries
WHEN new.content_type = 'text' BEGIN
    INSERT INTO entries_fts(rowid, text_content) VALUES (new.id, new.text_content);
END;

-- Keep FTS in sync on DELETE.
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, text_content)
        VALUES ('delete', old.id, COALESCE(old.text_content, ''));
END;
"#;

// ─── Database Handle ─────────────────────────────────────────────────────

/// Wrapper around a SQLite connection with WarpStash-specific operations.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) the database at the given path and apply schema.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        info!("Opening database at {}", path.display());

        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("Failed to open database: {}", path.display()))?;

        // Apply schema (idempotent via IF NOT EXISTS).
        conn.execute_batch(SCHEMA)
            .context("Failed to apply database schema")?;

        debug!("Database schema applied successfully");
        Ok(Self { conn })
    }

    /// Open a read-only connection (for the UI).
    pub fn open_readonly(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("Failed to open database read-only: {}", path.display()))?;

        // Set busy timeout for WAL readers.
        conn.execute_batch("PRAGMA busy_timeout = 5000;")
            .context("Failed to set PRAGMA busy_timeout")?;

        Ok(Self { conn })
    }

    // ─── Write Operations (Daemon) ───────────────────────────────────────

    /// Insert a new clipboard entry. If an entry with the same content_hash
    /// already exists, it is deleted first (to bump created_at), and a new
    /// row is inserted. Returns the new row ID.
    pub fn insert_entry(&self, entry: &NewEntry) -> Result<i64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Delete any existing entry with the same hash (dedup / bump).
        // The FTS trigger handles cleanup.
        self.conn.execute(
            "DELETE FROM entries WHERE content_hash = ?1",
            params![entry.content_hash],
        )?;

        self.conn.execute(
            r#"INSERT INTO entries
                   (content_hash, content_type, mime_type, text_content,
                    blob_content, preview, byte_size, pinned, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)"#,
            params![
                entry.content_hash,
                entry.content_type.as_str(),
                entry.mime_type,
                entry.text_content,
                entry.blob_content,
                entry.preview,
                entry.byte_size as i64,
                now,
            ],
        )?;

        let id = self.conn.last_insert_rowid();
        debug!(id, hash = %entry.content_hash, "Inserted clipboard entry");
        Ok(id)
    }

    /// Enforce the maximum history size by evicting the oldest unpinned
    /// entries. Call this after every insert.
    pub fn enforce_max_history(&self, max_history: usize) -> Result<usize> {
        let deleted = self.conn.execute(
            r#"DELETE FROM entries
               WHERE id NOT IN (
                   SELECT id FROM entries
                   ORDER BY pinned DESC, created_at DESC
                   LIMIT ?1
               )"#,
            params![max_history as i64],
        )?;

        if deleted > 0 {
            debug!(deleted, max_history, "Evicted old entries");
        }
        Ok(deleted)
    }

    /// Check if the most recent entry has the given content hash.
    /// Used for deduplication — skip storing if the user just pasted
    /// this entry back (or copied the same thing twice).
    pub fn most_recent_hash(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT content_hash FROM entries ORDER BY created_at DESC LIMIT 1")?;

        let hash = stmt.query_row([], |row| row.get::<_, String>(0)).ok();

        Ok(hash)
    }

    /// Delete a single entry by ID.
    pub fn delete_entry(&self, id: i64) -> Result<bool> {
        let deleted = self
            .conn
            .execute("DELETE FROM entries WHERE id = ?1", params![id])?;
        Ok(deleted > 0)
    }

    /// Toggle the pinned state of an entry.
    pub fn toggle_pin(&self, id: i64) -> Result<bool> {
        self.conn.execute(
            "UPDATE entries SET pinned = CASE WHEN pinned = 0 THEN 1 ELSE 0 END WHERE id = ?1",
            params![id],
        )?;

        // Return the new pinned state.
        let pinned: bool = self.conn.query_row(
            "SELECT pinned FROM entries WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(pinned)
    }

    // ─── Read Operations (UI) ────────────────────────────────────────────

    /// Fetch entry previews for the UI list, ordered by pinned DESC then
    /// created_at DESC. Does NOT load blob_content.
    pub fn list_previews(&self, limit: usize) -> Result<Vec<EntryPreview>> {
        let mut stmt = self.conn.prepare_cached(
            r#"SELECT id, content_type, mime_type, preview, byte_size, pinned, created_at
               FROM entries
               ORDER BY pinned DESC, created_at DESC
               LIMIT ?1"#,
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(EntryPreview {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?)
                    .unwrap_or(ContentType::Text),
                mime_type: row.get(2)?,
                preview: row.get(3)?,
                byte_size: row.get::<_, i64>(4)? as usize,
                pinned: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Full-text search over text entries. Returns previews matching the
    /// query, scored by relevance.
    pub fn search_previews(&self, query: &str, limit: usize) -> Result<Vec<EntryPreview>> {
        let mut stmt = self.conn.prepare_cached(
            r#"SELECT e.id, e.content_type, e.mime_type, e.preview,
                      e.byte_size, e.pinned, e.created_at
               FROM entries e
               JOIN entries_fts fts ON e.id = fts.rowid
               WHERE entries_fts MATCH ?1
               ORDER BY e.pinned DESC, rank
               LIMIT ?2"#,
        )?;

        let rows = stmt.query_map(params![query, limit as i64], |row| {
            Ok(EntryPreview {
                id: row.get(0)?,
                content_type: ContentType::from_str(&row.get::<_, String>(1)?)
                    .unwrap_or(ContentType::Text),
                mime_type: row.get(2)?,
                preview: row.get(3)?,
                byte_size: row.get::<_, i64>(4)? as usize,
                pinned: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Load a full entry by ID (including blob_content for images).
    /// Used when the user selects an entry to paste.
    pub fn get_entry(&self, id: i64) -> Result<Option<ClipboardEntry>> {
        let mut stmt = self.conn.prepare_cached(
            r#"SELECT id, content_hash, content_type, mime_type, text_content,
                      blob_content, preview, byte_size, pinned, created_at
               FROM entries
               WHERE id = ?1"#,
        )?;

        let entry = stmt
            .query_row(params![id], |row| {
                Ok(ClipboardEntry {
                    id: row.get(0)?,
                    content_hash: row.get(1)?,
                    content_type: ContentType::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or(ContentType::Text),
                    mime_type: row.get(3)?,
                    text_content: row.get(4)?,
                    blob_content: row.get(5)?,
                    preview: row.get(6)?,
                    byte_size: row.get::<_, i64>(7)? as usize,
                    pinned: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .ok();

        Ok(entry)
    }

    /// Get the total number of entries in the database.
    pub fn entry_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentType, NewEntry};

    fn test_db() -> Database {
        Database::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn insert_and_read_text() {
        let db = test_db();
        let entry = NewEntry {
            content_hash: "abc123".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("Hello, world!".into()),
            blob_content: None,
            preview: "Hello, world!".into(),
            byte_size: 13,
        };
        let id = db.insert_entry(&entry).unwrap();
        assert!(id > 0);

        let loaded = db.get_entry(id).unwrap().unwrap();
        assert_eq!(loaded.text_content.as_deref(), Some("Hello, world!"));
        assert_eq!(loaded.content_type, ContentType::Text);
    }

    #[test]
    fn dedup_bumps_entry() {
        let db = test_db();
        let entry = NewEntry {
            content_hash: "same_hash".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("first".into()),
            blob_content: None,
            preview: "first".into(),
            byte_size: 5,
        };
        db.insert_entry(&entry).unwrap();

        // Insert again with same hash but different text — should replace.
        let entry2 = NewEntry {
            content_hash: "same_hash".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("second".into()),
            blob_content: None,
            preview: "second".into(),
            byte_size: 6,
        };
        db.insert_entry(&entry2).unwrap();

        assert_eq!(db.entry_count().unwrap(), 1);
    }

    #[test]
    fn enforce_max_history() {
        let db = test_db();
        for i in 0..10 {
            let entry = NewEntry {
                content_hash: format!("hash_{i}"),
                content_type: ContentType::Text,
                mime_type: "text/plain".into(),
                text_content: Some(format!("entry {i}")),
                blob_content: None,
                preview: format!("entry {i}"),
                byte_size: 7,
            };
            db.insert_entry(&entry).unwrap();
        }
        assert_eq!(db.entry_count().unwrap(), 10);

        db.enforce_max_history(5).unwrap();
        assert_eq!(db.entry_count().unwrap(), 5);
    }

    #[test]
    fn most_recent_hash_works() {
        let db = test_db();
        assert_eq!(db.most_recent_hash().unwrap(), None);

        let entry = NewEntry {
            content_hash: "latest".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("test".into()),
            blob_content: None,
            preview: "test".into(),
            byte_size: 4,
        };
        db.insert_entry(&entry).unwrap();
        assert_eq!(db.most_recent_hash().unwrap(), Some("latest".into()));
    }

    #[test]
    fn pin_toggle() {
        let db = test_db();
        let entry = NewEntry {
            content_hash: "pin_test".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("pin me".into()),
            blob_content: None,
            preview: "pin me".into(),
            byte_size: 6,
        };
        let id = db.insert_entry(&entry).unwrap();

        assert!(db.toggle_pin(id).unwrap()); // now pinned
        assert!(!db.toggle_pin(id).unwrap()); // now unpinned
    }

    #[test]
    fn delete_entry_works() {
        let db = test_db();
        let entry = NewEntry {
            content_hash: "del_test".into(),
            content_type: ContentType::Text,
            mime_type: "text/plain".into(),
            text_content: Some("delete me".into()),
            blob_content: None,
            preview: "delete me".into(),
            byte_size: 9,
        };
        let id = db.insert_entry(&entry).unwrap();
        assert!(db.delete_entry(id).unwrap());
        assert_eq!(db.entry_count().unwrap(), 0);
    }

    #[test]
    fn fts_search_works() {
        let db = test_db();
        let entries = vec![
            ("h1", "Hello world"),
            ("h2", "Goodbye world"),
            ("h3", "Rust programming"),
        ];
        for (hash, text) in entries {
            let entry = NewEntry {
                content_hash: hash.into(),
                content_type: ContentType::Text,
                mime_type: "text/plain".into(),
                text_content: Some(text.into()),
                blob_content: None,
                preview: text.into(),
                byte_size: text.len(),
            };
            db.insert_entry(&entry).unwrap();
        }

        let results = db.search_previews("world", 10).unwrap();
        assert_eq!(results.len(), 2);

        let results = db.search_previews("Rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].preview, "Rust programming");
    }
}
