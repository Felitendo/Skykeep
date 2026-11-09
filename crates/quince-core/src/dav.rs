//! Calendar accounts other than iCloud, as the library keeps them: Nextcloud, or any CalDAV
//! server.
//!
//! The screens read `calendars` and `events`, as they do for iCloud: search, alerts, export and
//! the outbox need nothing else. Beside them is what only this kind of account has: the
//! account, where each calendar lives, and each event's iCalendar text as the server holds it.

use crate::calendar::{Calendar, Event};
use crate::{Library, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where the guid of such an account's calendars and events begins:
/// `quince:dav:<account>:<calendar>` and `quince:dav:<account>:<calendar>:<event>`.
pub const PREFIX: &str = "quince:dav:";

/// Whether a calendar or event guid is such an account's.
#[must_use]
pub fn is_dav(guid: &str) -> bool {
    guid.starts_with(PREFIX)
}

/// The account a calendar or event guid belongs to.
#[must_use]
pub fn account_of(guid: &str) -> Option<&str> {
    guid.strip_prefix(PREFIX)?.split(':').next()
}

/// A calendar account, its password in the system keyring.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DavAccount {
    pub id: String,
    /// `nextcloud` or `caldav`.
    pub kind: String,
    /// What the sidebar calls it.
    pub title: String,
    /// The address signed in to.
    pub server: String,
    pub username: String,
    /// The address its calendars are listed from.
    pub home: String,
    /// The hosts its client may reach.
    pub hosts: Vec<String>,
    pub synced_at: Option<i64>,
    /// Why the last pass did not land, in words; cleared by the next that does.
    pub problem: Option<String>,
    /// The server refused the password: not asked again until the person signs in.
    pub needs_sign_in: bool,
}

/// A calendar of such an account, and where it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavCalendar {
    pub calendar: Calendar,
    pub url: String,
}

/// One event's text as the server holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavResource {
    /// The event's guid; for a repeating one, what its occurrences' guids begin with.
    pub guid: String,
    pub calendar: String,
    pub href: String,
    pub etag: Option<String>,
    pub ics: String,
}

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DavAccount> {
    let hosts: String = row.get(6)?;
    Ok(DavAccount {
        id: row.get(0)?,
        kind: row.get(1)?,
        title: row.get(2)?,
        server: row.get(3)?,
        username: row.get(4)?,
        home: row.get(5)?,
        hosts: serde_json::from_str(&hosts).unwrap_or_default(),
        synced_at: row.get(7)?,
        problem: row.get(8)?,
        needs_sign_in: row.get::<_, i64>(9)? != 0,
    })
}

const ACCOUNT_COLUMNS: &str =
    "id, kind, title, server, username, home, hosts, synced_at, problem, needs_sign_in";

fn resource_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DavResource> {
    Ok(DavResource {
        guid: row.get(0)?,
        calendar: row.get(1)?,
        href: row.get(2)?,
        etag: row.get(3)?,
        ics: row.get(4)?,
    })
}

impl Library {
    pub fn dav_accounts(&self) -> Result<Vec<DavAccount>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM dav_accounts ORDER BY rowid"
        ))?;
        let rows = stmt.query_map([], account_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn dav_account(&self, id: &str) -> Result<Option<DavAccount>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {ACCOUNT_COLUMNS} FROM dav_accounts WHERE id = ?1"),
                params![id],
                account_from_row,
            )
            .optional()?)
    }

    pub fn put_dav_account(&self, a: &DavAccount) -> Result<()> {
        self.conn.execute(
            "INSERT INTO dav_accounts (id, kind, title, server, username, home, hosts, synced_at,
                                       problem, needs_sign_in)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               kind = excluded.kind, title = excluded.title, server = excluded.server,
               username = excluded.username, home = excluded.home, hosts = excluded.hosts,
               synced_at = excluded.synced_at, problem = excluded.problem,
               needs_sign_in = excluded.needs_sign_in",
            params![
                a.id,
                a.kind,
                a.title,
                a.server,
                a.username,
                a.home,
                serde_json::to_string(&a.hosts).unwrap_or_else(|_| "[]".into()),
                a.synced_at,
                a.problem,
                i64::from(a.needs_sign_in),
            ],
        )?;
        Ok(())
    }

    /// How the last pass over an account ended.
    pub fn set_dav_account_state(
        &self,
        id: &str,
        synced_at: Option<i64>,
        problem: Option<&str>,
        needs_sign_in: bool,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE dav_accounts SET synced_at = COALESCE(?2, synced_at), problem = ?3,
                                     needs_sign_in = ?4 WHERE id = ?1",
            params![id, synced_at, problem, i64::from(needs_sign_in)],
        )?;
        Ok(())
    }

    /// Everything of an account gone from this computer: its calendars, their events, what
    /// waited to be sent to it.
    pub fn forget_dav_account(&mut self, id: &str, queue: &str) -> Result<()> {
        let tx = self.conn.savepoint()?;
        tx.execute(
            "DELETE FROM calendars WHERE guid IN
               (SELECT guid FROM dav_calendars WHERE account = ?1)",
            params![id],
        )?;
        tx.execute("DELETE FROM dav_accounts WHERE id = ?1", params![id])?;
        tx.execute("DELETE FROM outbox WHERE service = ?1", params![queue])?;
        tx.commit()?;
        Ok(())
    }

    pub fn dav_calendars(&self, account: &str) -> Result<Vec<DavCalendar>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.guid, c.title, c.colour, c.ctag, c.sort_order, c.enabled, c.writable,
                    c.is_default, d.url
               FROM dav_calendars d JOIN calendars c ON c.guid = d.guid
              WHERE d.account = ?1 ORDER BY c.sort_order, c.title",
        )?;
        let rows = stmt.query_map(params![account], |row| {
            Ok(DavCalendar {
                calendar: Calendar {
                    guid: row.get(0)?,
                    title: row.get(1)?,
                    colour: row.get(2)?,
                    ctag: row.get(3)?,
                    sort_order: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    writable: row.get::<_, i64>(6)? != 0,
                    is_default: row.get::<_, i64>(7)? != 0,
                },
                url: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Where a calendar lives.
    pub fn dav_calendar_url(&self, guid: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT url FROM dav_calendars WHERE guid = ?1",
                params![guid],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The account's calendars as the server listed them. One no longer listed was deleted
    /// there: it goes, with its events.
    pub fn put_dav_calendars(&mut self, account: &str, calendars: &[DavCalendar]) -> Result<()> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        let tx = self.conn.savepoint()?;
        for d in calendars {
            let c = &d.calendar;
            tx.execute(
                "INSERT INTO calendars (guid, title, colour, ctag, sort_order, enabled, writable,
                                        is_default, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, colour = excluded.colour, ctag = excluded.ctag,
                   sort_order = excluded.sort_order, writable = excluded.writable,
                   updated_at = excluded.updated_at",
                params![
                    c.guid,
                    c.title,
                    c.colour,
                    c.ctag,
                    c.sort_order,
                    i64::from(c.enabled),
                    i64::from(c.writable),
                    i64::from(c.is_default),
                    stamp
                ],
            )?;
            tx.execute(
                "INSERT INTO dav_calendars (guid, account, url) VALUES (?1, ?2, ?3)
                 ON CONFLICT(guid) DO UPDATE SET url = excluded.url",
                params![c.guid, account, d.url],
            )?;
        }
        let keep: Vec<&str> = calendars.iter().map(|d| d.calendar.guid.as_str()).collect();
        tx.execute(
            "DELETE FROM calendars WHERE guid IN
               (SELECT guid FROM dav_calendars WHERE account = ?1
                   AND guid NOT IN (SELECT value FROM json_each(?2)))",
            params![account, crate::id_list(&keep)],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn dav_resources(&self, calendar: &str) -> Result<Vec<DavResource>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, calendar, href, etag, ics FROM dav_resources WHERE calendar = ?1",
        )?;
        let rows = stmt.query_map(params![calendar], resource_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One event's text, by the guid of the event or of its series.
    pub fn dav_resource(&self, guid: &str) -> Result<Option<DavResource>> {
        Ok(self
            .conn
            .query_row(
                "SELECT guid, calendar, href, etag, ics FROM dav_resources WHERE guid = ?1",
                params![guid],
                resource_from_row,
            )
            .optional()?)
    }

    /// Each stored event's address and tag: what a pass compares the server's list against.
    pub fn dav_etags(&self, calendar: &str) -> Result<HashMap<String, Option<String>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT href, etag FROM dav_resources WHERE calendar = ?1")?;
        let rows = stmt.query_map(params![calendar], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn put_dav_resource(&self, r: &DavResource) -> Result<()> {
        self.conn.execute(
            "INSERT INTO dav_resources (guid, calendar, href, etag, ics)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(guid) DO UPDATE SET
               calendar = excluded.calendar, href = excluded.href,
               etag = excluded.etag, ics = excluded.ics",
            params![r.guid, r.calendar, r.href, r.etag, r.ics],
        )?;
        Ok(())
    }

    /// An event gone from the server, and every occurrence of it here.
    pub fn forget_dav_resource(&mut self, guid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM dav_resources WHERE guid = ?1", params![guid])?;
        self.replace_dav_events(guid, &[])
    }

    /// Everything laid out from one event replaced: the event, or every occurrence of it.
    pub fn replace_dav_events(&mut self, series: &str, events: &[Event]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM events WHERE guid = ?1 OR substr(guid, 1, length(?1) + 2) = ?1 || '__'",
            params![series],
        )?;
        self.put_events(events)
    }

    /// Everything laid out from one calendar replaced.
    pub fn replace_dav_calendar_events(&mut self, calendar: &str, events: &[Event]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM events WHERE calendar_guid = ?1",
            params![calendar],
        )?;
        self.put_events(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> DavAccount {
        DavAccount {
            id: id.into(),
            kind: "nextcloud".into(),
            title: "Nextcloud".into(),
            server: "https://cloud.example.org".into(),
            username: "felix".into(),
            home: "https://cloud.example.org/remote.php/dav/calendars/felix/".into(),
            hosts: vec!["cloud.example.org".into()],
            ..Default::default()
        }
    }

    fn calendar(account: &str, name: &str) -> DavCalendar {
        DavCalendar {
            calendar: Calendar {
                guid: format!("{PREFIX}{account}:{name}"),
                title: name.into(),
                enabled: true,
                writable: true,
                ..Default::default()
            },
            url: format!("https://cloud.example.org/{name}/"),
        }
    }

    fn event(guid: &str, calendar: &str) -> Event {
        Event {
            guid: guid.into(),
            calendar_guid: calendar.into(),
            title: "Standup".into(),
            starts_at: 1_000,
            ends_at: 2_000,
            timezone: Some("UTC".into()),
            ..Default::default()
        }
    }

    fn with_account() -> (Library, String) {
        let mut lib = Library::in_memory().unwrap();
        lib.put_dav_account(&account("a1")).unwrap();
        let cal = calendar("a1", "home");
        let guid = cal.calendar.guid.clone();
        lib.put_dav_calendars("a1", &[cal]).unwrap();
        (lib, guid)
    }

    /// Apple's list of calendars says what is Apple's. Read as the whole truth, every pass
    /// deleted the calendars of the Nextcloud account and all their events.
    #[test]
    fn a_pass_over_icloud_leaves_another_accounts_calendars_alone() {
        let (mut lib, guid) = with_account();
        lib.put_events(&[event(&format!("{guid}:e1"), &guid)])
            .unwrap();
        let apple = Calendar {
            guid: "apple".into(),
            title: "Home".into(),
            enabled: true,
            ..Default::default()
        };
        lib.put_calendars(&[apple]).unwrap();
        lib.put_calendars(&[]).unwrap();
        assert!(lib.calendars().unwrap().iter().any(|c| c.guid == guid));
        assert!(lib.event(&format!("{guid}:e1")).unwrap().is_some());
        assert!(
            !lib.calendars().unwrap().iter().any(|c| c.guid == "apple"),
            "Apple's own are still judged by Apple's list"
        );
    }

    #[test]
    fn a_calendar_no_longer_listed_goes_with_its_events_and_text() {
        let (mut lib, guid) = with_account();
        lib.put_dav_resource(&DavResource {
            guid: format!("{guid}:e1"),
            calendar: guid.clone(),
            href: "/e1.ics".into(),
            etag: None,
            ics: "BEGIN:VCALENDAR".into(),
        })
        .unwrap();
        lib.put_events(&[event(&format!("{guid}:e1"), &guid)])
            .unwrap();
        lib.put_dav_calendars("a1", &[]).unwrap();
        assert!(lib.calendars().unwrap().is_empty());
        assert!(lib.event(&format!("{guid}:e1")).unwrap().is_none());
        assert!(lib.dav_resource(&format!("{guid}:e1")).unwrap().is_none());
    }

    #[test]
    fn replacing_a_series_leaves_an_event_whose_guid_only_begins_the_same() {
        let (mut lib, guid) = with_account();
        let series = format!("{guid}:e1");
        let other = format!("{guid}:e10");
        lib.put_events(&[
            event(&series, &guid),
            event(&format!("{series}__1000"), &guid),
            event(&other, &guid),
        ])
        .unwrap();
        lib.replace_dav_events(&series, &[]).unwrap();
        assert!(lib.event(&series).unwrap().is_none());
        assert!(lib.event(&format!("{series}__1000")).unwrap().is_none());
        assert!(lib.event(&other).unwrap().is_some());
    }

    #[test]
    fn letting_an_account_go_takes_everything_of_it() {
        let (mut lib, guid) = with_account();
        lib.put_events(&[event(&format!("{guid}:e1"), &guid)])
            .unwrap();
        lib.outbox_append("caldav:a1", "r", "save", "{}").unwrap();
        lib.forget_dav_account("a1", "caldav:a1").unwrap();
        assert!(lib.dav_accounts().unwrap().is_empty());
        assert!(lib.calendars().unwrap().is_empty());
        assert!(lib.outbox_pending("caldav:a1").unwrap().is_empty());
    }

    #[test]
    fn an_account_is_read_back_as_it_was_kept() {
        let (lib, _) = with_account();
        assert_eq!(lib.dav_account("a1").unwrap(), Some(account("a1")));
        lib.set_dav_account_state("a1", Some(9), Some("refused"), true)
            .unwrap();
        let a = lib.dav_account("a1").unwrap().unwrap();
        assert_eq!(a.synced_at, Some(9));
        assert_eq!(a.problem.as_deref(), Some("refused"));
        assert!(a.needs_sign_in);
    }

    #[test]
    fn a_guid_names_its_account() {
        assert_eq!(account_of("quince:dav:a1:c2:e3__4"), Some("a1"));
        assert_eq!(account_of("BCE46561"), None);
        assert!(is_dav("quince:dav:a1:c2"));
        assert!(!is_dav("quince:sub:a1"));
    }
}
