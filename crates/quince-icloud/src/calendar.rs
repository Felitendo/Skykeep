//! Apple's calendar web service.
//!
//! `/ca/startup` returns the collections plus the events in a window; `/ca/events` returns
//! events alone. Both are shaped for the icloud.com web client rather than for us.

use crate::{Error, Result};
use jiff::{civil, tz::TimeZone};
use quince_core::calendar::{Calendar, Event};
use serde_json::Value;

/// Apple encodes an instant as `[YYYYMMDD, year, month, day, hour, minute, minsSinceMidnight]`
/// alongside a separate named timezone — not as a timestamp.
///
/// Converting needs the zone, because the same wall-clock reading means different instants in
/// different places. A missing or unknown zone falls back to UTC rather than failing the whole
/// sync over one event.
fn parse_apple_date(raw: &Value, timezone: Option<&str>) -> Result<i64> {
    let parts = raw
        .as_array()
        .ok_or_else(|| Error::Protocol("a date was not an array".into()))?;
    // Index 0 is a packed YYYYMMDD that duplicates the fields after it; the components are
    // what we read. Anything shorter than six entries is unusable.
    if parts.len() < 6 {
        return Err(Error::Protocol(format!(
            "a date had {} components, expected at least 6",
            parts.len()
        )));
    }
    let at = |i: usize| -> Result<i64> {
        parts[i]
            .as_i64()
            .ok_or_else(|| Error::Protocol(format!("date component {i} was not a number")))
    };

    let (year, month, day, hour, minute) = (at(1)?, at(2)?, at(3)?, at(4)?, at(5)?);

    // `civil::date` PANICS on an out-of-range component, and these numbers come off the
    // network. The checked constructor is the only safe way to build one from untrusted input.
    let narrow = |v: i64, what: &str| -> Result<i8> {
        i8::try_from(v).map_err(|_| Error::Protocol(format!("implausible {what} {v}")))
    };
    let civil_dt = civil::DateTime::new(
        i16::try_from(year).map_err(|_| Error::Protocol(format!("implausible year {year}")))?,
        narrow(month, "month")?,
        narrow(day, "day")?,
        narrow(hour, "hour")?,
        narrow(minute, "minute")?,
        0,
        0,
    )
    .map_err(|e| {
        Error::Protocol(format!(
            "{year}-{month}-{day} {hour}:{minute} is not a date: {e}"
        ))
    })?;

    // An unknown zone is not worth failing a sync over; UTC keeps the event visible.
    let zone = timezone
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or(TimeZone::UTC);

    let zoned = civil_dt
        .to_zoned(zone)
        .map_err(|e| Error::Protocol(format!("couldn't place {civil_dt} in its timezone: {e}")))?;
    Ok(zoned.timestamp().as_second())
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Turn one entry of Apple's `Collection` array into a calendar.
fn collection_from(v: &Value) -> Option<Calendar> {
    Some(Calendar {
        guid: string(v, "guid")?,
        title: string(v, "title").unwrap_or_else(|| "Untitled".into()),
        colour: string(v, "color"),
        ctag: string(v, "ctag"),
        sort_order: v.get("order").and_then(Value::as_i64).unwrap_or(0),
        // Apple omits `enabled` for calendars that are on.
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Turn one entry of Apple's `Event` array into an event.
///
/// Returns `Ok(None)` for entries that carry no usable identity, so one odd row cannot fail
/// an otherwise good sync.
fn event_from(v: &Value) -> Result<Option<Event>> {
    let (Some(guid), Some(calendar_guid)) = (string(v, "guid"), string(v, "pGuid")) else {
        return Ok(None);
    };
    let timezone = string(v, "tz");
    let Some(start_raw) = v.get("startDate") else {
        return Ok(None);
    };

    let starts_at = parse_apple_date(start_raw, timezone.as_deref())?;
    let ends_at = match v.get("endDate") {
        Some(raw) => parse_apple_date(raw, timezone.as_deref())?,
        // Apple omits endDate on some entries; a duration in minutes is the fallback.
        None => {
            let minutes = v.get("duration").and_then(Value::as_i64).unwrap_or(0);
            starts_at + minutes * 60
        }
    };

    Ok(Some(Event {
        guid,
        calendar_guid,
        title: string(v, "title").unwrap_or_else(|| "Untitled".into()),
        location: string(v, "location"),
        starts_at,
        // A zero-length event still needs to be selectable, and an end before its start is
        // nonsense we should not store.
        ends_at: ends_at.max(starts_at),
        all_day: v.get("allDay").and_then(Value::as_bool).unwrap_or(false),
        timezone,
        url: string(v, "url"),
        notes: string(v, "description"),
        etag: string(v, "etag"),
    }))
}

/// Everything one calendar refresh yields.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
}

/// Read a `/ca/startup` or `/ca/events` body.
///
/// Malformed individual events are skipped rather than failing the batch: Apple's calendar is
/// long-lived data and one unparseable row from years ago should not block today's sync.
pub fn parse_snapshot(body: &Value) -> Snapshot {
    let calendars = body
        .get("Collection")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(collection_from).collect())
        .unwrap_or_default();

    let events = body
        .get("Event")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| event_from(v).ok().flatten())
                .collect()
        })
        .unwrap_or_default();

    Snapshot { calendars, events }
}

/// Apple's calendar service.
pub struct CalendarClient<'a> {
    http: &'a reqwest::Client,
    root: String,
    params: Vec<(String, String)>,
}

impl<'a> CalendarClient<'a> {
    pub fn new(http: &'a reqwest::Client, root: &str, params: Vec<(String, String)>) -> Self {
        Self {
            http,
            root: root.trim_end_matches('/').to_string(),
            params,
        }
    }

    /// Collections plus the events between two dates, formatted `YYYY-MM-DD`.
    pub async fn startup(&self, from: &str, to: &str, tz: &str) -> Result<Snapshot> {
        let body = self.get("/ca/startup", from, to, tz).await?;
        Ok(parse_snapshot(&body))
    }

    /// Events alone, for refreshing a window without refetching the collections.
    pub async fn events(&self, from: &str, to: &str, tz: &str) -> Result<Snapshot> {
        let body = self.get("/ca/events", from, to, tz).await?;
        Ok(parse_snapshot(&body))
    }

    async fn get(&self, path: &str, from: &str, to: &str, tz: &str) -> Result<Value> {
        let mut query = self.params.clone();
        query.push(("startDate".into(), from.into()));
        query.push(("endDate".into(), to.into()));
        query.push(("lang".into(), "en-us".into()));
        query.push(("usertz".into(), tz.into()));

        let response = self
            .http
            .get(format!("{}{path}", self.root))
            .query(&query)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "{path} returned {status}: {}",
                text.chars().take(160).collect::<String>()
            )));
        }
        serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("{path} returned invalid JSON: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_date_is_read_in_its_own_timezone() {
        // 14:30 in Kolkata is 09:00 UTC — the zone is what makes this unambiguous.
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let seconds = parse_apple_date(&raw, Some("Asia/Kolkata")).unwrap();
        let utc = jiff::Timestamp::from_second(seconds).unwrap();
        assert_eq!(utc.to_string(), "2026-09-12T09:00:00Z");
    }

    #[test]
    fn the_same_reading_in_another_zone_is_another_instant() {
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let kolkata = parse_apple_date(&raw, Some("Asia/Kolkata")).unwrap();
        let new_york = parse_apple_date(&raw, Some("America/New_York")).unwrap();
        assert_ne!(
            kolkata, new_york,
            "wall-clock time alone does not identify an instant"
        );
        // New York is behind, so the same reading happens later in absolute terms.
        assert!(new_york > kolkata);
    }

    /// The hour that does not exist on a spring-forward day must not fail a sync.
    #[test]
    fn a_daylight_saving_gap_resolves_rather_than_erroring() {
        // 02:30 on 8 March 2026 never happens in New York.
        let raw = json!([20260308, 2026, 3, 8, 2, 30, 150]);
        let seconds = parse_apple_date(&raw, Some("America/New_York")).unwrap();
        assert!(
            seconds > 0,
            "a nonexistent local time should shift, not blow up"
        );
    }

    #[test]
    fn an_unknown_timezone_falls_back_to_utc() {
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let unknown = parse_apple_date(&raw, Some("Mars/Olympus_Mons")).unwrap();
        let utc = parse_apple_date(&raw, None).unwrap();
        assert_eq!(unknown, utc, "one bad zone must not lose the event");
    }

    /// Network input must never panic, however malformed.
    #[test]
    fn malformed_dates_are_errors_not_panics() {
        for bad in [
            json!("2026-09-12"),
            json!([]),
            json!([20260912, 2026, 9]),
            json!([20260912, 2026, 99, 12, 14, 30]), // month out of range
            json!([20260912, 2026, 2, 31, 14, 30]),  // a day that does not exist
            json!([20260912, 2026, 9, 12, 99, 30]),  // hour out of range
            json!([20260912, 2026, 9, 12, 14, 999]), // minute out of range
            json!([20260912, 2026, 0, 12, 14, 30]),  // zero month
            json!([20260912, 2026, 9, 12, "two", 30]),
            json!([20260912, 999_999, 9, 12, 14, 30]),
        ] {
            assert!(parse_apple_date(&bad, None).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn a_snapshot_reads_collections_and_events() {
        let body = json!({
            "Collection": [
                { "guid": "cal-1", "title": "Work", "color": "#C8912A", "ctag": "c1", "order": 1 },
                { "guid": "cal-2", "title": "Home", "enabled": false }
            ],
            "Event": [
                {
                    "guid": "ev-1", "pGuid": "cal-1", "title": "Standup",
                    "location": "Room 2", "tz": "Asia/Kolkata",
                    "startDate": [20260912, 2026, 9, 12, 9, 30, 570],
                    "endDate":   [20260912, 2026, 9, 12, 9, 45, 585],
                    "etag": "e1"
                }
            ]
        });
        let snap = parse_snapshot(&body);

        assert_eq!(snap.calendars.len(), 2);
        assert_eq!(snap.calendars[0].colour.as_deref(), Some("#C8912A"));
        assert!(
            snap.calendars[0].enabled,
            "a calendar with no `enabled` flag is on"
        );
        assert!(!snap.calendars[1].enabled);

        assert_eq!(snap.events.len(), 1);
        let event = &snap.events[0];
        assert_eq!(event.title, "Standup");
        assert_eq!(event.calendar_guid, "cal-1");
        assert_eq!(event.ends_at - event.starts_at, 15 * 60);
    }

    #[test]
    fn an_event_without_an_end_uses_its_duration() {
        let body = json!({ "Event": [{
            "guid": "ev-1", "pGuid": "cal-1", "title": "Focus",
            "startDate": [20260912, 2026, 9, 12, 9, 0, 540],
            "duration": 90
        }]});
        let snap = parse_snapshot(&body);
        assert_eq!(snap.events[0].ends_at - snap.events[0].starts_at, 90 * 60);
    }

    #[test]
    fn an_end_before_its_start_is_clamped() {
        let body = json!({ "Event": [{
            "guid": "ev-1", "pGuid": "cal-1", "title": "Backwards",
            "startDate": [20260912, 2026, 9, 12, 10, 0, 600],
            "endDate":   [20260912, 2026, 9, 12, 9, 0, 540]
        }]});
        let event = &parse_snapshot(&body).events[0];
        assert_eq!(
            event.ends_at, event.starts_at,
            "a negative duration is not storable"
        );
    }

    /// One unparseable row from years ago must not block today's sync.
    #[test]
    fn one_bad_event_does_not_lose_the_good_ones() {
        let body = json!({ "Event": [
            { "guid": "bad", "pGuid": "cal-1", "startDate": "not-an-array" },
            { "guid": "no-identity", "title": "orphan" },
            { "guid": "good", "pGuid": "cal-1", "title": "Fine",
              "startDate": [20260912, 2026, 9, 12, 9, 0, 540], "duration": 30 }
        ]});
        let snap = parse_snapshot(&body);
        assert_eq!(snap.events.len(), 1);
        assert_eq!(snap.events[0].guid, "good");
    }

    #[test]
    fn an_empty_body_is_an_empty_snapshot() {
        let snap = parse_snapshot(&json!({}));
        assert!(snap.calendars.is_empty() && snap.events.is_empty());
    }
}
