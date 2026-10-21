//! Calendars and events in the local library.
//!
//! Writes are upserts inside a transaction: a sync either lands whole or not at all, so an
//! interrupted refresh can never leave the interface showing half a calendar.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub guid: String,
    pub title: String,
    pub colour: Option<String>,
    /// Apple's collection tag. Unchanged means nothing inside moved.
    pub ctag: Option<String>,
    pub sort_order: i64,
    pub enabled: bool,
    /// False for a subscribed feed or a calendar shared for reading only. Nothing may be
    /// written to one, so nothing should offer to.
    #[serde(default = "yes")]
    pub writable: bool,
    /// The one Apple puts a new event in. Named outright rather than guessed at.
    #[serde(default)]
    pub is_default: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub guid: String,
    pub calendar_guid: String,
    pub title: String,
    pub location: Option<String>,
    /// Unix seconds, UTC.
    pub starts_at: i64,
    pub ends_at: i64,
    pub all_day: bool,
    pub timezone: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub etag: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Replace the set of calendars, dropping any Apple no longer reports.
    ///
    /// Removal cascades to their events, which is what should happen when a calendar is
    /// deleted on another device.
    pub fn put_calendars(&mut self, calendars: &[Calendar]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO calendars (guid, title, colour, ctag, sort_order, enabled,
                                        writable, is_default, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title,
                   colour = excluded.colour,
                   ctag = excluded.ctag,
                   sort_order = excluded.sort_order,
                   enabled = excluded.enabled,
                   writable = excluded.writable,
                   is_default = excluded.is_default,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for c in calendars {
                stmt.execute(params![
                    c.guid,
                    c.title,
                    c.colour,
                    c.ctag,
                    c.sort_order,
                    i64::from(c.enabled),
                    i64::from(c.writable),
                    i64::from(c.is_default),
                    stamp
                ])?;
            }

            // Anything Apple stopped reporting was deleted elsewhere.
            let keep: Vec<&str> = calendars.iter().map(|c| c.guid.as_str()).collect();
            if keep.is_empty() {
                tx.execute("DELETE FROM calendars", [])?;
            } else {
                let placeholders = vec!["?"; keep.len()].join(",");
                tx.execute(
                    &format!("DELETE FROM calendars WHERE guid NOT IN ({placeholders})"),
                    rusqlite::params_from_iter(keep),
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Upsert events. Existing rows for the same guid are updated in place.
    pub fn put_events(&mut self, events: &[Event]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO events (guid, calendar_guid, title, location, starts_at, ends_at,
                                     all_day, timezone, url, notes, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(guid) DO UPDATE SET
                   calendar_guid = excluded.calendar_guid,
                   title = excluded.title,
                   location = excluded.location,
                   starts_at = excluded.starts_at,
                   ends_at = excluded.ends_at,
                   all_day = excluded.all_day,
                   timezone = excluded.timezone,
                   url = excluded.url,
                   notes = excluded.notes,
                   etag = excluded.etag,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for e in events {
                stmt.execute(params![
                    e.guid,
                    e.calendar_guid,
                    e.title,
                    e.location,
                    e.starts_at,
                    e.ends_at,
                    i64::from(e.all_day),
                    e.timezone,
                    e.url,
                    e.notes,
                    e.etag,
                    stamp
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, colour, ctag, sort_order, enabled, writable, is_default
             FROM calendars ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Calendar {
                guid: row.get(0)?,
                title: row.get(1)?,
                colour: row.get(2)?,
                ctag: row.get(3)?,
                sort_order: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                writable: row.get::<_, i64>(6)? != 0,
                is_default: row.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Events overlapping `[from, to)`, from enabled calendars only.
    ///
    /// Overlap rather than containment: a meeting that started before the window but is
    /// still running belongs in today's view.
    pub fn events_between(&self, from: i64, to: i64) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.guid, e.calendar_guid, e.title, e.location, e.starts_at, e.ends_at,
                    e.all_day, e.timezone, e.url, e.notes, e.etag
             FROM events e
             JOIN calendars c ON c.guid = e.calendar_guid
             WHERE c.enabled = 1 AND e.starts_at < ?2 AND e.ends_at > ?1
             ORDER BY e.starts_at, e.title",
        )?;
        let rows = stmt.query_map(params![from, to], |row| {
            Ok(Event {
                guid: row.get(0)?,
                calendar_guid: row.get(1)?,
                title: row.get(2)?,
                location: row.get(3)?,
                starts_at: row.get(4)?,
                ends_at: row.get(5)?,
                all_day: row.get::<_, i64>(6)? != 0,
                timezone: row.get(7)?,
                url: row.get(8)?,
                notes: row.get(9)?,
                etag: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Events whose title, place or notes hold every word of `query`, accent- and
    /// case-insensitively, soonest first — what Calendar's search field finds.
    pub fn search_events(&self, query: &str) -> Result<Vec<Event>> {
        let words = crate::fold::words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare("SELECT guid FROM events")?;
        let guids: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut found: Vec<Event> = guids
            .iter()
            .filter_map(|g| self.event(g).ok().flatten())
            .filter(|e| {
                let text = [
                    Some(e.title.as_str()),
                    e.location.as_deref(),
                    e.notes.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
                crate::fold::holds_all(&crate::fold::fold(&text), &words)
            })
            .collect();
        found.sort_by_key(|e| e.starts_at);
        found.truncate(200);
        Ok(found)
    }

    /// One event by its guid, if the library still holds it.
    pub fn event(&self, guid: &str) -> Result<Option<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, calendar_guid, title, location, starts_at, ends_at, all_day,
                    timezone, url, notes, etag
             FROM events WHERE guid = ?1",
        )?;
        let mut rows = stmt.query_map(params![guid], |row| {
            Ok(Event {
                guid: row.get(0)?,
                calendar_guid: row.get(1)?,
                title: row.get(2)?,
                location: row.get(3)?,
                starts_at: row.get(4)?,
                ends_at: row.get(5)?,
                all_day: row.get::<_, i64>(6)? != 0,
                timezone: row.get(7)?,
                url: row.get(8)?,
                notes: row.get(9)?,
                etag: row.get(10)?,
            })
        })?;
        rows.next().transpose().map_err(Into::into)
    }

    /// Every stored event starting in `[from, to)`, as (guid, calendar), whatever its
    /// calendar's switch — what a pass over that window has to account for.
    pub fn events_starting_between(&self, from: i64, to: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, calendar_guid FROM events WHERE starts_at >= ?1 AND starts_at < ?2",
        )?;
        let rows = stmt.query_map(params![from, to], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Drop events Apple no longer has.
    pub fn forget_events(&mut self, guids: &[String]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare("DELETE FROM events WHERE guid = ?1")?;
            for guid in guids {
                stmt.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Record that a service synced successfully.
    pub fn mark_synced(&self, service: &str, token: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (service, token, last_synced_at, last_error)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(service) DO UPDATE SET
               token = excluded.token,
               last_synced_at = excluded.last_synced_at,
               last_error = NULL",
            params![service, token, now()],
        )?;
        Ok(())
    }

    /// When a service last synced, if it ever has.
    pub fn last_synced(&self, service: &str) -> Result<Option<i64>> {
        let found = self
            .conn
            .query_row(
                "SELECT last_synced_at FROM sync_state WHERE service = ?1",
                params![service],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(guid: &str, title: &str) -> Calendar {
        Calendar {
            guid: guid.into(),
            title: title.into(),
            colour: Some("#C8912A".into()),
            ctag: Some("ctag-1".into()),
            sort_order: 0,
            enabled: true,
            writable: true,
            is_default: false,
        }
    }

    fn event(guid: &str, cal: &str, starts_at: i64, ends_at: i64) -> Event {
        Event {
            guid: guid.into(),
            calendar_guid: cal.into(),
            title: format!("Event {guid}"),
            location: None,
            starts_at,
            ends_at,
            all_day: false,
            timezone: Some("Asia/Kolkata".into()),
            url: None,
            notes: None,
            etag: None,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_calendars(&[calendar("cal-1", "Work"), calendar("cal-2", "Home")])
            .unwrap();
        lib
    }

    #[test]
    fn calendars_round_trip_and_stay_ordered() {
        let lib = seeded();
        let found = lib.calendars().unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].title, "Home",
            "ordering is by sort order then title"
        );
        assert_eq!(found[0].colour.as_deref(), Some("#C8912A"));
    }

    /// A calendar deleted on another device must disappear here, with its events.
    #[test]
    fn calendars_apple_stops_reporting_are_removed() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-2", 100, 200)]).unwrap();

        lib.put_calendars(&[calendar("cal-1", "Work")]).unwrap();

        assert_eq!(lib.calendars().unwrap().len(), 1);
        assert!(
            lib.events_between(0, 1_000).unwrap().is_empty(),
            "events of a removed calendar must go with it"
        );
    }

    /// Overlap, not containment: a meeting already under way belongs in the view.
    #[test]
    fn the_range_query_includes_events_already_running() {
        let mut lib = seeded();
        lib.put_events(&[
            event("before", "cal-1", 0, 50),       // ends before the window
            event("straddling", "cal-1", 50, 150), // started earlier, still running
            event("inside", "cal-1", 110, 120),
            event("after", "cal-1", 300, 400), // starts after the window
        ])
        .unwrap();

        let found = lib.events_between(100, 200).unwrap();
        let guids: Vec<&str> = found.iter().map(|e| e.guid.as_str()).collect();
        assert_eq!(guids, vec!["straddling", "inside"]);
    }

    #[test]
    fn disabled_calendars_are_hidden_from_the_view() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-1", 100, 200)]).unwrap();
        assert_eq!(lib.events_between(0, 1_000).unwrap().len(), 1);

        let mut hidden = calendar("cal-1", "Work");
        hidden.enabled = false;
        lib.put_calendars(&[hidden, calendar("cal-2", "Home")])
            .unwrap();

        assert!(lib.events_between(0, 1_000).unwrap().is_empty());
    }

    #[test]
    fn re_syncing_updates_events_rather_than_duplicating_them() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-1", 100, 200)]).unwrap();

        let mut moved = event("ev-1", "cal-1", 500, 600);
        moved.title = "Moved".into();
        lib.put_events(&[moved]).unwrap();

        let all = lib.events_between(0, 10_000).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Moved");
        assert_eq!(all[0].starts_at, 500);
    }

    #[test]
    fn sync_state_is_recorded() {
        let lib = seeded();
        assert!(lib.last_synced("calendar").unwrap().is_none());
        lib.mark_synced("calendar", Some("token-1")).unwrap();
        assert!(lib.last_synced("calendar").unwrap().is_some());
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_finds_every_word_in_title_place_or_notes_ignoring_accents() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_calendars(&[Calendar {
            guid: "c".into(),
            title: "Work".into(),
            enabled: true,
            writable: true,
            ..Default::default()
        }])
        .unwrap();
        let ev = |g: &str, t: &str, place: Option<&str>, at: i64| Event {
            guid: g.into(),
            calendar_guid: "c".into(),
            title: t.into(),
            location: place.map(Into::into),
            starts_at: at,
            ends_at: at + 60,
            ..Default::default()
        };
        lib.put_events(&[
            ev("a", "Dentist", Some("Café Road"), 200),
            ev("b", "Design review", None, 100),
            ev("c", "Dentist", None, 50),
        ])
        .unwrap();
        let titles = |q: &str| {
            lib.search_events(q)
                .unwrap()
                .into_iter()
                .map(|e| e.guid)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("dentist"), ["c", "a"], "soonest first");
        assert_eq!(
            titles("dentist cafe"),
            ["a"],
            "every word, accents folded, place included"
        );
        assert!(titles("  ").is_empty());
    }
}
