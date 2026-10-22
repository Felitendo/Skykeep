//! The local library: SQLite, and the only thing the UI ever reads.
//!
//! iCloud.com is slow because every scroll is a network round trip. Quince never renders
//! from the network — the sync engine reconciles into this database, and the interface reads
//! it. That is the whole reason the app can be instant and work offline.

pub mod alarms;
pub mod calendar;
pub mod contacts;
pub mod drive;
pub mod export;
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

/// Every table holding what an account brought: all but `app_state`, which is sorted key by key,
/// and the search index's, which are not rows to delete one by one (`forget_account` empties it).
fn account_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' AND name != 'app_state'
         AND name NOT LIKE 'search\\_%' ESCAPE '\\'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Run every migration past the library's version, each with its version bump as one
/// transaction: a migration that fails part way, or a crash in the middle of one, leaves the
/// library at the version before it with none of it applied — never half a schema change
/// that the next open would try to make again over itself, nor a change made whose version
/// was never written.
fn migrate_with(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (version, sql) in migrations.iter().enumerate() {
        let target = u32::try_from(version).unwrap_or(u32::MAX) + 1;
        if current < target {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            // `user_version` lives in the database header, written inside the transaction.
            tx.pragma_update(None, "user_version", i64::from(target))?;
            tx.commit()?;
        }
    }
    Ok(())
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

    /// A second, read-only handle on the library under `dir`, which must already be open (and
    /// migrated) through [`Library::open`]: WAL lets it read while the one that writes holds its
    /// lock through a sync pass, so a search never waits behind one. It sees what was
    /// committed, never half a pass.
    pub fn open_reader(dir: &Path) -> Result<Self> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(
            dir.join("library.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        conn.busy_timeout(std::time::Duration::from_millis(250))?;
        Ok(Self { conn })
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
        let mut library = Self { conn };
        library.migrate()?;
        Ok(library)
    }

    /// Bring the schema up to date. Idempotent, and safe to call on every open.
    fn migrate(&mut self) -> Result<()> {
        migrate_with(&mut self.conn, schema::MIGRATIONS)
    }

    /// Run `apply` as one change to the library: everything it stores lands, or — if it
    /// fails, or the process dies part way — none of it does, and the library is as it was.
    ///
    /// A sync pass stores a service's records, tombstones, order and position in many calls;
    /// each was its own transaction, so a failure or a crash between them left half a pass —
    /// rows pruned and their replacements never stored, or records stored under a position
    /// that said they were not. Nests: every write in this crate is a savepoint, so one call
    /// inside another is part of the outer one.
    pub fn atomically<T, E>(
        &mut self,
        apply: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        const NAME: &str = "quince_atomically";
        self.conn
            .execute_batch(&format!("SAVEPOINT {NAME}"))
            .map_err(|e| E::from(Error::from(e)))?;
        let undo = |conn: &Connection| {
            let _ = conn.execute_batch(&format!("ROLLBACK TO {NAME}; RELEASE {NAME}"));
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(&mut *self)));
        match outcome {
            Ok(Ok(value)) => match self.conn.execute_batch(&format!("RELEASE {NAME}")) {
                Ok(()) => Ok(value),
                Err(e) => {
                    undo(&self.conn);
                    Err(E::from(Error::from(e)))
                }
            },
            Ok(Err(e)) => {
                undo(&self.conn);
                Err(e)
            }
            // Left open, every later write would sit inside this savepoint and never commit.
            Err(panic) => {
                undo(&self.conn);
                std::panic::resume_unwind(panic)
            }
        }
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
        let tx = self.conn.savepoint()?;
        for table in account_tables(&tx)? {
            tx.execute(&format!("DELETE FROM \"{table}\""), [])?;
        }
        // The search index (migration 45) is not a table of records but words: its triggers
        // emptied it with the records, and this makes sure of it without reading it row by row.
        tx.execute_batch(
            "INSERT INTO search_text (search_text) VALUES ('delete-all'); DELETE FROM search_docs;",
        )?;
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

    /// A migration that fails part way leaves nothing of itself and the version where it was;
    /// the ones before it stay applied. Run again (fixed), it applies whole.
    #[test]
    fn a_failed_migration_leaves_the_library_at_the_version_before_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        let good = "CREATE TABLE a (x INTEGER);";
        // The first statement would succeed on its own; the second fails.
        let bad = "CREATE TABLE b (x INTEGER); INSERT INTO nowhere VALUES (1);";
        assert!(migrate_with(&mut conn, &[good, bad]).is_err());
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1, "the version bump went with the migration");
        let has = |conn: &Connection, t: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?1)",
                [t],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(has(&conn, "a"), "the migration before it stays");
        assert!(!has(&conn, "b"), "nothing of the failed one persists");
        let fixed = "CREATE TABLE b (x INTEGER);";
        migrate_with(&mut conn, &[good, fixed]).unwrap();
        assert!(has(&conn, "b"));
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    /// The same on disk, through a real file: what a crash mid-migration would find on reopen.
    #[test]
    fn a_failed_migration_is_not_on_disk_either() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            let _ = migrate_with(
                &mut conn,
                &[
                    "CREATE TABLE a (x INTEGER);",
                    "ALTER TABLE a ADD COLUMN y INTEGER; SELECT RAISE(ABORT, 'boom');",
                ],
            );
        }
        let conn = Connection::open(&path).unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let columns: i64 = conn
            .query_row("SELECT count(*) FROM pragma_table_info('a')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            columns, 1,
            "the column of the failed migration is not there"
        );
    }

    /// One change made of many core writes — each its own savepoint — is all or nothing.
    #[test]
    fn an_atomic_change_that_fails_leaves_nothing_behind() {
        let mut lib = Library::in_memory().unwrap();
        lib.set_state("kept", "before").unwrap();
        let failed: std::result::Result<(), Error> = lib.atomically(|l| {
            l.set_state("kept", "after")?;
            l.mark_synced("notes", Some("new-token"))?;
            l.forget_account()?;
            // A failure after all of that: none of it may persist.
            Err(Error::Sqlite(rusqlite::Error::InvalidQuery))
        });
        assert!(failed.is_err());
        assert_eq!(lib.state("kept").unwrap().as_deref(), Some("before"));
        assert_eq!(lib.sync_token("notes").unwrap(), None);
        assert!(lib.conn().is_autocommit(), "nothing left open");

        lib.atomically(|l| -> Result<()> {
            l.set_state("kept", "after")?;
            // Nested: part of the outer change.
            l.atomically(|l| l.set_state("inner", "1"))
        })
        .unwrap();
        assert_eq!(lib.state("kept").unwrap().as_deref(), Some("after"));
        assert_eq!(lib.state("inner").unwrap().as_deref(), Some("1"));

        // A nested change that fails takes only itself back when the outer one carries on.
        lib.atomically(|l| -> Result<()> {
            l.set_state("outer", "1")?;
            let _ = l.atomically(|l| -> Result<()> {
                l.set_state("inner", "2")?;
                Err(Error::Sqlite(rusqlite::Error::InvalidQuery))
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(lib.state("outer").unwrap().as_deref(), Some("1"));
        assert_eq!(lib.state("inner").unwrap().as_deref(), Some("1"));
    }

    /// A panic part way rolls back and leaves the connection usable, not stuck inside a
    /// savepoint that would swallow every later write.
    #[test]
    fn a_panic_in_an_atomic_change_rolls_it_back() {
        let mut lib = Library::in_memory().unwrap();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = lib.atomically(|l| {
                l.set_state("k", "v").unwrap();
                panic!("mid-apply");
            });
        }));
        assert!(caught.is_err());
        assert!(lib.conn().is_autocommit());
        assert_eq!(lib.state("k").unwrap(), None);
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
