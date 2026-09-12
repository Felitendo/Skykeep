//! Bringing Apple's calendar into the local library.
//!
//! The interface never waits on this. A sync reconciles into SQLite and the UI reads SQLite,
//! so a slow or failed refresh leaves the last known good data on screen rather than an
//! empty window.

use crate::calendar::CalendarClient;
use crate::Result;
use quince_core::Library;

/// What one refresh changed.
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub calendars: usize,
    pub events: usize,
}

/// Fetch the window `[from, to]` (both `YYYY-MM-DD`) and reconcile it into the library.
///
/// Collections are written before events so the foreign key always resolves; an event whose
/// calendar Apple did not report would otherwise be rejected.
pub async fn calendar(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut Library,
    from: &str,
    to: &str,
    timezone: &str,
) -> Result<SyncSummary> {
    let client = CalendarClient::new(http, root, params);

    // `/ca/startup` reliably carries the collections but does not always carry events — on
    // some accounts it returns collections alone. Asking `/ca/events` for the same window is
    // what actually guarantees them, so do both and merge.
    let startup = client.startup(from, to, timezone).await?;
    let windowed = client.events(from, to, timezone).await?;

    library.put_calendars(&startup.calendars)?;

    let mut snapshot = startup;
    if !windowed.events.is_empty() {
        let known: std::collections::HashSet<&str> =
            snapshot.events.iter().map(|e| e.guid.as_str()).collect();
        let extra: Vec<_> = windowed
            .events
            .iter()
            .filter(|e| !known.contains(e.guid.as_str()))
            .cloned()
            .collect();
        snapshot.events.extend(extra);
    }

    // Drop events belonging to calendars Apple did not report: storing them would fail the
    // foreign key, and they are not ours to show.
    let known: std::collections::HashSet<&str> =
        snapshot.calendars.iter().map(|c| c.guid.as_str()).collect();
    let events: Vec<_> = snapshot
        .events
        .iter()
        .filter(|e| known.contains(e.calendar_guid.as_str()))
        .cloned()
        .collect();

    library.put_events(&events)?;
    library.mark_synced("calendar", None)?;

    Ok(SyncSummary {
        calendars: snapshot.calendars.len(),
        events: events.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        json!({
            "Collection": [{ "guid": "cal-1", "title": "Work", "ctag": "c1" }],
            "Event": [
                { "guid": "ev-1", "pGuid": "cal-1", "title": "Standup", "tz": "Asia/Kolkata",
                  "startDate": [20260912, 2026, 9, 12, 9, 30, 570],
                  "endDate":   [20260912, 2026, 9, 12, 9, 45, 585] },
                // Belongs to a calendar Apple did not report — must be dropped, not stored.
                { "guid": "ev-2", "pGuid": "cal-missing", "title": "Orphan",
                  "startDate": [20260912, 2026, 9, 12, 11, 0, 660], "duration": 30 }
            ]
        })
    }

    /// Some accounts get collections from /ca/startup and nothing else; the events arrive
    /// only from /ca/events. A sync that trusts startup alone silently shows an empty
    /// calendar.
    #[tokio::test]
    async fn events_are_collected_even_when_startup_omits_them() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Collection": [{ "guid": "cal-1", "title": "Work" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Event": [{ "guid": "ev-9", "pGuid": "cal-1", "title": "From events",
                            "startDate": [20260912, 2026, 9, 12, 9, 0, 540], "duration": 30 }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        let summary = calendar(
            &http,
            &server.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC",
        )
        .await
        .unwrap();

        assert_eq!(
            summary.events, 1,
            "events must come from /ca/events when startup omits them"
        );
        assert_eq!(
            library.events_between(0, i64::MAX).unwrap()[0].title,
            "From events"
        );
    }

    #[tokio::test]
    async fn a_sync_lands_in_the_library() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();

        let summary = calendar(
            &http,
            &server.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "Asia/Kolkata",
        )
        .await
        .unwrap();

        assert_eq!(summary.calendars, 1);
        assert_eq!(summary.events, 1, "the orphaned event must be dropped");

        let stored = library.events_between(0, i64::MAX).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "Standup");
        assert!(library.last_synced("calendar").unwrap().is_some());
    }

    /// Running twice must converge, not accumulate.
    #[tokio::test]
    async fn syncing_twice_is_idempotent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        for _ in 0..3 {
            calendar(
                &http,
                &server.uri(),
                vec![],
                &mut library,
                "2026-09-01",
                "2026-09-30",
                "UTC",
            )
            .await
            .unwrap();
        }
        assert_eq!(library.events_between(0, i64::MAX).unwrap().len(), 1);
        assert_eq!(library.calendars().unwrap().len(), 1);
    }

    /// A failed refresh must leave the last good data alone.
    #[tokio::test]
    async fn a_failed_sync_does_not_empty_the_library() {
        let ok = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&ok)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&ok)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        calendar(
            &http,
            &ok.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC",
        )
        .await
        .unwrap();

        let broken = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&broken)
            .await;

        assert!(calendar(
            &http,
            &broken.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC"
        )
        .await
        .is_err());
        assert_eq!(
            library.events_between(0, i64::MAX).unwrap().len(),
            1,
            "a failed refresh must not clear what we already had"
        );
    }
}
