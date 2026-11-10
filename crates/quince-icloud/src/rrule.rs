//! Repeat rules as calendars outside Apple write them (RFC 5545 `RRULE`, from a phone's calendar,
//! an .ics file or a CalDAV server) and as Apple's repeat holds them ([`EventRepeat`]).
//!
//! One reading for every way in, so a rule is never taken one way from a file and another from a
//! phone. What Apple's repeat can hold is taken exactly: several weekdays, an end day, the second
//! Tuesday or last Friday of a month, several days of a month, several months of a year, the
//! fourth Thursday of November. Anything more is refused with a sentence saying why, never
//! brought in as a different repeat. A rule must also start on a day it falls on: Apple moves a
//! start that doesn't to the next day that does, which would be a different event.

use crate::calendar::EventRepeat;
use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::ToSpan;

const DAYS: [(&str, Weekday); 7] = [
    ("MO", Weekday::Monday),
    ("TU", Weekday::Tuesday),
    ("WE", Weekday::Wednesday),
    ("TH", Weekday::Thursday),
    ("FR", Weekday::Friday),
    ("SA", Weekday::Saturday),
    ("SU", Weekday::Sunday),
];

/// Apple's positions (`frequencyDays`) and the `BYDAY` ordinals they are.
const POSITIONS: [(&str, i8); 5] = [("1st", 1), ("2nd", 2), ("3rd", 3), ("4th", 4), ("last", -1)];

const UNREADABLE: &str = "Its repeat can’t be read.";
const OTHER_DAY: &str = "It starts on a day it doesn’t repeat on, which iCloud can’t keep.";

fn code_of(day: Weekday) -> &'static str {
    DAYS.iter()
        .find(|(_, d)| *d == day)
        .map_or("MO", |(c, _)| c)
}

/// A `BYDAY` entry: its ordinal (`2` in `2TU`, `-1` in `-1FR`), if any, and its weekday code.
fn by_day_entry(s: &str) -> Option<(Option<i8>, &'static str)> {
    let s = s.trim();
    let at = s.len().checked_sub(2)?;
    let (n, code) = s.split_at(at);
    let code = DAYS.iter().find(|(c, _)| c.eq_ignore_ascii_case(code))?.0;
    let n = if n.is_empty() {
        None
    } else {
        Some(n.trim_start_matches('+').parse().ok()?)
    };
    Some((n, code))
}

/// Which of its weekday in the month `day` is, as Apple's positions count: the 1st to 4th, and
/// also `last` when no later one of that weekday is in the month.
fn positions_of(day: Date) -> (i8, bool) {
    let nth = (day.day() - 1) / 7 + 1;
    (nth, day.day() + 7 > day.days_in_month())
}

fn numbers(value: &str, range: std::ops::RangeInclusive<i64>) -> Result<Vec<i64>, String> {
    let mut out: Vec<i64> = Vec::new();
    for n in value.split(',') {
        let n: i64 = n.trim().parse().map_err(|_| UNREADABLE)?;
        if !range.contains(&n) {
            return Err(String::new());
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.sort_unstable();
    Ok(out)
}

/// The repeat an `RRULE` value says (`FREQ=WEEKLY;BYDAY=MO,WE`, with or without `RRULE:`), for
/// an event whose first time is `first` on its own clock `zone` (an all-day event's first day,
/// at midnight, when `all_day`). `Ok(None)`: it ends before it
/// would happen a second time, so it is a one-off. `Err`: a sentence for the person, saying what
/// about it Apple's repeat can't hold.
pub fn from_rrule(
    rule: &str,
    first: DateTime,
    zone: &TimeZone,
    all_day: bool,
) -> Result<Option<EventRepeat>, String> {
    let rule = rule.trim();
    let rule = rule
        .strip_prefix("RRULE:")
        .or_else(|| rule.strip_prefix("rrule:"))
        .unwrap_or(rule);
    let mut freq = None;
    let mut interval = 1i64;
    let mut count = None;
    let mut until = None;
    let mut by_day: Vec<(Option<i8>, &'static str)> = Vec::new();
    let mut set_pos: Option<i8> = None;
    let mut month_days = Vec::new();
    let mut months = Vec::new();
    for part in rule.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, value) = part.split_once('=').ok_or(UNREADABLE)?;
        let value = value.trim();
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.to_ascii_uppercase().as_str() {
                    "DAILY" => "daily",
                    "WEEKLY" => "weekly",
                    "MONTHLY" => "monthly",
                    "YEARLY" => "yearly",
                    "HOURLY" | "MINUTELY" | "SECONDLY" => {
                        return Err(
                            "It repeats more than once a day, which iCloud can’t keep.".into()
                        )
                    }
                    _ => return Err(UNREADABLE.into()),
                });
            }
            "INTERVAL" => {
                interval = value.parse().ok().filter(|n| *n >= 1).ok_or(UNREADABLE)?;
            }
            "COUNT" => {
                count = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|n: &i64| *n >= 1)
                        .ok_or(UNREADABLE)?,
                )
            }
            "UNTIL" => until = Some(until_day(value, first, zone, all_day).ok_or(UNREADABLE)?),
            "WKST" => {}
            "BYDAY" => {
                for entry in value.split(',') {
                    let entry = by_day_entry(entry).ok_or(UNREADABLE)?;
                    if !by_day.contains(&entry) {
                        by_day.push(entry);
                    }
                }
            }
            "BYSETPOS" => {
                set_pos = Some(
                    value
                        .parse()
                        .map_err(|_| "It repeats in a way iCloud can’t keep.")?,
                )
            }
            "BYMONTHDAY" => {
                month_days = numbers(value, 1..=31).map_err(|e| {
                    if e.is_empty() {
                        "It repeats on days counted from the end of the month, which iCloud can’t keep.".to_string()
                    } else {
                        e
                    }
                })?;
            }
            "BYMONTH" => months = numbers(value, 1..=12).map_err(|_| UNREADABLE.to_string())?,
            _ => return Err("It repeats in a way iCloud can’t keep.".into()),
        }
    }
    let freq = freq.ok_or(UNREADABLE)?;
    if count.is_some() && until.is_some() {
        return Err(UNREADABLE.into());
    }
    let day = first.date();
    let start_code = code_of(day.weekday());
    // `BYDAY=TU;BYSETPOS=2` is how some calendars write `BYDAY=2TU`.
    if let Some(n) = set_pos {
        match by_day.as_slice() {
            [(None, code)] => by_day = vec![(Some(n), code)],
            _ => return Err("It repeats in a way iCloud can’t keep.".into()),
        }
    }
    let mut repeat = EventRepeat {
        freq: freq.to_string(),
        interval,
        count,
        until,
        ..EventRepeat::default()
    };
    let plain_days: Vec<&'static str> = by_day
        .iter()
        .filter(|(n, _)| n.is_none())
        .map(|(_, c)| *c)
        .collect();
    let numbered = by_day.len() - plain_days.len();
    match freq {
        "daily" | "weekly" => {
            if numbered > 0
                || !month_days.is_empty() && month_days != [i64::from(day.day())]
                || !months.is_empty() && months != [i64::from(day.month())]
            {
                return Err("It repeats in a way iCloud can’t keep.".into());
            }
            if !plain_days.is_empty() && plain_days != [start_code] {
                if freq == "daily" && interval > 1 {
                    return Err("It repeats in a way iCloud can’t keep.".into());
                }
                // Every day on some weekdays ("every weekday") is each week on those days.
                repeat.freq = "weekly".into();
                if !plain_days.contains(&start_code) {
                    return Err(OTHER_DAY.into());
                }
                let mut days = plain_days;
                days.sort_by_key(|c| DAYS.iter().position(|(d, _)| d == c));
                repeat.week_days = days.into_iter().map(String::from).collect();
            }
        }
        "monthly" => {
            if !months.is_empty() {
                return Err("It repeats in some months only, which iCloud can’t keep.".into());
            }
            match (by_day.as_slice(), month_days.as_slice()) {
                ([], []) => {}
                ([], days) => {
                    if !days.contains(&i64::from(day.day())) {
                        return Err(OTHER_DAY.into());
                    }
                    if days.len() > 1 {
                        repeat.month_days = days.to_vec();
                    }
                }
                ([(Some(n), code)], []) => {
                    repeat.position = Some(position(*n, code, day)?.into());
                    repeat.week_days = vec![(*code).into()];
                }
                ([(None, _)], []) | ([_, _, ..], []) if numbered == 0 => {
                    return Err("It repeats on every one of some weekdays of the month, which iCloud can’t keep.".into())
                }
                _ => {
                    return Err("It repeats on several weekdays of the month, which iCloud can’t keep.".into())
                }
            }
        }
        _ => {
            if !month_days.is_empty() && month_days != [i64::from(day.day())] {
                return Err(
                    "It repeats on several days of the year, which iCloud can’t keep.".into(),
                );
            }
            match by_day.as_slice() {
                [] => {
                    if !months.is_empty() && !months.contains(&i64::from(day.month())) {
                        return Err(OTHER_DAY.into());
                    }
                    if months.len() > 1 {
                        repeat.months = months;
                    }
                }
                // The fourth Thursday of November: one weekday, in one month.
                [(Some(n), code)] if months.len() == 1 => {
                    if months[0] != i64::from(day.month()) {
                        return Err(OTHER_DAY.into());
                    }
                    repeat.position = Some(position(*n, code, day)?.into());
                    repeat.week_days = vec![(*code).into()];
                    repeat.months = months;
                }
                _ => {
                    return Err(
                        "It repeats on weekdays of the year, which iCloud can’t keep.".into(),
                    )
                }
            }
        }
    }
    if let Some(last) = &repeat.until {
        // An end before its second time: it happens once, a one-off. (A repeat on several days
        // with one time left is kept as it is; Apple shows it once all the same.)
        let last: Date = last.parse().map_err(|_| UNREADABLE)?;
        let step = match freq {
            "daily" => interval.days(),
            "weekly" => interval.weeks(),
            "monthly" => interval.months(),
            _ => interval.years(),
        };
        let several = !repeat.week_days.is_empty() && repeat.position.is_none()
            || !repeat.month_days.is_empty()
            || !repeat.months.is_empty();
        if !several
            && repeat.position.is_none()
            && last < day.checked_add(step).map_err(|_| UNREADABLE)?
        {
            return Ok(None);
        }
    }
    Ok(Some(repeat))
}

/// Apple's name for the `n`th `code` of a month, checked against the start: the start must be
/// that weekday, and that one of it in its month.
fn position(n: i8, code: &str, day: Date) -> Result<&'static str, String> {
    let name = POSITIONS
        .iter()
        .find(|(_, p)| *p == n)
        .map(|(name, _)| *name)
        .ok_or("It repeats on a weekday of the month iCloud can’t name (the fifth, or counted from the end).")?;
    let (nth, last) = positions_of(day);
    if code != code_of(day.weekday()) || (n == -1 && !last) || (n > 0 && n != nth) {
        return Err(OTHER_DAY.into());
    }
    Ok(name)
}

/// `UNTIL` as the last day the repeat may fall on, on the event's clock. A date is that day. A
/// time is the instant it names (UTC with `Z`, else the event's clock); if it is earlier in its
/// day than the event's time, that day's time is already past it, so the day before is the last.
/// An all-day event has no clock: its end is the day written, whatever time a phone put on it
/// (Android writes `UNTIL=20261214T235959Z` for an all-day repeat ending on the 14th).
fn until_day(value: &str, first: DateTime, zone: &TimeZone, all_day: bool) -> Option<String> {
    let value = value.trim();
    let date = |s: &str| -> Option<Date> {
        if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Date::new(
            s[..4].parse().ok()?,
            s[4..6].parse().ok()?,
            s[6..].parse().ok()?,
        )
        .ok()
    };
    let day = if let Some(d) = date(value) {
        d
    } else {
        let (rest, utc) = match value.strip_suffix(['Z', 'z']) {
            Some(rest) => (rest, true),
            None => (value, false),
        };
        let (d, t) = rest.split_once(['T', 't'])?;
        if all_day {
            return Some(date(d)?.to_string());
        }
        if t.len() < 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let time = jiff::civil::Time::new(
            t[..2].parse().ok()?,
            t[2..4].parse().ok()?,
            t.get(4..6).map_or(Some(0), |x| x.parse().ok())?.min(59),
            0,
        )
        .ok()?;
        let mut at = date(d)?.to_datetime(time);
        if utc {
            at = at
                .to_zoned(TimeZone::UTC)
                .ok()?
                .with_time_zone(zone.clone())
                .datetime();
        }
        if at.time() < first.time() {
            at.date().checked_sub(1.day()).ok()?
        } else {
            at.date()
        }
    };
    Some(day.to_string())
}

/// The `RRULE` value for a repeat (without `RRULE:`), as a CalDAV calendar or a phone writes it.
#[must_use]
pub fn to_rrule(r: &EventRepeat) -> String {
    let mut line = format!("FREQ={}", r.freq.to_ascii_uppercase());
    if r.interval > 1 {
        line.push_str(&format!(";INTERVAL={}", r.interval));
    }
    let join = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    if let Some(n) = r
        .position
        .as_deref()
        .and_then(|p| POSITIONS.iter().find(|(name, _)| *name == p))
    {
        if let Some(code) = r.week_days.first() {
            line.push_str(&format!(";BYDAY={}{code}", n.1));
        }
    } else if r.freq == "weekly" && !r.week_days.is_empty() {
        line.push_str(&format!(";BYDAY={}", r.week_days.join(",")));
    }
    if r.freq == "monthly" && !r.month_days.is_empty() {
        line.push_str(&format!(";BYMONTHDAY={}", join(&r.month_days)));
    }
    if r.freq == "yearly" && !r.months.is_empty() {
        line.push_str(&format!(";BYMONTH={}", join(&r.months)));
    }
    if let Some(day) = r.until.as_deref().and_then(|d| d.parse::<Date>().ok()) {
        line.push_str(&format!(";UNTIL={}", day.strftime("%Y%m%d")));
    } else if let Some(count) = r.count {
        line.push_str(&format!(";COUNT={count}"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(rule: &str, first: &str) -> Result<Option<EventRepeat>, String> {
        from_rrule(
            rule,
            first.parse().unwrap(),
            &TimeZone::get("Asia/Kolkata").unwrap(),
            false,
        )
    }
    fn repeat(freq: &str, interval: i64) -> EventRepeat {
        EventRepeat {
            freq: freq.into(),
            interval,
            ..EventRepeat::default()
        }
    }
    fn days(d: &[&str]) -> Vec<String> {
        d.iter().map(|s| (*s).to_string()).collect()
    }

    // Monday 16 Nov 2026 (the 3rd Monday); Tuesday 8 Dec 2026 (the 2nd Tuesday); Thursday 31 Dec
    // 2026 (the last Thursday, and the 5th); Thursday 26 Nov 2026 (the 4th and last Thursday).
    const MON: &str = "2026-11-16T10:00";

    /// Every form Apple's repeat holds comes in exactly; the start's own weekday, day or month
    /// said again is the plain repeat.
    #[test]
    fn what_apple_holds_is_taken_exactly() {
        assert_eq!(
            read("RRULE:FREQ=WEEKLY", MON),
            Ok(Some(repeat("weekly", 1)))
        );
        assert_eq!(
            read("FREQ=WEEKLY;BYDAY=MO;WKST=SU", MON),
            Ok(Some(repeat("weekly", 1)))
        );
        assert_eq!(
            read("FREQ=WEEKLY;INTERVAL=2;BYDAY=WE,MO,MO", MON),
            Ok(Some(EventRepeat {
                week_days: days(&["MO", "WE"]),
                ..repeat("weekly", 2)
            }))
        );
        // "Every weekday", as phones write it either way.
        let weekdays = Some(EventRepeat {
            week_days: days(&["MO", "TU", "WE", "TH", "FR"]),
            ..repeat("weekly", 1)
        });
        assert_eq!(
            read("FREQ=DAILY;BYDAY=MO,TU,WE,TH,FR", MON),
            Ok(weekdays.clone())
        );
        assert_eq!(read("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", MON), Ok(weekdays));
        assert_eq!(
            read("FREQ=MONTHLY;BYMONTHDAY=16;COUNT=6", MON),
            Ok(Some(EventRepeat {
                count: Some(6),
                ..repeat("monthly", 1)
            }))
        );
        // The second Tuesday, written either way.
        let second_tuesday = Ok(Some(EventRepeat {
            position: Some("2nd".into()),
            week_days: days(&["TU"]),
            ..repeat("monthly", 1)
        }));
        assert_eq!(
            read("FREQ=MONTHLY;BYDAY=2TU", "2026-12-08T19:00"),
            second_tuesday
        );
        assert_eq!(
            read("FREQ=MONTHLY;BYDAY=TU;BYSETPOS=2", "2026-12-08T19:00"),
            second_tuesday
        );
        // The last Thursday, though it is also the fifth.
        assert_eq!(
            read("FREQ=MONTHLY;BYDAY=-1TH", "2026-12-31T19:00")
                .unwrap()
                .unwrap()
                .position
                .as_deref(),
            Some("last")
        );
        assert_eq!(
            read("FREQ=MONTHLY;BYMONTHDAY=15,1", "2026-12-01T09:00"),
            Ok(Some(EventRepeat {
                month_days: vec![1, 15],
                ..repeat("monthly", 1)
            }))
        );
        assert_eq!(
            read("FREQ=YEARLY;BYMONTH=9,3", "2027-03-01T09:00"),
            Ok(Some(EventRepeat {
                months: vec![3, 9],
                ..repeat("yearly", 1)
            }))
        );
        assert_eq!(
            read("FREQ=YEARLY;BYMONTH=11;BYDAY=4TH", "2026-11-26T12:00"),
            Ok(Some(EventRepeat {
                position: Some("4th".into()),
                week_days: days(&["TH"]),
                months: vec![11],
                ..repeat("yearly", 1)
            }))
        );
    }

    /// An end is the last day on the event's own clock: a date as it is; a UTC time where it
    /// falls in the event's zone; a time earlier in its day than the event's, the day before.
    #[test]
    fn an_end_is_the_last_day_on_the_events_clock() {
        let until = |rule| read(rule, MON).unwrap().unwrap().until.unwrap();
        assert_eq!(until("FREQ=WEEKLY;UNTIL=20261214"), "2026-12-14");
        // 04:30 UTC is 10:00 in Kolkata: that day's time still counts.
        assert_eq!(until("FREQ=WEEKLY;UNTIL=20261214T043000Z"), "2026-12-14");
        // 04:29 UTC is 09:59 there, before the 10:00 time: the day before is the last.
        assert_eq!(until("FREQ=WEEKLY;UNTIL=20261214T042900Z"), "2026-12-13");
        // Ending before it would happen again: once.
        assert_eq!(read("FREQ=WEEKLY;UNTIL=20261120", MON), Ok(None));
        assert_eq!(
            read("FREQ=WEEKLY;COUNT=2;UNTIL=20261214", MON),
            Err(UNREADABLE.into())
        );
        // All day: the day written, though 23:59:59 UTC is the 15th in Kolkata.
        let all_day = from_rrule(
            "FREQ=DAILY;UNTIL=20261214T235959Z",
            "2026-12-01T00:00".parse().unwrap(),
            &TimeZone::get("Asia/Kolkata").unwrap(),
            true,
        );
        assert_eq!(
            all_day.unwrap().unwrap().until.as_deref(),
            Some("2026-12-14")
        );
    }

    /// What Apple's repeat can't hold is refused, saying why, never taken as another repeat;
    /// so is a start on a day the repeat doesn't fall on, which Apple would move.
    #[test]
    fn what_apple_cannot_hold_is_refused_saying_why() {
        let why = |rule, first| read(rule, first).unwrap_err();
        assert!(why("FREQ=MONTHLY;BYDAY=1MO,3MO", MON).contains("several weekdays of the month"));
        assert!(why("FREQ=MONTHLY;BYDAY=MO", MON).contains("every one of some weekdays"));
        assert!(why("FREQ=MONTHLY;BYDAY=5TH", "2026-12-31T19:00").contains("can’t name"));
        assert!(why("FREQ=MONTHLY;BYMONTHDAY=-1", MON).contains("end of the month"));
        assert!(why("FREQ=MONTHLY;BYMONTH=3", MON).contains("some months only"));
        assert!(why("FREQ=YEARLY;BYDAY=20MO", MON).contains("weekdays of the year"));
        assert!(why("FREQ=HOURLY", MON).contains("more than once a day"));
        assert!(why("FREQ=WEEKLY;BYWEEKNO=3", MON).contains("iCloud can’t keep"));
        // Starts the repeat doesn't fall on.
        assert_eq!(why("FREQ=WEEKLY;BYDAY=TU,TH", MON), OTHER_DAY);
        assert_eq!(why("FREQ=MONTHLY;BYDAY=2MO", MON), OTHER_DAY);
        assert_eq!(why("FREQ=MONTHLY;BYMONTHDAY=1,15", MON), OTHER_DAY);
        assert_eq!(why("FREQ=YEARLY;BYMONTH=3,9", MON), OTHER_DAY);
    }

    /// Written back as a rule and read again, every form is the same repeat.
    #[test]
    fn a_repeat_written_as_a_rule_reads_back_the_same() {
        for (r, first) in [
            (repeat("daily", 3), MON),
            (
                EventRepeat {
                    week_days: days(&["MO", "FR"]),
                    until: Some("2027-01-29".into()),
                    ..repeat("weekly", 2)
                },
                MON,
            ),
            (
                EventRepeat {
                    count: Some(12),
                    ..repeat("yearly", 1)
                },
                MON,
            ),
            (
                EventRepeat {
                    position: Some("3rd".into()),
                    week_days: days(&["MO"]),
                    ..repeat("monthly", 1)
                },
                MON,
            ),
            (
                EventRepeat {
                    month_days: vec![1, 16],
                    ..repeat("monthly", 2)
                },
                MON,
            ),
            (
                EventRepeat {
                    position: Some("last".into()),
                    week_days: days(&["TH"]),
                    months: vec![11],
                    ..repeat("yearly", 1)
                },
                "2026-11-26T12:00",
            ),
        ] {
            assert_eq!(read(&to_rrule(&r), first), Ok(Some(r)));
        }
    }
}
