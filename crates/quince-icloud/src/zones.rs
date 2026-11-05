//! The zone a calendar names, however it names it.
//!
//! A `TZID` (and Apple's own `tz` on an event) is usually an IANA name, but Outlook and
//! Exchange write Windows' names ("W. Europe Standard Time"), Thunderbird a longer id with the
//! IANA name at its end, and Outlook sometimes a name of its own ("Customized Time Zone") that
//! only the file's `VTIMEZONE` defines. Read as nothing, each became UTC or this computer's
//! clock, and the event was drawn hours off.
//!
//! Windows' names are Unicode CLDR's `windowsZones` mapping, kept as data in
//! `windows_zones.tsv` with the release it came from.

use jiff::tz::TimeZone;

/// CLDR's territory "001" rows: a Windows name, a tab, the IANA zone.
const WINDOWS_ZONES: &str = include_str!("windows_zones.tsv");

/// CLDR keeps the IANA ids it first published; IANA has since renamed these (its `backward`
/// file). The current name is what Calendar shows and what is written back to Apple; the old
/// one is still read where a computer's zone database predates the rename.
const RENAMED: &[(&str, &str)] = &[
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Godthab", "America/Nuuk"),
    ("America/Indianapolis", "America/Indiana/Indianapolis"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Europe/Kiev", "Europe/Kyiv"),
];

/// The IANA zone CLDR maps a Windows zone name to, by its current IANA name.
#[must_use]
pub fn windows_zone(name: &str) -> Option<&'static str> {
    let name = name.trim();
    let iana = WINDOWS_ZONES
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .find(|(windows, _)| windows.eq_ignore_ascii_case(name))
        .map(|(_, iana)| iana.trim())?;
    Some(
        RENAMED
            .iter()
            .find(|(old, _)| *old == iana)
            .map_or(iana, |(_, new)| new),
    )
}

/// A zone found, and the IANA name it goes by where it has one. A zone only a file's
/// `VTIMEZONE` defines has none.
#[derive(Debug, Clone)]
pub struct Zone {
    pub tz: TimeZone,
    pub name: Option<String>,
}

fn by_iana(name: &str) -> Option<Zone> {
    let tz = TimeZone::get(name).ok()?;
    let name = tz.iana_name().unwrap_or(name).to_string();
    Some(Zone {
        tz,
        name: Some(name),
    })
}

/// A CLDR zone by its current name, or by the name CLDR gives it where this computer's zone
/// database has only that.
fn by_windows(name: &str) -> Option<Zone> {
    let iana = windows_zone(name)?;
    by_iana(iana).or_else(|| {
        let old = RENAMED.iter().find(|(_, new)| *new == iana)?.0;
        let tz = TimeZone::get(old).ok()?;
        Some(Zone {
            tz,
            name: Some(old.to_string()),
        })
    })
}

fn cleaned(tzid: &str) -> &str {
    tzid.trim().trim_matches('"').trim_start_matches('/')
}

/// An IANA name inside a longer id, as Thunderbird writes them
/// (`/mozilla.org/20050126_1/Europe/Berlin`).
fn by_segments(tzid: &str) -> Option<Zone> {
    let segments: Vec<&str> = tzid.split('/').collect();
    (1..segments.len()).find_map(|start| by_iana(&segments[start..].join("/")))
}

/// A zone by name alone, where there is no file to define it: an IANA name, a Windows name,
/// or an IANA name at the end of a longer id.
#[must_use]
pub fn named(tzid: &str) -> Option<Zone> {
    let tzid = cleaned(tzid);
    if tzid.is_empty() {
        return None;
    }
    by_iana(tzid)
        .or_else(|| by_windows(tzid))
        .or_else(|| by_segments(tzid))
}

/// The zones a calendar file defines in its `VTIMEZONE`s, by their `TZID`.
#[derive(Debug, Clone, Default)]
pub struct Defined {
    zones: Vec<(String, TimeZone)>,
}

impl Defined {
    /// Every `VTIMEZONE` in a calendar file whose rules can be read.
    #[must_use]
    pub fn read(text: &str) -> Self {
        let mut zones = Vec::new();
        let mut tzid: Option<String> = None;
        let mut observances: Vec<Observance> = Vec::new();
        let mut current: Option<Observance> = None;
        let mut inside = false;
        for line in unfold(text) {
            let Some((name, value)) = name_value(&line) else {
                continue;
            };
            let upper = value.trim().to_ascii_uppercase();
            match (name.as_str(), upper.as_str()) {
                ("BEGIN", "VTIMEZONE") => {
                    inside = true;
                    tzid = None;
                    observances.clear();
                }
                ("END", "VTIMEZONE") => {
                    if let (Some(id), Some(tz)) = (tzid.take(), zone_of(&observances)) {
                        zones.push((id, tz));
                    }
                    inside = false;
                }
                ("BEGIN", kind @ ("STANDARD" | "DAYLIGHT")) if inside => {
                    current = Some(Observance {
                        daylight: kind == "DAYLIGHT",
                        ..Observance::default()
                    });
                }
                ("END", "STANDARD" | "DAYLIGHT") if inside => {
                    observances.extend(current.take());
                }
                _ if !inside => {}
                ("TZID", _) if current.is_none() => {
                    tzid = Some(cleaned(&value).to_string());
                }
                _ => {
                    if let Some(o) = current.as_mut() {
                        let value = value.trim().to_string();
                        match name.as_str() {
                            "DTSTART" => o.start = value,
                            "TZOFFSETFROM" => o.from = offset(&value),
                            "TZOFFSETTO" => o.to = offset(&value),
                            "RRULE" => o.rule = Some(value),
                            _ => {}
                        }
                    }
                }
            }
        }
        Defined { zones }
    }

    fn get(&self, tzid: &str) -> Option<&TimeZone> {
        let tzid = cleaned(tzid);
        self.zones
            .iter()
            .find(|(id, _)| id == tzid)
            .or_else(|| {
                self.zones
                    .iter()
                    .find(|(id, _)| id.eq_ignore_ascii_case(tzid))
            })
            .map(|(_, tz)| tz)
    }

    /// The zone a `TZID` in this file means.
    ///
    /// An IANA name is the zone database's, which knows its history; so is one at the end of
    /// a longer id. Any other name is what the file's own `VTIMEZONE` says it is (RFC 5545),
    /// named for Apple by CLDR where it is a Windows name; and without one, CLDR's zone.
    #[must_use]
    pub fn zone(&self, tzid: &str) -> Option<Zone> {
        let clean = cleaned(tzid);
        if clean.is_empty() {
            return None;
        }
        if let Some(z) = by_iana(clean).or_else(|| by_segments(clean)) {
            return Some(z);
        }
        if let Some(tz) = self.get(clean) {
            return Some(Zone {
                tz: tz.clone(),
                name: by_windows(clean).and_then(|z| z.name),
            });
        }
        by_windows(clean)
    }
}

/// One `STANDARD` or `DAYLIGHT` of a `VTIMEZONE`.
#[derive(Debug, Clone, Default)]
struct Observance {
    daylight: bool,
    /// `DTSTART`, a local time on the clock before this observance began.
    start: String,
    from: Option<i32>,
    to: Option<i32>,
    rule: Option<String>,
}

impl Observance {
    /// A yearly rule with no end: the observance as it still stands.
    fn ongoing_rule(&self) -> Option<&str> {
        let rule = self.rule.as_deref()?;
        let parts = rule_parts(rule);
        let yearly = parts
            .iter()
            .any(|(k, v)| k == "FREQ" && v.eq_ignore_ascii_case("YEARLY"));
        let ends = parts.iter().any(|(k, _)| k == "UNTIL" || k == "COUNT");
        (yearly && !ends).then_some(rule)
    }
}

fn rule_parts(rule: &str) -> Vec<(String, String)> {
    rule.split(';')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_string()))
        .collect()
}

/// `+0100`, `-0530`, `+013000`: seconds east of UTC.
fn offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => (1, s),
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = digits[0..2].parse().ok()?;
    let m: i32 = digits[2..4].parse().ok()?;
    let sec: i32 = digits.get(4..6).map_or(Some(0), |x| x.parse().ok())?;
    (h < 24 && m < 60 && sec < 60).then_some(sign * (h * 3600 + m * 60 + sec))
}

/// A `VTIMEZONE`'s rules as a zone: its standing yearly change between standard time and
/// daylight time, or the offset it last settled on where it no longer changes.
fn zone_of(observances: &[Observance]) -> Option<TimeZone> {
    let latest = |daylight: bool, ongoing: bool| {
        observances
            .iter()
            .filter(|o| o.daylight == daylight && (!ongoing || o.ongoing_rule().is_some()))
            .max_by(|a, b| a.start.cmp(&b.start))
    };
    if let (Some(std), Some(dst)) = (latest(false, true), latest(true, true)) {
        if let Some(tz) = posix_of(std, dst).and_then(|p| TimeZone::posix(&p).ok()) {
            return Some(tz);
        }
    }
    // No standing change: the offset of whichever observance began last.
    let last = observances.iter().max_by(|a, b| a.start.cmp(&b.start))?;
    let to = last.to?;
    jiff::tz::Offset::from_seconds(to).ok().map(TimeZone::fixed)
}

/// The two yearly changes as a POSIX TZ string, which keeps applying them for every year.
fn posix_of(std: &Observance, dst: &Observance) -> Option<String> {
    let std_offset = std.to?;
    let dst_offset = dst.to?;
    let begins = change_of(dst)?;
    let ends = change_of(std)?;
    Some(format!(
        "STD{}DST{},{begins},{ends}",
        posix_offset(std_offset),
        posix_offset(dst_offset)
    ))
}

/// POSIX writes an offset as how far behind UTC the clock is: UTC+1 is `-1`.
fn posix_offset(east: i32) -> String {
    let west = -east;
    let sign = if west < 0 { "-" } else { "" };
    let a = west.unsigned_abs();
    format!("{sign}{}:{:02}:{:02}", a / 3600, a / 60 % 60, a % 60)
}

/// When an observance begins each year, as POSIX says it: `M3.5.0/02:00:00` (the last Sunday
/// of March, at 2:00 on the clock before), or `J80/00:00:00` (a fixed date).
fn change_of(o: &Observance) -> Option<String> {
    let rule = rule_parts(o.ongoing_rule()?);
    let get = |k: &str| {
        rule.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    let (date, time) = o.start.split_once(['T', 't'])?;
    let time = time.trim_end_matches(['Z', 'z']);
    if date.len() != 8 || time.len() < 4 || !time.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let month: u8 = match get("BYMONTH") {
        Some(m) => m.parse().ok()?,
        None => date[4..6].parse().ok()?,
    };
    if !(1..=12).contains(&month) {
        return None;
    }
    let at = format!(
        "/{}:{}:{}",
        &time[0..2],
        &time[2..4],
        time.get(4..6).unwrap_or("00")
    );
    let month_days: Vec<i8> = get("BYMONTHDAY")
        .map(|v| v.split(',').filter_map(|d| d.trim().parse().ok()).collect())
        .unwrap_or_default();
    let day = match get("BYDAY") {
        Some(by_day) => {
            let by_day = by_day.trim();
            if by_day.contains(',') || by_day.len() < 2 {
                return None;
            }
            let (n, code) = by_day.split_at(by_day.len() - 2);
            let weekday = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"]
                .iter()
                .position(|c| c.eq_ignore_ascii_case(code))?;
            let n: i8 = match n.trim_start_matches('+') {
                "" => week_of_days(&month_days)?,
                n => n.parse().ok()?,
            };
            let week = match n {
                1..=4 => n,
                5 | -1 => 5,
                _ => return None,
            };
            format!("M{month}.{week}.{weekday}")
        }
        None => {
            // A fixed date each year, counted as POSIX's `J` does: never a 29 February.
            let &[d] = month_days.as_slice() else {
                return None;
            };
            let first_of_months = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
            let days_in = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let m = usize::from(month - 1);
            if !(1..=days_in[m]).contains(&d) {
                return None;
            }
            format!("J{}", first_of_months[m] + i32::from(d))
        }
    };
    Some(format!("{day}{at}"))
}

/// A weekday given with the seven days of the month it falls in (`BYMONTHDAY=8,…,14;BYDAY=SU`,
/// the second Sunday), as the week POSIX counts.
fn week_of_days(days: &[i8]) -> Option<i8> {
    let first = *days.iter().min()?;
    if days.len() != 7 {
        return None;
    }
    match first {
        1 | 8 | 15 | 22 => Some((first - 1) / 7 + 1),
        -7 => Some(5),
        _ => None,
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

/// A line's name and value, a colon inside a quoted parameter not ending the name.
fn name_value(line: &str) -> Option<(String, String)> {
    let mut quoted = false;
    let at = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let name = line[..at].split(';').next()?.trim().to_ascii_uppercase();
    Some((name, line[at + 1..].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::DateTime;

    /// Seconds east of UTC that `tz` is at a clock reading on it.
    fn offset_at(tz: &TimeZone, clock: &str) -> i32 {
        let dt: DateTime = clock.parse().unwrap();
        dt.to_zoned(tz.clone()).unwrap().offset().seconds()
    }

    #[test]
    fn every_cldr_row_is_a_zone_this_computer_knows() {
        let rows: Vec<&str> = WINDOWS_ZONES
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect();
        assert_eq!(rows.len(), 142, "CLDR 49 has 142 territory 001 rows");
        for row in rows {
            let (windows, _) = row.split_once('\t').expect("a tab between the two names");
            let z = named(windows).unwrap_or_else(|| panic!("{windows} is no zone here"));
            assert!(z.name.is_some(), "{windows}");
        }
    }

    #[test]
    fn windows_names_are_cldrs_001_zone_by_its_current_name() {
        assert_eq!(
            windows_zone("W. Europe Standard Time"),
            Some("Europe/Berlin")
        );
        assert_eq!(windows_zone("india standard time"), Some("Asia/Kolkata"));
        assert_eq!(windows_zone("FLE Standard Time"), Some("Europe/Kyiv"));
        assert_eq!(windows_zone("UTC"), Some("Etc/UTC"));
        assert_eq!(windows_zone("Mars Standard Time"), None);
        let z = named("\"India Standard Time\"").unwrap();
        assert_eq!(z.name.as_deref(), Some("Asia/Kolkata"));
        assert_eq!(offset_at(&z.tz, "2026-01-15T09:00"), 19_800);
        // An IANA name stands as it is; one at the end of a longer id is found.
        assert_eq!(
            named("America/New_York").unwrap().name.as_deref(),
            Some("America/New_York")
        );
        assert_eq!(
            named("/mozilla.org/20050126_1/Europe/Berlin")
                .unwrap()
                .name
                .as_deref(),
            Some("Europe/Berlin")
        );
        assert!(named("Mars/Olympus_Mons").is_none());
        assert!(named("").is_none());
    }

    /// Outlook's own export of a meeting in Berlin time: the Windows name, and its rules.
    const OUTLOOK: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Microsoft Corporation//Outlook 16.0 MIMEDIR//EN\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VTIMEZONE\r\nTZID:W. Europe Standard Time\r\nBEGIN:STANDARD\r\nDTSTART:16011028T030000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010325T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n";

    #[test]
    fn a_vtimezone_defines_its_own_name() {
        let defined = Defined::read(OUTLOOK);
        let z = defined.zone("W. Europe Standard Time").unwrap();
        assert_eq!(z.name.as_deref(), Some("Europe/Berlin"));
        assert_eq!(offset_at(&z.tz, "2026-07-15T10:00"), 7_200);
        assert_eq!(offset_at(&z.tz, "2026-01-15T10:00"), 3_600);
        // The changes fall where Berlin's do: 29 March and 25 October 2026.
        assert_eq!(offset_at(&z.tz, "2026-03-29T01:30"), 3_600);
        assert_eq!(offset_at(&z.tz, "2026-03-29T03:30"), 7_200);
        assert_eq!(offset_at(&z.tz, "2026-10-25T03:30"), 3_600);
    }

    /// Outlook's "Customized Time Zone": no name anyone else knows, only its rules — here
    /// Sydney's, a southern summer that spans the new year, written with Exchange's
    /// month-day form for "the first Sunday".
    const CUSTOM: &str = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Customized Time Zone\r\nBEGIN:STANDARD\r\nDTSTART:16010401T030000\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYMONTH=4;BYMONTHDAY=1,2,3,4,5,6,7;BYDAY=SU\r\nTZOFFSETFROM:+1100\r\nTZOFFSETTO:+1000\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16011007T020000\r\nRRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=10\r\nTZOFFSETFROM:+1000\r\nTZOFFSETTO:+1100\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VTIMEZONE\r\nTZID:(UTC+05:30) Chennai, Kolkata, Mumbai, New Delhi\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nTZOFFSETFROM:+0530\r\nTZOFFSETTO:+0530\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n";

    #[test]
    fn a_customized_zone_is_its_rules_and_has_no_name() {
        let defined = Defined::read(CUSTOM);
        let sydney = defined.zone("Customized Time Zone").unwrap();
        assert!(sydney.name.is_none());
        assert_eq!(offset_at(&sydney.tz, "2026-01-10T09:00"), 39_600);
        assert_eq!(offset_at(&sydney.tz, "2026-06-10T09:00"), 36_000);
        // 5 April 2026 and 4 October 2026 are the first Sundays.
        assert_eq!(offset_at(&sydney.tz, "2026-04-05T03:30"), 36_000);
        assert_eq!(offset_at(&sydney.tz, "2026-10-04T03:30"), 39_600);

        let india = defined
            .zone("(UTC+05:30) Chennai, Kolkata, Mumbai, New Delhi")
            .unwrap();
        assert_eq!(offset_at(&india.tz, "2026-07-01T12:00"), 19_800);
        assert!(defined.zone("Nowhere Standard Time").is_none());
    }

    #[test]
    fn an_iana_name_is_the_zone_databases_even_with_a_vtimezone() {
        // A VTIMEZONE that claims New York is always UTC-5 does not move a July meeting:
        // the zone database knows New York.
        let text = "BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:-0500\r\nTZOFFSETTO:-0500\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
        let z = Defined::read(text).zone("America/New_York").unwrap();
        assert_eq!(offset_at(&z.tz, "2026-07-14T09:00"), -4 * 3600);
    }

    #[test]
    fn a_zone_that_stopped_changing_keeps_its_last_offset() {
        // Daylight time ended by an UNTIL, then standard time for good.
        let text = "BEGIN:VTIMEZONE\r\nTZID:Somewhere\r\nBEGIN:DAYLIGHT\r\nDTSTART:20000326T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3;UNTIL=20100328T010000Z\r\nTZOFFSETFROM:+0300\r\nTZOFFSETTO:+0400\r\nEND:DAYLIGHT\r\nBEGIN:STANDARD\r\nDTSTART:20101031T030000\r\nTZOFFSETFROM:+0400\r\nTZOFFSETTO:+0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n";
        let z = Defined::read(text).zone("Somewhere").unwrap();
        assert_eq!(offset_at(&z.tz, "2026-07-01T12:00"), 3 * 3600);
        assert_eq!(offset_at(&z.tz, "2026-01-01T12:00"), 3 * 3600);
    }

    #[test]
    fn offsets_and_unreadable_rules() {
        assert_eq!(offset("+0530"), Some(19_800));
        assert_eq!(offset("-0330"), Some(-12_600));
        assert_eq!(offset("+013000"), Some(5_400));
        assert_eq!(offset("+2500"), None);
        assert_eq!(offset("+05"), None);
        assert_eq!(posix_offset(3_600), "-1:00:00");
        assert_eq!(posix_offset(-12_600), "3:30:00");
        // A rule POSIX can't say (the second-last Sunday) falls back to the last offset.
        let text = "BEGIN:VTIMEZONE\r\nTZID:Odd\r\nBEGIN:STANDARD\r\nDTSTART:16011028T030000\r\nRRULE:FREQ=YEARLY;BYDAY=-2SU;BYMONTH=10\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010325T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n";
        assert!(Defined::read(text).zone("Odd").is_some());
        // Nothing in it at all: not a zone.
        let empty = "BEGIN:VTIMEZONE\r\nTZID:Empty\r\nEND:VTIMEZONE\r\n";
        assert!(Defined::read(empty).zone("Empty").is_none());
    }
}
