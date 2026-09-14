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
pub(crate) fn parse_apple_date(raw: &Value, timezone: Option<&str>) -> Result<i64> {
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

/// The inverse of [`parse_apple_date`]: Apple's `[YYYYMMDD, y, m, d, h, min, minsSinceMidnight]`.
///
/// Writes must speak the same dialect they read, so this is what a completion timestamp or a
/// due date looks like going back.
pub(crate) fn format_apple_date(seconds: i64, timezone: Option<&str>) -> Value {
    let zone = timezone
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or(TimeZone::UTC);
    let zoned = jiff::Timestamp::from_second(seconds)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(zone);
    let (y, m, d) = (zoned.year(), zoned.month(), zoned.day());
    let (h, min) = (zoned.hour(), zoned.minute());
    let packed = i64::from(y) * 10_000 + i64::from(m) * 100 + i64::from(d);
    serde_json::json!([packed, y, m, d, h, min, i64::from(h) * 60 + i64::from(min)])
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

/// `YYYY-MM-DD` for the day an instant falls on, in the event's own timezone.
fn day_of(seconds: i64, timezone: Option<&str>) -> String {
    let zone = timezone
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or(TimeZone::UTC);
    jiff::Timestamp::from_second(seconds)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(zone)
        .strftime("%Y-%m-%d")
        .to_string()
}

/// What the web client tells the service it already has.
///
/// Sending the collection's ctag is how Apple decides whether to answer with the whole
/// calendar or just the change, which is the difference between a quick write and refetching
/// the month on every save.
fn client_state(calendar_guid: &str, ctag: Option<&str>) -> Value {
    serde_json::json!({
        "Collection": [{ "guid": calendar_guid, "ctag": ctag }],
        "fullState": false,
        "userTime": 0,
        "alarmRange": 60,
    })
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

    /// Create an event, or replace one that already exists.
    ///
    /// Apple's calendar service takes the whole event on every write rather than a patch, so
    /// an edit sends the event as it should end up. `guid` addresses it; a fresh one is a
    /// create and an existing one is an update.
    ///
    /// The window in the query is not decoration: Apple answers with the events in it, and
    /// sending the day the event falls on is what makes the response carry the event back so
    /// a caller can confirm what was stored rather than assume.
    /// Save an event, creating it where it has no guid yet.
    ///
    /// The calendar id is a path segment, so an empty one does not fail loudly — it builds
    /// `/ca/events//<guid>`, which is a different request entirely. An account with no
    /// calendars is a real account, so it is caught here rather than sent.
    pub async fn put_event(&self, event: &Event, ctag: Option<&str>) -> Result<Snapshot> {
        if event.calendar_guid.trim().is_empty() {
            return Err(Error::Protocol(
                "an event has to go in a calendar, and this account has none yet".into(),
            ));
        }
        let day = day_of(event.starts_at, event.timezone.as_deref());
        // What Apple already has, if this event exists. An event carries far more than this
        // app models — its alerts, its guests, its attachments, its link to the series it
        // repeats in — and Apple takes the whole event on every write. Building one from
        // our own fields alone did not merely drop those, it cleared them: a weekly meeting
        // edited here came back a one-off with no alerts and no guests.
        let existing = self.event_detail(event, &day).await;
        let body = serde_json::json!({
            "Event": self.event_body(event, existing.as_ref()),
            "ClientState": client_state(&event.calendar_guid, ctag),
        });
        let path = format!("/ca/events/{}/{}", event.calendar_guid, event.guid);
        let answer = self
            .post(&path, &body, &day, &day, event.timezone.as_deref(), None)
            .await?;
        Ok(parse_snapshot(&answer))
    }

    /// Remove an event.
    ///
    /// `etag` is what stops a delete from landing on a version of the event someone changed
    /// on another device in the meantime.
    pub async fn delete_event(&self, event: &Event, ctag: Option<&str>) -> Result<()> {
        if event.calendar_guid.trim().is_empty() {
            return Err(Error::Protocol("that event names no calendar".into()));
        }
        let day = day_of(event.starts_at, event.timezone.as_deref());
        let body = serde_json::json!({
            "Event": { "guid": event.guid, "pGuid": event.calendar_guid, "etag": event.etag },
            "ClientState": client_state(&event.calendar_guid, ctag),
        });
        let path = format!("/ca/events/{}/{}", event.calendar_guid, event.guid);
        self.post(
            &path,
            &body,
            &day,
            &day,
            event.timezone.as_deref(),
            Some("DELETE"),
        )
        .await?;
        Ok(())
    }

    /// Apple's own full record of an event, or `None` for one it has never heard of.
    ///
    /// Asked for before every write so an edit can be merged into what is really there.
    /// A failure here is not fatal — it only means there is nothing to merge into, which is
    /// exactly the case for a new event.
    async fn event_detail(&self, event: &Event, day: &str) -> Option<Value> {
        let mut query = self.params.clone();
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("lang".into(), "en-us".into()));
        query.push((
            "usertz".into(),
            event.timezone.clone().unwrap_or_else(|| "UTC".into()),
        ));
        query.push(("startDate".into(), day.to_string()));
        query.push(("endDate".into(), day.to_string()));

        let url = format!(
            "{}/ca/eventdetail/{}/{}",
            self.root, event.calendar_guid, event.guid
        );
        let response = self.http.get(url).query(&query).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
        body.pointer("/Event/0")
            .or_else(|| body.pointer("/Event"))
            .filter(|e| e.is_object())
            .cloned()
    }

    /// Apple's event object, in the shape its own web client sends.
    ///
    /// When Apple already has the event, its own record is the starting point and only the
    /// fields this app actually models are written over it — so alerts, guests,
    /// attachments, private comments and the dates linking it to a repeating series all
    /// survive an edit untouched. Only a genuinely new event is built from nothing.
    fn event_body(&self, event: &Event, existing: Option<&Value>) -> Value {
        let zone = event.timezone.as_deref();
        let minutes = ((event.ends_at - event.starts_at) / 60).max(0);

        let mut body = existing.cloned().unwrap_or_else(|| {
            // A new event has no alerts or guests to keep, so saying so is accurate here in
            // a way it never was for an edit.
            serde_json::json!({
                "icon": 0,
                "recurrence": Value::Null,
                "changeRecurring": Value::Null,
                "alarms": [],
                "invitees": [],
                "hasAttachments": false,
                "isJunk": false,
                "readOnly": false,
                "extendedDetailsAreIncluded": true,
            })
        });
        let object = body.as_object_mut().expect("an event is an object");

        for (key, value) in [
            ("guid", Value::String(event.guid.clone())),
            ("pGuid", Value::String(event.calendar_guid.clone())),
            ("title", Value::String(event.title.clone())),
            (
                "location",
                Value::String(event.location.clone().unwrap_or_default()),
            ),
            (
                "description",
                Value::String(event.notes.clone().unwrap_or_default()),
            ),
            ("url", event.url.clone().map_or(Value::Null, Value::String)),
            ("startDate", format_apple_date(event.starts_at, zone)),
            ("endDate", format_apple_date(event.ends_at, zone)),
            ("localStartDate", format_apple_date(event.starts_at, zone)),
            ("localEndDate", format_apple_date(event.ends_at, zone)),
            ("duration", Value::from(minutes)),
            ("allDay", Value::Bool(event.all_day)),
            (
                "tz",
                zone.map_or(Value::Null, |z| Value::String(z.to_string())),
            ),
            (
                "etag",
                event.etag.clone().map_or(Value::Null, Value::String),
            ),
        ] {
            object.insert(key.to_string(), value);
        }
        body
    }

    async fn post(
        &self,
        path: &str,
        body: &Value,
        from: &str,
        to: &str,
        tz: Option<&str>,
        method_override: Option<&str>,
    ) -> Result<Value> {
        let mut query = self.params.clone();
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("requestID".into(), "4".into()));
        query.push(("startDate".into(), from.into()));
        query.push(("endDate".into(), to.into()));
        query.push(("lang".into(), "en-us".into()));
        query.push(("usertz".into(), tz.unwrap_or("UTC").into()));
        if let Some(method) = method_override {
            query.push(("methodOverride".into(), method.into()));
        }

        let response = self
            .http
            .post(format!("{}{path}", self.root))
            .query(&query)
            .json(body)
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
                text.chars().take(200).collect::<String>()
            )));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str, from: &str, to: &str, tz: &str) -> Result<Value> {
        let mut query = self.params.clone();
        // Apple's calendar service returns an empty body unless the web client's own
        // versioning parameters are present. Without `clientVersion` it answers 200 with no
        // events at all, which looks exactly like an empty calendar.
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("requestID".into(), "4".into()));
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

    /// A date written back must read back as the same instant, or a completion would land on
    /// the wrong day.
    #[test]
    fn dates_survive_a_round_trip() {
        for zone in [Some("Asia/Kolkata"), Some("America/New_York"), None] {
            for seconds in [0_i64, 1_757_000_000, 1_788_888_888] {
                let encoded = format_apple_date(seconds, zone);
                let decoded = parse_apple_date(&encoded, zone).unwrap();
                // Apple's format carries no seconds, so equality is to the minute.
                assert_eq!(
                    decoded,
                    seconds - seconds.rem_euclid(60),
                    "{seconds} in {zone:?} did not survive"
                );
            }
        }
    }

    #[test]
    fn an_encoded_date_has_apples_shape() {
        let encoded = format_apple_date(1_757_000_000, Some("UTC"));
        let parts = encoded.as_array().unwrap();
        assert_eq!(parts.len(), 7);
        // First element packs the date as YYYYMMDD.
        let packed = parts[0].as_i64().unwrap();
        assert_eq!(packed / 10_000, parts[1].as_i64().unwrap(), "year");
        assert_eq!((packed / 100) % 100, parts[2].as_i64().unwrap(), "month");
        assert_eq!(packed % 100, parts[3].as_i64().unwrap(), "day");
        // Last element is minutes since midnight.
        assert_eq!(
            parts[6].as_i64().unwrap(),
            parts[4].as_i64().unwrap() * 60 + parts[5].as_i64().unwrap()
        );
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

#[cfg(test)]
mod write_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn event() -> Event {
        Event {
            guid: "E-1".into(),
            calendar_guid: "work".into(),
            title: "Dentist".into(),
            location: Some("Gachibowli".into()),
            // 2026-09-14T09:30:00+05:30
            starts_at: 1_789_358_400,
            ends_at: 1_789_362_000,
            all_day: false,
            timezone: Some("Asia/Kolkata".into()),
            url: None,
            notes: None,
            etag: None,
        }
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1",
                    "pGuid": "work",
                    "title": "Dentist",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "stored"
                }]
            })))
            .mount(&server)
            .await;
        server
    }

    /// The write itself, not the detail fetch that precedes it — a write reads what Apple
    /// already has so an edit can be merged into it rather than replace it.
    async fn request(server: &MockServer) -> Request {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.method == reqwest::Method::POST)
            .expect("a write was sent")
    }

    #[tokio::test]
    async fn a_write_addresses_the_event_by_calendar_and_guid() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), Some("ctag-1")).await.unwrap();
        assert_eq!(request(&server).await.url.path(), "/ca/events/work/E-1");
    }

    /// Apple answers with the events in the window it was given, so sending the event's own
    /// day is what makes the reply carry the stored event back.
    #[tokio::test]
    async fn the_window_is_the_day_the_event_falls_on_locally() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), None).await.unwrap();

        let sent = request(&server).await;
        let query: std::collections::HashMap<_, _> = sent.url.query_pairs().collect();
        // 04:00 UTC on the 14th is 09:30 on the 14th in Kolkata — the local day is what counts.
        assert_eq!(query["startDate"], "2026-09-14");
        assert_eq!(query["endDate"], "2026-09-14");
        assert_eq!(query["usertz"], "Asia/Kolkata");
    }

    #[tokio::test]
    async fn the_whole_event_is_sent_in_apples_own_shape() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), Some("ctag-1")).await.unwrap();

        let sent = request(&server).await;
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        let e = &body["Event"];
        assert_eq!(e["title"], "Dentist");
        assert_eq!(e["location"], "Gachibowli");
        assert_eq!(e["duration"], 60, "duration is minutes, not seconds");
        assert_eq!(e["tz"], "Asia/Kolkata");
        // The date is Apple's component array in the event's own zone, not a timestamp.
        assert_eq!(
            e["startDate"],
            serde_json::json!([20260914, 2026, 9, 14, 9, 30, 570])
        );
        assert_eq!(body["ClientState"]["Collection"][0]["ctag"], "ctag-1");
    }

    /// What comes back is what gets stored — never what was sent.
    #[tokio::test]
    async fn the_stored_event_is_read_from_apples_reply() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let answer = client.put_event(&event(), None).await.unwrap();
        assert_eq!(answer.events.len(), 1);
        assert_eq!(answer.events[0].etag.as_deref(), Some("stored"));
    }

    #[tokio::test]
    async fn deleting_says_so_in_the_query_and_carries_the_etag() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut gone = event();
        gone.etag = Some("v3".into());
        client.delete_event(&gone, None).await.unwrap();

        let sent = request(&server).await;
        let query: std::collections::HashMap<_, _> = sent.url.query_pairs().collect();
        assert_eq!(query["methodOverride"], "DELETE");
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body["Event"]["etag"], "v3",
            "a delete must not race another device"
        );
    }

    /// An all-day event has no meaningful duration, and a negative one is nonsense.
    #[tokio::test]
    async fn a_zero_length_event_reports_zero_minutes() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut instant = event();
        instant.ends_at = instant.starts_at;
        client.put_event(&instant, None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        assert_eq!(body["Event"]["duration"], 0);
    }

    /// The one that matters. An event carries alerts, guests, attachments and the dates
    /// linking it to the series it repeats in; Apple takes the whole event on every write.
    /// Building one from our own fields alone did not merely drop those, it cleared them —
    /// a weekly meeting edited here came back a one-off with no alerts and no guests.
    #[tokio::test]
    async fn an_edit_keeps_the_alerts_guests_and_repeat_apple_holds() {
        let server = MockServer::start().await;
        // Apple's own record of the event, with everything this app does not model.
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1",
                    "pGuid": "work",
                    "title": "Dentist",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "v1",
                    "alarms": [{ "messageType": "message", "measurement": { "before": true, "minutes": 15 } }],
                    "invitees": [{ "email": "someone@example.invalid" }],
                    "attachments": [{ "name": "form.pdf" }],
                    "privateComments": [{ "comment": "bring the referral" }],
                    "recurrenceMaster": true,
                    "masterStartDate": [20260901, 2026, 9, 1, 9, 30, 570],
                    "transparent": true
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut renamed = event();
        renamed.title = "Dentist — moved".into();
        client.put_event(&renamed, None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        let sent = &body["Event"];

        assert_eq!(sent["title"], "Dentist — moved", "the change lands");
        // And nothing else was taken away with it.
        assert_eq!(
            sent["alarms"][0]["measurement"]["minutes"], 15,
            "alerts survive"
        );
        assert_eq!(
            sent["invitees"][0]["email"], "someone@example.invalid",
            "guests survive"
        );
        assert_eq!(
            sent["attachments"][0]["name"], "form.pdf",
            "attachments survive"
        );
        assert_eq!(
            sent["privateComments"][0]["comment"], "bring the referral",
            "private comments survive"
        );
        assert_eq!(sent["recurrenceMaster"], true, "the repeat survives");
        assert!(
            sent.get("masterStartDate").is_some(),
            "its series link survives"
        );
        assert_eq!(sent["transparent"], true, "free/busy survives");
    }

    /// A genuinely new event has no alerts or guests to keep, so saying so is accurate
    /// there in a way it never was for an edit.
    #[tokio::test]
    async fn a_new_event_is_still_built_from_nothing() {
        let server = MockServer::start().await;
        // Apple has never heard of it: no detail comes back.
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        assert_eq!(body["Event"]["title"], "Dentist");
        assert_eq!(body["Event"]["alarms"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn a_rejection_reads_as_a_sentence() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad event"))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let message = client
            .put_event(&event(), None)
            .await
            .unwrap_err()
            .to_string();
        assert!(message.contains("400"), "{message}");
        assert!(message.contains("bad event"), "{message}");
    }
}
