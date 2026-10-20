//! Changes made on this computer that Apple has not yet taken. The library shows each at once;
//! the app sends them behind it, in the order they were made, and keeps them here — across a
//! restart, or while offline — until Apple has them.

use crate::{Library, Result};
use rusqlite::params;

/// One change waiting for Apple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    /// Which part of iCloud it is for: "reminders", …
    pub service: String,
    /// The record it changes.
    pub record: String,
    /// What it does, in the service's own words: "done", …
    pub kind: String,
    /// The change itself, as JSON.
    pub payload: String,
    pub tries: i64,
}

impl Library {
    /// Queue a change. One still waiting of the same kind on the same record is replaced: only
    /// the last word on a field needs to reach Apple (ticked, unticked, ticked is ticked).
    pub fn outbox_push(
        &mut self,
        service: &str,
        record: &str,
        kind: &str,
        payload: &str,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM outbox WHERE service = ?1 AND record = ?2 AND kind = ?3 AND tries = 0",
            params![service, record, kind],
        )?;
        tx.execute(
            "INSERT INTO outbox (service, record, kind, payload, made_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![service, record, kind, payload, jiff::Timestamp::now().as_second()],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// What is waiting for one service, oldest first.
    pub fn outbox_pending(&self, service: &str) -> Result<Vec<Pending>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, service, record, kind, payload, tries FROM outbox WHERE service = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![service], |r| {
            Ok(Pending {
                id: r.get(0)?,
                service: r.get(1)?,
                record: r.get(2)?,
                kind: r.get(3)?,
                payload: r.get(4)?,
                tries: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Apple has it (or has refused it for good): no longer waiting.
    pub fn outbox_done(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Not taken this time; tried again later. The reason is kept for the record.
    pub fn outbox_retry(&mut self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET tries = tries + 1, error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// The list a reminder is in, if it is in the library.
    pub fn reminder_list_of(&self, guid: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(
                "SELECT list_guid FROM reminders WHERE guid = ?1",
                params![guid],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A reminder ticked or unticked here, before Apple hears of it.
    pub fn set_reminder_completed(&mut self, guid: &str, at: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET completed_at = ?2 WHERE guid = ?1",
            params![guid, at],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_in_order_and_keeps_only_the_last_word() {
        let mut lib = Library::in_memory().unwrap();
        let a = lib.outbox_push("reminders", "r1", "done", "true").unwrap();
        lib.outbox_push("reminders", "r2", "done", "true").unwrap();
        // Unticked before it was sent: the tick is replaced, not sent and then undone.
        lib.outbox_push("reminders", "r1", "done", "false").unwrap();
        let p = lib.outbox_pending("reminders").unwrap();
        assert_eq!(
            p.iter()
                .map(|x| (x.record.as_str(), x.payload.as_str()))
                .collect::<Vec<_>>(),
            vec![("r2", "true"), ("r1", "false")]
        );
        assert!(p.iter().all(|x| x.id != a));
        // One already being tried is never replaced: it may be on its way.
        lib.outbox_retry(p[0].id, "offline").unwrap();
        lib.outbox_push("reminders", "r2", "done", "false").unwrap();
        assert_eq!(lib.outbox_pending("reminders").unwrap().len(), 3);
        for x in lib.outbox_pending("reminders").unwrap() {
            lib.outbox_done(x.id).unwrap();
        }
        assert!(lib.outbox_pending("reminders").unwrap().is_empty());
        assert!(lib.outbox_pending("calendar").unwrap().is_empty());
    }
}
