//! Bringing Apple's calendar into the local library.
//!
//! The interface never waits on this. A sync reconciles into SQLite and the UI reads SQLite,
//! so a slow or failed refresh leaves the last known good data on screen rather than an
//! empty window.

use crate::calendar::{CalendarClient, Snapshot};
use crate::{Error, Result};
use jiff::civil::Date;
use quince_core::Library;

/// Apple rejects a calendar window wider than about a month with a bare 400, so requests are
/// split into chunks below that. Measured against the live service: one month is accepted,
/// two months is not.
const MAX_WINDOW_DAYS: i32 = 28;

fn parse_date(value: &str) -> Result<Date> {
    value
        .parse::<Date>()
        .map_err(|e| Error::Protocol(format!("{value} is not a date: {e}")))
}

/// Split `[from, to]` into windows Apple will accept, as inclusive `YYYY-MM-DD` pairs.
fn chunks(from: Date, to: Date) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut start = from;
    while start <= to {
        // `- 1` keeps the window inclusive, so chunks meet without overlapping.
        let end = start
            .checked_add(jiff::Span::new().days(MAX_WINDOW_DAYS - 1))
            .unwrap_or(to)
            .min(to);
        out.push((start.to_string(), end.to_string()));
        let Ok(next) = end.checked_add(jiff::Span::new().days(1)) else {
            break;
        };
        start = next;
    }
    out
}

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
    let snapshot = fetch_calendar(http, root, params, from, to, timezone).await?;
    apply_calendar(library, snapshot)
}

/// Ask Apple for a window of the calendar. Touches no local state, so the slow half runs
/// without holding the library every screen reads from.
pub async fn fetch_calendar(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    from: &str,
    to: &str,
    timezone: &str,
) -> Result<Snapshot> {
    let client = CalendarClient::new(http, root, params);
    let windows = chunks(parse_date(from)?, parse_date(to)?);

    let mut calendars = Vec::new();
    let mut events = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (start, end) in &windows {
        // `/ca/startup` reliably carries the collections but not always the events — on some
        // accounts it returns collections alone. Asking `/ca/events` for the same window is
        // what guarantees them, so do both and merge.
        let startup = client.startup(start, end, timezone).await?;
        if calendars.is_empty() {
            calendars = startup.calendars;
        }
        let windowed = client.events(start, end, timezone).await?;

        for event in startup.events.into_iter().chain(windowed.events) {
            if seen.insert(event.guid.clone()) {
                events.push(event);
            }
        }
    }

    Ok(Snapshot { calendars, events })
}

/// Store a fetched window. Touches no network.
pub fn apply_calendar(library: &mut Library, snapshot: Snapshot) -> Result<SyncSummary> {
    library.put_calendars(&snapshot.calendars)?;

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

#[cfg(test)]
mod window_tests {
    use super::*;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    /// Apple answers 400 for anything much wider than a month, so no chunk may exceed the cap.
    #[test]
    fn no_chunk_exceeds_what_apple_accepts() {
        let windows = chunks(d("2026-01-01"), d("2026-12-31"));
        assert!(!windows.is_empty());
        for (start, end) in &windows {
            let span = d(end) - d(start);
            let days = i64::from(span.get_days());
            assert!(
                days < i64::from(MAX_WINDOW_DAYS),
                "{start}..{end} spans {days} days, over the limit"
            );
        }
    }

    /// Chunks must tile the range exactly: no gap would silently lose events, and no overlap
    /// means no wasted request.
    #[test]
    fn chunks_tile_the_range_without_gaps_or_overlap() {
        let (from, to) = (d("2026-01-01"), d("2026-06-30"));
        let windows = chunks(from, to);

        assert_eq!(windows[0].0, from.to_string());
        assert_eq!(windows.last().unwrap().1, to.to_string());

        for pair in windows.windows(2) {
            let previous_end = d(&pair[0].1);
            let next_start = d(&pair[1].0);
            assert_eq!(
                (next_start - previous_end).get_days(),
                1,
                "chunks must meet exactly, not skip or repeat a day"
            );
        }
    }

    #[test]
    fn a_short_range_is_a_single_window() {
        let windows = chunks(d("2026-03-01"), d("2026-03-10"));
        assert_eq!(
            windows,
            vec![("2026-03-01".to_string(), "2026-03-10".to_string())]
        );
    }

    #[test]
    fn a_single_day_still_produces_a_window() {
        assert_eq!(chunks(d("2026-03-01"), d("2026-03-01")).len(), 1);
    }

    /// A backwards range should produce nothing rather than looping.
    #[test]
    fn an_inverted_range_is_empty() {
        assert!(chunks(d("2026-06-01"), d("2026-01-01")).is_empty());
    }
}
