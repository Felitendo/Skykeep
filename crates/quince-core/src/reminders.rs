//! Reminder lists and reminders in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderList {
    pub guid: String,
    pub title: String,
    pub colour: Option<String>,
    pub ctag: Option<String>,
    pub sort_order: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub guid: String,
    pub list_guid: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<i64>,
    /// `None` while outstanding.
    pub completed_at: Option<i64>,
    pub priority: i64,
    pub etag: Option<String>,
    /// Apple's original JSON for this reminder, kept verbatim so a write can echo every
    /// field back rather than only the ones we model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl Reminder {
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.completed_at.is_some()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Upsert lists without removing anything.
    ///
    /// An incremental sync returns only what changed, so absence means "unchanged", not
    /// "deleted". Using the replacing variant there would wipe the whole library on the
    /// first incremental refresh.
    pub fn merge_reminder_lists(&mut self, lists: &[ReminderList]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminder_lists (guid, title, colour, ctag, sort_order, enabled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, colour = excluded.colour, ctag = excluded.ctag,
                   sort_order = excluded.sort_order, enabled = excluded.enabled,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for l in lists {
                stmt.execute(params![
                    l.guid,
                    l.title,
                    l.colour,
                    l.ctag,
                    l.sort_order,
                    i64::from(l.enabled),
                    stamp
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Remove reminders and lists the server reports as gone.
    pub fn forget_reminders(
        &mut self,
        reminder_guids: &[String],
        list_guids: &[String],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut drop_reminder = tx.prepare("DELETE FROM reminders WHERE guid = ?1")?;
            for guid in reminder_guids {
                drop_reminder.execute(params![guid])?;
            }
            // Removing a list takes its reminders with it, by foreign key.
            let mut drop_list = tx.prepare("DELETE FROM reminder_lists WHERE guid = ?1")?;
            for guid in list_guids {
                drop_list.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Where a service's last sync got to, for resuming.
    pub fn sync_token(&self, service: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT token FROM sync_state WHERE service = ?1",
                params![service],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten())
    }

    /// Replace the set of lists, dropping any Apple no longer reports.
    pub fn put_reminder_lists(&mut self, lists: &[ReminderList]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminder_lists (guid, title, colour, ctag, sort_order, enabled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, colour = excluded.colour, ctag = excluded.ctag,
                   sort_order = excluded.sort_order, enabled = excluded.enabled,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for l in lists {
                stmt.execute(params![
                    l.guid,
                    l.title,
                    l.colour,
                    l.ctag,
                    l.sort_order,
                    i64::from(l.enabled),
                    stamp
                ])?;
            }
            let keep: Vec<&str> = lists.iter().map(|l| l.guid.as_str()).collect();
            if keep.is_empty() {
                tx.execute("DELETE FROM reminder_lists", [])?;
            } else {
                let placeholders = vec!["?"; keep.len()].join(",");
                tx.execute(
                    &format!("DELETE FROM reminder_lists WHERE guid NOT IN ({placeholders})"),
                    rusqlite::params_from_iter(keep),
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn put_reminders(&mut self, reminders: &[Reminder]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminders (guid, list_guid, title, notes, due_at, completed_at,
                                        priority, etag, raw, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   notes = excluded.notes, due_at = excluded.due_at,
                   completed_at = excluded.completed_at, priority = excluded.priority,
                   etag = excluded.etag, raw = excluded.raw,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for r in reminders {
                stmt.execute(params![
                    r.guid,
                    r.list_guid,
                    r.title,
                    r.notes,
                    r.due_at,
                    r.completed_at,
                    r.priority,
                    r.etag,
                    r.raw,
                    stamp
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reminder_lists(&self) -> Result<Vec<ReminderList>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, colour, ctag, sort_order, enabled
             FROM reminder_lists ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReminderList {
                guid: row.get(0)?,
                title: row.get(1)?,
                colour: row.get(2)?,
                ctag: row.get(3)?,
                sort_order: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Reminders in a list, outstanding first, then by due date.
    ///
    /// Ordering matters: a list that buries today's task under last month's completed ones
    /// is useless.
    pub fn reminders_in(&self, list_guid: &str, include_done: bool) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, list_guid, title, notes, due_at, completed_at, priority, etag, raw
             FROM reminders
             WHERE list_guid = ?1 AND (?2 = 1 OR completed_at IS NULL)
             ORDER BY completed_at IS NOT NULL,
                      due_at IS NULL, due_at,
                      priority DESC,
                      title",
        )?;
        let rows = stmt.query_map(params![list_guid, i64::from(include_done)], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                etag: row.get(7)?,
                raw: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(guid: &str) -> ReminderList {
        ReminderList {
            guid: guid.into(),
            title: format!("List {guid}"),
            colour: Some("#cc73e1".into()),
            ctag: None,
            sort_order: 0,
            enabled: true,
        }
    }

    fn reminder(guid: &str, due: Option<i64>, done: Option<i64>) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "l1".into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done,
            priority: 0,
            etag: None,
            raw: None,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[list("l1")]).unwrap();
        lib
    }

    #[test]
    fn outstanding_reminders_come_before_completed_ones() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("done", Some(100), Some(150)),
            reminder("undated", None, None),
            reminder("due-soon", Some(200), None),
        ])
        .unwrap();

        let visible = lib.reminders_in("l1", false).unwrap();
        assert_eq!(
            visible.iter().map(|r| r.guid.as_str()).collect::<Vec<_>>(),
            vec!["due-soon", "undated"],
            "dated items first, and completed ones hidden"
        );

        let all = lib.reminders_in("l1", true).unwrap();
        assert_eq!(
            all.last().unwrap().guid,
            "done",
            "completed items sink to the bottom"
        );
    }

    /// The bug this guards: an incremental sync returns only what changed, so treating
    /// absence as deletion would wipe everything already stored.
    #[test]
    fn merging_does_not_remove_what_was_not_mentioned() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();

        // A later sync mentions a different list entirely.
        lib.merge_reminder_lists(&[list("l2")]).unwrap();

        assert_eq!(
            lib.reminder_lists().unwrap().len(),
            2,
            "the original list must survive"
        );
        assert_eq!(
            lib.reminders_in("l1", true).unwrap().len(),
            1,
            "and so must its reminders"
        );
    }

    #[test]
    fn forgetting_removes_only_what_was_named() {
        let mut lib = seeded();
        lib.put_reminder_lists(&[list("l1"), list("l2")]).unwrap();
        lib.put_reminders(&[reminder("r1", None, None), reminder("r2", None, None)])
            .unwrap();

        lib.forget_reminders(&["r1".to_string()], &[]).unwrap();
        let left = lib.reminders_in("l1", true).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].guid, "r2");

        lib.forget_reminders(&[], &["l1".to_string()]).unwrap();
        assert!(
            lib.reminders_in("l1", true).unwrap().is_empty(),
            "its reminders go too"
        );
        assert_eq!(lib.reminder_lists().unwrap().len(), 1);
    }

    #[test]
    fn a_sync_token_round_trips() {
        let lib = seeded();
        assert!(lib.sync_token("reminders").unwrap().is_none());
        lib.mark_synced("reminders", Some("token-abc")).unwrap();
        assert_eq!(
            lib.sync_token("reminders").unwrap().as_deref(),
            Some("token-abc")
        );
    }

    #[test]
    fn deleting_a_list_takes_its_reminders() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();
        lib.put_reminder_lists(&[]).unwrap();
        assert!(lib.reminders_in("l1", true).unwrap().is_empty());
    }

    #[test]
    fn ticking_a_reminder_updates_it_rather_than_duplicating() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();
        lib.put_reminders(&[reminder("r1", None, Some(999))])
            .unwrap();

        let all = lib.reminders_in("l1", true).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].is_done());
    }

    #[test]
    fn an_existing_library_migrates_to_the_new_schema() {
        let lib = Library::in_memory().unwrap();
        assert_eq!(
            lib.schema_version().unwrap() as usize,
            crate::schema::MIGRATIONS.len(),
            "every migration must apply"
        );
    }
}
