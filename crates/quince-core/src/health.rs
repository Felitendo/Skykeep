//! What each service's last refresh did, and whether it worked.
//!
//! The library is the interface's only source of truth, so a failed refresh must never
//! blank a screen: the last good data stays, and the failure is recorded here instead. A
//! single control in the window reads this to say which parts of the account are current
//! and which are not.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// One service's standing after the last refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceHealth {
    pub service: String,
    /// When this service last came back clean. `None` means it never has.
    pub last_synced_at: Option<i64>,
    /// The most recent failure, cleared by the next success.
    pub last_error: Option<String>,
}

impl ServiceHealth {
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.last_error.is_some() || self.last_synced_at.is_none()
    }
}

impl Library {
    /// Record that a service's refresh failed.
    ///
    /// The previous `last_synced_at` is deliberately left alone: the data on screen really
    /// is as of then, and overwriting it would claim a freshness the library does not have.
    pub fn mark_sync_failed(&self, service: &str, message: &str) -> Result<()> {
        // A protocol error can carry a whole response body; a person needs the first line.
        let trimmed: String = message.chars().take(300).collect();
        self.conn.execute(
            "INSERT INTO sync_state (service, token, last_synced_at, last_error)
             VALUES (?1, NULL, NULL, ?2)
             ON CONFLICT(service) DO UPDATE SET last_error = excluded.last_error",
            params![service, trimmed],
        )?;
        Ok(())
    }

    /// Every service the library has ever tried to refresh.
    pub fn sync_health(&self) -> Result<Vec<ServiceHealth>> {
        let mut stmt = self.conn.prepare(
            "SELECT service, last_synced_at, last_error FROM sync_state ORDER BY service",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ServiceHealth {
                service: row.get(0)?,
                last_synced_at: row.get(1)?,
                last_error: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The oldest successful refresh across `services`, which is how fresh the library as a
    /// whole can honestly claim to be. `None` if any of them has never synced.
    pub fn oldest_sync(&self, services: &[&str]) -> Result<Option<i64>> {
        let health = self.sync_health()?;
        let mut oldest: Option<i64> = None;
        for service in services {
            let found = health
                .iter()
                .find(|h| h.service == *service)
                .and_then(|h| h.last_synced_at);
            match found {
                None => return Ok(None),
                Some(at) => oldest = Some(oldest.map_or(at, |o: i64| o.min(at))),
            }
        }
        Ok(oldest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Library {
        Library::in_memory().unwrap()
    }

    #[test]
    fn a_failure_is_recorded_without_claiming_freshness() {
        let lib = library();
        lib.mark_synced("notes", Some("token")).unwrap();
        let synced = lib.last_synced("notes").unwrap();
        assert!(synced.is_some());

        lib.mark_sync_failed("notes", "Apple said 503").unwrap();
        let health = lib.sync_health().unwrap();
        let notes = health.iter().find(|h| h.service == "notes").unwrap();
        assert_eq!(notes.last_error.as_deref(), Some("Apple said 503"));
        assert_eq!(
            notes.last_synced_at, synced,
            "a failure must not move the freshness stamp"
        );
        assert!(notes.is_stale());
    }

    /// A failure before a service has ever synced must still be visible.
    #[test]
    fn a_first_ever_failure_still_lands() {
        let lib = library();
        lib.mark_sync_failed("drive", "no network").unwrap();
        let health = lib.sync_health().unwrap();
        assert_eq!(health.len(), 1);
        assert_eq!(health[0].last_synced_at, None);
        assert_eq!(health[0].last_error.as_deref(), Some("no network"));
    }

    #[test]
    fn success_clears_the_previous_failure() {
        let lib = library();
        lib.mark_sync_failed("notes", "Apple said 503").unwrap();
        lib.mark_synced("notes", None).unwrap();
        let health = lib.sync_health().unwrap();
        assert_eq!(health[0].last_error, None);
        assert!(!health[0].is_stale());
    }

    /// A wall of text from a failing endpoint must not become the interface's message.
    #[test]
    fn a_huge_error_is_cut_down() {
        let lib = library();
        lib.mark_sync_failed("notes", &"x".repeat(5_000)).unwrap();
        let health = lib.sync_health().unwrap();
        assert_eq!(
            health[0].last_error.as_deref().unwrap().chars().count(),
            300
        );
    }

    /// The window can only claim the freshness of its stalest part.
    #[test]
    fn the_oldest_success_is_what_the_library_can_claim() {
        let lib = library();
        lib.mark_synced("notes", None).unwrap();
        lib.mark_synced("reminders", None).unwrap();
        lib.conn
            .execute(
                "UPDATE sync_state SET last_synced_at = 1000 WHERE service = 'reminders'",
                [],
            )
            .unwrap();
        assert_eq!(
            lib.oldest_sync(&["notes", "reminders"]).unwrap(),
            Some(1000)
        );
    }

    #[test]
    fn a_service_that_never_synced_makes_the_whole_claim_unknown() {
        let lib = library();
        lib.mark_synced("notes", None).unwrap();
        assert_eq!(lib.oldest_sync(&["notes", "calendar"]).unwrap(), None);
    }
}
