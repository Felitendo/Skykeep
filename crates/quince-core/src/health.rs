//! What each service's last refresh did, and whether it worked.
//!
//! The library is the interface's only source of truth, so a failed refresh must never
//! blank a screen: the last good data stays, and the failure is recorded here instead. A
//! single control in the window reads this to say which parts of the account are current
//! and which are not.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

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

    /// Whether this service is due a full pass rather than a delta.
    ///
    /// A delta cannot remove a record whose tombstone it never saw, so a full pass runs
    /// occasionally to reconcile against the whole truth. Never having done one counts as
    /// due, which is what heals a library that predates this bookkeeping.
    pub fn needs_full_sync(&self, service: &str, every_seconds: i64) -> Result<bool> {
        let last: Option<i64> = self
            .conn
            .query_row(
                "SELECT last_full_at FROM sync_state WHERE service = ?1",
                params![service],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(match last {
            None => true,
            Some(at) => now().saturating_sub(at) >= every_seconds,
        })
    }

    /// Record that a service reconciled against everything Apple has.
    pub fn mark_full_sync(&self, service: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (service, last_full_at) VALUES (?1, ?2)
             ON CONFLICT(service) DO UPDATE SET last_full_at = excluded.last_full_at",
            params![service, now()],
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

    /// A library that has never done a full pass is due one — which is what heals a library
    /// that predates this bookkeeping and is holding records whose tombstones it missed.
    #[test]
    fn never_having_reconciled_counts_as_due() {
        let lib = library();
        assert!(lib.needs_full_sync("reminders", 86_400).unwrap());
        lib.mark_synced("reminders", Some("token")).unwrap();
        assert!(
            lib.needs_full_sync("reminders", 86_400).unwrap(),
            "a delta is not a full pass"
        );
    }

    #[test]
    fn a_recent_full_pass_is_not_repeated() {
        let lib = library();
        lib.mark_full_sync("reminders").unwrap();
        assert!(!lib.needs_full_sync("reminders", 86_400).unwrap());
    }

    #[test]
    fn a_stale_full_pass_comes_due_again() {
        let lib = library();
        lib.mark_full_sync("reminders").unwrap();
        lib.conn
            .execute(
                "UPDATE sync_state SET last_full_at = last_full_at - 90000 WHERE service = 'reminders'",
                [],
            )
            .unwrap();
        assert!(lib.needs_full_sync("reminders", 86_400).unwrap());
    }

    /// Recording a full pass must not disturb the freshness stamp or the resume token.
    #[test]
    fn recording_a_full_pass_leaves_everything_else_alone() {
        let lib = library();
        lib.mark_synced("reminders", Some("token")).unwrap();
        let before = lib.last_synced("reminders").unwrap();
        lib.mark_full_sync("reminders").unwrap();
        assert_eq!(lib.last_synced("reminders").unwrap(), before);
        assert_eq!(
            lib.sync_token("reminders").unwrap().as_deref(),
            Some("token")
        );
    }

    #[test]
    fn a_service_that_never_synced_makes_the_whole_claim_unknown() {
        let lib = library();
        lib.mark_synced("notes", None).unwrap();
        assert_eq!(lib.oldest_sync(&["notes", "calendar"]).unwrap(), None);
    }
}
