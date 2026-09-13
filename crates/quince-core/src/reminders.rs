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
    #[serde(default)]
    pub flagged: bool,
    /// Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. `None` if it does
    /// not repeat.
    #[serde(default)]
    pub recur_frequency: Option<i64>,
    #[serde(default)]
    pub recur_interval: Option<i64>,
    pub etag: Option<String>,
    /// Apple's original JSON for this reminder, kept verbatim so a write can echo every
    /// field back rather than only the ones we model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

/// Apple's built-in views over every list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmartList {
    Today,
    Scheduled,
    Flagged,
    All,
}

impl Reminder {
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.completed_at.is_some()
    }

    /// True when this repeats, so completing it advances rather than closes it.
    #[must_use]
    pub fn repeats(&self) -> bool {
        self.recur_frequency.is_some()
    }

    /// The next occurrence this reminder is due, given that today's was just ticked off.
    ///
    /// Apple's frequency is 0 daily, 1 weekly, 2 monthly, 3 yearly. Months and years are
    /// added as calendar units rather than fixed spans, so a monthly reminder on the 31st
    /// lands where a person expects rather than drifting.
    #[must_use]
    pub fn next_occurrence(&self) -> Option<i64> {
        self.next_occurrence_after(now())
    }

    /// The first occurrence strictly after `after`.
    ///
    /// It always advances at least one interval, then keeps advancing while the result is
    /// still in the past: tick a daily reminder you skipped for a week and it lands on
    /// tomorrow, not on the morning after the one you missed — which would leave it
    /// overdue and looking as though the tick did nothing.
    ///
    /// The arithmetic runs in UTC. Apple stores the due date as an absolute instant, so a
    /// daily repeat means exactly twenty-four hours later regardless of where the machine
    /// running this happens to be.
    #[must_use]
    pub fn next_occurrence_after(&self, after: i64) -> Option<i64> {
        let due = self.due_at?;
        let frequency = self.recur_frequency?;
        // An interval outside this range is not a schedule anyone keeps, and unbounded
        // values are how a span constructor is made to fail.
        let interval = self.recur_interval.unwrap_or(1).clamp(1, 1_000);

        let span = match frequency {
            0 => jiff::Span::new().days(interval),
            1 => jiff::Span::new().weeks(interval),
            2 => jiff::Span::new().months(interval),
            3 => jiff::Span::new().years(interval),
            _ => return None,
        };
        let mut zoned = jiff::Timestamp::from_second(due)
            .ok()?
            .to_zoned(jiff::tz::TimeZone::UTC);

        // The cap stops a daily reminder abandoned a decade ago from spinning; eleven
        // years of daily steps is far past any real backlog.
        for _ in 0..MAX_RECURRENCE_STEPS {
            zoned = zoned.checked_add(span).ok()?;
            if zoned.timestamp().as_second() > after {
                break;
            }
        }
        Some(zoned.timestamp().as_second())
    }
}

/// How far `next_occurrence_after` will walk forward before giving up.
const MAX_RECURRENCE_STEPS: usize = 4_000;

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

    /// Dated reminders falling inside `[from, to)`, for the calendar.
    ///
    /// Apple's Calendar shows reminders alongside events, which is most of what fills a
    /// month for someone who lives in Reminders rather than Calendar.
    pub fn reminders_due_between(&self, from: i64, to: i64) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at,
                    r.priority, r.flagged, r.recur_frequency, r.recur_interval, r.etag, r.raw
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.due_at IS NOT NULL
               AND r.due_at >= ?1 AND r.due_at < ?2
             ORDER BY r.due_at",
        )?;
        let rows = stmt.query_map(params![from, to], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                recur_frequency: row.get(8)?,
                recur_interval: row.get(9)?,
                etag: row.get(10)?,
                raw: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One of Apple's smart lists.
    ///
    /// These are the entry points people actually use: what is due today, everything with a
    /// date, and whatever has been flagged. They are queries rather than stored lists, which
    /// is what lets them stay correct as the clock moves.
    pub fn reminders_smart(&self, kind: SmartList, until: i64) -> Result<Vec<Reminder>> {
        // Only Today is bounded by the clock, so only Today binds a value — binding one
        // regardless is what SQLite rejects as a parameter-count mismatch.
        let (clause, args): (&str, Vec<i64>) = match kind {
            // Overdue counts as today: Apple shows it there rather than hiding it in the past.
            SmartList::Today => (
                "r.completed_at IS NULL AND r.due_at IS NOT NULL AND r.due_at < ?1",
                vec![until],
            ),
            SmartList::Scheduled => ("r.completed_at IS NULL AND r.due_at IS NOT NULL", vec![]),
            SmartList::Flagged => ("r.completed_at IS NULL AND r.flagged = 1", vec![]),
            SmartList::All => ("r.completed_at IS NULL", vec![]),
        };
        let sql = format!(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at,
                    r.priority, r.flagged, r.recur_frequency, r.recur_interval, r.etag, r.raw
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND {clause}
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                recur_frequency: row.get(8)?,
                recur_interval: row.get(9)?,
                etag: row.get(10)?,
                raw: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

    /// Keep only these reminders, dropping everything else.
    ///
    /// A full sync is the whole truth: Apple reported every reminder it has, so anything
    /// still here that is not in that set no longer exists. Tombstones cover the usual
    /// case, but a tombstone is only ever delivered once — if a sync consumed one before
    /// the code knew what to do with it, that record would otherwise stay on screen
    /// forever. This is the floor under that.
    pub fn retain_reminders(&mut self, keep: &[String]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let removed = {
            let mut known: std::collections::HashSet<&str> =
                keep.iter().map(String::as_str).collect();
            // A reminder with no list would fail its foreign key anyway; leave those alone.
            let mut stmt = tx.prepare("SELECT guid FROM reminders")?;
            let all: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut delete = tx.prepare("DELETE FROM reminders WHERE guid = ?1")?;
            let mut count = 0;
            for guid in &all {
                if !known.remove(guid.as_str()) {
                    delete.execute(params![guid])?;
                    count += 1;
                }
            }
            count
        };
        tx.commit()?;
        Ok(removed)
    }

    pub fn put_reminders(&mut self, reminders: &[Reminder]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminders (guid, list_guid, title, notes, due_at, completed_at,
                                        priority, flagged, recur_frequency, recur_interval,
                                        etag, raw, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   notes = excluded.notes, due_at = excluded.due_at,
                   completed_at = excluded.completed_at, priority = excluded.priority,
                   flagged = excluded.flagged,
                   recur_frequency = excluded.recur_frequency,
                   recur_interval = excluded.recur_interval,
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
                    i64::from(r.flagged),
                    r.recur_frequency,
                    r.recur_interval,
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
            "SELECT guid, list_guid, title, notes, due_at, completed_at, priority, flagged,
                    recur_frequency, recur_interval, etag, raw
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
                flagged: row.get::<_, i64>(7)? != 0,
                recur_frequency: row.get(8)?,
                recur_interval: row.get(9)?,
                etag: row.get(10)?,
                raw: row.get(11)?,
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
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
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
    fn dated_reminders_are_found_by_range() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("before", Some(50), None),
            reminder("inside", Some(150), None),
            reminder("also-inside", Some(199), Some(400)), // completed still shows on its day
            reminder("after", Some(250), None),
            reminder("undated", None, None),
        ])
        .unwrap();

        let found = lib.reminders_due_between(100, 200).unwrap();
        assert_eq!(
            found.iter().map(|r| r.guid.as_str()).collect::<Vec<_>>(),
            vec!["inside", "also-inside"],
            "only dated reminders inside the window, in due order"
        );
    }

    #[test]
    fn a_disabled_list_hides_its_reminders_from_the_calendar() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", Some(150), None)])
            .unwrap();
        assert_eq!(lib.reminders_due_between(0, 1000).unwrap().len(), 1);

        let mut hidden = list("l1");
        hidden.enabled = false;
        lib.put_reminder_lists(&[hidden]).unwrap();
        assert!(lib.reminders_due_between(0, 1000).unwrap().is_empty());
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

#[cfg(test)]
mod smart_tests {
    use super::*;

    const DAY_START: i64 = 1_757_000_000;
    const DAY_END: i64 = DAY_START + 86_400;

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "l1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
        }])
        .unwrap();
        lib
    }

    fn reminder(guid: &str, due: Option<i64>, flagged: bool, done: Option<i64>) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "l1".into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done,
            priority: 0,
            flagged,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
        }
    }

    /// Overdue belongs in Today: Apple shows it there rather than hiding it in the past.
    #[test]
    fn today_includes_overdue_and_excludes_the_future() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("overdue", Some(DAY_START - 200_000), false, None),
            reminder("today", Some(DAY_START + 3600), false, None),
            reminder("tomorrow", Some(DAY_END + 3600), false, None),
            reminder("undated", None, false, None),
            reminder(
                "done-today",
                Some(DAY_START + 10),
                false,
                Some(DAY_START + 20),
            ),
        ])
        .unwrap();

        let guids: Vec<String> = lib
            .reminders_smart(SmartList::Today, DAY_END)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect();
        assert_eq!(guids, vec!["overdue", "today"]);
    }

    #[test]
    fn scheduled_is_everything_dated_and_still_open() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("dated", Some(DAY_END + 99_999), false, None),
            reminder("undated", None, false, None),
        ])
        .unwrap();
        let found = lib.reminders_smart(SmartList::Scheduled, DAY_END).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].guid, "dated");
    }

    #[test]
    fn flagged_shows_only_what_is_flagged_and_open() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("flag", None, true, None),
            reminder("flag-done", None, true, Some(1)),
            reminder("plain", None, false, None),
        ])
        .unwrap();
        let found = lib.reminders_smart(SmartList::Flagged, DAY_END).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].guid, "flag");
    }

    /// Hiding a list must hide it from the smart lists too, or the counts disagree.
    #[test]
    fn a_disabled_list_is_excluded_everywhere() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("a", Some(DAY_START), true, None)])
            .unwrap();
        assert_eq!(
            lib.reminders_smart(SmartList::All, DAY_END).unwrap().len(),
            1
        );

        lib.put_reminder_lists(&[ReminderList {
            guid: "l1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: false,
        }])
        .unwrap();
        for kind in [
            SmartList::Today,
            SmartList::Scheduled,
            SmartList::Flagged,
            SmartList::All,
        ] {
            assert!(lib.reminders_smart(kind, DAY_END).unwrap().is_empty());
        }
    }

    #[test]
    fn flagging_round_trips_through_storage() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("a", None, true, None)])
            .unwrap();
        assert!(lib.reminders_in("l1", true).unwrap()[0].flagged);
    }
}
