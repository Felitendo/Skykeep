//! Reminder lists and reminders in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A heading inside a reminders list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderSection {
    pub guid: String,
    pub list_guid: String,
    pub title: String,
    pub created_at: Option<i64>,
}

/// The most reminders any one view hands to the window at once.
///
/// The plan rules out unbounded payloads crossing into the webview, and "All" on an account
/// ten years deep is genuinely unbounded. A thousand is far more than fits on a screen and
/// still weighs only a couple of hundred kilobytes; the true totals are counted separately, so
/// a view that reaches this can say plainly that it has.
pub const MOST_AT_ONCE: usize = 1_000;

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
    /// The reminder this one is a subtask of, if any.
    #[serde(default)]
    pub parent_guid: Option<String>,
    /// Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. `None` if it does
    /// not repeat.
    #[serde(default)]
    pub recur_frequency: Option<i64>,
    #[serde(default)]
    pub recur_interval: Option<i64>,
    pub etag: Option<String>,
    /// Apple's original JSON for this reminder, kept verbatim so a write can echo every
    /// field back rather than only the ones we model.
    ///
    /// Never sent to the window. It averages five kilobytes a reminder on a real account,
    /// against about fourteen bytes of the title and notes anyone actually reads — so a list
    /// of seven hundred carried three and a half megabytes of it, and twenty thousand would
    /// have carried ninety. The interface has never looked at a byte of it: writes name a
    /// reminder by its guid and the record is read back from the library on this side.
    #[serde(default, skip_serializing)]
    pub raw: Option<String>,
    /// The named zone Apple keeps beside the due date. Nine in the morning is nine in the
    /// morning wherever the person is, and that is not something a timestamp can say.
    #[serde(default)]
    pub timezone: Option<String>,
    /// The section of its list this reminder sits in, if any. Set from the list's membership
    /// file, never from the reminder — Apple does not record it on the reminder at all.
    #[serde(default)]
    pub section_guid: Option<String>,
}

/// What every entry in the sidebar is holding.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderCounts {
    pub today: usize,
    pub scheduled: usize,
    pub flagged: usize,
    pub all: usize,
    /// Outstanding reminders per list, keyed by list guid.
    pub lists: Vec<(String, usize)>,
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
        // Stepped in the reminder's own zone, not in UTC. A day in UTC is always exactly
        // twenty-four hours, so on the two days a year a clock moves, stepping there carried a
        // nine o'clock reminder to ten and six months later back to nine. In a real zone a day
        // is whatever that day is, and the hour holds.
        let zone = self
            .timezone
            .as_deref()
            .and_then(|name| jiff::tz::TimeZone::get(name).ok())
            .unwrap_or_else(jiff::tz::TimeZone::system);
        let mut zoned = jiff::Timestamp::from_second(due).ok()?.to_zoned(zone);

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
    /// Store sections as Apple has them now, and drop the ones Apple says are gone.
    pub fn put_reminder_sections(
        &mut self,
        sections: &[ReminderSection],
        gone: &[String],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut put = tx.prepare(
                "INSERT INTO reminder_sections (guid, list_guid, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   created_at = excluded.created_at, updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for section in sections {
                put.execute(params![
                    section.guid,
                    section.list_guid,
                    section.title,
                    section.created_at,
                    stamp
                ])?;
            }
            let mut drop = tx.prepare("DELETE FROM reminder_sections WHERE guid = ?1")?;
            let mut loose =
                tx.prepare("UPDATE reminders SET section_guid = NULL WHERE section_guid = ?1")?;
            for guid in gone {
                drop.execute(params![guid])?;
                // A deleted section's reminders stay in the list; they just have no heading.
                loose.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Which section each reminder in a list sits in, as the list's membership file says.
    ///
    /// The file is the whole truth for that list, so a reminder it does not place is set
    /// outside every section rather than left where it was. A placement naming a section
    /// this library does not hold — one deleted since — is treated the same way, because
    /// a heading nobody can see is not somewhere to put anything.
    pub fn set_reminder_sections(
        &mut self,
        list_guid: &str,
        placed: &[(String, String)],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            tx.execute(
                "UPDATE reminders SET section_guid = NULL WHERE list_guid = ?1",
                params![list_guid],
            )?;
            let mut set = tx.prepare(
                "UPDATE reminders SET section_guid = ?2
                 WHERE guid = ?1 AND list_guid = ?3
                   AND EXISTS (SELECT 1 FROM reminder_sections WHERE guid = ?2)",
            )?;
            for (reminder, section) in placed {
                set.execute(params![reminder, section, list_guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// A list's sections, in the order they were made.
    pub fn reminder_sections(&self, list_guid: &str) -> Result<Vec<ReminderSection>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, list_guid, title, created_at FROM reminder_sections
             WHERE list_guid = ?1 ORDER BY created_at, title",
        )?;
        let rows = stmt.query_map(params![list_guid], |row| {
            Ok(ReminderSection {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Apple's whole record for one reminder, for the write path that echoes it back.
    ///
    /// Fetched on its own rather than carried on every listed reminder. It averages five
    /// kilobytes a row, so reading it for twenty thousand of them to answer a question about
    /// titles and dates cost about a hundred megabytes of nothing.
    pub fn reminder_raw(&self, guid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw FROM reminders WHERE guid = ?1")?;
        let mut rows = stmt.query(params![guid])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    pub fn reminders_due_between(&self, from: i64, to: i64) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at,
                    r.priority, r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval,
                    r.etag, r.timezone, r.section_guid
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.due_at IS NOT NULL
               AND r.due_at >= ?1 AND r.due_at < ?2
             ORDER BY r.due_at
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![from, to, MOST_AT_ONCE], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One of Apple's smart lists.
    ///
    /// These are the entry points people actually use: what is due today, everything with a
    /// date, and whatever has been flagged. They are queries rather than stored lists, which
    /// is what lets them stay correct as the clock moves.
    /// How many reminders each smart list and each list is holding.
    ///
    /// Apple puts a count on every entry in the sidebar, and it is the number that makes
    /// the sidebar worth looking at. Counting in SQL rather than by fetching each list and
    /// measuring it is the difference between one cheap query and a dozen full reads.
    pub fn reminder_counts(&self, until: i64) -> Result<ReminderCounts> {
        // The same conditions the smart lists themselves use, so a tile can never disagree
        // with what opening it shows.
        let one = |clause: &str, args: &[i64]| -> Result<usize> {
            let sql = format!(
                "SELECT COUNT(*) FROM reminders r
                 JOIN reminder_lists l ON l.guid = r.list_guid
                 WHERE l.enabled = 1 AND {clause}"
            );
            let count: i64 = self.conn.query_row(
                &sql,
                rusqlite::params_from_iter(args.iter().copied()),
                |row| row.get(0),
            )?;
            Ok(usize::try_from(count).unwrap_or(0))
        };

        let mut stmt = self.conn.prepare(
            "SELECT l.guid, COUNT(r.guid)
             FROM reminder_lists l
             LEFT JOIN reminders r ON r.list_guid = l.guid AND r.completed_at IS NULL
             GROUP BY l.guid",
        )?;
        let lists = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(guid, n)| (guid, usize::try_from(n).unwrap_or(0)))
            .collect();

        Ok(ReminderCounts {
            today: one(
                "r.completed_at IS NULL AND r.due_at IS NOT NULL AND r.due_at < ?1",
                &[until],
            )?,
            scheduled: one("r.completed_at IS NULL AND r.due_at IS NOT NULL", &[])?,
            flagged: one("r.completed_at IS NULL AND r.flagged = 1", &[])?,
            all: one("r.completed_at IS NULL", &[])?,
            lists,
        })
    }

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
                    r.priority, r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval,
                    r.etag, r.timezone, r.section_guid
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND {clause}
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title
             LIMIT {MOST_AT_ONCE}"
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
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
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
                                        priority, flagged, parent_guid, recur_frequency,
                                        recur_interval, etag, raw, timezone, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   notes = excluded.notes, due_at = excluded.due_at,
                   completed_at = excluded.completed_at, priority = excluded.priority,
                   flagged = excluded.flagged, parent_guid = excluded.parent_guid,
                   recur_frequency = excluded.recur_frequency,
                   recur_interval = excluded.recur_interval,
                   etag = excluded.etag, raw = excluded.raw,
                   timezone = excluded.timezone,
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
                    r.parent_guid,
                    r.recur_frequency,
                    r.recur_interval,
                    r.etag,
                    r.raw,
                    r.timezone,
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
    /// Reminders still to do whose title or notes hold every word of `query`, across every
    /// enabled list.
    ///
    /// Matched in Rust rather than SQL for the reason Notes search is: SQLite folds case for
    /// ASCII alone. Bounded like every other view, so a search that matches everything on a
    /// ten-year account does not carry all of it into the window.
    pub fn search_reminders(&self, query: &str) -> Result<Vec<Reminder>> {
        let words = crate::fold::words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at, r.priority,
                    r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval, r.etag,
                    r.timezone, r.section_guid
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.completed_at IS NULL
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
            })
        })?;
        let mut found = Vec::new();
        for row in rows {
            let r = row?;
            let haystack = format!("{}\n{}", r.title, r.notes.as_deref().unwrap_or(""));
            if crate::fold::holds_all(&crate::fold::fold(&haystack), &words) {
                found.push(r);
                if found.len() == MOST_AT_ONCE {
                    break;
                }
            }
        }
        Ok(found)
    }

    pub fn reminders_in(&self, list_guid: &str, include_done: bool) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, list_guid, title, notes, due_at, completed_at, priority, flagged,
                    parent_guid, recur_frequency, recur_interval, etag, timezone, section_guid
             FROM reminders
             WHERE list_guid = ?1 AND (?2 = 1 OR completed_at IS NULL)
             ORDER BY completed_at IS NOT NULL,
                      due_at IS NULL, due_at,
                      priority DESC,
                      title
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![list_guid, i64::from(include_done), MOST_AT_ONCE],
            |row| {
                Ok(Reminder {
                    guid: row.get(0)?,
                    list_guid: row.get(1)?,
                    title: row.get(2)?,
                    notes: row.get(3)?,
                    due_at: row.get(4)?,
                    completed_at: row.get(5)?,
                    priority: row.get(6)?,
                    flagged: row.get::<_, i64>(7)? != 0,
                    parent_guid: row.get(8)?,
                    recur_frequency: row.get(9)?,
                    recur_interval: row.get(10)?,
                    etag: row.get(11)?,
                    raw: None,
                    timezone: row.get(12)?,
                    section_guid: row.get(13)?,
                })
            },
        )?;
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
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
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
            parent_guid: None,
            flagged,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
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

#[cfg(test)]
mod count_tests {
    use super::*;

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[
            ReminderList {
                guid: "L1".into(),
                title: "Daily".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
            },
            ReminderList {
                guid: "L2".into(),
                title: "Off".into(),
                colour: None,
                ctag: None,
                sort_order: 1,
                enabled: false,
            },
        ])
        .unwrap();
        lib
    }

    fn reminder(guid: &str, list: &str, due: Option<i64>, done: bool, flagged: bool) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: list.into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done.then_some(1),
            priority: 0,
            parent_guid: None,
            flagged,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    /// A count on a tile that disagrees with what opening it shows is worse than no count,
    /// so both come from the same conditions. This holds them to it.
    #[test]
    fn every_count_matches_what_opening_it_shows() {
        let mut lib = library();
        let now = 1_000_000;
        lib.put_reminders(&[
            reminder("overdue", "L1", Some(now - 500), false, false),
            reminder("later", "L1", Some(now + 5_000), false, true),
            reminder("undated", "L1", None, false, false),
            reminder("done", "L1", Some(now - 100), true, false),
        ])
        .unwrap();

        let counts = lib.reminder_counts(now).unwrap();
        for (kind, count) in [
            (SmartList::Today, counts.today),
            (SmartList::Scheduled, counts.scheduled),
            (SmartList::Flagged, counts.flagged),
            (SmartList::All, counts.all),
        ] {
            assert_eq!(
                lib.reminders_smart(kind, now).unwrap().len(),
                count,
                "{kind:?} count must equal what the list shows"
            );
        }
        assert_eq!(counts.today, 1, "overdue counts as today");
        assert_eq!(counts.all, 3, "completed does not count");
    }

    /// A list nobody has switched on contributes to nothing.
    #[test]
    fn a_disabled_list_is_left_out_of_the_smart_counts() {
        let mut lib = library();
        lib.put_reminders(&[reminder("hidden", "L2", Some(10), false, true)])
            .unwrap();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        assert_eq!(counts.all, 0);
        assert_eq!(counts.flagged, 0);
    }

    /// Every list appears, including the empty ones — a sidebar that hid them would be
    /// missing rows that are really there.
    #[test]
    fn every_list_gets_a_count_even_at_zero() {
        let mut lib = library();
        lib.put_reminders(&[reminder("a", "L1", None, false, false)])
            .unwrap();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        let by_guid: std::collections::HashMap<_, _> = counts.lists.into_iter().collect();
        assert_eq!(by_guid.get("L1"), Some(&1));
        assert_eq!(by_guid.get("L2"), Some(&0), "an empty list still shows");
    }

    #[test]
    fn an_empty_library_counts_zero_rather_than_failing() {
        let lib = library();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        assert_eq!(
            (counts.today, counts.scheduled, counts.flagged, counts.all),
            (0, 0, 0, 0)
        );
    }
}

/// What a screenful of reminders weighs on its way to the window.
#[cfg(test)]
mod payload_tests {
    use super::*;

    /// Apple's record for one reminder runs to about five kilobytes. It is kept so a write can
    /// echo back every field this app does not model, and it is read on this side only — the
    /// interface names a reminder by its guid and has never looked at a byte of it.
    ///
    /// Sending it anyway made a list of seven hundred weigh three and a half megabytes.
    #[test]
    fn apples_own_record_never_leaves_this_side() {
        let apples_record = format!(r#"{{"fields":{{"padding":"{}"}}}}"#, "x".repeat(5_000));
        let reminder = Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/1".into(),
            title: "Buy milk".into(),
            notes: None,
            due_at: Some(1_789_000_000),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("e1".into()),
            raw: Some(apples_record),
            timezone: None,
            section_guid: None,
        };

        let sent = serde_json::to_string(&reminder).unwrap();
        assert!(
            !sent.contains("padding"),
            "Apple's record must not cross into the window"
        );
        assert!(
            sent.len() < 400,
            "a reminder should weigh a few hundred bytes, not a few thousand: {}",
            sent.len()
        );
        assert!(sent.contains("Buy milk"), "what a person reads still goes");
    }

    /// A thousand reminders is an ordinary list on a busy account, and it has to stay small
    /// enough to cross a message channel without anyone noticing.
    #[test]
    fn a_thousand_reminders_stay_under_a_megabyte() {
        let many: Vec<Reminder> = (0..1_000)
            .map(|i| Reminder {
                guid: format!("Reminder/{i}"),
                list_guid: "List/1".into(),
                title: format!("Something to do number {i}"),
                notes: Some("with a note on it".into()),
                due_at: Some(1_789_000_000 + i),
                completed_at: None,
                priority: 0,
                flagged: false,
                parent_guid: None,
                recur_frequency: None,
                recur_interval: None,
                etag: Some(format!("e{i}")),
                raw: Some("y".repeat(5_000)),
                timezone: None,
                section_guid: None,
            })
            .collect();
        let bytes = serde_json::to_vec(&many).unwrap().len();
        assert!(
            bytes < 1_048_576,
            "a thousand reminders came to {bytes} bytes"
        );
    }
}

#[cfg(test)]
mod raw_column_tests {
    use super::*;

    fn with_raw(guid: &str) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "List/1".into(),
            title: "Buy milk".into(),
            notes: None,
            due_at: Some(1_789_000_000),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("e1".into()),
            raw: Some("z".repeat(5_000)),
            timezone: None,
            section_guid: None,
        }
    }

    fn stored() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
        }])
        .unwrap();
        lib.put_reminders(&[with_raw("Reminder/1"), with_raw("Reminder/2")])
            .unwrap();
        lib
    }

    /// Apple's record averages five kilobytes a reminder. Reading it for every row to answer a
    /// question about titles and dates cost about a hundred megabytes on a large account, and
    /// nothing outside the write path has ever wanted it.
    #[test]
    fn a_listed_reminder_does_not_drag_apples_record_along() {
        let lib = stored();
        for listed in [
            lib.reminders_in("List/1", true).unwrap(),
            lib.reminders_due_between(0, i64::MAX).unwrap(),
            lib.reminders_smart(SmartList::All, i64::MAX).unwrap(),
        ] {
            assert!(!listed.is_empty(), "the fixture has reminders in it");
            for r in listed {
                assert_eq!(r.raw, None, "{} carried Apple's record", r.guid);
            }
        }
    }

    /// It is still kept, and still there for the write path that echoes it back.
    #[test]
    fn the_record_is_stored_and_can_be_asked_for() {
        let lib = stored();
        assert_eq!(
            lib.reminder_raw("Reminder/1").unwrap().map(|r| r.len()),
            Some(5_000)
        );
        assert_eq!(lib.reminder_raw("nothing-like-this").unwrap(), None);
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;

    fn many(n: usize) -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/1".into(),
            title: "Everything".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
        }])
        .unwrap();
        let rows: Vec<Reminder> = (0..n)
            .map(|i| Reminder {
                guid: format!("Reminder/{i}"),
                list_guid: "List/1".into(),
                title: format!("Task {i}"),
                notes: None,
                due_at: Some(1_789_000_000 + i as i64),
                completed_at: None,
                priority: 0,
                flagged: true,
                parent_guid: None,
                recur_frequency: None,
                recur_interval: None,
                etag: None,
                raw: None,
                timezone: None,
                section_guid: None,
            })
            .collect();
        lib.put_reminders(&rows).unwrap();
        lib
    }

    /// Ten years of Reminders is genuinely unbounded, and the whole of it crossing into the
    /// window is what the plan rules out. Every view that can grow without limit is capped.
    #[test]
    fn no_view_hands_over_more_than_it_should() {
        let lib = many(MOST_AT_ONCE + 500);
        assert_eq!(
            lib.reminders_in("List/1", true).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_due_between(0, i64::MAX).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_smart(SmartList::All, i64::MAX).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_smart(SmartList::Flagged, i64::MAX)
                .unwrap()
                .len(),
            MOST_AT_ONCE
        );
    }

    /// The totals are counted rather than measured off the capped list, so a view that reaches
    /// the cap can still say how many there really are.
    #[test]
    fn the_count_is_the_truth_even_when_the_list_is_not_all_of_it() {
        let lib = many(MOST_AT_ONCE + 500);
        let counts = lib.reminder_counts(i64::MAX).unwrap();
        assert_eq!(counts.all, MOST_AT_ONCE + 500);
        assert_eq!(counts.flagged, MOST_AT_ONCE + 500);
    }

    /// An ordinary account is nowhere near the cap and must be unaffected by it.
    #[test]
    fn an_ordinary_account_sees_everything() {
        let lib = many(40);
        assert_eq!(lib.reminders_in("List/1", true).unwrap().len(), 40);
        assert_eq!(lib.reminders_due_between(0, i64::MAX).unwrap().len(), 40);
    }
}

/// Repeating reminders where the clock moves.
#[cfg(test)]
mod recurrence_daylight_tests {
    use super::*;

    fn at(zone: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
        jiff::civil::DateTime::new(y, m, d, h, min, 0, 0)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::get(zone).unwrap())
            .unwrap()
            .timestamp()
            .as_second()
    }

    fn wall(zone: &str, seconds: i64) -> String {
        jiff::Timestamp::from_second(seconds)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::get(zone).unwrap())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    fn daily(zone: &str, due: i64) -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/1".into(),
            title: "Take the tablets".into(),
            notes: None,
            due_at: Some(due),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: Some(0),
            recur_interval: Some(1),
            etag: None,
            raw: None,
            timezone: Some(zone.into()),
            section_guid: None,
        }
    }

    /// Nine in the morning every day means nine in the morning on the day the clocks go
    /// forward too. Stepping a whole number of hours instead of a day moves it to ten, and
    /// back to nine six months later — twice a year, for everyone who does not live somewhere
    /// that keeps one offset all year. This account's zone is one of the few that does, which
    /// is why it was never seen.
    #[test]
    fn a_daily_reminder_keeps_its_hour_across_a_spring_forward() {
        let zone = "America/New_York";
        let before = at(zone, 2026, 3, 7, 9, 0);
        let next = daily(zone, before)
            .next_occurrence_after(before)
            .expect("a daily reminder has a next occurrence");
        assert_eq!(wall(zone, next), "2026-03-08 09:00");
    }

    /// And on the day they go back.
    #[test]
    fn a_daily_reminder_keeps_its_hour_across_a_fall_back() {
        let zone = "America/New_York";
        let before = at(zone, 2026, 10, 31, 9, 0);
        let next = daily(zone, before).next_occurrence_after(before).unwrap();
        assert_eq!(wall(zone, next), "2026-11-01 09:00");
    }

    /// Southern hemisphere, and a zone whose clock moves by half an hour rather than a whole
    /// one — the case that catches anything reasoning in whole hours.
    #[test]
    fn the_hour_holds_in_the_awkward_zones_too() {
        for (zone, y, m, d) in [
            ("Australia/Sydney", 2026, 10, 3),
            ("Australia/Lord_Howe", 2026, 10, 3),
            ("Europe/London", 2026, 3, 28),
            ("Pacific/Chatham", 2026, 9, 26),
        ] {
            let before = at(zone, y, m, d, 9, 0);
            let next = daily(zone, before).next_occurrence_after(before).unwrap();
            assert!(
                wall(zone, next).ends_with("09:00"),
                "{zone}: {} became {}",
                wall(zone, before),
                wall(zone, next)
            );
        }
    }

    /// A reminder that never said where it was must still advance, in this machine's own zone.
    #[test]
    fn a_reminder_with_no_zone_still_advances_a_whole_day() {
        let mut r = daily("UTC", 1_789_000_000);
        r.timezone = None;
        let next = r.next_occurrence_after(1_789_000_000).unwrap();
        assert!(next > 1_789_000_000);
        assert!((next - 1_789_000_000 - 86_400).abs() <= 3_600);
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;

    fn reminder(guid: &str, list: &str) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: list.into(),
            title: guid.into(),
            notes: None,
            due_at: None,
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/L".into(),
            title: "Life Goals".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
        }])
        .unwrap();
        lib.put_reminders(&[
            reminder("Reminder/A", "List/L"),
            reminder("Reminder/B", "List/L"),
            reminder("Reminder/C", "List/L"),
        ])
        .unwrap();
        lib.put_reminder_sections(
            &[ReminderSection {
                guid: "ListSection/S".into(),
                list_guid: "List/L".into(),
                title: "🌅".into(),
                created_at: Some(1),
            }],
            &[],
        )
        .unwrap();
        lib
    }

    fn section_of(lib: &Library, guid: &str) -> Option<String> {
        lib.reminders_in("List/L", true)
            .unwrap()
            .into_iter()
            .find(|r| r.guid == guid)
            .and_then(|r| r.section_guid)
    }

    /// The membership file is the whole truth for its list: placed reminders go in, and one
    /// it no longer places comes back out.
    #[test]
    fn the_membership_file_decides_and_is_the_whole_truth() {
        let mut lib = library();
        lib.set_reminder_sections(
            "List/L",
            &[
                ("Reminder/A".into(), "ListSection/S".into()),
                ("Reminder/B".into(), "ListSection/S".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/A").as_deref(),
            Some("ListSection/S")
        );
        assert_eq!(section_of(&lib, "Reminder/C"), None);

        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/B"),
            None,
            "moved out on the phone, out here"
        );
    }

    /// A sync rewrites reminders wholesale and knows nothing about sections; it must not
    /// knock them out of the ones they sit in.
    #[test]
    fn storing_reminders_again_keeps_their_sections() {
        let mut lib = library();
        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        lib.put_reminders(&[reminder("Reminder/A", "List/L")])
            .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/A").as_deref(),
            Some("ListSection/S")
        );
    }

    /// A placement naming a section deleted since is a heading nobody can see.
    #[test]
    fn a_placement_in_a_section_that_is_gone_places_nothing() {
        let mut lib = library();
        lib.set_reminder_sections(
            "List/L",
            &[("Reminder/A".into(), "ListSection/GONE".into())],
        )
        .unwrap();
        assert_eq!(section_of(&lib, "Reminder/A"), None);
    }

    /// Deleting a section leaves its reminders in the list, without a heading.
    #[test]
    fn deleting_a_section_keeps_its_reminders() {
        let mut lib = library();
        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        lib.put_reminder_sections(&[], &["ListSection/S".into()])
            .unwrap();
        assert!(lib.reminder_sections("List/L").unwrap().is_empty());
        assert_eq!(section_of(&lib, "Reminder/A"), None);
        assert_eq!(lib.reminders_in("List/L", true).unwrap().len(), 3);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn r(guid: &str, title: &str, notes: Option<&str>, done: bool) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "List/L".into(),
            title: title.into(),
            notes: notes.map(ToString::to_string),
            due_at: None,
            completed_at: done.then_some(1),
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/L".into(),
            title: "GYM".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
        }])
        .unwrap();
        lib.put_reminders(&[
            r("A", "Evening run", Some("Legs today"), false),
            r("B", "Water the plants", None, false),
            r("C", "Evening walk", None, true),
            r("D", "Café run", None, false),
        ])
        .unwrap();
        lib
    }

    fn found(q: &str) -> Vec<String> {
        library()
            .search_reminders(q)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect()
    }

    #[test]
    fn title_and_notes_are_both_searched() {
        assert_eq!(found("plants"), vec!["B"]);
        assert_eq!(found("legs"), vec!["A"]);
    }

    #[test]
    fn finished_reminders_are_not_found() {
        assert_eq!(found("evening"), vec!["A"]);
    }

    #[test]
    fn case_folds_beyond_ascii_and_every_word_counts() {
        assert_eq!(found("CAFÉ"), vec!["D"]);
        assert_eq!(found("cafe"), vec!["D"]);
        assert_eq!(found("run evening"), vec!["A"]);
        assert!(found("evening plants").is_empty());
        assert!(found("  ").is_empty());
    }
}
