//! Changes made on this computer that Apple has not yet taken. The library shows each at once;
//! the app sends them behind it, in the order they were made, and keeps them here — across a
//! restart, or while offline — until Apple has them.
//!
//! A change is recorded here in the same library as the records it changes, before anything
//! is sent: a crash at any point after the command returned finds it still waiting. One being
//! sent is counted as tried *before* it goes, so a change that may already be at Apple is never
//! rewritten in place — later edits queue behind it instead. One Apple turns down for good is
//! kept, marked refused, until the person sends it again or lets it go: never dropped unseen.

use crate::{Library, Result};
use rusqlite::{params, OptionalExtension};

/// One change waiting for Apple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub id: i64,
    /// Which part of iCloud it is for: "reminders", "notes", "contacts".
    pub service: String,
    /// The record it changes.
    pub record: String,
    /// What it does, in the service's own words: "done", "save", "create", …
    pub kind: String,
    /// The change itself, as JSON.
    pub payload: String,
    /// How many times it has been sent (or begun to be).
    pub tries: i64,
}

/// A change Apple turned down for good, kept with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub change: Pending,
    pub reason: String,
    pub made_at: i64,
    pub refused_at: i64,
}

const COLUMNS: &str = "id, service, record, kind, payload, tries";

fn pending(r: &rusqlite::Row<'_>) -> rusqlite::Result<Pending> {
    Ok(Pending {
        id: r.get(0)?,
        service: r.get(1)?,
        record: r.get(2)?,
        kind: r.get(3)?,
        payload: r.get(4)?,
        tries: r.get(5)?,
    })
}

impl Library {
    /// Queue a change. One still waiting of the same kind on the same record and never sent is
    /// replaced, and the new one goes to the back: only the last word on a field needs to reach
    /// Apple (ticked, unticked, ticked is ticked).
    pub fn outbox_push(
        &mut self,
        service: &str,
        record: &str,
        kind: &str,
        payload: &str,
    ) -> Result<i64> {
        let tx = self.conn.savepoint()?;
        tx.execute(
            "DELETE FROM outbox WHERE service = ?1 AND record = ?2 AND kind = ?3 AND tries = 0
               AND refused_at IS NULL",
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

    /// Queue a change at the back, whatever else waits.
    pub fn outbox_append(
        &mut self,
        service: &str,
        record: &str,
        kind: &str,
        payload: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO outbox (service, record, kind, payload, made_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![service, record, kind, payload, jiff::Timestamp::now().as_second()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Rewrite a waiting change where it stands in the queue — only while it has never been
    /// sent. Whether it was: `false` means it may be on its way, and the caller queues anew.
    pub fn outbox_amend(&mut self, id: i64, payload: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE outbox SET payload = ?2 WHERE id = ?1 AND tries = 0 AND refused_at IS NULL",
            params![id, payload],
        )?;
        Ok(n == 1)
    }

    /// What is waiting (not refused) for one record of one kind, oldest first.
    pub fn outbox_waiting(&self, service: &str, record: &str, kind: &str) -> Result<Vec<Pending>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox WHERE service = ?1 AND record = ?2 AND kind = ?3
               AND refused_at IS NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![service, record, kind], pending)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// What is waiting for one service, oldest first. Refused changes are not waiting.
    pub fn outbox_pending(&self, service: &str) -> Result<Vec<Pending>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox WHERE service = ?1 AND refused_at IS NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![service], pending)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Everything waiting, every service, oldest first.
    pub fn outbox_all_pending(&self) -> Result<Vec<Pending>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM outbox WHERE refused_at IS NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map([], pending)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The oldest change waiting for a service, counted as tried before it goes: from here on
    /// it may reach Apple, so it is never amended or replaced, only followed.
    pub fn outbox_take(&mut self, service: &str) -> Result<Option<Pending>> {
        let tx = self.conn.savepoint()?;
        let next = tx
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM outbox WHERE service = ?1 AND refused_at IS NULL
                     ORDER BY id LIMIT 1"
                ),
                params![service],
                pending,
            )
            .optional()?;
        let Some(mut next) = next else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE outbox SET tries = tries + 1 WHERE id = ?1",
            params![next.id],
        )?;
        tx.commit()?;
        next.tries += 1;
        Ok(Some(next))
    }

    /// Every change not yet at Apple — waiting or refused — whatever its service.
    pub fn outbox_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Apple has it: no longer waiting.
    pub fn outbox_done(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Not taken this time; tried again later. The reason is kept for the record.
    pub fn outbox_retry(&mut self, id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Turned down for good: kept, with why, and no longer sent.
    pub fn outbox_refuse(&mut self, id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET error = ?2, refused_at = ?3 WHERE id = ?1",
            params![id, reason, jiff::Timestamp::now().as_second()],
        )?;
        Ok(())
    }

    /// Every refused change, oldest first.
    pub fn outbox_refused(&self) -> Result<Vec<Refused>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS}, coalesce(error, ''), made_at, refused_at FROM outbox
             WHERE refused_at IS NOT NULL ORDER BY id"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok(Refused {
                change: pending(r)?,
                reason: r.get(6)?,
                made_at: r.get(7)?,
                refused_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One refused change, by id.
    pub fn outbox_refused_one(&self, id: i64) -> Result<Option<Refused>> {
        Ok(self
            .outbox_refused()?
            .into_iter()
            .find(|r| r.change.id == id))
    }

    /// A refused change sent again, at the back of the queue, as if just made.
    pub fn outbox_resend(&mut self, id: i64) -> Result<bool> {
        let tx = self.conn.savepoint()?;
        let row = tx
            .query_row(
                "SELECT service, record, kind, payload FROM outbox
                 WHERE id = ?1 AND refused_at IS NOT NULL",
                params![id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((service, record, kind, payload)) = row else {
            return Ok(false);
        };
        tx.execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        tx.execute(
            "INSERT INTO outbox (service, record, kind, payload, made_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![service, record, kind, payload, jiff::Timestamp::now().as_second()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// A refused change let go of, by the person's choice.
    pub fn outbox_discard(&mut self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM outbox WHERE id = ?1 AND refused_at IS NOT NULL",
            params![id],
        )?;
        Ok(())
    }

    /// A waiting change taken back before any of it was sent: gone from the queue. False when
    /// it may already be on its way (or is not waiting), and so cannot be taken back.
    pub fn outbox_withdraw(&mut self, id: i64) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM outbox WHERE id = ?1 AND tries = 0 AND refused_at IS NULL",
            params![id],
        )?;
        Ok(n == 1)
    }

    /// A change taken to send that reached nothing at Apple — it failed before its first
    /// write — counted as never tried, so it can still be taken back.
    pub fn outbox_untry(&mut self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE outbox SET tries = 0 WHERE id = ?1 AND refused_at IS NULL",
            params![id],
        )?;
        Ok(())
    }

    /// The list a reminder is in, if it is in the library.
    pub fn reminder_list_of(&self, guid: &str) -> Result<Option<String>> {
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
        // One already taken to be sent is never replaced: it may be on its way.
        let taken = lib.outbox_take("reminders").unwrap().unwrap();
        assert_eq!((taken.record.as_str(), taken.tries), ("r2", 1));
        lib.outbox_retry(taken.id, "offline").unwrap();
        lib.outbox_push("reminders", "r2", "done", "false").unwrap();
        assert_eq!(lib.outbox_pending("reminders").unwrap().len(), 3);
        for x in lib.outbox_pending("reminders").unwrap() {
            lib.outbox_done(x.id).unwrap();
        }
        assert!(lib.outbox_pending("reminders").unwrap().is_empty());
        assert!(lib.outbox_pending("calendar").unwrap().is_empty());
    }

    /// An amendment rewrites a change where it stands — only while it was never sent.
    #[test]
    fn a_change_is_amended_in_place_until_it_is_taken() {
        let mut lib = Library::in_memory().unwrap();
        let edit = lib.outbox_append("reminders", "r1", "edit", "{a}").unwrap();
        lib.outbox_append("reminders", "r1", "done", "true")
            .unwrap();
        assert!(lib.outbox_amend(edit, "{a,b}").unwrap());
        let p = lib.outbox_pending("reminders").unwrap();
        assert_eq!(
            p.iter().map(|x| x.payload.as_str()).collect::<Vec<_>>(),
            vec!["{a,b}", "true"],
            "still first, as made"
        );
        lib.outbox_take("reminders").unwrap();
        assert!(
            !lib.outbox_amend(edit, "{a,b,c}").unwrap(),
            "taken: may be at Apple"
        );
        assert_eq!(
            lib.outbox_waiting("reminders", "r1", "edit").unwrap()[0].payload,
            "{a,b}"
        );
    }

    /// Recorded, then the app dies before anything is sent: reopened, the change is there,
    /// in order, and goes once.
    #[test]
    fn a_change_survives_a_crash_between_record_and_send() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut lib = Library::open(dir.path()).unwrap();
            lib.outbox_append("notes", "N1", "save", "one").unwrap();
            lib.outbox_append("notes", "N2", "save", "two").unwrap();
            // Taken and on its way when the process died: still there, counted as tried.
            lib.outbox_take("notes").unwrap();
        }
        let mut lib = Library::open(dir.path()).unwrap();
        let p = lib.outbox_pending("notes").unwrap();
        assert_eq!(
            p.iter()
                .map(|x| (x.record.as_str(), x.tries))
                .collect::<Vec<_>>(),
            vec![("N1", 1), ("N2", 0)]
        );
        // Amending the one that may have gone is refused; queued behind instead.
        assert!(!lib.outbox_amend(p[0].id, "one again").unwrap());
        let first = lib.outbox_take("notes").unwrap().unwrap();
        assert_eq!((first.record.as_str(), first.tries), ("N1", 2));
        lib.outbox_done(first.id).unwrap();
        let second = lib.outbox_take("notes").unwrap().unwrap();
        assert_eq!(second.record, "N2");
        lib.outbox_done(second.id).unwrap();
        assert!(
            lib.outbox_take("notes").unwrap().is_none(),
            "each sent once"
        );
    }

    /// A refused change is kept, with its reason and content, out of the queue; sent again it
    /// goes to the back; let go of, it is gone.
    #[test]
    fn a_refused_change_is_kept_until_sent_again_or_let_go() {
        let mut lib = Library::in_memory().unwrap();
        let a = lib
            .outbox_append("notes", "N1", "save", "my words")
            .unwrap();
        lib.outbox_append("notes", "N2", "save", "other").unwrap();
        lib.outbox_refuse(a, "changed in the same place").unwrap();
        assert_eq!(lib.outbox_pending("notes").unwrap().len(), 1);
        assert!(lib
            .outbox_take("notes")
            .unwrap()
            .is_some_and(|p| p.record == "N2"));
        let refused = lib.outbox_refused().unwrap();
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].change.payload, "my words");
        assert_eq!(refused[0].reason, "changed in the same place");
        assert_eq!(lib.outbox_count().unwrap(), 2, "counted as unsent");
        // Never replaced by a push of the same kind.
        lib.outbox_push("notes", "N1", "save", "newer").unwrap();
        assert_eq!(lib.outbox_refused().unwrap().len(), 1);
        assert!(lib.outbox_resend(a).unwrap());
        let p = lib.outbox_pending("notes").unwrap();
        assert_eq!(
            p.iter().map(|x| x.payload.as_str()).collect::<Vec<_>>(),
            vec!["other", "newer", "my words"]
        );
        let again = p[2].id;
        lib.outbox_refuse(again, "still no").unwrap();
        lib.outbox_discard(again).unwrap();
        assert!(lib.outbox_refused().unwrap().is_empty());
    }
}
