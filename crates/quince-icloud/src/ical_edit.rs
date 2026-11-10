//! One event's iCalendar text, changed as little as a change needs.
//!
//! A CalDAV server holds an event as a file, and everything in it that Skykeep has no field for
//! (a guest's answer, a client's own `X-` lines, a conference link) has to still be there after
//! a save. So a change is made to the text it came from, a line at a time, and only the fields
//! the person changed are written: another device's change to another field is kept.

use crate::calendar::{EventRepeat, Guest, Scope};
use crate::calendar_feeds::{
    self as feeds, duration, instant, key_of, parse_date, parse_datetime, property, starts,
    unescape, unfold, when_of, Clock, Property, When,
};
use crate::{zones, Error, Result};
use jiff::civil::Date;
use jiff::tz::{Offset, TimeZone};
use jiff::{Timestamp, ToSpan};
use quince_core::calendar::{instant_of, wall_clock, Event};

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Line(String),
    Part(Part),
}

/// A `BEGIN:` … `END:` block, its lines as they were written.
#[derive(Debug, Clone, PartialEq, Default)]
struct Part {
    name: String,
    nodes: Vec<Node>,
}

/// A line written out at no more than 75 octets, a character never cut in two.
fn fold(line: &str, out: &mut String) {
    let mut used = 0;
    for c in line.chars() {
        if used + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += c.len_utf8();
    }
    out.push_str("\r\n");
}

fn name_of(line: &str) -> String {
    property(line).map(|p| p.name).unwrap_or_default()
}

impl Part {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
        }
    }

    fn write(&self, out: &mut String) {
        fold(&format!("BEGIN:{}", self.name), out);
        for node in &self.nodes {
            match node {
                Node::Line(l) => fold(l, out),
                Node::Part(p) => p.write(out),
            }
        }
        fold(&format!("END:{}", self.name), out);
    }

    fn prop(&self, name: &str) -> Option<Property> {
        self.nodes.iter().find_map(|n| match n {
            Node::Line(l) => property(l).filter(|p| p.name == name),
            Node::Part(_) => None,
        })
    }

    fn props(&self, name: &str) -> Vec<Property> {
        self.nodes
            .iter()
            .filter_map(|n| match n {
                Node::Line(l) => property(l).filter(|p| p.name == name),
                Node::Part(_) => None,
            })
            .collect()
    }

    fn remove(&mut self, name: &str) {
        self.nodes
            .retain(|n| !matches!(n, Node::Line(l) if name_of(l) == name));
    }

    /// A line added after the block's own lines, before the blocks inside it.
    fn add(&mut self, line: String) {
        let at = self
            .nodes
            .iter()
            .position(|n| matches!(n, Node::Part(_)))
            .unwrap_or(self.nodes.len());
        self.nodes.insert(at, Node::Line(line));
    }

    /// `name` is exactly this one line, where the first of it was.
    fn set(&mut self, name: &str, line: String) {
        let at = self
            .nodes
            .iter()
            .position(|n| matches!(n, Node::Line(l) if name_of(l) == name));
        self.remove(name);
        match at {
            Some(at) => self
                .nodes
                .insert(at.min(self.nodes.len()), Node::Line(line)),
            None => self.add(line),
        }
    }

    fn parts(&self, name: &str) -> impl Iterator<Item = &Part> {
        let name = name.to_string();
        self.nodes.iter().filter_map(move |n| match n {
            Node::Part(p) if p.name == name => Some(p),
            _ => None,
        })
    }

    fn drop_parts(&mut self, name: &str) {
        self.nodes
            .retain(|n| !matches!(n, Node::Part(p) if p.name == name));
    }
}

fn read(text: &str) -> Option<Part> {
    let mut stack: Vec<Part> = Vec::new();
    for line in unfold(text) {
        let Some(p) = property(&line) else { continue };
        match p.name.as_str() {
            "BEGIN" => stack.push(Part::new(&p.value.trim().to_ascii_uppercase())),
            "END" => {
                let done = stack.pop()?;
                match stack.last_mut() {
                    Some(parent) => parent.nodes.push(Node::Part(done)),
                    None => return (done.name == "VCALENDAR").then_some(done),
                }
            }
            _ => stack.last_mut()?.nodes.push(Node::Line(line)),
        }
    }
    None
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

fn utc_text(t: Timestamp) -> String {
    t.to_zoned(TimeZone::UTC)
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn at_second(seconds: i64) -> Result<Timestamp> {
    Timestamp::from_second(seconds)
        .map_err(|_| Error::Refused("That date is out of the calendar's range.".into()))
}

fn date_text(d: Date) -> String {
    d.strftime("%Y%m%d").to_string()
}

fn day_in(seconds: i64, zone: &TimeZone) -> Result<Date> {
    Ok(at_second(seconds)?.to_zoned(zone.clone()).date())
}

/// A parameter's value, in quotes where it needs them.
fn param_value(value: &str) -> String {
    if value.contains([':', ';', ',']) {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

/// A `VTIMEZONE` for a zone by its IANA name: the changes around `at`, enough for the file to
/// be valid. Calendars read the name itself for the years beyond them.
fn vtimezone(name: &str, at: i64) -> Option<Part> {
    let offset_text = |o: Offset| {
        let s = o.seconds();
        let (sign, s) = if s < 0 { ('-', -s) } else { ('+', s) };
        format!("{sign}{:02}{:02}", s / 3600, s % 3600 / 60)
    };
    let tz = TimeZone::get(name).ok()?;
    let at = Timestamp::from_second(at).ok()?;
    let horizon = at.as_second() + 400 * 86_400;
    let mut changes = Vec::new();
    if let Some(t) = tz.preceding(at).next() {
        changes.push((
            t.timestamp(),
            t.offset(),
            t.abbreviation().to_string(),
            t.dst(),
        ));
    }
    for t in tz.following(at) {
        if t.timestamp().as_second() > horizon {
            break;
        }
        changes.push((
            t.timestamp(),
            t.offset(),
            t.abbreviation().to_string(),
            t.dst(),
        ));
    }
    let mut part = Part::new("VTIMEZONE");
    part.add(format!("TZID:{name}"));
    if changes.is_empty() {
        let info = tz.to_offset_info(at);
        let mut only = Part::new("STANDARD");
        only.add("DTSTART:19700101T000000".into());
        only.add(format!("TZOFFSETFROM:{}", offset_text(info.offset())));
        only.add(format!("TZOFFSETTO:{}", offset_text(info.offset())));
        only.add(format!("TZNAME:{}", info.abbreviation()));
        part.nodes.push(Node::Part(only));
        return Some(part);
    }
    for (when, to, abbreviation, dst) in changes {
        let before = tz.to_offset(when.checked_sub(1.second()).ok()?);
        let mut one = Part::new(if dst.is_dst() { "DAYLIGHT" } else { "STANDARD" });
        one.add(format!(
            "DTSTART:{}",
            before.to_datetime(when).strftime("%Y%m%dT%H%M%S")
        ));
        one.add(format!("TZOFFSETFROM:{}", offset_text(before)));
        one.add(format!("TZOFFSETTO:{}", offset_text(to)));
        one.add(format!("TZNAME:{abbreviation}"));
        part.nodes.push(Node::Part(one));
    }
    Some(part)
}

/// A change made here, as the outbox holds it.
#[derive(Debug, Clone, Copy)]
pub struct Change<'a> {
    pub event: &'a Event,
    pub base: Option<&'a Event>,
    pub alerts: Option<&'a [i64]>,
    /// `Some(None)`: no longer repeats.
    pub repeat: Option<Option<&'a EventRepeat>>,
    pub scope: Option<Scope>,
}

/// What a change came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    Whole(String),
    /// "This and all after": the series up to here, and a new event from here on.
    Split {
        kept: String,
        rest: String,
        uid: String,
    },
}

/// What a deletion came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    Whole,
    Kept(String),
}

/// The occurrence an event's guid names, when it names one.
#[must_use]
pub fn occurrence_key(guid: &str) -> Option<i64> {
    guid.rsplit_once("__")?.1.parse().ok()
}

fn when_differs(a: &Event, b: &Event) -> bool {
    (a.starts_at, a.ends_at, a.all_day, &a.timezone)
        != (b.starts_at, b.ends_at, b.all_day, &b.timezone)
}

fn rule_line(r: &EventRepeat) -> String {
    format!("RRULE:{}", crate::rrule::to_rrule(r))
}

fn alarm(minutes: i64) -> Part {
    let trigger = match minutes {
        0 => "PT0S".to_string(),
        m if m > 0 => format!("-PT{m}M"),
        m => format!("PT{}M", -m),
    };
    let mut part = Part::new("VALARM");
    part.add("ACTION:DISPLAY".into());
    part.add("DESCRIPTION:Reminder".into());
    part.add(format!("TRIGGER:{trigger}"));
    part
}

fn set_alerts(v: &mut Part, minutes: &[i64]) {
    v.drop_parts("VALARM");
    v.nodes
        .extend(minutes.iter().map(|m| Node::Part(alarm(*m))));
}

fn bump(v: &mut Part, now: i64) -> Result<()> {
    let stamp = utc_text(at_second(now)?);
    let sequence = v
        .prop("SEQUENCE")
        .and_then(|p| p.value.trim().parse::<i64>().ok())
        .unwrap_or(0);
    v.set("SEQUENCE", format!("SEQUENCE:{}", sequence + 1));
    v.set("DTSTAMP", format!("DTSTAMP:{stamp}"));
    v.set("LAST-MODIFIED", format!("LAST-MODIFIED:{stamp}"));
    Ok(())
}

/// The event's title, place, notes and link, where they differ from what it was made from.
fn apply_words(v: &mut Part, base: Option<&Event>, mine: &Event) {
    if base.is_none_or(|b| b.title != mine.title) {
        v.set("SUMMARY", format!("SUMMARY:{}", escape(&mine.title)));
    }
    let optional =
        |v: &mut Part, name: &str, was: Option<&Option<String>>, now: &Option<String>| {
            if was.is_some_and(|w| w == now) {
                return;
            }
            match now {
                Some(text) if name == "URL" => v.set(name, format!("URL:{text}")),
                Some(text) => v.set(name, format!("{name}:{}", escape(text))),
                None => v.remove(name),
            }
        };
    optional(v, "LOCATION", base.map(|b| &b.location), &mine.location);
    optional(v, "DESCRIPTION", base.map(|b| &b.notes), &mine.notes);
    optional(v, "URL", base.map(|b| &b.url), &mine.url);
}

/// `DTSTART` and `DTEND` for an event, and the zone they name when a file has to define it.
fn when_lines(e: &Event, here: &TimeZone) -> Result<(String, String, Option<String>)> {
    if e.all_day {
        let first = day_in(e.starts_at, here)?;
        // Stored as the midnight after its last day: a second back is that last day. Written
        // as `DTEND`, the day after it, as iCalendar means an end.
        let last = day_in((e.ends_at - 1).max(e.starts_at), here)?;
        let end = last
            .checked_add(1.day())
            .map_err(|_| Error::Refused("That date is out of the calendar's range.".into()))?;
        return Ok((
            format!("DTSTART;VALUE=DATE:{}", date_text(first)),
            format!("DTEND;VALUE=DATE:{}", date_text(end)),
            None,
        ));
    }
    let (start, end) = (at_second(e.starts_at)?, at_second(e.ends_at)?);
    let utc = || {
        (
            format!("DTSTART:{}", utc_text(start)),
            format!("DTEND:{}", utc_text(end)),
            None,
        )
    };
    let named = e
        .timezone
        .clone()
        .or_else(|| here.iana_name().map(ToString::to_string));
    let Some(name) = named else {
        let clock = |t: Timestamp| {
            t.to_zoned(here.clone())
                .strftime("%Y%m%dT%H%M%S")
                .to_string()
        };
        return Ok((
            format!("DTSTART:{}", clock(start)),
            format!("DTEND:{}", clock(end)),
            None,
        ));
    };
    if name == "UTC" {
        return Ok(utc());
    }
    let Ok(tz) = TimeZone::get(&name) else {
        return Ok(utc());
    };
    let clock = |t: Timestamp| t.to_zoned(tz.clone()).strftime("%Y%m%dT%H%M%S").to_string();
    let id = param_value(&name);
    Ok((
        format!("DTSTART;TZID={id}:{}", clock(start)),
        format!("DTEND;TZID={id}:{}", clock(end)),
        Some(name),
    ))
}

/// A line's text before and after the colon that ends its name and parameters.
fn split_line(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Every date in a line such as `EXDATE` moved on the clock by `seconds`.
fn shift_line(line: &str, seconds: i64) -> Option<String> {
    let (head, values) = split_line(line)?;
    let moved: Vec<String> = values
        .split(',')
        .map(|value| {
            let value = value.trim();
            if let Some((dt, utc)) = parse_datetime(value) {
                let Ok(next) = dt.checked_add(seconds.seconds()) else {
                    return value.to_string();
                };
                format!(
                    "{}{}",
                    next.strftime("%Y%m%dT%H%M%S"),
                    if utc { "Z" } else { "" }
                )
            } else if let Some(d) = parse_date(value) {
                d.checked_add((seconds / 86_400).days())
                    .map_or_else(|_| value.to_string(), date_text)
            } else {
                value.to_string()
            }
        })
        .collect();
    Some(format!("{head}:{}", moved.join(",")))
}

/// A calendar file being changed.
struct Doc<'a> {
    cal: Part,
    zones: zones::Defined,
    here: &'a TimeZone,
    now: i64,
}

impl<'a> Doc<'a> {
    fn read(ics: &str, here: &'a TimeZone, now: i64) -> Result<Self> {
        let cal = read(ics)
            .ok_or_else(|| Error::Protocol("the server's copy of that event is damaged".into()))?;
        Ok(Self {
            cal,
            zones: zones::Defined::read(ics),
            here,
            now,
        })
    }

    fn text(&self) -> String {
        let mut out = String::new();
        self.cal.write(&mut out);
        out
    }

    fn master_at(&self) -> Option<usize> {
        self.cal.nodes.iter().position(
            |n| matches!(n, Node::Part(p) if p.name == "VEVENT" && p.prop("RECURRENCE-ID").is_none()),
        )
    }

    fn master(&self) -> Result<usize> {
        self.master_at()
            .ok_or_else(|| Error::Refused("That event can't be changed from here.".into()))
    }

    fn event_mut(&mut self, at: usize) -> &mut Part {
        match &mut self.cal.nodes[at] {
            Node::Part(p) => p,
            Node::Line(_) => unreachable!("an index found among the calendar's events"),
        }
    }

    fn event(&self, at: usize) -> &Part {
        match &self.cal.nodes[at] {
            Node::Part(p) => p,
            Node::Line(_) => unreachable!("an index found among the calendar's events"),
        }
    }

    fn clock(&self) -> Clock<'_> {
        Clock {
            here: self.here,
            zones: &self.zones,
        }
    }

    fn override_key(&self, v: &Part) -> Option<i64> {
        v.prop("RECURRENCE-ID")
            .and_then(|p| when_of(&p, &self.clock()))
            .and_then(|w| key_of(&w))
    }

    /// The changed occurrence standing in for `key`, when there is one.
    fn override_at(&self, key: i64) -> Option<usize> {
        self.cal.nodes.iter().position(|n| {
            matches!(n, Node::Part(p) if p.name == "VEVENT" && self.override_key(p) == Some(key))
        })
    }

    fn start_of(&self, v: &Part) -> Result<(Property, When)> {
        let p = v
            .prop("DTSTART")
            .ok_or_else(|| Error::Protocol("an event with no start".into()))?;
        let w = when_of(&p, &self.clock())
            .ok_or_else(|| Error::Protocol("an event whose start can't be read".into()))?;
        Ok((p, w))
    }

    fn is_first(&self, master: usize, key: i64) -> Result<bool> {
        let (_, w) = self.start_of(self.event(master))?;
        Ok(key_of(&w) == Some(key))
    }

    /// `EXDATE` or `RECURRENCE-ID` for the occurrence `key`, written the way the series' own
    /// start is, so a calendar matches them.
    fn instance_line(&self, name: &str, master: usize, key: i64) -> Result<String> {
        let (p, w) = self.start_of(self.event(master))?;
        Ok(match w {
            When::Day(_) => format!("{name};VALUE=DATE:{key:08}"),
            When::At(_, tz) => {
                let t = at_second(key)?;
                if p.value.trim().ends_with(['Z', 'z']) {
                    format!("{name}:{}", utc_text(t))
                } else {
                    let clock = t.to_zoned(tz).strftime("%Y%m%dT%H%M%S").to_string();
                    match p.param("TZID") {
                        Some(id) => format!("{name};TZID={}:{clock}", param_value(id)),
                        None => format!("{name}:{clock}"),
                    }
                }
            }
        })
    }

    /// The series' rule, ended just before the occurrence `key`.
    fn end_before(&mut self, master: usize, key: i64) -> Result<()> {
        let (p, w) = self.start_of(self.event(master))?;
        let until = match w {
            When::Day(_) => {
                let d = Date::new(
                    i16::try_from(key / 10_000).unwrap_or(0),
                    i8::try_from(key / 100 % 100).unwrap_or(0),
                    i8::try_from(key % 100).unwrap_or(0),
                )
                .ok()
                .and_then(|d| d.checked_sub(1.day()).ok())
                .ok_or_else(|| Error::Protocol("an occurrence with no date".into()))?;
                date_text(d)
            }
            When::At(_, tz) => {
                let t = at_second(key - 1)?;
                if p.value.trim().ends_with(['Z', 'z']) || p.param("TZID").is_some() {
                    utc_text(t)
                } else {
                    t.to_zoned(tz).strftime("%Y%m%dT%H%M%S").to_string()
                }
            }
        };
        let rule = self
            .event(master)
            .prop("RRULE")
            .ok_or_else(|| Error::Refused("That event doesn't repeat.".into()))?;
        let kept: Vec<&str> = rule
            .value
            .split(';')
            .filter(|part| {
                let name = part.split('=').next().unwrap_or("").trim();
                !name.eq_ignore_ascii_case("COUNT") && !name.eq_ignore_ascii_case("UNTIL")
            })
            .collect();
        self.event_mut(master)
            .set("RRULE", format!("RRULE:{};UNTIL={until}", kept.join(";")));
        Ok(())
    }

    /// Changed occurrences at or after `key`, whose days the series no longer holds.
    fn drop_overrides_from(&mut self, key: i64) {
        let doomed: Vec<usize> = self
            .cal
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| match n {
                Node::Part(p) if p.name == "VEVENT" => {
                    self.override_key(p).filter(|k| *k >= key).map(|_| i)
                }
                _ => None,
            })
            .collect();
        for i in doomed.into_iter().rev() {
            self.cal.nodes.remove(i);
        }
    }

    fn drop_all_overrides(&mut self) {
        self.cal.nodes.retain(
            |n| !matches!(n, Node::Part(p) if p.name == "VEVENT" && p.prop("RECURRENCE-ID").is_some()),
        );
    }

    /// The file defines the zone `name`. Answers whether a block was put in for it, in front
    /// of every event.
    fn ensure_zone(&mut self, name: &str, at: i64) -> bool {
        let have = self
            .cal
            .parts("VTIMEZONE")
            .any(|z| z.prop("TZID").is_some_and(|p| p.value.trim() == name));
        if have {
            return false;
        }
        let Some(zone) = vtimezone(name, at) else {
            return false;
        };
        let at = self
            .cal
            .nodes
            .iter()
            .position(|n| matches!(n, Node::Part(_)))
            .unwrap_or(self.cal.nodes.len());
        self.cal.nodes.insert(at, Node::Part(zone));
        true
    }

    /// The event at `at` happens when `e` says.
    fn set_when(&mut self, at: usize, e: &Event) -> Result<()> {
        let (start, end, zone) = when_lines(e, self.here)?;
        let added = zone.is_some_and(|zone| self.ensure_zone(&zone, e.starts_at));
        let v = self.event_mut(at + usize::from(added));
        v.remove("DURATION");
        v.set("DTSTART", start);
        v.set("DTEND", end);
        Ok(())
    }

    fn change_one(&mut self, key: i64, c: &Change<'_>) -> Result<()> {
        match self.override_at(key) {
            Some(mut at) => {
                apply_words(self.event_mut(at), c.base, c.event);
                if c.base.is_none_or(|b| when_differs(b, c.event)) {
                    self.set_when(at, c.event)?;
                    at = self.override_at(key).ok_or_else(|| {
                        Error::Protocol("a changed occurrence went missing".into())
                    })?;
                }
                if let Some(alerts) = c.alerts {
                    set_alerts(self.event_mut(at), alerts);
                }
                let now = self.now;
                bump(self.event_mut(at), now)
            }
            None => {
                let master = self.master()?;
                let line = self.instance_line("RECURRENCE-ID", master, key)?;
                let mut one = self.event(master).clone();
                for name in ["RRULE", "RDATE", "EXDATE"] {
                    one.remove(name);
                }
                one.set("RECURRENCE-ID", line);
                apply_words(&mut one, None, c.event);
                if let Some(alerts) = c.alerts {
                    set_alerts(&mut one, alerts);
                }
                bump(&mut one, self.now)?;
                let at = self.cal.nodes.len();
                self.cal.nodes.push(Node::Part(one));
                self.set_when(at, c.event)
            }
        }
    }

    /// How far on the clock an edit moved an occurrence: read in its own zone, so a move over
    /// a change of the clocks is the hours it looks like.
    fn wall_shift(&self, base: &Event, mine: &Event) -> i64 {
        let zone = mine
            .timezone
            .as_deref()
            .and_then(|n| TimeZone::get(n).ok())
            .unwrap_or_else(|| self.here.clone());
        wall_clock(mine.starts_at, &zone) - wall_clock(base.starts_at, &zone)
    }

    fn change_series(&mut self, key: Option<i64>, c: &Change<'_>) -> Result<()> {
        let master = self.master()?;
        let never = matches!(c.repeat, Some(None));
        apply_words(self.event_mut(master), c.base, c.event);
        let moved = c.base.is_none_or(|b| when_differs(b, c.event));
        if moved {
            match (key, c.base) {
                // The occurrence moved: the series moves by as much, from its own first date.
                // One no longer repeating stays on the day being looked at.
                (Some(_), Some(base)) if !never => {
                    let shift = self.wall_shift(base, c.event);
                    let (_, w) = self.start_of(self.event(master))?;
                    let zone = match &w {
                        When::At(_, tz) => tz.clone(),
                        When::Day(_) => self.here.clone(),
                    };
                    let first = instant(&w)
                        .ok_or_else(|| {
                            Error::Protocol("an event whose start can't be read".into())
                        })?
                        .as_second();
                    let start = instant_of(wall_clock(first, &zone) + shift, &zone);
                    let series = Event {
                        starts_at: start,
                        ends_at: start + (c.event.ends_at - c.event.starts_at),
                        ..c.event.clone()
                    };
                    self.set_when(master, &series)?;
                    if shift != 0 {
                        self.shift_instances(shift);
                    }
                }
                _ => self.set_when(master, c.event)?,
            }
        }
        let master = self.master()?;
        match c.repeat {
            Some(Some(rule)) => {
                let v = self.event_mut(master);
                v.set("RRULE", rule_line(rule));
                v.remove("EXDATE");
                v.remove("RDATE");
                self.drop_all_overrides();
            }
            Some(None) => {
                let v = self.event_mut(master);
                for name in ["RRULE", "RDATE", "EXDATE"] {
                    v.remove(name);
                }
                self.drop_all_overrides();
            }
            None => {}
        }
        let master = self.master()?;
        if let Some(alerts) = c.alerts {
            set_alerts(self.event_mut(master), alerts);
        }
        let now = self.now;
        bump(self.event_mut(master), now)
    }

    /// `EXDATE` and `RECURRENCE-ID` still name the occurrences they did once the series moved.
    fn shift_instances(&mut self, seconds: i64) {
        for node in &mut self.cal.nodes {
            let Node::Part(v) = node else { continue };
            if v.name != "VEVENT" {
                continue;
            }
            for line in &mut v.nodes {
                let Node::Line(text) = line else { continue };
                if matches!(name_of(text).as_str(), "EXDATE" | "RECURRENCE-ID") {
                    if let Some(moved) = shift_line(text, seconds) {
                        *text = moved;
                    }
                }
            }
        }
    }
}

/// A new event's whole file.
pub fn new_event(uid: &str, c: &Change<'_>, now: i64, here: &TimeZone) -> Result<String> {
    let mut v = Part::new("VEVENT");
    v.add(format!("UID:{uid}"));
    let stamp = utc_text(at_second(now)?);
    v.add(format!("DTSTAMP:{stamp}"));
    v.add(format!("CREATED:{stamp}"));
    v.add(format!("LAST-MODIFIED:{stamp}"));
    apply_words(&mut v, None, c.event);
    if let Some(Some(rule)) = c.repeat {
        v.add(rule_line(rule));
    }
    if let Some(alerts) = c.alerts {
        set_alerts(&mut v, alerts);
    }
    let mut cal = Part::new("VCALENDAR");
    cal.add("VERSION:2.0".into());
    cal.add("PRODID:-//Skykeep//Calendar//EN".into());
    cal.add("CALSCALE:GREGORIAN".into());
    cal.nodes.push(Node::Part(v));
    let mut doc = Doc {
        cal,
        zones: zones::Defined::default(),
        here,
        now,
    };
    doc.set_when(3, c.event)?;
    Ok(doc.text())
}

/// The event's file, changed as `c` asks.
pub fn change(ics: &str, c: &Change<'_>, now: i64, here: &TimeZone) -> Result<Written> {
    let mut doc = Doc::read(ics, here, now)?;
    let key = occurrence_key(&c.event.guid);
    match (key, c.scope) {
        (Some(key), Some(Scope::This) | None) => doc.change_one(key, c)?,
        (Some(key), Some(Scope::Future)) => {
            let master = doc.master()?;
            if !doc.is_first(master, key)? {
                return split(ics, doc, master, key, c);
            }
            doc.change_series(Some(key), c)?;
        }
        (key, _) => doc.change_series(key, c)?,
    }
    Ok(Written::Whole(doc.text()))
}

/// "This and all after": the series ended before this occurrence, and a new event from it on.
fn split(ics: &str, mut kept: Doc<'_>, master: usize, key: i64, c: &Change<'_>) -> Result<Written> {
    let mut rest = Doc {
        cal: kept.cal.clone(),
        zones: kept.zones.clone(),
        here: kept.here,
        now: kept.now,
    };
    kept.end_before(master, key)?;
    let now = kept.now;
    bump(kept.event_mut(master), now)?;
    kept.drop_overrides_from(key);

    rest.drop_all_overrides();
    let at = rest.master()?;
    let uid = uuid::Uuid::new_v4().to_string().to_uppercase();
    // A rule ending after so many times still ends where it did: fewer are left.
    let before = feeds::parse(ics, kept.here)?
        .items
        .iter()
        .find(|i| i.replaces.is_none())
        .map_or(0, |item| {
            let to = c
                .base
                .map_or(c.event.starts_at, |b| b.starts_at.max(c.event.starts_at));
            starts(item, to + 1)
                .iter()
                .filter(|w| key_of(w).is_some_and(|k| k < key))
                .count()
        });
    let v = rest.event_mut(at);
    v.set("UID", format!("UID:{uid}"));
    if let Some(rule) = v.prop("RRULE") {
        let parts: Vec<String> = rule
            .value
            .split(';')
            .map(|part| match part.split_once('=') {
                Some((name, n)) if name.trim().eq_ignore_ascii_case("COUNT") => {
                    let left = n
                        .trim()
                        .parse::<i64>()
                        .map_or(1, |n| (n - i64::try_from(before).unwrap_or(0)).max(1));
                    format!("COUNT={left}")
                }
                _ => part.to_string(),
            })
            .collect();
        v.set("RRULE", format!("RRULE:{}", parts.join(";")));
    }
    v.remove("RECURRENCE-ID");
    apply_words(v, None, c.event);
    if let Some(alerts) = c.alerts {
        set_alerts(v, alerts);
    }
    bump(v, kept.now)?;
    rest.set_when(at, c.event)?;
    Ok(Written::Split {
        kept: kept.text(),
        rest: rest.text(),
        uid,
    })
}

/// The event's file with the occurrences asked for gone; or that the whole file goes.
pub fn remove(
    ics: &str,
    event: &Event,
    scope: Option<Scope>,
    now: i64,
    here: &TimeZone,
) -> Result<Removal> {
    let Some(key) = occurrence_key(&event.guid) else {
        return Ok(Removal::Whole);
    };
    if scope == Some(Scope::All) {
        return Ok(Removal::Whole);
    }
    let mut doc = Doc::read(ics, here, now)?;
    // An occurrence one was invited to alone: the file is that occurrence.
    let Some(master) = doc.master_at() else {
        return Ok(Removal::Whole);
    };
    if scope == Some(Scope::Future) {
        if doc.is_first(master, key)? {
            return Ok(Removal::Whole);
        }
        doc.end_before(master, key)?;
        doc.drop_overrides_from(key);
    } else {
        if let Some(at) = doc.override_at(key) {
            doc.cal.nodes.remove(at);
        }
        let master = doc.master()?;
        let line = doc.instance_line("EXDATE", master, key)?;
        doc.event_mut(master).add(line);
    }
    let master = doc.master()?;
    bump(doc.event_mut(master), now)?;
    Ok(Removal::Kept(doc.text()))
}

/// How the file's series repeats, as the event form shows it: read as every way in reads a rule
/// ([`crate::rrule`]). A rule saying more than Apple's repeat holds (the second Tuesday) still
/// shows its frequency, as the form can say no more; the file's own rule stays as it is unless
/// the repeat is changed.
#[must_use]
pub fn repeat_of(ics: &str, here: &TimeZone) -> Option<EventRepeat> {
    let doc = Doc::read(ics, here, 0).ok()?;
    let master = doc.event(doc.master_at()?);
    let rule = master.prop("RRULE")?;
    let (first, zone, all_day) = match doc.start_of(master).ok()?.1 {
        When::Day(d) => (
            d.to_datetime(jiff::civil::Time::midnight()),
            here.clone(),
            true,
        ),
        When::At(at, tz) => (at, tz, false),
    };
    if let Ok(found) = crate::rrule::from_rrule(&rule.value, first, &zone, all_day) {
        return found;
    }
    let mut repeat = EventRepeat {
        interval: 1,
        ..EventRepeat::default()
    };
    for part in rule.value.split(';') {
        let (name, value) = part.split_once('=')?;
        match name.trim().to_ascii_uppercase().as_str() {
            "FREQ" => repeat.freq = value.trim().to_ascii_lowercase(),
            "INTERVAL" => repeat.interval = value.trim().parse().ok()?,
            "COUNT" => repeat.count = value.trim().parse().ok(),
            _ => {}
        }
    }
    matches!(
        repeat.freq.as_str(),
        "daily" | "weekly" | "monthly" | "yearly"
    )
    .then_some(repeat)
}

/// The block for this event: a changed occurrence's own, or the series'.
fn block_for<'d>(doc: &'d Doc<'_>, guid: &str) -> Option<&'d Part> {
    let own = occurrence_key(guid).and_then(|k| doc.override_at(k));
    let at = own.or_else(|| doc.master_at())?;
    Some(doc.event(at))
}

/// Its alerts: minutes before the start, `0` at the time, less than `0` after it.
#[must_use]
pub fn alerts_of(ics: &str, guid: &str, here: &TimeZone) -> Vec<i64> {
    let Ok(doc) = Doc::read(ics, here, 0) else {
        return Vec::new();
    };
    let Some(v) = block_for(&doc, guid) else {
        return Vec::new();
    };
    let mut minutes: Vec<i64> = v
        .parts("VALARM")
        .filter_map(|a| {
            let t = a.prop("TRIGGER")?;
            if t.param("VALUE")
                .is_some_and(|x| x.eq_ignore_ascii_case("DATE-TIME"))
                || t.param("RELATED")
                    .is_some_and(|x| x.eq_ignore_ascii_case("END"))
            {
                return None;
            }
            Some(-duration(&t.value)? / 60)
        })
        .collect();
    minutes.sort_unstable();
    minutes.dedup();
    minutes
}

/// Its guests and their answers, the organizer first, then by name.
#[must_use]
pub fn guests_of(ics: &str, guid: &str, me: &str, here: &TimeZone) -> Vec<Guest> {
    let Ok(doc) = Doc::read(ics, here, 0) else {
        return Vec::new();
    };
    let Some(v) = block_for(&doc, guid) else {
        return Vec::new();
    };
    let person = |p: &Property, organizer: bool| {
        let email = p
            .value
            .trim()
            .strip_prefix("mailto:")
            .or_else(|| p.value.trim().strip_prefix("MAILTO:"))
            .map(|m| unescape(m.trim()));
        let status = match p
            .param("PARTSTAT")
            .unwrap_or("")
            .to_ascii_uppercase()
            .as_str()
        {
            "ACCEPTED" => "accepted",
            "DECLINED" => "declined",
            "TENTATIVE" => "tentative",
            _ if organizer => "accepted",
            _ => "pending",
        };
        Guest {
            name: p.param("CN").map(ToString::to_string),
            is_me: email.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(me)),
            email,
            status,
            is_organizer: organizer,
        }
    };
    let mut guests: Vec<Guest> = v
        .props("ORGANIZER")
        .iter()
        .map(|p| person(p, true))
        .collect();
    let organizers: Vec<Option<String>> = guests.iter().map(|g| g.email.clone()).collect();
    guests.extend(
        v.props("ATTENDEE")
            .iter()
            .map(|p| person(p, false))
            .filter(|g| g.email.is_none() || !organizers.contains(&g.email)),
    );
    guests.sort_by_key(|g| (!g.is_organizer, g.name.clone().or_else(|| g.email.clone())));
    guests
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn at(s: &str) -> i64 {
        let (dt, _) = parse_datetime(s).unwrap();
        dt.to_zoned(berlin()).unwrap().timestamp().as_second()
    }

    const SINGLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Nextcloud//EN\r\n\
BEGIN:VEVENT\r\nUID:one\r\nSUMMARY:Lunch\r\nLOCATION:Cafe\r\n\
DTSTART;TZID=Europe/Berlin:20261105T120000\r\nDTEND;TZID=Europe/Berlin:20261105T130000\r\n\
X-NEXTCLOUD-THING:keep me\r\nATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.org\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    const WEEKLY: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:w\r\n\
SUMMARY:Standup\r\nDTSTART;TZID=Europe/Berlin:20261102T090000\r\n\
DTEND;TZID=Europe/Berlin:20261102T093000\r\nRRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn event(guid: &str, title: &str, start: &str, end: &str) -> Event {
        Event {
            guid: guid.into(),
            calendar_guid: "cal".into(),
            title: title.into(),
            starts_at: at(start),
            ends_at: at(end),
            timezone: Some("Europe/Berlin".into()),
            ..Default::default()
        }
    }

    fn whole(w: Written) -> String {
        match w {
            Written::Whole(t) => t,
            Written::Split { .. } => panic!("expected one file"),
        }
    }

    fn plain(c: &Change<'_>, ics: &str) -> String {
        whole(change(ics, c, NOW, &berlin()).unwrap())
    }

    fn change_of<'a>(event: &'a Event, base: Option<&'a Event>) -> Change<'a> {
        Change {
            event,
            base,
            alerts: None,
            repeat: None,
            scope: None,
        }
    }

    fn occurrences(ics: &str) -> Vec<Event> {
        let feed = feeds::parse(ics, &berlin()).unwrap();
        feeds::account_events(
            &feed,
            "cal",
            "s",
            None,
            at("20261101T000000"),
            at("20261201T000000"),
        )
    }

    /// Whatever Skykeep has no field for stays in the file.
    #[test]
    fn a_title_change_touches_nothing_else() {
        let base = Event {
            location: Some("Cafe".into()),
            ..event("one", "Lunch", "20261105T120000", "20261105T130000")
        };
        let mine = Event {
            title: "Long lunch; with, friends".into(),
            ..base.clone()
        };
        let out = plain(&change_of(&mine, Some(&base)), SINGLE);
        assert!(
            out.contains("SUMMARY:Long lunch\\; with\\, friends"),
            "{out}"
        );
        assert!(out.contains("X-NEXTCLOUD-THING:keep me"));
        assert!(out.contains("ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.org"));
        assert!(out.contains("LOCATION:Cafe"));
        assert!(out.contains("TRIGGER:-PT15M"));
        assert!(out.contains("DTSTART;TZID=Europe/Berlin:20261105T120000"));
        assert!(out.contains("SEQUENCE:1"));
    }

    /// Saved against a copy another device has changed since: a field changed only there stays.
    #[test]
    fn a_field_changed_on_another_device_is_kept() {
        let base = Event {
            location: Some("Old place".into()),
            ..event("one", "Lunch", "20261105T120000", "20261105T130000")
        };
        let mine = Event {
            title: "Brunch".into(),
            ..base.clone()
        };
        // Base said "Old place"; the server's copy (SINGLE) says "Cafe"; this edit left it.
        let out = plain(&change_of(&mine, Some(&base)), SINGLE);
        assert!(out.contains("LOCATION:Cafe"), "{out}");
        assert!(out.contains("SUMMARY:Brunch"));
    }

    #[test]
    fn a_moved_event_writes_its_new_times_and_a_cleared_field_goes() {
        let base = Event {
            location: Some("Cafe".into()),
            ..event("one", "Lunch", "20261105T120000", "20261105T130000")
        };
        let mine = Event {
            location: None,
            ..event("one", "Lunch", "20261105T140000", "20261105T153000")
        };
        let out = plain(&change_of(&mine, Some(&base)), SINGLE);
        assert!(
            out.contains("DTSTART;TZID=Europe/Berlin:20261105T140000"),
            "{out}"
        );
        assert!(out.contains("DTEND;TZID=Europe/Berlin:20261105T153000"));
        assert!(!out.contains("LOCATION"));
        assert!(
            out.contains("BEGIN:VTIMEZONE"),
            "the zone it names is defined"
        );
    }

    #[test]
    fn alerts_set_here_replace_the_files_and_read_back() {
        let base = event("one", "Lunch", "20261105T120000", "20261105T130000");
        let c = Change {
            alerts: Some(&[30, 0]),
            ..change_of(&base, Some(&base))
        };
        let out = plain(&c, SINGLE);
        assert!(!out.contains("-PT15M"));
        assert_eq!(alerts_of(&out, "one", &berlin()), vec![0, 30]);
        assert_eq!(alerts_of(SINGLE, "one", &berlin()), vec![15]);
    }

    /// "On the day at 9 AM" is an alert after an all-day event's midnight start.
    #[test]
    fn an_alert_after_the_start_keeps_its_sign() {
        let base = event("one", "Lunch", "20261105T120000", "20261105T130000");
        let c = Change {
            alerts: Some(&[-540]),
            ..change_of(&base, Some(&base))
        };
        let out = plain(&c, SINGLE);
        assert!(out.contains("TRIGGER:PT540M"), "{out}");
        assert_eq!(alerts_of(&out, "one", &berlin()), vec![-540]);
    }

    #[test]
    fn one_occurrence_changed_alone_leaves_the_series() {
        let key = at("20261109T090000");
        let guid = format!("s__{key}");
        let base = event(&guid, "Standup", "20261109T090000", "20261109T093000");
        let mine = Event {
            title: "Standup, moved".into(),
            ..event(&guid, "Standup", "20261109T100000", "20261109T103000")
        };
        let c = Change {
            scope: Some(Scope::This),
            ..change_of(&mine, Some(&base))
        };
        let out = plain(&c, WEEKLY);
        assert!(
            out.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261109T090000"),
            "{out}"
        );
        let all = occurrences(&out);
        assert_eq!(all.len(), 5, "{all:?}");
        let moved = all.iter().find(|e| e.guid == guid).unwrap();
        assert_eq!(moved.title, "Standup, moved");
        assert_eq!(moved.starts_at, at("20261109T100000"));
        assert_eq!(
            all.iter().filter(|e| e.title == "Standup").count(),
            4,
            "the others as they were"
        );

        // Changed again: the same occurrence, not a second one.
        let again = Event {
            title: "Third".into(),
            ..moved.clone()
        };
        let c = Change {
            scope: Some(Scope::This),
            ..change_of(&again, Some(moved))
        };
        let twice = plain(&c, &out);
        assert_eq!(twice.matches("RECURRENCE-ID").count(), 1, "{twice}");
        assert_eq!(
            occurrences(&twice)
                .iter()
                .find(|e| e.guid == guid)
                .unwrap()
                .title,
            "Third"
        );
    }

    #[test]
    fn the_whole_series_moves_by_what_one_occurrence_moved() {
        let key = at("20261116T090000");
        let guid = format!("s__{key}");
        let base = event(&guid, "Standup", "20261116T090000", "20261116T093000");
        let mine = event(&guid, "Standup", "20261116T110000", "20261116T113000");
        let c = Change {
            scope: Some(Scope::All),
            ..change_of(&mine, Some(&base))
        };
        let out = plain(&c, WEEKLY);
        assert!(
            out.contains("DTSTART;TZID=Europe/Berlin:20261102T110000"),
            "{out}"
        );
        assert_eq!(occurrences(&out)[0].starts_at, at("20261102T110000"));
    }

    #[test]
    fn a_deleted_occurrence_stays_deleted_when_the_series_moves() {
        let key = at("20261109T090000");
        let gone = event(
            &format!("s__{key}"),
            "Standup",
            "20261109T090000",
            "20261109T093000",
        );
        let Removal::Kept(without) =
            remove(WEEKLY, &gone, Some(Scope::This), NOW, &berlin()).unwrap()
        else {
            panic!("the series stays");
        };
        assert!(!occurrences(&without).iter().any(|e| e.starts_at == key));

        let key2 = at("20261116T090000");
        let guid = format!("s__{key2}");
        let base = event(&guid, "Standup", "20261116T090000", "20261116T093000");
        let mine = event(&guid, "Standup", "20261116T110000", "20261116T113000");
        let c = Change {
            scope: Some(Scope::All),
            ..change_of(&mine, Some(&base))
        };
        let out = plain(&c, &without);
        assert!(
            !occurrences(&out)
                .iter()
                .any(|e| e.starts_at == at("20261109T110000")),
            "{out}"
        );
    }

    #[test]
    fn this_and_all_after_splits_the_series() {
        let key = at("20261116T090000");
        let guid = format!("s__{key}");
        let base = event(&guid, "Standup", "20261116T090000", "20261116T093000");
        let mine = Event {
            title: "New standup".into(),
            ..event(&guid, "Standup", "20261116T100000", "20261116T103000")
        };
        let c = Change {
            scope: Some(Scope::Future),
            ..change_of(&mine, Some(&base))
        };
        let Written::Split { kept, rest, uid } = change(WEEKLY, &c, NOW, &berlin()).unwrap() else {
            panic!("expected a split");
        };
        assert_eq!(occurrences(&kept).len(), 2, "the two before it: {kept}");
        assert!(occurrences(&kept).iter().all(|e| e.title == "Standup"));
        let later = occurrences(&rest);
        assert_eq!(later.len(), 3, "{rest}");
        assert_eq!(later[0].starts_at, at("20261116T100000"));
        assert!(later.iter().all(|e| e.title == "New standup"));
        assert!(rest.contains(&format!("UID:{uid}")));
        assert_ne!(uid, "w");
    }

    #[test]
    fn this_and_all_after_on_the_first_is_the_whole_series() {
        let key = at("20261102T090000");
        let guid = format!("s__{key}");
        let base = event(&guid, "Standup", "20261102T090000", "20261102T093000");
        let mine = Event {
            title: "Renamed".into(),
            ..base.clone()
        };
        let c = Change {
            scope: Some(Scope::Future),
            ..change_of(&mine, Some(&base))
        };
        assert!(matches!(
            change(WEEKLY, &c, NOW, &berlin()).unwrap(),
            Written::Whole(_)
        ));
    }

    #[test]
    fn a_count_left_after_a_split_is_what_remains() {
        let counted = WEEKLY.replace("RRULE:FREQ=WEEKLY", "RRULE:FREQ=WEEKLY;COUNT=5");
        let key = at("20261116T090000");
        let guid = format!("s__{key}");
        let base = event(&guid, "Standup", "20261116T090000", "20261116T093000");
        let c = Change {
            scope: Some(Scope::Future),
            ..change_of(&base, Some(&base))
        };
        let Written::Split { kept, rest, .. } = change(&counted, &c, NOW, &berlin()).unwrap()
        else {
            panic!("expected a split");
        };
        assert!(rest.contains("COUNT=3"), "{rest}");
        assert!(!kept.contains("COUNT"), "{kept}");
    }

    #[test]
    fn deleting_this_and_after_ends_the_series() {
        let key = at("20261116T090000");
        let e = event(
            &format!("s__{key}"),
            "Standup",
            "20261116T090000",
            "20261116T093000",
        );
        let Removal::Kept(out) = remove(WEEKLY, &e, Some(Scope::Future), NOW, &berlin()).unwrap()
        else {
            panic!("the first two stay");
        };
        assert_eq!(occurrences(&out).len(), 2, "{out}");
        let all = remove(WEEKLY, &e, Some(Scope::All), NOW, &berlin()).unwrap();
        assert_eq!(all, Removal::Whole);
        let single = event("one", "Lunch", "20261105T120000", "20261105T130000");
        assert_eq!(
            remove(SINGLE, &single, None, NOW, &berlin()).unwrap(),
            Removal::Whole
        );
    }

    #[test]
    fn a_new_event_is_a_whole_valid_file_that_reads_back() {
        let mine = Event {
            notes: Some("line one\nline two".into()),
            url: Some("https://example.org/a,b".into()),
            ..event("x", "Dentist", "20261105T120000", "20261105T130000")
        };
        let repeat = EventRepeat {
            freq: "weekly".into(),
            interval: 2,
            count: Some(3),
            ..EventRepeat::default()
        };
        let c = Change {
            alerts: Some(&[10]),
            repeat: Some(Some(&repeat)),
            ..change_of(&mine, None)
        };
        let out = new_event("UID-1", &c, NOW, &berlin()).unwrap();
        assert!(out.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(out.contains("UID:UID-1"));
        assert!(out.contains("DESCRIPTION:line one\\nline two"));
        assert!(out.contains("URL:https://example.org/a,b"));
        assert!(out.contains("BEGIN:VTIMEZONE"));
        assert_eq!(repeat_of(&out, &berlin()), Some(repeat));
        assert_eq!(alerts_of(&out, "x", &berlin()), vec![10]);
        let vt = out.find("BEGIN:VTIMEZONE").unwrap();
        assert!(vt < out.find("BEGIN:VEVENT").unwrap());
        let read = occurrences(&out);
        assert_eq!(read[0].title, "Dentist");
        assert_eq!(read[0].notes.as_deref(), Some("line one\nline two"));
        assert_eq!(read[0].starts_at, at("20261105T120000"));
    }

    #[test]
    fn an_all_day_event_ends_on_the_day_after_its_last() {
        let start = berlin_midnight("20261105");
        let e = Event {
            all_day: true,
            timezone: None,
            starts_at: start,
            ends_at: berlin_midnight("20261107"),
            ..event("x", "Trip", "20261105T000000", "20261105T000000")
        };
        let out = new_event("U", &change_of(&e, None), NOW, &berlin()).unwrap();
        assert!(out.contains("DTSTART;VALUE=DATE:20261105"), "{out}");
        assert!(out.contains("DTEND;VALUE=DATE:20261107"), "{out}");
        assert!(!out.contains("VTIMEZONE"));

        // Ending on the same instant it starts: still one whole day.
        let one = Event {
            ends_at: start,
            ..e
        };
        let out = new_event("U", &change_of(&one, None), NOW, &berlin()).unwrap();
        assert!(out.contains("DTEND;VALUE=DATE:20261106"), "{out}");
    }

    fn berlin_midnight(day: &str) -> i64 {
        parse_date(day)
            .unwrap()
            .to_zoned(berlin())
            .unwrap()
            .timestamp()
            .as_second()
    }

    #[test]
    fn a_long_line_is_folded_without_cutting_a_character() {
        let base = event("one", "Lunch", "20261105T120000", "20261105T130000");
        let mine = Event {
            title: "ä".repeat(120),
            ..base.clone()
        };
        let out = plain(&change_of(&mine, Some(&base)), SINGLE);
        assert!(out.split("\r\n").all(|l| l.len() <= 75), "{out}");
        assert_eq!(occurrences(&out)[0].title, "ä".repeat(120));
    }

    #[test]
    fn guests_are_read_with_the_organizer_first() {
        let ics = SINGLE.replace(
            "X-NEXTCLOUD-THING:keep me",
            "ORGANIZER;CN=Org:mailto:org@example.org\r\n\
             ATTENDEE;CN=Org;PARTSTAT=ACCEPTED:mailto:org@example.org\r\n\
             ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.org",
        );
        let guests = guests_of(&ics, "one", "me@example.org", &berlin());
        assert_eq!(guests.len(), 3, "the organizer once: {guests:?}");
        assert!(guests[0].is_organizer);
        assert_eq!(guests[0].email.as_deref(), Some("org@example.org"));
        let me = guests.iter().find(|g| g.is_me).unwrap();
        assert_eq!(me.status, "pending");
        let sam = guests
            .iter()
            .find(|g| g.name.as_deref() == Some("Sam"))
            .unwrap();
        assert_eq!(sam.status, "accepted");
    }

    #[test]
    fn a_weekly_rule_reads_as_the_form_shows_it() {
        assert_eq!(
            repeat_of(WEEKLY, &berlin()),
            Some(EventRepeat {
                freq: "weekly".into(),
                interval: 1,
                ..EventRepeat::default()
            })
        );
        assert_eq!(repeat_of(SINGLE, &berlin()), None);
    }

    #[test]
    fn a_damaged_file_is_refused_not_written_over() {
        let e = event("one", "Lunch", "20261105T120000", "20261105T130000");
        assert!(change("not a calendar", &change_of(&e, None), NOW, &berlin()).is_err());
    }
}
