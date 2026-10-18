//! The merge records inside an Apple Notes or Reminders document.
//!
//! The text a document carries is a cache. What Apple's devices actually merge is a list of
//! substrings, each naming who wrote a stretch of characters and whether it has since been
//! deleted. A phone reading a document rebuilds the note from those records, so text with no
//! records behind it shows as an empty note, and text whose records describe something else
//! shows as whatever the records say — a word repeated, a line that will not go away.
//!
//! Measured on real documents written by a phone, every one of them the same:
//!
//! - The substrings sit in the note at field 3, in reading order, each pointing at the next
//!   by index (field 5). The first is a start marker with id (0, 0); the last is an end marker
//!   with id (0, 0xFFFFFFFF). Neither holds text.
//! - A substring is `{1: id, 2: length, 3: timestamp, 4: deleted, 5: next}`, where an id and
//!   a timestamp are `{1: writer, 2: clock}`. Its characters are numbered clock, clock+1, …
//! - Lengths are UTF-16 units, and the lengths of the ones not deleted add up to the text.
//! - The writers are listed at field 4 as `{1: {1: uuid, 2: {1: next character},
//!   2: {1: next timestamp}}, …}`, and an id's writer is its 1-based place in that list.
//!
//! An edit here is made the way a device makes one: what was removed is marked deleted, never
//! dropped, and what was typed is a new substring under this app's own writer. Characters
//! other devices wrote keep their ids, so their copies of the note line up with this one.

use std::sync::OnceLock;

/// The id of the end marker's characters.
const END: u64 = 0xFFFF_FFFF;

/// This app's writer id: one per installation, so two computers never number characters
/// from the same writer. Until one is set, a fresh id per run is still correct — only less
/// tidy, since each run that edits a note adds itself to that note's list of writers.
static WRITER: OnceLock<[u8; 16]> = OnceLock::new();

/// Adopt this installation's writer id. Only the first call counts.
pub fn set_writer(id: [u8; 16]) {
    let _ = WRITER.set(id);
}

/// This installation's writer id, as Apple writes a replica id: an uppercase UUID.
#[must_use]
pub fn writer_id() -> String {
    uuid::Uuid::from_bytes(writer()).to_string().to_uppercase()
}

fn writer() -> [u8; 16] {
    *WRITER.get_or_init(|| *uuid::Uuid::new_v4().as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Substring {
    writer: u64,
    clock: u64,
    length: u64,
    stamp_writer: u64,
    stamp_clock: u64,
    deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Writer {
    uuid: Vec<u8>,
    /// Next character clock, then next timestamp clock. Kept as a list because that is what
    /// the field is, and a writer entry with some other count must survive untouched.
    clocks: Vec<u64>,
}

/// The records of one note, in the order they are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Topo {
    substrings: Vec<Substring>,
    writers: Vec<Writer>,
}

/// The raw graph of a note's records, for looking at one that will not read: each
/// substring's id, length, deleted flag and links.
#[must_use]
pub fn debug_graph(note: &[u8]) -> Vec<String> {
    let Some(all) = fields(note) else {
        return vec!["unreadable fields".into()];
    };
    let mut out = Vec::new();
    for (i, (field, wire, payload, _)) in all.iter().filter(|f| f.0 == 3 && f.1 == 2).enumerate() {
        let _ = (field, wire);
        match substring(payload) {
            Some((s, next)) => out.push(format!(
                "{i}: id {}:{} len {} stamp {}:{} del {} next {:?}",
                s.writer, s.clock, s.length, s.stamp_writer, s.stamp_clock, s.deleted, next
            )),
            None => out.push(format!(
                "{i}: UNPARSEABLE {:?}",
                fields(payload).map(|f| f.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>())
            )),
        }
    }
    out
}

// ── Reading ────────────────────────────────────────────────────────────────────────────

/// One field: its number, its wire type, the payload of a length-delimited one, and the
/// value of a varint.
type Field<'a> = (u64, u64, &'a [u8], u64);

fn fields(bytes: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let (key, next) = varint(bytes, at)?;
        at = next;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                let (v, next) = varint(bytes, at)?;
                at = next;
                out.push((field, wire, &bytes[0..0], v));
            }
            2 => {
                let (n, next) = varint(bytes, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                out.push((field, wire, bytes.get(next..end)?, 0));
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= bytes.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= bytes.len())?,
            _ => return None,
        }
    }
    Some(out)
}

fn varint(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7F) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

fn id(bytes: &[u8]) -> Option<(u64, u64)> {
    let mut id = (0, 0);
    for (field, wire, _, v) in fields(bytes)? {
        match (field, wire) {
            (1, 0) => id.0 = v,
            (2, 0) => id.1 = v,
            _ => {}
        }
    }
    Some(id)
}

fn substring(bytes: &[u8]) -> Option<(Substring, Vec<u64>)> {
    let mut s = Substring {
        writer: 0,
        clock: 0,
        length: 0,
        stamp_writer: 0,
        stamp_clock: 0,
        deleted: false,
    };
    let mut next = Vec::new();
    for (field, wire, payload, v) in fields(bytes)? {
        match (field, wire) {
            (1, 2) => (s.writer, s.clock) = id(payload)?,
            (2, 0) => s.length = v,
            (3, 2) => (s.stamp_writer, s.stamp_clock) = id(payload)?,
            (4, 0) => s.deleted = v != 0,
            (5, 0) => next.push(v),
            // A field this does not know means a document this does not understand.
            _ => return None,
        }
    }
    Some((s, next))
}

fn writers(bytes: &[u8]) -> Option<Vec<Writer>> {
    let mut out = Vec::new();
    for (field, wire, payload, _) in fields(bytes)? {
        if (field, wire) != (1, 2) {
            return None;
        }
        let mut w = Writer {
            uuid: Vec::new(),
            clocks: Vec::new(),
        };
        for (field, wire, payload, _) in fields(payload)? {
            match (field, wire) {
                (1, 2) => w.uuid = payload.to_vec(),
                (2, 2) => {
                    let mut clock = None;
                    for (field, wire, _, v) in fields(payload)? {
                        match (field, wire) {
                            (1, 0) => clock = Some(v),
                            _ => return None,
                        }
                    }
                    w.clocks.push(clock.unwrap_or(0));
                }
                _ => return None,
            }
        }
        out.push(w);
    }
    Some(out)
}

impl Topo {
    /// The records of a note message, or `None` when it has none or they are not shaped the
    /// way Apple writes them. A graph that is not a single chain in reading order is refused
    /// rather than guessed at.
    pub(crate) fn read(note: &[u8]) -> Option<Self> {
        let mut substrings = Vec::new();
        let mut links = Vec::new();
        let mut writer_list = None;
        for (field, wire, payload, _) in fields(note)? {
            match (field, wire) {
                (3, 2) => {
                    let (s, next) = substring(payload)?;
                    substrings.push(s);
                    links.push(next);
                }
                (4, 2) => writer_list = Some(writers(payload)?),
                _ => {}
            }
        }
        if substrings.len() < 2 {
            return None;
        }
        // Apple lists the substrings in reading order. A note edited on two devices at once
        // branches — a substring with two successors, the branches joining further on — and
        // stays in reading order; every link then points forward in the list, though not
        // always to the next one. That is read as the order it is listed in. A link that
        // points back, or a substring that leads nowhere, is a graph this does not know how
        // to order, and it is refused rather than guessed at.
        let last = links.len() - 1;
        for (i, next) in links.iter().enumerate() {
            if i == last {
                if !next.is_empty() {
                    return None;
                }
            } else if next.is_empty() || next.iter().any(|&n| n as usize <= i || n as usize > last)
            {
                return None;
            }
        }
        // And every substring but the start is reached from one before it: nothing hangs loose.
        let mut reached = vec![false; links.len()];
        reached[0] = true;
        for next in &links {
            for &n in next {
                reached[n as usize] = true;
            }
        }
        if reached.iter().any(|r| !r) {
            return None;
        }
        let first = &substrings[0];
        let end = &substrings[last];
        if (first.writer, first.clock, first.length) != (0, 0, 0)
            || (end.writer, end.clock, end.length) != (0, END, 0)
        {
            return None;
        }
        Some(Self {
            substrings,
            writers: writer_list.unwrap_or_default(),
        })
    }

    /// Records for a note that holds `length` units of text, all written by this app.
    pub(crate) fn fresh(length: u64) -> Self {
        let mut topo = Self {
            substrings: vec![
                Substring {
                    writer: 0,
                    clock: 0,
                    length: 0,
                    stamp_writer: 0,
                    stamp_clock: 0,
                    deleted: false,
                },
                Substring {
                    writer: 0,
                    clock: END,
                    length: 0,
                    stamp_writer: 0,
                    stamp_clock: END,
                    deleted: false,
                },
            ],
            writers: Vec::new(),
        };
        topo.insert(0, length);
        topo
    }

    /// How much text the records say is there.
    pub(crate) fn live_length(&self) -> u64 {
        self.substrings
            .iter()
            .filter(|s| !s.deleted)
            .map(|s| s.length)
            .sum()
    }

    // ── Editing ────────────────────────────────────────────────────────────────────────

    /// Make `at` fall on a boundary between substrings, splitting the one it lands inside.
    fn split_at(&mut self, at: u64) {
        let mut offset = 0;
        for i in 0..self.substrings.len() {
            let s = &self.substrings[i];
            if s.deleted {
                continue;
            }
            if offset < at && at < offset + s.length {
                let keep = at - offset;
                let mut tail = s.clone();
                tail.clock += keep;
                tail.length -= keep;
                self.substrings[i].length = keep;
                self.substrings.insert(i + 1, tail);
                return;
            }
            offset += s.length;
        }
    }

    /// This app's place in the list of writers, adding it if it is not there yet.
    fn own_writer(&mut self) -> usize {
        let me = writer();
        if let Some(i) = self.writers.iter().position(|w| w.uuid == me) {
            while self.writers[i].clocks.len() < 2 {
                self.writers[i].clocks.push(0);
            }
            return i;
        }
        self.writers.push(Writer {
            uuid: me.to_vec(),
            clocks: vec![0, 0],
        });
        self.writers.len() - 1
    }

    /// New text of `length` units at `at`, as a substring of this app's own.
    fn insert(&mut self, at: u64, length: u64) {
        if length == 0 {
            return;
        }
        self.split_at(at);
        let me = self.own_writer();
        let clock = self.writers[me].clocks[0];
        let stamp = self.writers[me].clocks[1];

        // Right after the last live character before `at`; the start marker when `at` is 0.
        let mut offset = 0;
        let mut place = 1;
        for (i, s) in self.substrings.iter().enumerate().skip(1) {
            if offset == at {
                place = i;
                break;
            }
            if !s.deleted {
                offset += s.length;
            }
            place = i + 1;
        }
        let place = place.min(self.substrings.len() - 1);

        self.substrings.insert(
            place,
            Substring {
                writer: me as u64 + 1,
                clock,
                length,
                stamp_writer: me as u64 + 1,
                stamp_clock: stamp,
                deleted: false,
            },
        );
        self.writers[me].clocks[0] = clock + length;
        self.writers[me].clocks[1] = stamp + 1;
    }

    /// Mark `length` units from `at` deleted. The characters keep their ids, which is what
    /// lets another device's copy agree that they are gone.
    fn delete(&mut self, at: u64, length: u64) {
        if length == 0 {
            return;
        }
        self.split_at(at);
        self.split_at(at + length);
        let mut offset = 0;
        for s in &mut self.substrings {
            if s.deleted {
                continue;
            }
            if offset >= at && offset + s.length <= at + length && s.length > 0 {
                s.deleted = true;
            }
            offset += s.length;
        }
    }

    /// Apply one edit: `removed` units from `from` replaced by `inserted` new ones.
    pub(crate) fn edit(&mut self, from: u64, removed: u64, inserted: u64) {
        self.delete(from, removed);
        self.insert(from, inserted);
    }

    // ── Writing ────────────────────────────────────────────────────────────────────────

    /// The note fields these records occupy: every substring at 3, then the writers at 4.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let last = self.substrings.len() - 1;
        for (i, s) in self.substrings.iter().enumerate() {
            let mut m = message(1, &pair(s.writer, s.clock));
            m.extend(varint_field(2, s.length));
            m.extend(message(3, &pair(s.stamp_writer, s.stamp_clock)));
            if s.deleted {
                m.extend(varint_field(4, 1));
            }
            if i != last {
                m.extend(varint_field(5, i as u64 + 1));
            }
            out.extend(message(3, &m));
        }
        let mut list = Vec::new();
        for w in &self.writers {
            let mut entry = message(1, &w.uuid);
            for clock in &w.clocks {
                entry.extend(message(2, &varint_field(1, *clock)));
            }
            list.extend(message(1, &entry));
        }
        out.extend(message(4, &list));
        out
    }
}

fn write_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

fn varint_field(tag: u64, value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint(tag << 3, &mut out);
    write_varint(value, &mut out);
    out
}

fn message(tag: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint((tag << 3) | 2, &mut out);
    write_varint(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    out
}

/// An id or timestamp: both halves written even when zero, as Apple writes them.
fn pair(writer: u64, clock: u64) -> Vec<u8> {
    let mut out = varint_field(1, writer);
    out.extend(varint_field(2, clock));
    out
}

#[cfg(test)]
impl Topo {
    /// Records as another device would have written them for `length` units.
    pub(crate) fn fresh_by(uuid: [u8; 16], length: u64) -> Self {
        let mut t = Self::fresh(0);
        t.writers.push(Writer {
            uuid: uuid.to_vec(),
            clocks: vec![length, 1],
        });
        t.substrings.insert(
            1,
            Substring {
                writer: 1,
                clock: 0,
                length,
                stamp_writer: 1,
                stamp_clock: 0,
                deleted: false,
            },
        );
        t
    }

    /// Every writer, with its clocks.
    pub(crate) fn writer_clocks(&self) -> Vec<(Vec<u8>, Vec<u64>)> {
        self.writers
            .iter()
            .map(|w| (w.uuid.clone(), w.clocks.clone()))
            .collect()
    }

    /// Which writer wrote each live unit, in reading order — what a phone rebuilds from.
    pub(crate) fn live_ids(&self) -> Vec<(u64, u64)> {
        self.substrings
            .iter()
            .filter(|s| !s.deleted)
            .flat_map(|s| (0..s.length).map(move |k| (s.writer, s.clock + k)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(t: &Topo) -> Topo {
        let mut note = message(2, b"text");
        note.extend(t.encode());
        Topo::read(&note).expect("our own records read back")
    }

    /// The exact bytes a phone wrote for a reminder titled "Charge the laptop 🔋", except for
    /// the writer id, which is the phone's own.
    #[test]
    fn a_fresh_document_is_shaped_like_the_phones() {
        set_writer([0x67; 16]);
        let t = Topo::fresh(20);
        let s = &t.substrings;
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].writer, s[0].clock, s[0].length), (0, 0, 0));
        assert_eq!((s[1].writer, s[1].clock, s[1].length), (1, 0, 20));
        assert_eq!((s[1].stamp_writer, s[1].stamp_clock), (1, 0));
        assert_eq!((s[2].writer, s[2].clock, s[2].stamp_clock), (0, END, END));
        assert_eq!(t.writers[0].clocks, vec![20, 1]);
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn typing_in_the_middle_keeps_everyone_elses_characters() {
        let mut t = Topo::fresh(10);
        let before = t.live_ids();
        t.edit(4, 0, 3);
        let after = t.live_ids();
        assert_eq!(t.live_length(), 13);
        assert_eq!(&after[..4], &before[..4]);
        assert_eq!(&after[7..], &before[4..]);
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn deleting_marks_characters_rather_than_dropping_them() {
        let mut t = Topo::fresh(10);
        let all: u64 = t.substrings.iter().map(|s| s.length).sum();
        t.edit(2, 5, 0);
        assert_eq!(t.live_length(), 5);
        // Nothing vanished: the deleted stretch is still there, marked.
        assert_eq!(t.substrings.iter().map(|s| s.length).sum::<u64>(), all);
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn replacing_a_word_is_a_delete_and_an_insert() {
        let mut t = Topo::fresh(11); // "hello world"
        t.edit(6, 5, 5); // "hello there"
        assert_eq!(t.live_length(), 11);
        let ids = t.live_ids();
        // The kept prefix is the original characters; the new word is new ones.
        assert_eq!(ids[..6], Topo::fresh(11).live_ids()[..6]);
        assert!(ids[6..].iter().all(|&(_, c)| c >= 11));
    }

    #[test]
    fn many_edits_keep_the_records_consistent() {
        let mut t = Topo::fresh(0);
        let mut len = 0u64;
        // A deterministic scatter of inserts and deletes all over the text.
        for step in 0..200u64 {
            let at = (step * 7919) % (len + 1);
            if step % 3 == 0 && len > 0 {
                let n = (step % 5 + 1).min(len - at.min(len));
                t.edit(at.min(len), n, 0);
                len -= n;
            } else {
                t.edit(at, 0, step % 4 + 1);
                len += step % 4 + 1;
            }
            assert_eq!(t.live_length(), len, "step {step}");
        }
        let ids = t.live_ids();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "no character id is used twice");
        assert_eq!(roundtrip(&t), t);
    }

    #[test]
    fn another_writers_entry_is_left_as_it_was() {
        let mut t = Topo::fresh(5);
        t.writers.insert(
            0,
            Writer {
                uuid: vec![9; 16],
                clocks: vec![74800, 31],
            },
        );
        // Our own substring now refers to the second writer.
        t.substrings[1].writer = 2;
        t.substrings[1].stamp_writer = 2;
        t.edit(5, 0, 2);
        assert_eq!(t.writers[0].clocks, vec![74800, 31]);
        assert_eq!(t.writers.len(), 2);
        assert_eq!(t.substrings[2].writer, 2);
    }

    #[test]
    fn a_note_edited_on_two_devices_at_once_reads_in_list_order() {
        // As a real note was found: the start leads to two branches, which join at the end.
        let sub = |w: u64, c: u64, len: u64, del: bool, next: &[u64]| {
            let mut s = message(1, &pair(w, c));
            s.extend(varint_field(2, len));
            s.extend(message(3, &pair(w, c)));
            if del {
                s.extend(varint_field(4, 1));
            }
            for &n in next {
                s.extend(varint_field(5, n));
            }
            message(3, &s)
        };
        let mut note = message(2, b"abcX");
        note.extend(sub(0, 0, 0, false, &[1, 3]));
        note.extend(sub(1, 0, 3, false, &[2]));
        note.extend(sub(1, 3, 2, true, &[4]));
        note.extend(sub(2, 0, 1, false, &[4]));
        note.extend(sub(0, END, 0, false, &[]));
        let t = Topo::read(&note).expect("a branched graph in reading order reads");
        assert_eq!(t.live_length(), 4);
        // A link back, or a substring nothing leads to, is still refused.
        let mut back = message(2, b"abc");
        back.extend(sub(0, 0, 0, false, &[1]));
        back.extend(sub(1, 0, 3, false, &[0]));
        back.extend(sub(0, END, 0, false, &[]));
        assert!(Topo::read(&back).is_none());
    }

    #[test]
    fn a_graph_that_is_not_one_chain_is_refused() {
        let t = Topo::fresh(3);
        let mut bytes = message(2, b"abc");
        bytes.extend(t.encode());
        // Point the start marker somewhere other than the next record.
        let mut forked = message(2, b"abc");
        let mut s0 = message(1, &pair(0, 0));
        s0.extend(varint_field(2, 0));
        s0.extend(message(3, &pair(0, 0)));
        s0.extend(varint_field(5, 2));
        forked.extend(message(3, &s0));
        let rest = Topo {
            substrings: t.substrings[1..].to_vec(),
            writers: t.writers.clone(),
        };
        forked.extend(rest.encode());
        assert!(Topo::read(&bytes).is_some());
        assert!(Topo::read(&forked).is_none());
        assert!(Topo::read(&[0xFF]).is_none());
        assert!(Topo::read(&message(2, b"no records")).is_none());
    }
}
