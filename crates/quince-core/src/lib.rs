//! The local library: SQLite, and the only thing the UI ever reads.
//!
//! iCloud.com is slow because every scroll is a network round trip. Quince never renders
//! from the network — the sync engine reconciles into this database, and the interface reads
//! it. That is the whole reason the app can be instant and work offline.

pub mod calendar;
pub mod contacts;
pub mod drive;
pub mod health;
pub mod notes;
pub mod reminders;
pub mod schema;

use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the local library couldn't be opened: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A handle to the local library.
pub struct Library {
    conn: Connection,
}

impl Library {
    /// Open (creating if needed) the library under `dir`, applying any pending migrations.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Self::from_connection(Connection::open(dir.join("library.db"))?)
    }

    /// An in-memory library. Tests, and nothing else.
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // WAL lets the sync engine write while the interface reads, which is the normal
        // state of this app. NORMAL is the right durability trade for a cache that can
        // always be refetched from Apple.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let library = Self { conn };
        library.migrate()?;
        Ok(library)
    }

    /// Bring the schema up to date. Idempotent, and safe to call on every open.
    fn migrate(&self) -> Result<()> {
        let current: u32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (version, sql) in schema::MIGRATIONS.iter().enumerate() {
            let target = u32::try_from(version).unwrap_or(u32::MAX) + 1;
            if current < target {
                self.conn.execute_batch(sql)?;
                self.conn
                    .pragma_update(None, "user_version", i64::from(target))?;
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_library_is_migrated_to_the_latest_schema() {
        let library = Library::in_memory().unwrap();
        assert_eq!(
            library.schema_version().unwrap() as usize,
            schema::MIGRATIONS.len()
        );
    }

    #[test]
    fn opening_twice_does_not_re_run_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let first = Library::open(dir.path()).unwrap();
        let version = first.schema_version().unwrap();
        drop(first);

        // A second open must be a no-op, not an error and not a reset.
        let second = Library::open(dir.path()).unwrap();
        assert_eq!(second.schema_version().unwrap(), version);
    }

    #[test]
    fn deleting_a_calendar_takes_its_events_with_it() {
        let library = Library::in_memory().unwrap();
        let conn = library.conn();
        conn.execute(
            "INSERT INTO calendars (guid, title, updated_at) VALUES ('cal-1', 'Work', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (guid, calendar_guid, title, starts_at, ends_at, updated_at)
             VALUES ('ev-1', 'cal-1', 'Standup', 100, 200, 0)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM calendars WHERE guid = 'cal-1'", [])
            .unwrap();

        let orphans: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(orphans, 0, "events must not outlive their calendar");
    }

    /// Re-syncing the same event must update it, not duplicate it.
    #[test]
    fn upserting_an_event_replaces_it() {
        let library = Library::in_memory().unwrap();
        let conn = library.conn();
        conn.execute(
            "INSERT INTO calendars (guid, title, updated_at) VALUES ('cal-1', 'Work', 0)",
            [],
        )
        .unwrap();

        for title in ["Standup", "Standup (moved)"] {
            conn.execute(
                "INSERT INTO events (guid, calendar_guid, title, starts_at, ends_at, updated_at)
                 VALUES ('ev-1', 'cal-1', ?1, 100, 200, 0)
                 ON CONFLICT(guid) DO UPDATE SET title = excluded.title",
                [title],
            )
            .unwrap();
        }

        let (count, title): (i64, String) = conn
            .query_row("SELECT count(*), max(title) FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(count, 1, "a re-sync must not duplicate rows");
        assert_eq!(title, "Standup (moved)");
    }
}
