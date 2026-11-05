//! Calendars subscribed to by address — a school's timetable, a team's fixtures, a trip — as
//! Calendar's New Calendar Subscription reads them: an iCalendar file fetched from where it is
//! published, read-only, refreshed now and then, its repeating events laid out day by day.
//!
//! Nothing here is Apple's. The file is fetched with a client that can reach the hosts the
//! person subscribed to and nothing else (`quince_auth::listed_only`), and only once they have.

use crate::{zones, Error, Result};
use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan};
use quince_core::calendar::{Calendar, Event};
use serde::{Deserialize, Serialize};

/// Where a subscription's calendar guid begins; the rest is the subscription's own id.
pub const PREFIX: &str = "quince:sub:";

/// The largest file a subscription is read from. A sports season is a few hundred kilobytes;
/// a server sending more than this is sending something else.
pub const MOST_BYTES: usize = 25 * 1024 * 1024;

/// One calendar subscribed to by address, as it is kept on this computer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub id: String,
    /// The address it is fetched from, `webcal://` already read as `https://`.
    pub url: String,
    pub title: String,
    pub colour: String,
    /// How often it is fetched again, in minutes; 0 for never on its own.
    pub refresh_minutes: u32,
    /// When it was last fetched whole, unix seconds.
    #[serde(default)]
    pub fetched_at: Option<i64>,
    /// Why the last fetch did not land, in words; cleared by the next that does.
    #[serde(default)]
    pub problem: Option<String>,
}

impl Subscription {
    /// The calendar its events are drawn in.
    #[must_use]
    pub fn calendar_guid(&self) -> String {
        format!("{PREFIX}{}", self.id)
    }

    /// As the sidebar lists it: read-only, after the account's own calendars.
    #[must_use]
    pub fn calendar(&self, order: i64) -> Calendar {
        Calendar {
            guid: self.calendar_guid(),
            title: self.title.clone(),
            colour: Some(self.colour.clone()),
            ctag: None,
            sort_order: 800 + order,
            enabled: true,
            writable: false,
            is_default: false,
        }
    }

    /// Whether it is due a fetch at `now`.
    #[must_use]
    pub fn due(&self, now: i64) -> bool {
        self.refresh_minutes > 0
            && self
                .fetched_at
                .is_none_or(|t| now - t >= i64::from(self.refresh_minutes) * 60)
    }
}

/// An address as typed, made one that can be fetched: `webcal://` is `https://`, as Calendar
/// reads it; a bare `example.com/a.ics` is `https://` too. Only web addresses are taken.
pub fn normalise_url(raw: &str) -> Result<reqwest::Url> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let fixed = if let Some(rest) = lower
        .strip_prefix("webcals://")
        .or_else(|| lower.strip_prefix("webcal://"))
    {
        format!("https://{}", &raw[raw.len() - rest.len()..])
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        raw.to_string()
    } else if raw.contains("://") {
        return Err(Error::Refused(
            "That address isn’t a web address. Use one that begins with webcal:// or https://."
                .into(),
        ));
    } else {
        format!("https://{raw}")
    };
    let url = reqwest::Url::parse(&fixed).map_err(|_| {
        Error::Refused("That address can’t be read. Check it and try again.".into())
    })?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(Error::Refused(
            "That address has no server in it. Check it and try again.".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Refused(
            "Subscriptions are to public calendars; that address carries a password.".into(),
        ));
    }
    Ok(url)
}

/// The host an address names, as the subscription client lists it.
#[must_use]
pub fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Fetch a subscription's file. The client is the one that reaches subscribed hosts alone.
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<String> {
    let mut response = http.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Refused(match status.as_u16() {
            401 | 403 => "That calendar is private. Subscriptions are to public calendars.".into(),
            404 | 410 => "There’s no calendar at that address any more.".into(),
            _ => format!(
                "The server didn’t send the calendar (it answered {}).",
                status.as_u16()
            ),
        }));
    }
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MOST_BYTES {
            return Err(Error::Refused(
                "That file is too large to be a calendar.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = if bytes.starts_with(&[0x1f, 0x8b]) {
        // Within the same ceiling as the file itself (crate::bounded): too large is refused,
        // never read cut short as if whole.
        let out = crate::bounded::gunzip(&bytes, MOST_BYTES as u64)
            .ok_or_else(|| Error::Refused("That calendar file couldn’t be opened.".into()))?;
        String::from_utf8_lossy(&out).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(Error::Refused(
            "There’s no calendar at that address — it sent a web page or another file.".into(),
        ));
    }
    Ok(text)
}

/* ---- reading the file --------------------------------------------------------------- */

#[derive(Debug, Clone)]
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }
    lines
}

/// `NAME;P=v;Q="a:b":value`, a colon inside a quoted parameter not ending the name.
fn property(line: &str) -> Option<Property> {
    let mut quoted = false;
    let mut split = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                split = Some(i);
                break;
            }
            _ => {}
        }
    }
    let at = split?;
    let (head, value) = (&line[..at], &line[at + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect();
    Some(Property {
        name,
        params,
        value: value.to_string(),
    })
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// When something happens, as the file writes it.
#[derive(Debug, Clone, PartialEq)]
enum When {
    /// A date: an all-day event.
    Day(Date),
    /// A clock time in a zone (UTC for a time written with `Z`).
    At(DateTime, TimeZone),
}

fn parse_date(s: &str) -> Option<Date> {
    let s = s.trim();
    if s.len() < 8 || !s.as_bytes()[..8].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Date::new(
        s[0..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..8].parse().ok()?,
    )
    .ok()
}

fn parse_datetime(s: &str) -> Option<(DateTime, bool)> {
    let s = s.trim();
    let (d, t) = s.split_once(['T', 't'])?;
    let date = parse_date(d)?;
    let utc = t.ends_with(['Z', 'z']);
    let t = t.trim_end_matches(['Z', 'z']);
    if t.len() < 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let second: i8 = t.get(4..6).map_or(Some(0), |x| x.parse().ok())?;
    let time = Time::new(
        t[0..2].parse().ok()?,
        t[2..4].parse().ok()?,
        second.min(59),
        0,
    )
    .ok()?;
    Some((date.to_datetime(time), utc))
}

/// What a file's times are read against: this computer's clock for a floating time, and the
/// zones the file itself defines in its `VTIMEZONE`s.
struct Clock<'a> {
    here: &'a TimeZone,
    zones: &'a zones::Defined,
}

/// A date or date-time property, read in `here` when it floats (no zone given).
fn when_of(p: &Property, here: &Clock) -> Option<When> {
    when_value(&p.value, p, here)
}

fn when_value(value: &str, p: &Property, here: &Clock) -> Option<When> {
    let date_only = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || (value.trim().len() == 8 && !value.contains(['T', 't']));
    if date_only {
        return parse_date(value).map(When::Day);
    }
    let (dt, utc) = parse_datetime(value)?;
    let zone = if utc {
        TimeZone::UTC
    } else {
        p.param("TZID")
            .and_then(|tzid| here.zones.zone(tzid))
            .map_or_else(|| here.here.clone(), |z| z.tz)
    };
    Some(When::At(dt, zone))
}

/// An iCalendar duration in seconds: `PT1H30M`, `-P1D`, `P2W`.
fn duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    let mut any = false;
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => number.push(c),
            'T' => in_time = true,
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                any = true;
                let unit: i64 = match (unit, in_time) {
                    ('W', _) => 7 * 86_400,
                    ('D', _) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                // A feed's number can be anything: one too large is no duration, not a crash.
                total = total.checked_add(n.checked_mul(unit)?)?;
            }
        }
    }
    (any && number.is_empty()).then_some(sign * total)
}

/// How an event repeats: the parts of an `RRULE` people publish.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<i64>,
    until: Option<When>,
    /// `BYDAY`: a weekday, and for a month or year which one of it (2nd, last = -1); 0 for every.
    by_day: Vec<(i8, Weekday)>,
    by_month_day: Vec<i8>,
    by_month: Vec<i8>,
    by_set_pos: Vec<i32>,
    week_start: Weekday,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

fn weekday_of(code: &str) -> Option<Weekday> {
    Some(match code.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Monday,
        "TU" => Weekday::Tuesday,
        "WE" => Weekday::Wednesday,
        "TH" => Weekday::Thursday,
        "FR" => Weekday::Friday,
        "SA" => Weekday::Saturday,
        "SU" => Weekday::Sunday,
        _ => return None,
    })
}

fn rule_of(p: &Property, here: &Clock) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        by_set_pos: Vec::new(),
        week_start: Weekday::Monday,
    };
    let mut freq = None;
    for part in p.value.split(';') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let list = || v.split(',').map(str::trim).filter(|s| !s.is_empty());
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match v.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    // Hourly and finer: not a calendar's kind of repeat; shown once.
                    _ => return None,
                });
            }
            "INTERVAL" => rule.interval = v.trim().parse().ok().filter(|n: &i64| *n > 0)?,
            "COUNT" => rule.count = v.trim().parse().ok(),
            "UNTIL" => {
                let bare = Property {
                    name: "UNTIL".into(),
                    params: Vec::new(),
                    value: v.trim().into(),
                };
                rule.until = when_of(&bare, here);
            }
            "BYDAY" => {
                for d in list() {
                    // The day is the last two letters; anything else there (`é1`) is no day,
                    // and never a cut through the middle of a character.
                    let split = d.len().saturating_sub(2);
                    let (Some(n), Some(code)) = (d.get(..split), d.get(split..)) else {
                        return None;
                    };
                    let n: i8 = if n.is_empty() || n == "+" {
                        0
                    } else {
                        n.trim_start_matches('+').parse().ok()?
                    };
                    rule.by_day.push((n, weekday_of(code)?));
                }
            }
            "BYMONTHDAY" => rule.by_month_day = list().filter_map(|d| d.parse().ok()).collect(),
            "BYMONTH" => rule.by_month = list().filter_map(|d| d.parse().ok()).collect(),
            "BYSETPOS" => rule.by_set_pos = list().filter_map(|d| d.parse().ok()).collect(),
            "WKST" => rule.week_start = weekday_of(v.trim()).unwrap_or(Weekday::Monday),
            _ => {}
        }
    }
    rule.freq = freq?;
    Some(rule)
}

/// One event in a subscribed file, before its repeats are laid out.
#[derive(Debug, Clone)]
struct Item {
    uid: String,
    title: String,
    location: Option<String>,
    notes: Option<String>,
    url: Option<String>,
    start: When,
    /// Seconds for a timed event; whole days for an all-day one.
    length: i64,
    rule: Option<Rule>,
    extra: Vec<When>,
    except: Vec<When>,
    /// For a changed occurrence: the start of the one it replaces.
    replaces: Option<When>,
    cancelled: bool,
}

/// A subscribed calendar's file, read.
#[derive(Debug, Clone, Default)]
pub struct Feed {
    /// `X-WR-CALNAME`: what the publisher calls it.
    pub name: Option<String>,
    /// `X-APPLE-CALENDAR-COLOR`, when the publisher gave one.
    pub colour: Option<String>,
    /// How often the publisher asks to be fetched, in minutes (`REFRESH-INTERVAL`,
    /// `X-PUBLISHED-TTL`).
    pub refresh_minutes: Option<u32>,
    items: Vec<Item>,
}

impl Feed {
    /// How many events it holds, a repeating one counted once.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.iter().filter(|i| i.replaces.is_none()).count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Read a subscribed calendar's file. Events that cannot be read are left out, not the file;
/// a file with no calendar in it is an error. Floating times are read on `here`'s clock.
pub fn parse(text: &str, here: &TimeZone) -> Result<Feed> {
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(Error::Refused("That isn’t a calendar file.".into()));
    }
    let zones = zones::Defined::read(text);
    let here = &Clock {
        here,
        zones: &zones,
    };
    let mut feed = Feed::default();
    // Nesting: VCALENDAR > VEVENT > VALARM. Only a VEVENT's own lines are its own.
    let mut stack: Vec<String> = Vec::new();
    let mut current: Vec<Property> = Vec::new();
    for line in unfold(text) {
        let Some(p) = property(&line) else { continue };
        match p.name.as_str() {
            "BEGIN" => {
                let what = p.value.trim().to_ascii_uppercase();
                if what == "VEVENT" {
                    current.clear();
                }
                stack.push(what);
            }
            "END" => {
                let what = p.value.trim().to_ascii_uppercase();
                if what == "VEVENT" && stack.last().map(String::as_str) == Some("VEVENT") {
                    if let Some(item) = item_of(&current, here) {
                        feed.items.push(item);
                    }
                }
                stack.pop();
            }
            _ => match stack.last().map(String::as_str) {
                Some("VEVENT") => current.push(p),
                Some("VCALENDAR") => match p.name.as_str() {
                    "X-WR-CALNAME" => {
                        let name = unescape(p.value.trim());
                        if !name.is_empty() {
                            feed.name = Some(name);
                        }
                    }
                    "X-APPLE-CALENDAR-COLOR" => {
                        let c = p.value.trim();
                        // Hex only: a feed is anyone's file, and slicing a colour with a
                        // multi-byte character in it ("#12345€") panicked mid-character.
                        if c.starts_with('#')
                            && (c.len() == 7 || c.len() == 9)
                            && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
                        {
                            feed.colour = Some(c[..7].to_ascii_lowercase());
                        }
                    }
                    "REFRESH-INTERVAL" | "X-PUBLISHED-TTL" => {
                        feed.refresh_minutes = duration(&p.value)
                            .filter(|s| *s > 0)
                            .map(|s| u32::try_from(s / 60).unwrap_or(u32::MAX).max(15));
                    }
                    _ => {}
                },
                _ => {}
            },
        }
    }
    Ok(feed)
}

fn item_of(props: &[Property], here: &Clock) -> Option<Item> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    let text = |name: &str| {
        get(name)
            .map(|p| unescape(p.value.trim()))
            .filter(|s| !s.is_empty())
    };
    let start = when_of(get("DTSTART")?, here)?;
    let length = match (&start, get("DTEND").and_then(|p| when_of(p, here))) {
        (When::Day(a), Some(When::Day(b))) => (b - *a).get_days().max(1).into(),
        (When::Day(_), _) => get("DURATION")
            .and_then(|p| duration(&p.value))
            .map_or(1, |s| (s / 86_400).max(1)),
        (When::At(..), Some(end @ When::At(..))) => {
            (instant(&end)?.as_second() - instant(&start)?.as_second()).max(0)
        }
        (When::At(..), _) => get("DURATION")
            .and_then(|p| duration(&p.value))
            .unwrap_or(0)
            .max(0),
    };
    let many = |name: &str| -> Vec<When> {
        props
            .iter()
            .filter(|p| p.name == name)
            .flat_map(|p| {
                p.value
                    .split(',')
                    .filter_map(|v| when_value(v, p, here))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    Some(Item {
        uid: text("UID").unwrap_or_else(|| format!("{:?}", start)),
        title: text("SUMMARY").unwrap_or_else(|| "Untitled".into()),
        location: text("LOCATION"),
        notes: text("DESCRIPTION"),
        url: text("URL"),
        rule: get("RRULE").and_then(|p| rule_of(p, here)),
        extra: many("RDATE"),
        except: many("EXDATE"),
        replaces: get("RECURRENCE-ID").and_then(|p| when_of(p, here)),
        cancelled: text("STATUS").is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED")),
        start,
        length,
    })
}

fn instant(w: &When) -> Option<Timestamp> {
    match w {
        When::Day(d) => d.to_zoned(TimeZone::system()).ok().map(|z| z.timestamp()),
        When::At(dt, tz) => dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp()),
    }
}

/// The same moment for matching an exception or a changed occurrence to the one it is for:
/// a date by its date, a time by its instant.
fn key_of(w: &When) -> Option<i64> {
    match w {
        When::Day(d) => {
            Some(i64::from(d.year()) * 10_000 + i64::from(d.month()) * 100 + i64::from(d.day()))
        }
        When::At(..) => instant(w).map(|t| t.as_second()),
    }
}

/* ---- laying out repeats ---------------------------------------------------------------- */

/// The most periods a rule is walked through: a daily rule from 1970 reaches today in under
/// 21,000, and a broken one never runs away.
const MOST_PERIODS: i64 = 40_000;

fn nth_weekday_in(dates: &[Date], n: i8, wd: Weekday) -> Vec<Date> {
    let all: Vec<Date> = dates
        .iter()
        .copied()
        .filter(|d| d.weekday() == wd)
        .collect();
    if n == 0 {
        return all;
    }
    let i = if n > 0 {
        usize::try_from(n - 1).ok()
    } else {
        all.len().checked_sub(usize::from(n.unsigned_abs()))
    };
    i.and_then(|i| all.get(i).copied()).into_iter().collect()
}

fn month_dates(year: i16, month: i8) -> Vec<Date> {
    let Ok(first) = Date::new(year, month, 1) else {
        return Vec::new();
    };
    (0..first.days_in_month())
        .filter_map(|i| first.checked_add(i64::from(i).days()).ok())
        .collect()
}

fn month_day(year: i16, month: i8, day: i8) -> Option<Date> {
    let first = Date::new(year, month, 1).ok()?;
    let n = first.days_in_month();
    let d = if day > 0 { day } else { n + 1 + day };
    (1..=n)
        .contains(&d)
        .then(|| Date::new(year, month, d).ok())
        .flatten()
}

/// A span of `n` days, months or years, or none where the calendar has no such span: a
/// feed's `INTERVAL` is anyone's number, and jiff's own `.days()` panics past its range.
fn days(n: i64) -> Option<Span> {
    Span::new().try_days(n).ok()
}

/// The dates one period of a rule holds, in order. None at all past the calendar's range.
fn period_dates(rule: &Rule, first: Date, k: i64) -> Vec<Date> {
    let Some(step) = k.checked_mul(rule.interval) else {
        return Vec::new();
    };
    let mut out: Vec<Date> = match rule.freq {
        Freq::Daily => days(step)
            .and_then(|span| first.checked_add(span).ok())
            .into_iter()
            .filter(|d| {
                rule.by_day.is_empty() || rule.by_day.iter().any(|(_, w)| *w == d.weekday())
            })
            .collect(),
        Freq::Weekly => {
            let back = i64::from(
                (first.weekday().to_monday_zero_offset() - rule.week_start.to_monday_zero_offset())
                    .rem_euclid(7),
            );
            let Some(Ok(week)) = step
                .checked_mul(7)
                .and_then(|d| d.checked_sub(back))
                .and_then(days)
                .map(|span| first.checked_add(span))
            else {
                return Vec::new();
            };
            if rule.by_day.is_empty() {
                week.checked_add(back.days()).ok().into_iter().collect()
            } else {
                (0..7)
                    .filter_map(|i| week.checked_add(i.days()).ok())
                    .filter(|d| rule.by_day.iter().any(|(_, w)| *w == d.weekday()))
                    .collect()
            }
        }
        Freq::Monthly => {
            let Some(Ok(m)) = Span::new()
                .try_months(step)
                .ok()
                .map(|span| first.first_of_month().checked_add(span))
            else {
                return Vec::new();
            };
            month_set(rule, m.year(), m.month(), first.day())
        }
        Freq::Yearly => {
            let Ok(y) = Date::new(first.year(), 1, 1).and_then(|d| {
                Span::new()
                    .try_years(step)
                    .and_then(|span| d.checked_add(span))
            }) else {
                return Vec::new();
            };
            let year = y.year();
            if rule.by_month.is_empty() && !rule.by_day.is_empty() && rule.by_month_day.is_empty() {
                // The nth weekday of the year.
                let days: Vec<Date> = (1..=12).flat_map(|m| month_dates(year, m)).collect();
                rule.by_day
                    .iter()
                    .flat_map(|(n, w)| nth_weekday_in(&days, *n, *w))
                    .collect()
            } else {
                let months: Vec<i8> = if rule.by_month.is_empty() {
                    vec![first.month()]
                } else {
                    rule.by_month.clone()
                };
                months
                    .into_iter()
                    .flat_map(|m| month_set(rule, year, m, first.day()))
                    .collect()
            }
        }
    };
    if !rule.by_month.is_empty() && rule.freq != Freq::Yearly {
        out.retain(|d| rule.by_month.contains(&d.month()));
    }
    out.sort();
    out.dedup();
    if !rule.by_set_pos.is_empty() {
        let len = i32::try_from(out.len()).unwrap_or(0);
        let picked = rule
            .by_set_pos
            .iter()
            .filter_map(|p| {
                let i = if *p > 0 { p - 1 } else { len + p };
                usize::try_from(i).ok().and_then(|i| out.get(i).copied())
            })
            .collect::<Vec<_>>();
        out = picked;
        out.sort();
    }
    out
}

/// A month's dates under a rule's BYDAY and BYMONTHDAY (both: the days that are both, as
/// Friday the 13th); neither, the first date's day of the month — skipped where there is none.
fn month_set(rule: &Rule, year: i16, month: i8, day: i8) -> Vec<Date> {
    let by_day = |dates: &[Date]| -> Vec<Date> {
        rule.by_day
            .iter()
            .flat_map(|(n, w)| nth_weekday_in(dates, *n, *w))
            .collect()
    };
    match (rule.by_day.is_empty(), rule.by_month_day.is_empty()) {
        (true, true) => month_day(year, month, day).into_iter().collect(),
        (true, false) => rule
            .by_month_day
            .iter()
            .filter_map(|d| month_day(year, month, *d))
            .collect(),
        (false, true) => by_day(&month_dates(year, month)),
        (false, false) => {
            let days: Vec<Date> = rule
                .by_month_day
                .iter()
                .filter_map(|d| month_day(year, month, *d))
                .collect();
            by_day(&month_dates(year, month))
                .into_iter()
                .filter(|d| days.contains(d))
                .collect()
        }
    }
}

/// The starts of an item's occurrences, as `When`s, up to `to` (unix seconds).
fn starts(item: &Item, to: i64) -> Vec<When> {
    let Some(rule) = &item.rule else {
        let mut all = vec![item.start.clone()];
        all.extend(item.extra.iter().cloned());
        return all;
    };
    let (first, at) = match &item.start {
        When::Day(d) => (*d, None),
        When::At(dt, tz) => (dt.date(), Some((dt.time(), tz.clone()))),
    };
    let make = |d: Date| match &at {
        None => When::Day(d),
        Some((t, tz)) => When::At(d.to_datetime(*t), tz.clone()),
    };
    let until = rule.until.as_ref().and_then(|u| match u {
        When::Day(d) => Some(Err(*d)),
        When::At(..) => instant(u).map(|t| Ok(t.as_second())),
    });
    let first_key = instant(&item.start).map_or(i64::MIN, |t| t.as_second());
    let mut out = Vec::new();
    let mut n = 0i64;
    'walk: for k in 0..MOST_PERIODS {
        let dates = period_dates(rule, first, k);
        let mut beyond = false;
        for d in dates {
            if d < first {
                continue;
            }
            let w = make(d);
            let Some(t) = instant(&w).map(|t| t.as_second()) else {
                continue;
            };
            if t < first_key {
                continue;
            }
            let past_until = match until {
                Some(Err(day)) => d > day,
                Some(Ok(u)) => t > u,
                None => false,
            };
            if past_until {
                break 'walk;
            }
            n += 1;
            if rule.count.is_some_and(|c| n > c) {
                break 'walk;
            }
            if t >= to {
                beyond = true;
                break;
            }
            out.push(w);
        }
        // The first date counts even where a rule's days would not hold it, as RFC 5545 has it.
        if k == 0
            && out.first() != Some(&item.start)
            && instant(&item.start).is_some_and(|t| t.as_second() < to)
        {
            out.insert(0, item.start.clone());
            if rule.count.is_some() {
                n += 1;
            }
        }
        if beyond {
            break;
        }
    }
    out.extend(item.extra.iter().cloned());
    out
}

/// The feed's events overlapping `[from, to)`, as the calendar `calendar_guid` holds them —
/// each repeat laid out on its own day, exceptions left out and changed ones put in their place.
#[must_use]
pub fn events(feed: &Feed, calendar_guid: &str, from: i64, to: i64) -> Vec<Event> {
    use std::collections::{HashMap, HashSet};
    // Changed occurrences, by the series and the start they replace.
    let mut moved: HashMap<&str, HashSet<i64>> = HashMap::new();
    for i in feed.items.iter().filter(|i| i.replaces.is_some()) {
        if let Some(k) = i.replaces.as_ref().and_then(key_of) {
            moved.entry(i.uid.as_str()).or_default().insert(k);
        }
    }
    let mut out = Vec::new();
    for item in &feed.items {
        // A changed occurrence is one event of its own; a cancelled one is no event.
        let occurrences = if item.replaces.is_some() {
            vec![item.start.clone()]
        } else {
            starts(item, to)
        };
        let except: HashSet<i64> = item.except.iter().filter_map(key_of).collect();
        let replaced = moved.get(item.uid.as_str());
        for w in occurrences {
            let Some(key) = key_of(&w) else { continue };
            if item.replaces.is_none()
                && (except.contains(&key) || replaced.is_some_and(|r| r.contains(&key)))
            {
                continue;
            }
            if item.cancelled {
                continue;
            }
            let Some(event) = event_of(item, &w, calendar_guid, key) else {
                continue;
            };
            if event.starts_at < to && event.ends_at > from {
                out.push(event);
            }
        }
    }
    out.sort_by(|a, b| {
        a.starts_at
            .cmp(&b.starts_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

fn event_of(item: &Item, w: &When, calendar_guid: &str, key: i64) -> Option<Event> {
    let (starts_at, ends_at, all_day, timezone) = match w {
        When::Day(d) => {
            let zone = TimeZone::system();
            let start = d.to_zoned(zone.clone()).ok()?;
            let end = d
                .checked_add(days(item.length.max(1))?)
                .ok()?
                .to_zoned(zone)
                .ok()?;
            (
                start.timestamp().as_second(),
                end.timestamp().as_second(),
                true,
                None,
            )
        }
        When::At(dt, tz) => {
            let start = dt.to_zoned(tz.clone()).ok()?.timestamp().as_second();
            (
                start,
                start.checked_add(item.length)?,
                false,
                tz.iana_name().map(ToString::to_string),
            )
        }
    };
    Some(Event {
        guid: format!("{calendar_guid}:{}:{key}", item.uid),
        calendar_guid: calendar_guid.into(),
        title: item.title.clone(),
        location: item.location.clone(),
        starts_at,
        ends_at,
        all_day,
        timezone,
        url: item.url.clone(),
        notes: item.notes.clone(),
        etag: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A colour that is not hex — non-ASCII included — is ignored, never a panic.
    #[test]
    fn a_feed_colour_that_is_not_hex_is_ignored() {
        for bad in [
            "#12345\u{20ac}",
            "#zzzzzz",
            "#12\u{e9}456",
            "#1234567\u{20ac}",
        ] {
            let ics =
                format!("BEGIN:VCALENDAR\r\nX-APPLE-CALENDAR-COLOR:{bad}\r\nEND:VCALENDAR\r\n");
            let _ = parse(&ics, &jiff::tz::TimeZone::UTC);
        }
    }

    fn kolkata() -> TimeZone {
        TimeZone::get("Asia/Kolkata").unwrap()
    }

    fn at(zone: &str, s: &str) -> i64 {
        let (dt, _) = parse_datetime(s).unwrap();
        dt.to_zoned(TimeZone::get(zone).unwrap())
            .unwrap()
            .timestamp()
            .as_second()
    }

    fn wrap(body: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-CALNAME:Fixtures\r\nX-APPLE-CALENDAR-COLOR:#FF9500FF\r\nREFRESH-INTERVAL;VALUE=DURATION:PT6H\r\n{body}END:VCALENDAR\r\n"
        )
    }

    fn year(y: i16) -> (i64, i64) {
        let z = TimeZone::get("UTC").unwrap();
        let s = |y| {
            Date::new(y, 1, 1)
                .unwrap()
                .to_zoned(z.clone())
                .unwrap()
                .timestamp()
                .as_second()
        };
        (s(y), s(y + 1))
    }

    #[test]
    fn webcal_reads_as_https_and_only_web_addresses_are_taken() {
        assert_eq!(
            normalise_url(" webcal://p01-calendars.example.com/a.ics")
                .unwrap()
                .as_str(),
            "https://p01-calendars.example.com/a.ics"
        );
        assert_eq!(
            normalise_url("WEBCALS://Example.com/x").unwrap().scheme(),
            "https"
        );
        assert_eq!(
            normalise_url("example.org/team.ics").unwrap().as_str(),
            "https://example.org/team.ics"
        );
        assert!(normalise_url("ftp://example.org/a.ics").is_err());
        assert!(normalise_url("file:///etc/passwd").is_err());
        assert!(normalise_url("https://me:secret@example.org/a.ics").is_err());
        assert_eq!(
            host_of("https://Cal.Example.org:8443/a").as_deref(),
            Some("cal.example.org")
        );
    }

    #[test]
    fn the_name_colour_and_refresh_come_from_the_file() {
        let feed = parse(&wrap(""), &kolkata()).unwrap();
        assert_eq!(feed.name.as_deref(), Some("Fixtures"));
        assert_eq!(feed.colour.as_deref(), Some("#ff9500"));
        assert_eq!(feed.refresh_minutes, Some(360));
        assert!(parse("<html>", &kolkata()).is_err());
    }

    #[test]
    fn a_weekly_class_keeps_its_clock_time_across_a_clock_change() {
        let text = wrap("BEGIN:VEVENT\r\nUID:gym\r\nSUMMARY:Gym\r\nDTSTART;TZID=America/New_York:20261026T070000\r\nDTEND;TZID=America/New_York:20261026T080000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=4\r\nEND:VEVENT\r\n");
        let feed = parse(&text, &kolkata()).unwrap();
        let (from, to) = year(2026);
        let got = events(&feed, "quince:sub:x", from, to);
        assert_eq!(got.len(), 4);
        // 26 Oct is daylight time, 2 Nov standard: 7:00 on the clock both days.
        assert_eq!(got[0].starts_at, at("America/New_York", "20261026T070000"));
        assert_eq!(got[1].starts_at, at("America/New_York", "20261102T070000"));
        assert_eq!(got[1].ends_at - got[1].starts_at, 3600);
        assert_eq!(got[3].timezone.as_deref(), Some("America/New_York"));
        assert!(got.iter().all(|e| e.calendar_guid == "quince:sub:x"));
        let guids: std::collections::HashSet<_> = got.iter().map(|e| &e.guid).collect();
        assert_eq!(guids.len(), 4, "every occurrence is its own event");
    }

    #[test]
    fn exceptions_are_left_out_and_a_moved_occurrence_takes_its_place() {
        let text = wrap(concat!(
            "BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Standup\r\nDTSTART:20260105T040000Z\r\nDURATION:PT15M\r\n",
            "RRULE:FREQ=DAILY;UNTIL=20260109T040000Z\r\nEXDATE:20260107T040000Z\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20260108T040000Z\r\nSUMMARY:Standup (late)\r\n",
            "DTSTART:20260108T060000Z\r\nDTEND:20260108T061500Z\r\nEND:VEVENT\r\n",
        ));
        let feed = parse(&text, &kolkata()).unwrap();
        let (from, to) = year(2026);
        let got = events(&feed, "c", from, to);
        let titles: Vec<_> = got
            .iter()
            .map(|e| (e.title.as_str(), e.starts_at))
            .collect();
        assert_eq!(titles.len(), 4, "{titles:?}");
        assert!(!got
            .iter()
            .any(|e| e.starts_at == at("UTC", "20260107T040000")));
        assert!(got
            .iter()
            .any(|e| e.title == "Standup (late)" && e.starts_at == at("UTC", "20260108T060000")));
        assert!(!got
            .iter()
            .any(|e| e.title == "Standup" && e.starts_at == at("UTC", "20260108T040000")));
        assert_eq!(feed.len(), 1);
    }

    #[test]
    fn monthly_by_weekday_and_by_last_day() {
        let text = wrap(concat!(
            "BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Board\r\nDTSTART;VALUE=DATE:20260112\r\n",
            "RRULE:FREQ=MONTHLY;BYDAY=2MO;COUNT=3\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Rent\r\nDTSTART;VALUE=DATE:20260131\r\n",
            "RRULE:FREQ=MONTHLY;BYMONTHDAY=-1;COUNT=3\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:Payday\r\nDTSTART;VALUE=DATE:20260130\r\n",
            "RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=2\r\nEND:VEVENT\r\n",
        ));
        let feed = parse(&text, &kolkata()).unwrap();
        let (from, to) = year(2026);
        let days = |t: &str| -> Vec<String> {
            events(&feed, "c", from - 86_400, to)
                .into_iter()
                .filter(|e| e.title == t)
                .map(|e| {
                    Timestamp::from_second(e.starts_at)
                        .unwrap()
                        .to_zoned(TimeZone::system())
                        .date()
                        .to_string()
                })
                .collect()
        };
        assert_eq!(days("Board"), ["2026-01-12", "2026-02-09", "2026-03-09"]);
        assert_eq!(days("Rent"), ["2026-01-31", "2026-02-28", "2026-03-31"]);
        assert_eq!(days("Payday"), ["2026-01-30", "2026-02-27"]);
        assert!(events(&feed, "c", from, to).iter().all(|e| e.all_day));
    }

    #[test]
    fn a_birthday_feed_repeats_yearly_and_feb_29_is_skipped_in_other_years() {
        let text = wrap(concat!(
            "BEGIN:VEVENT\r\nUID:leap\r\nSUMMARY:Leap\r\nDTSTART;VALUE=DATE:20240229\r\nDTEND;VALUE=DATE:20240301\r\n",
            "RRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n",
        ));
        let feed = parse(&text, &kolkata()).unwrap();
        let (f26, t26) = year(2026);
        assert!(events(&feed, "c", f26, t26).is_empty());
        let (f28, t28) = year(2028);
        assert_eq!(events(&feed, "c", f28, t28).len(), 1);
    }

    #[test]
    fn only_the_window_is_laid_out_and_an_endless_rule_ends() {
        let text = wrap("BEGIN:VEVENT\r\nUID:d\r\nSUMMARY:Daily\r\nDTSTART:19900101T090000\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\n");
        let feed = parse(&text, &kolkata()).unwrap();
        let from = at("Asia/Kolkata", "20261001T000000");
        let to = at("Asia/Kolkata", "20261008T000000");
        let got = events(&feed, "c", from, to);
        assert_eq!(got.len(), 7);
        // Floating: read on this computer's clock (here, Kolkata's).
        assert_eq!(got[0].starts_at, at("Asia/Kolkata", "20261001T090000"));
    }

    #[test]
    fn folded_lines_escapes_outlook_zones_and_cancelled_events() {
        let text = wrap(concat!(
            "BEGIN:VEVENT\r\nUID:o\r\nSUMMARY:Parent\\, teacher\r\n  meeting\r\nDESCRIPTION:Room 4\\nBring forms\r\n",
            "LOCATION:Main hall\r\nURL:https://school.example/meet\r\n",
            "DTSTART;TZID=\"India Standard Time\":20261015T160000\r\nDTEND;TZID=\"India Standard Time\":20261015T170000\r\n",
            "BEGIN:VALARM\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Reminder\r\nEND:VALARM\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Called off\r\nSTATUS:CANCELLED\r\nDTSTART:20261016T040000Z\r\nEND:VEVENT\r\n",
        ));
        let feed = parse(&text, &TimeZone::UTC).unwrap();
        let (from, to) = year(2026);
        let got = events(&feed, "c", from, to);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Parent, teacher meeting");
        assert_eq!(got[0].notes.as_deref(), Some("Room 4\nBring forms"));
        assert_eq!(got[0].location.as_deref(), Some("Main hall"));
        assert_eq!(got[0].url.as_deref(), Some("https://school.example/meet"));
        assert_eq!(got[0].starts_at, at("Asia/Kolkata", "20261015T160000"));
    }

    /// An Outlook-published calendar (Outlook on the web's "Publish calendar"): a zone Outlook
    /// named itself, defined only by the file's VTIMEZONE, and a Windows name the file defines
    /// too. Read on this computer's clock (UTC here) both were hours off.
    #[test]
    fn an_outlook_feeds_own_zones_are_read_by_their_rules() {
        let text = concat!(
            "BEGIN:VCALENDAR\r\nMETHOD:PUBLISH\r\nPRODID:Microsoft Exchange Server 2010\r\nVERSION:2.0\r\nX-WR-CALNAME:Team\r\n",
            "BEGIN:VTIMEZONE\r\nTZID:Customized Time Zone\r\n",
            "BEGIN:STANDARD\r\nDTSTART:16010101T030000\r\nTZOFFSETFROM:+1030\r\nTZOFFSETTO:+0930\r\n",
            "RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=4\r\nEND:STANDARD\r\n",
            "BEGIN:DAYLIGHT\r\nDTSTART:16010101T020000\r\nTZOFFSETFROM:+0930\r\nTZOFFSETTO:+1030\r\n",
            "RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=10\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n",
            "BEGIN:VTIMEZONE\r\nTZID:Jordan Standard Time\r\n",
            "BEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n",
            "BEGIN:VEVENT\r\nUID:weekly\r\nSUMMARY:Sync\r\n",
            "DTSTART;TZID=Customized Time Zone:20260329T090000\r\nDTEND;TZID=Customized Time Zone:20260329T093000\r\n",
            "RRULE:FREQ=WEEKLY;COUNT=2\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:amman\r\nSUMMARY:Amman\r\n",
            "DTSTART;TZID=Jordan Standard Time:20260715T100000\r\nDTEND;TZID=Jordan Standard Time:20260715T110000\r\nEND:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        let feed = parse(text, &TimeZone::UTC).unwrap();
        let (from, to) = year(2026);
        let mut got = events(&feed, "c", from, to);
        got.sort_by_key(|e| e.starts_at);
        assert_eq!(got.len(), 3);
        // Adelaide's rules: 09:00 at +10:30 on 29 March, at +09:30 a week on (5 April).
        assert_eq!(
            got[0].starts_at,
            at("Australia/Adelaide", "20260329T090000")
        );
        assert_eq!(got[0].ends_at - got[0].starts_at, 1800);
        assert_eq!(
            got[1].starts_at,
            at("Australia/Adelaide", "20260405T090000")
        );
        // Amman, as the file defines it.
        assert_eq!(got[2].starts_at, at("Asia/Amman", "20260715T100000"));
    }

    #[test]
    fn a_subscription_is_due_by_its_own_interval() {
        let mut s = Subscription {
            id: "a".into(),
            url: "https://example.org/a.ics".into(),
            title: "A".into(),
            colour: "#ff9500".into(),
            refresh_minutes: 60,
            fetched_at: None,
            problem: None,
        };
        assert!(s.due(1_000));
        s.fetched_at = Some(1_000);
        assert!(!s.due(1_000 + 59 * 60));
        assert!(s.due(1_000 + 60 * 60));
        s.refresh_minutes = 0;
        assert!(!s.due(i64::MAX / 2));
        assert_eq!(s.calendar(0).guid, "quince:sub:a");
        assert!(!s.calendar(0).writable);
    }
    /// A public feed is anyone's file: no rule, date or length in it stops the calendar. A rule
    /// that names no real date walks to its end and gives nothing; one too large for the
    /// calendar stops; a date that does not exist is left out.
    #[test]
    fn a_hostile_feed_never_panics() {
        let rules = [
            "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30",
            "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=31;BYDAY=MO",
            "FREQ=DAILY;INTERVAL=1000000",
            "FREQ=WEEKLY;INTERVAL=9223372036854775807",
            "FREQ=MONTHLY;INTERVAL=9223372036854775807",
            "FREQ=YEARLY;INTERVAL=9223372036854775807",
            "FREQ=YEARLY;INTERVAL=20000",
            "FREQ=MONTHLY;INTERVAL=300000",
            "FREQ=WEEKLY;INTERVAL=3000000;BYDAY=MO,TU",
            "FREQ=MONTHLY;BYDAY=\u{e9}1",
            "FREQ=WEEKLY;BYDAY=\u{20ac}",
            "FREQ=WEEKLY;BYDAY=1",
            "FREQ=WEEKLY;BYDAY=\u{1F600}MO",
            "FREQ=MONTHLY;BYDAY=127MO,-128FR",
            "FREQ=MONTHLY;BYSETPOS=-2147483648,2147483647;BYDAY=MO",
            "FREQ=YEARLY;BYDAY=-53SU;COUNT=9223372036854775807",
            "FREQ=DAILY;UNTIL=99999999T999999Z",
        ];
        let starts = [
            "DTSTART;VALUE=DATE:20240229",
            "DTSTART:99991231T235959Z",
            "DTSTART;VALUE=DATE:00010101",
            "DTSTART;TZID=Europe/Berlin:20260329T023000",
        ];
        let (from, _) = year(2026);
        let (_, to) = year(9998);
        for rule in rules {
            for start in starts {
                let text = wrap(&format!(
                    "BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:x\r\n{start}\r\nDURATION:P9999999999999999W\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
                ));
                if let Ok(feed) = parse(&text, &kolkata()) {
                    let _ = events(&feed, "c", from, to);
                    let _ = events(&feed, "c", i64::MIN / 2, i64::MAX / 2);
                }
            }
        }
        // A date that does not exist is skipped, and the rest of the feed still reads.
        let text = wrap(
            "BEGIN:VEVENT\r\nUID:long\r\nSUMMARY:Long\r\nDTSTART;VALUE=DATE:20260301\r\nDTEND;VALUE=DATE:99991231\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:bad\r\nSUMMARY:Bad\r\nDTSTART;VALUE=DATE:20260230\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:ok\r\nSUMMARY:Ok\r\nDTSTART;VALUE=DATE:20260301\r\nEND:VEVENT\r\n",
        );
        let feed = parse(&text, &kolkata()).unwrap();
        let got = events(&feed, "c", from, to);
        assert!(got.iter().any(|e| e.title == "Ok"));
        assert!(!got.iter().any(|e| e.title == "Bad"));
        // A length too large to be one is none, not a wrapped-around number.
        assert_eq!(duration("P9999999999999999W"), None);
        assert_eq!(duration("PT1H30M"), Some(5_400));
    }
}
