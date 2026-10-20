//! The local library: SQLite, and the only thing the UI ever reads.
//!
//! iCloud.com is slow because every scroll is a network round trip. Quince never renders
//! from the network — the sync engine reconciles into this database, and the interface reads
//! it. That is the whole reason the app can be instant and work offline.

pub mod alarms;
pub mod calendar;
pub mod contacts;
pub mod drive;
pub mod fold;
pub mod health;
pub mod notes;
pub mod outbox;
pub mod photos;
pub mod reminders;
pub mod schema;
pub mod search;

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

/// The `app_state` keys that are this computer's, not an account's.
const COMPUTER_KEYS: &str = "key LIKE 'settings.%' OR key IN \
     ('drive_folder_on', 'drive_folder_path', 'photos_folder_on', 'photos_folder_path')";

/// Every table holding what an account brought: all but `app_state`, which is sorted key by key.
fn account_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' AND name != 'app_state'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

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

impl Library {
    /// Remember something learned from the account, by name.
    pub fn set_state(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Everything learned from an account let go of — every reminder, note, event, contact,
    /// file and photo, every sync position — for another account to start clean. Kept: this
    /// computer's own choices (`settings.*`, which folders are on and where).
    ///
    /// Signed out and back in as someone else, the last account's data stayed and the new
    /// one's was laid over it, from the old account's sync positions.
    pub fn forget_account(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;
        for table in account_tables(&tx)? {
            tx.execute(&format!("DELETE FROM \"{table}\""), [])?;
        }
        tx.execute(
            &format!("DELETE FROM app_state WHERE NOT ({COMPUTER_KEYS})"),
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// This computer's own choices, which go with the computer, never with an account: carried
    /// over when one account's library is set aside for another's.
    pub fn computer_state(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT key, value FROM app_state WHERE {COMPUTER_KEYS}"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Whether anything of an account is here at all — a record, a sync position, a fact.
    pub fn holds_account(&self) -> Result<bool> {
        for table in account_tables(&self.conn)? {
            let any: bool = self.conn.query_row(
                &format!("SELECT EXISTS (SELECT 1 FROM \"{table}\")"),
                [],
                |r| r.get(0),
            )?;
            if any {
                return Ok(true);
            }
        }
        Ok(self.conn.query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM app_state WHERE NOT ({COMPUTER_KEYS}))"),
            [],
            |r| r.get(0),
        )?)
    }

    /// What was remembered under that name, if anything.
    pub fn state(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_state WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another account signing in finds nothing of the last one's, and this computer's own
    /// choices still where they were.
    #[test]
    fn an_account_is_forgotten_whole() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            "root",
            &[crate::drive::DriveItem {
                drivewsid: "FILE::x::1".into(),
                parent_id: Some("root".into()),
                name: "Plan".into(),
                extension: Some("pdf".into()),
                kind: "FILE".into(),
                size: Some(1),
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: None,
            }],
        )
        .unwrap();
        lib.set_state("contacts.me", "C1").unwrap();
        lib.set_state("settings.alerts.events", "1").unwrap();
        lib.set_state("drive_folder_on", "1").unwrap();
        lib.mark_synced("drive", Some("token")).unwrap();
        assert!(lib.holds_account().unwrap());
        assert_eq!(
            lib.computer_state().unwrap().len(),
            2,
            "the settings and the folder, nothing of the account's"
        );
        lib.forget_account().unwrap();
        assert!(!lib.holds_account().unwrap(), "nothing left to set aside");
        assert!(
            lib.drive_children("root").unwrap().is_empty(),
            "the files gone"
        );
        assert_eq!(
            lib.state("contacts.me").unwrap(),
            None,
            "the account's own facts gone"
        );
        assert_eq!(
            lib.sync_token("drive").unwrap(),
            None,
            "and where its sync had got to"
        );
        assert_eq!(
            lib.state("settings.alerts.events").unwrap().as_deref(),
            Some("1"),
            "this computer's settings kept"
        );
        assert_eq!(lib.state("drive_folder_on").unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn a_new_library_is_migrated_to_the_latest_schema() {
        let library = Library::in_memory().unwrap();
        assert_eq!(
            library.schema_version().unwrap() as usize,
            schema::MIGRATIONS.len()
        );
    }

    /// A library from before lists kept their look reads every list again once, or lists no
    /// one had touched would never get their colour and group. Only Reminders is reset.
    #[test]
    fn an_upgraded_library_reads_its_reminders_lists_whole() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = rusqlite::Connection::open(dir.path().join("library.db")).unwrap();
            for migration in &schema::MIGRATIONS[..21] {
                conn.execute_batch(migration).unwrap();
            }
            conn.pragma_update(None, "user_version", 21).unwrap();
            conn.execute_batch(
                "INSERT INTO sync_state (service, token) VALUES ('reminders', 'old'), ('notes', 'kept');",
            )
            .unwrap();
        }
        let library = Library::open(dir.path()).unwrap();
        assert_eq!(library.sync_token("reminders").unwrap(), None);
        // Notes is read from the start once too, by a later migration of its own: every note
        // has to have its formatting read, and only a full pass reaches the untouched ones.
        assert_eq!(library.sync_token("notes").unwrap(), None);
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
