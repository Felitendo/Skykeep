//! Reading the text out of CloudKit "document" fields.
//!
//! Reminders and Notes keep their title and body in fields CloudKit types as
//! `ENCRYPTED_BYTES`. On an account without Advanced Data Protection that name is a
//! misnomer: the payload begins `1f 8b 08` and is simply gzip. Inside is a protobuf whose
//! text sits at field path 2 → 3 → 2.
//!
//! Rather than model Apple's whole document schema — which is large, undocumented, and would
//! break on any change — this walks to that one path. Everything here treats the input as
//! hostile: a malformed payload yields `None`, never a panic and never an allocation sized
//! from an attacker-controlled length.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::io::Read;

/// Where the readable text lives inside the document protobuf.
const TEXT_PATH: &[u64] = &[2, 3, 2];

/// A payload larger than this is not a reminder title or a note body; refuse it rather than
/// decompress something hostile.
const MAX_DECOMPRESSED: u64 = 8 * 1024 * 1024;

/// Wire types we can advance past. Anything else means the bytes are not what we expect.
const WIRE_VARINT: u64 = 0;
const WIRE_64BIT: u64 = 1;
const WIRE_LEN: u64 = 2;
const WIRE_32BIT: u64 = 5;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn done(&self) -> bool {
        self.at >= self.bytes.len()
    }

    /// Protobuf base-128 varint. Bounded at ten bytes so a run of continuation bits cannot
    /// spin forever.
    fn varint(&mut self) -> Option<u64> {
        let mut value: u64 = 0;
        for shift in 0..10 {
            let byte = *self.bytes.get(self.at)?;
            self.at += 1;
            value |= u64::from(byte & 0x7F) << (shift * 7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    /// Skip a field whose contents we do not need.
    fn skip(&mut self, wire: u64) -> Option<()> {
        match wire {
            WIRE_VARINT => self.varint().map(|_| ()),
            WIRE_64BIT => self.take(8).map(|_| ()),
            WIRE_LEN => {
                let n = usize::try_from(self.varint()?).ok()?;
                self.take(n).map(|_| ())
            }
            WIRE_32BIT => self.take(4).map(|_| ()),
            _ => None,
        }
    }
}

/// Follow `path` through nested length-delimited fields and return the bytes at the end.
fn field_at<'a>(bytes: &'a [u8], path: &[u64]) -> Option<&'a [u8]> {
    let Some((&wanted, rest)) = path.split_first() else {
        return Some(bytes);
    };
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == wanted && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            let chunk = reader.take(n)?;
            // The first match that resolves wins; a later sibling with the same tag is a
            // different run of the document, not the text.
            if let Some(found) = field_at(chunk, rest) {
                return Some(found);
            }
        } else {
            reader.skip(wire)?;
        }
    }
    None
}

/// Decompress a CloudKit document payload.
///
/// Apple uses more than one wrapper. Most payloads are gzip, but some are raw zlib and some
/// are not compressed at all — a note body arriving as zlib decoded to nothing for as long as
/// only gzip was handled, which looked like an empty note rather than a decoder gap.
fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    if compressed.len() < 2 {
        return Some(compressed.to_vec());
    }

    let read_all = |mut reader: Box<dyn Read>| -> Option<Vec<u8>> {
        let mut out = Vec::new();
        reader.read_to_end(&mut out).ok()?;
        Some(out)
    };

    // gzip
    if compressed[0] == 0x1f && compressed[1] == 0x8b {
        return read_all(Box::new(
            flate2::read::GzDecoder::new(compressed).take(MAX_DECOMPRESSED),
        ));
    }

    // zlib: low nibble of the first byte is the method (8 = deflate), and the two-byte
    // header is a multiple of 31.
    let header = u16::from(compressed[0]) << 8 | u16::from(compressed[1]);
    if compressed[0] & 0x0f == 8 && header % 31 == 0 {
        if let Some(out) = read_all(Box::new(
            flate2::read::ZlibDecoder::new(compressed).take(MAX_DECOMPRESSED),
        )) {
            return Some(out);
        }
    }

    // Not compressed: titles and folder names arrive as bare bytes.
    Some(compressed.to_vec())
}

/// The readable text of a CloudKit document field, or `None` if there is none to find.
///
/// Takes the base64 string CloudKit puts in the field's `value`.
#[must_use]
pub fn text_from_base64(value: &str) -> Option<String> {
    let raw = B64.decode(value.trim()).ok()?;
    text_from_bytes(&raw)
}

#[must_use]
pub fn text_from_bytes(raw: &[u8]) -> Option<String> {
    let plain = inflate(raw)?;

    // Reminder titles and note bodies are wrapped in Apple's document protobuf.
    if let Some(found) = field_at(&plain, TEXT_PATH) {
        if let Some(text) = usable_text(found) {
            return Some(text);
        }
    }

    // Note and folder titles are not wrapped at all — the payload is the text. Falling back
    // rather than failing is what lets one decoder serve both.
    usable_text(&plain)
}

/// Valid, non-empty UTF-8 with nothing unprintable in it.
///
/// The printability check is what stops a protobuf being mistaken for text: raw wire bytes
/// occasionally decode as UTF-8 but are full of control characters.
fn usable_text(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Write `text` back in the shape Apple reads: protobuf at field path 2 → 3 → 2, gzipped,
/// base64-encoded.
///
/// Writes must speak the dialect reads decode, or a reminder created here would show up
/// blank on the phone.
#[must_use]
pub fn text_to_base64(text: &str) -> String {
    B64.encode(gzip(&document_bytes(text)))
}

/// Encode text the way Apple stores a note's title and snippet: gzipped UTF-8, with no
/// document wrapper around it.
///
/// The decoder already falls back to this shape on the way in; writing the wrapped form
/// here instead would put protobuf framing into the title a phone displays.
#[must_use]
pub fn bare_text_to_base64(text: &str) -> String {
    B64.encode(gzip(text.as_bytes()))
}

/// A length-delimited protobuf field: `(tag << 3) | 2`, then the length, then the payload.
fn length_delimited(tag: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 4);
    write_varint((tag << 3) | WIRE_LEN, &mut out);
    write_varint(payload.len() as u64, &mut out);
    out.extend_from_slice(payload);
    out
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

/// Which field of the note holds an attribute run.
const ATTRIBUTE_RUN: u64 = 5;

/// A protobuf varint field: `(tag << 3) | 0`, then the value.
fn varint_field(tag: u64, value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    write_varint(tag << 3, &mut out);
    write_varint(value, &mut out);
    out
}

/// The length Apple's attribute runs are measured in.
///
/// Not bytes and not characters: UTF-16 code units, which is what an emoji makes differ
/// from both. Measured against real notes, where the runs summed to exactly this.
fn utf16_len(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

/// The document Apple accepts: version, then a document, then a note holding the text and an
/// attribute run covering it.
///
/// Two things had to be right and only one of them was.
///
/// The attribute run is how Apple's clients map styling onto text, and something written with
/// no run has nothing to render against. A bare `{ length }` is a complete run, which is what
/// Apple writes for unstyled text.
///
/// The version fields are the other half, and they were simply missing. Every document the
/// phone writes opens `08 00` — a version at the top — and its inner document opens `08 00
/// 10 00` before the note itself. Ours went straight to the note. Our own reader did not mind
/// and handed the text back perfectly, which is why this survived so long; Apple's reader is
/// stricter, and a reminder or note created here arrived on the phone with nothing in it.
///
/// Checked by laying our bytes beside a reminder the phone wrote: the same fields, in the same
/// order, differing only in the text.
fn document_bytes(text: &str) -> Vec<u8> {
    let mut note = length_delimited(TEXT_PATH[2], text.as_bytes());
    note.extend(length_delimited(
        ATTRIBUTE_RUN,
        &varint_field(1, utf16_len(text)),
    ));

    // The note, inside a document that states its versions, inside a store that states its own.
    let mut document = varint_field(1, 0);
    document.extend(varint_field(2, 0));
    document.extend(length_delimited(TEXT_PATH[1], &note));

    let mut store = varint_field(1, 0);
    store.extend(length_delimited(TEXT_PATH[0], &document));
    store
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to an in-memory buffer cannot fail.
    let _ = encoder.write_all(bytes);
    encoder.finish().unwrap_or_default()
}

/// What a text edit did, in the UTF-16 units attribute runs are measured in.
///
/// Derived from the common prefix and suffix rather than a full diff: an edit is almost
/// always one contiguous change, and this describes exactly that — `removed` units at
/// `from` became `inserted` units.
fn utf16_change(old: &str, new: &str) -> (u64, u64, u64) {
    let old: Vec<u16> = old.encode_utf16().collect();
    let new: Vec<u16> = new.encode_utf16().collect();

    let prefix = old
        .iter()
        .zip(&new)
        .take_while(|(a, b)| a == b)
        .count()
        .min(old.len())
        .min(new.len());

    // The suffix may not run back past the prefix on either side.
    let max_suffix = (old.len() - prefix).min(new.len() - prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);

    (
        prefix as u64,
        (old.len() - prefix - suffix) as u64,
        (new.len() - prefix - suffix) as u64,
    )
}

/// Apply that change to a set of run lengths, the way an attributed string does it.
///
/// Runs entirely before the change keep their length; runs entirely after keep theirs; the
/// ones the change overlaps shrink by what it removed, and whichever run the change starts
/// in grows by what it inserted. A run left covering nothing is dropped.
fn adjust_run_lengths(lengths: &[u64], from: u64, removed: u64, inserted: u64) -> Vec<u64> {
    let removal_end = from + removed;
    let mut adjusted = Vec::with_capacity(lengths.len());
    let mut cursor = 0u64;
    let mut inserted_into = false;

    for &length in lengths {
        let start = cursor;
        let end = cursor + length;
        cursor = end;

        let overlap = removal_end.min(end).saturating_sub(from.max(start));
        let mut next = length - overlap;

        // Whichever run the change begins inside takes the new text. `end > from` rather
        // than `>=` so an insertion at a boundary lands in the run it is extending.
        if !inserted_into && (end > from || (end == from && start <= from)) {
            next += inserted;
            inserted_into = true;
        }
        adjusted.push(next);
    }

    // An edit past the end of every run — appending to a note whose runs were already short
    // — belongs to the last one.
    if !inserted_into && inserted > 0 {
        if let Some(last) = adjusted.last_mut() {
            *last += inserted;
        } else {
            adjusted.push(inserted);
        }
    }

    adjusted.retain(|&n| n > 0);
    adjusted
}

/// Make a set of run lengths add up to the text they describe, disturbing as little as
/// possible.
///
/// Runs that are already right are left alone; the shortfall or excess is taken at the end,
/// where unstyled trailing text sits. Flattening to a single run would also make the
/// arithmetic work, and would throw away every heading and list in the note to do it.
fn reconcile_run_lengths(mut lengths: Vec<u64>, wanted: u64) -> Vec<u64> {
    let total: u64 = lengths.iter().sum();
    if total == wanted {
        return lengths;
    }
    if lengths.is_empty() {
        return if wanted == 0 {
            Vec::new()
        } else {
            vec![wanted]
        };
    }

    if total < wanted {
        // The text runs past what the runs cover; the tail is unstyled.
        if let Some(last) = lengths.last_mut() {
            *last += wanted - total;
        }
        return lengths;
    }

    // The runs claim more than there is. Give back from the end.
    let mut excess = total - wanted;
    while excess > 0 {
        let Some(last) = lengths.last_mut() else {
            break;
        };
        let given = (*last).min(excess);
        *last -= given;
        excess -= given;
        if *last == 0 {
            lengths.pop();
        }
    }
    lengths.retain(|&n| n > 0);
    if lengths.is_empty() && wanted > 0 {
        lengths.push(wanted);
    }
    lengths
}

/// Whether a message carries this field at all.
fn has_field(message: &[u8], tag: u64) -> bool {
    let mut reader = Reader::new(message);
    while !reader.done() {
        let Some(key) = reader.varint() else {
            return false;
        };
        let (field, wire) = (key >> 3, key & 7);
        if field == tag {
            return true;
        }
        if reader.skip(wire).is_none() {
            return false;
        }
    }
    false
}

/// Put the version fields back where they are missing.
///
/// Documents this app wrote before it knew about them went straight to the note: no version at
/// the top, none on the document inside. Apple's reader is stricter than ours and such a
/// record arrives on a phone with nothing in it. Rewriting one is the chance to put them back,
/// so anything written the old way heals the first time it is edited rather than staying
/// broken for ever.
///
/// A version already there is left exactly as it is — this adds, it never overwrites.
fn ensure_versions(plain: &[u8]) -> Option<Vec<u8>> {
    let document = field_at(plain, &TEXT_PATH[..1])?;

    let mut fixed_document = document.to_vec();
    for tag in [2, 1] {
        if !has_field(&fixed_document, tag) {
            let mut with = varint_field(tag, 0);
            with.extend_from_slice(&fixed_document);
            fixed_document = with;
        }
    }

    let mut out = if fixed_document == document {
        plain.to_vec()
    } else {
        rewrite(plain, &TEXT_PATH[..1], &fixed_document)?
    };

    if !has_field(&out, 1) {
        let mut with = varint_field(1, 0);
        with.extend_from_slice(&out);
        out = with;
    }
    Some(out)
}

/// Replace a varint field's value, copying every other field through untouched.
fn set_varint_field(message: &[u8], tag: u64, value: u64) -> Option<Vec<u8>> {
    let mut reader = Reader::new(message);
    let mut out = Vec::with_capacity(message.len() + 8);
    let mut replaced = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == tag && wire == WIRE_VARINT {
            reader.varint()?;
            out.extend_from_slice(&varint_field(tag, value));
            replaced = true;
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(message.get(start..reader.at)?);
        }
    }

    if !replaced {
        // A run with styling but no length is still a run; give it one.
        let mut with_length = varint_field(tag, value);
        with_length.extend_from_slice(&out);
        return Some(with_length);
    }
    Some(out)
}

/// Rebuild a note with new text and runs that still describe it.
///
/// Every field other than the text and the run lengths is copied through byte for byte, so
/// each run keeps whatever styling it carried — the paragraph style, the font, the
/// identifiers Apple uses to merge edits. Only the lengths move.
fn rebuild_note(note: &[u8], old_text: &str, new_text: &str) -> Option<Vec<u8>> {
    let (from, removed, inserted) = utf16_change(old_text, new_text);

    // Collect the existing runs, whole.
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);
        if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            runs.push(reader.take(n)?.to_vec());
        } else {
            reader.skip(wire)?;
        }
    }

    let lengths: Vec<u64> = runs.iter().map(|run| run_length(run)).collect();

    let wanted = utf16_len(new_text);
    // Apply the edit, then make sure the result still adds up. A note whose runs were
    // already wrong before this edit — one of ours, written before any of this was
    // understood — is put right here rather than left broken, and reconciling at the tail
    // keeps every styled run it had. Flattening would also make the arithmetic work, and
    // would throw away every heading in the note to do it.
    let adjusted = reconcile_run_lengths(
        adjust_run_lengths(&lengths, from, removed, inserted),
        wanted,
    );

    let mut rebuilt_runs: Vec<Vec<u8>> = Vec::with_capacity(adjusted.len());
    let mut source = runs.iter();
    for length in &adjusted {
        // Runs are consumed in order, so each keeps its own styling; a dropped one takes
        // its styling with it, which is what dropping it means.
        let template = source.next().cloned().unwrap_or_default();
        rebuilt_runs.push(set_varint_field(&template, 1, *length)?);
    }
    if rebuilt_runs.is_empty() && wanted > 0 {
        rebuilt_runs.push(varint_field(1, wanted));
    }

    // Now the note itself: the text replaced, the runs replaced, everything else copied.
    let mut out = Vec::with_capacity(note.len() + new_text.len());
    let mut reader = Reader::new(note);
    let mut wrote_text = false;
    let mut wrote_runs = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);

        if field == TEXT_PATH[2] && wire == WIRE_LEN && !wrote_text {
            let n = usize::try_from(reader.varint()?).ok()?;
            reader.take(n)?;
            out.extend_from_slice(&length_delimited(field, new_text.as_bytes()));
            wrote_text = true;
        } else if field == ATTRIBUTE_RUN && wire == WIRE_LEN {
            let n = usize::try_from(reader.varint()?).ok()?;
            reader.take(n)?;
            // All of them go back in the place the first one held.
            if !wrote_runs {
                for run in &rebuilt_runs {
                    out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, run));
                }
                wrote_runs = true;
            }
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(note.get(start..reader.at)?);
        }
    }

    if !wrote_text {
        return None;
    }
    // A note that had no runs at all still needs them now.
    if !wrote_runs {
        for run in &rebuilt_runs {
            out.extend_from_slice(&length_delimited(ATTRIBUTE_RUN, run));
        }
    }
    Some(out)
}

/// Rewrite the text inside an existing document, leaving every other field exactly as it
/// was.
///
/// A note body is not just its text. Around it Apple keeps version vectors, folder links
/// and per-device merge state, and the earlier encoder — which builds the smallest document
/// Apple will read — would have replaced all of it with a bare string. Saving an edit that
/// way silently throws away everything the note knew about itself.
///
/// So this edits in place: walk the message, swap the one field that holds the text, copy
/// every other byte through untouched.
///
/// Returns `None` when the document is not one this understands well enough to rewrite,
/// which is the point. A note whose shape cannot be reproduced exactly is left alone rather
/// than flattened, and the interface says so instead of pretending the save worked.
#[must_use]
pub fn replace_text(original: &[u8], text: &str) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let current = field_at(&plain, TEXT_PATH)?;

    // The proof, before anything is written: putting the text back exactly as it came out
    // has to reproduce the original byte for byte. If it does, every other field survived
    // the walk, and swapping the text is safe. If it does not, this document has something
    // in it this code does not model, and it is not ours to rewrite.
    if rewrite(&plain, TEXT_PATH, current)? != plain {
        return None;
    }

    // Swapping the text alone is not enough: the note's attribute runs describe how long
    // the text is, and Apple's clients map styling onto it through them. Left stale they
    // point at units that are no longer there, and the edit does not appear at all on a
    // phone — which is exactly what happened. The note is rebuilt so the runs still
    // describe the text, keeping every other byte of each run intact.
    let old_text = std::str::from_utf8(current).ok()?;
    let note = field_at(&plain, &TEXT_PATH[..2])?;
    let rebuilt_note = rebuild_note(note, old_text, text)?;
    let swapped = rewrite(&plain, &TEXT_PATH[..2], &rebuilt_note)?;
    // An edit is also the chance to put back versions a document written the old way lacks.
    ensure_versions(&swapped)
}

/// Put a document's attribute runs back in step with the text it already holds, without
/// touching a single byte of that text.
///
/// For repairing what this app's own earlier writes damaged: notes left with no runs at
/// all, or with lengths that no longer add up. Going through [`replace_text`] would mean
/// reading the text out and writing it back, and reading it out trims surrounding
/// whitespace — so a repair would quietly edit the note it was meant to be fixing. This
/// hands the existing text bytes straight back instead.
///
/// Returns `None` when the runs are already right, so a caller can tell repair from no-op.
#[must_use]
pub fn repair_runs(original: &[u8]) -> Option<Vec<u8>> {
    let plain = inflate(original)?;
    let text = field_at(&plain, TEXT_PATH)?;
    let text = std::str::from_utf8(text).ok()?;

    let covered: u64 = debug_attribute_lengths(original).iter().sum();
    let runs_right = covered == utf16_len(text);
    let versioned = ensure_versions(&plain)?;
    let versions_right = versioned == plain;
    if runs_right && versions_right {
        return None;
    }

    let note = field_at(&versioned, &TEXT_PATH[..2])?;
    // Same text in and out, so only the run lengths can move.
    let rebuilt = rebuild_note(note, text, text)?;
    rewrite(&versioned, &TEXT_PATH[..2], &rebuilt)
}

/// [`repair_runs`] over the base64 a CloudKit field carries.
#[must_use]
pub fn repair_runs_in_base64(original: &str) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    Some(B64.encode(gzip(&repair_runs(&raw)?)))
}

/// Whether [`replace_text`] would accept this document, without building the result.
///
/// Answers "can this note be edited?" at sync time, so the interface can say so before
/// someone starts typing rather than after they press save.
#[must_use]
pub fn is_rewritable(original: &[u8]) -> bool {
    let Some(plain) = inflate(original) else {
        return false;
    };
    let Some(current) = field_at(&plain, TEXT_PATH) else {
        return false;
    };
    rewrite(&plain, TEXT_PATH, current).is_some_and(|rebuilt| rebuilt == plain)
}

/// [`replace_text`] over the base64 a CloudKit field carries, gzipped on the way back.
#[must_use]
pub fn replace_text_in_base64(original: &str, text: &str) -> Option<String> {
    let raw = B64.decode(original.trim()).ok()?;
    Some(B64.encode(gzip(&replace_text(&raw, text)?)))
}

/// Copy a message through, replacing the length-delimited field at `path` with `payload`.
///
/// Every field that is not on the path is copied as raw bytes, so nothing is re-encoded and
/// nothing can be lost in translation.
fn rewrite(message: &[u8], path: &[u64], payload: &[u8]) -> Option<Vec<u8>> {
    let (&wanted, rest) = path.split_first()?;
    let mut reader = Reader::new(message);
    let mut out = Vec::with_capacity(message.len() + payload.len());
    let mut replaced = false;

    while !reader.done() {
        let start = reader.at;
        let key = reader.varint()?;
        let (field, wire) = (key >> 3, key & 7);

        if field == wanted && wire == WIRE_LEN && !replaced {
            let n = usize::try_from(reader.varint()?).ok()?;
            let chunk = reader.take(n)?;
            let inner = if rest.is_empty() {
                payload.to_vec()
            } else {
                // A sibling with the same tag that does not contain the path is a different
                // part of the document, so copy it and keep looking.
                match rewrite(chunk, rest, payload) {
                    Some(done) => done,
                    None => {
                        out.extend_from_slice(message.get(start..reader.at)?);
                        continue;
                    }
                }
            };
            out.extend_from_slice(&length_delimited(field, &inner));
            replaced = true;
        } else {
            reader.skip(wire)?;
            out.extend_from_slice(message.get(start..reader.at)?);
        }
    }

    replaced.then_some(out)
}

/// Decompress a base64 document payload without interpreting it.
///
/// For measuring what Apple actually sends before writing anything back.
#[must_use]
pub fn debug_inflate(value: &str) -> Option<Vec<u8>> {
    inflate(&B64.decode(value.trim()).ok()?)
}

/// The length an attribute run declares, or 0 if it declares none.
///
/// Scans for the field rather than assuming it comes first: protobuf does not promise an
/// order, and a run with styling written ahead of its length is still a run. Reading only
/// the first field silently dropped such runs, which made healthy notes measure short.
fn run_length(run: &[u8]) -> u64 {
    let mut reader = Reader::new(run);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 1 && wire == WIRE_VARINT {
            return reader.varint().unwrap_or(0);
        }
        if reader.skip(wire).is_none() {
            break;
        }
    }
    // An omitted length is protobuf's default, which is zero — not "skip this run".
    0
}

/// The `length` of every attribute run inside a note document.
///
/// Apple's note is `NoteStoreProto{2: Document{3: Note}}`, and the Note carries its text at
/// field 2 and a repeated AttributeRun at field 5. Each run's own field 1 is a length. If
/// those lengths do not add up to the text, Apple's own clients have runs that no longer
/// describe what is there — which is what a note arriving blank on a phone looks like.
///
/// Returns the lengths in order, for comparing against the text they claim to cover.
#[must_use]
pub fn debug_attribute_lengths(bytes: &[u8]) -> Vec<u64> {
    let Some(plain) = inflate(bytes) else {
        return Vec::new();
    };
    // Down to the Note itself: NoteStoreProto -> Document -> Note is 2 -> 3.
    let Some(note) = field_at(&plain, &[2, 3]) else {
        return Vec::new();
    };

    let mut lengths = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 5 && wire == WIRE_LEN {
            let Some(n) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                break;
            };
            let Some(run) = reader.take(n) else { break };
            lengths.push(run_length(run));
        } else if reader.skip(wire).is_none() {
            break;
        }
    }
    lengths
}

/// The raw bytes of every attribute run in a note document, for seeing exactly what Apple
/// puts in one before writing one ourselves.
#[must_use]
pub fn debug_attribute_runs(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some(plain) = inflate(bytes) else {
        return Vec::new();
    };
    let Some(note) = field_at(&plain, &[2, 3]) else {
        return Vec::new();
    };

    let mut runs = Vec::new();
    let mut reader = Reader::new(note);
    while !reader.done() {
        let Some(key) = reader.varint() else { break };
        let (field, wire) = (key >> 3, key & 7);
        if field == 5 && wire == WIRE_LEN {
            let Some(n) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                break;
            };
            let Some(run) = reader.take(n) else { break };
            runs.push(run.to_vec());
        } else if reader.skip(wire).is_none() {
            break;
        }
    }
    runs
}

/// A readable outline of a protobuf message: one line per field, nested.
///
/// Rewriting a note body means preserving every field that is not the text, so the shape
/// has to be known rather than assumed. This is how it gets looked at.
#[must_use]
pub fn debug_tree(bytes: &[u8], depth_limit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    walk(bytes, 1, depth_limit, &mut lines);
    lines
}

fn walk(bytes: &[u8], depth: usize, limit: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth + 1);
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let Some(key) = reader.varint() else { return };
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            WIRE_VARINT => {
                let Some(value) = reader.varint() else { return };
                out.push(format!("{pad}{field}: varint {value}"));
            }
            WIRE_LEN => {
                let Some(len) = reader.varint().and_then(|n| usize::try_from(n).ok()) else {
                    return;
                };
                let Some(chunk) = reader.take(len) else {
                    return;
                };
                match std::str::from_utf8(chunk) {
                    // Printable UTF-8 of any length is text, and text is what a writer has
                    // to be able to find.
                    Ok(text) if usable_text(chunk).is_some() => out.push(format!(
                        "{pad}{field}: text {len}B {:?}",
                        text.chars().take(48).collect::<String>()
                    )),
                    _ if depth < limit && looks_like_message(chunk) => {
                        out.push(format!("{pad}{field}: message {len}B"));
                        walk(chunk, depth + 1, limit, out);
                    }
                    // Short opaque fields are shown as hex: at this size the bytes are the
                    // evidence, and a length alone says nothing.
                    _ if len <= 16 => out.push(format!(
                        "{pad}{field}: bytes {len}B {}",
                        chunk
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                    _ => out.push(format!("{pad}{field}: bytes {len}B")),
                }
            }
            WIRE_64BIT => {
                if reader.take(8).is_none() {
                    return;
                }
                out.push(format!("{pad}{field}: fixed64"));
            }
            WIRE_32BIT => {
                if reader.take(4).is_none() {
                    return;
                }
                out.push(format!("{pad}{field}: fixed32"));
            }
            _ => return,
        }
    }
}

/// Whether a chunk parses cleanly as a protobuf message all the way to its end.
///
/// Length-delimited bytes are ambiguous on the wire: a nested message and an opaque blob
/// look identical until you try to read one. Parsing to the exact end is the test.
fn looks_like_message(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let mut reader = Reader::new(bytes);
    while !reader.done() {
        let Some(key) = reader.varint() else {
            return false;
        };
        if key >> 3 == 0 || reader.skip(key & 7).is_none() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a length-delimited protobuf field.
    fn field(tag: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut key = (tag << 3) | WIRE_LEN;
        loop {
            let mut byte = (key & 0x7F) as u8;
            key >>= 7;
            if key != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if key == 0 {
                break;
            }
        }
        let mut len = payload.len() as u64;
        loop {
            let mut byte = (len & 0x7F) as u8;
            len >>= 7;
            if len != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(payload);
        out
    }

    /// A document shaped the way Apple's is, carrying `text`.
    fn document(text: &str) -> Vec<u8> {
        field(2, &field(3, &field(2, text.as_bytes())))
    }

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn reads_text_from_a_gzipped_document() {
        let payload = gzipped(&document("Move more"));
        assert_eq!(
            text_from_bytes(&payload).as_deref(),
            Some("Move more")
        );
    }

    /// Real titles carry emoji and accents, so the decode must be UTF-8, not ASCII.
    #[test]
    fn keeps_emoji_and_accents_intact() {
        for text in [
            "🏃 Move more",
            "Café — déjà vu",
            "日本語のリマインダー",
        ] {
            let payload = gzipped(&document(text));
            assert_eq!(text_from_bytes(&payload).as_deref(), Some(text));
        }
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Apple compresses some payloads with zlib rather than gzip. Handling only gzip made
    /// those notes render as empty, which reads as data loss rather than a decoder gap.
    #[test]
    fn reads_a_zlib_document_as_well_as_gzip() {
        let text = "🔴 UNIT 2: STATIC & DYNAMIC SOFTWARE TESTING";
        assert_eq!(
            text_from_bytes(&zlib(&document(text))).as_deref(),
            Some(text)
        );
        assert_eq!(
            text_from_bytes(&gzipped(&document(text))).as_deref(),
            Some(text)
        );
    }

    #[test]
    fn reads_zlib_wrapped_bare_text() {
        assert_eq!(
            text_from_bytes(&zlib(b"Recently Deleted")).as_deref(),
            Some("Recently Deleted")
        );
    }

    /// Bytes that merely start with 0x78 must not be mistaken for zlib.
    #[test]
    fn a_leading_0x78_that_is_not_zlib_still_reads_as_text() {
        // 'x' is 0x78, and "xy" fails the header checksum, so it must fall through to text.
        assert_eq!(text_from_bytes(b"xy note").as_deref(), Some("xy note"));
    }

    #[test]
    fn reads_an_uncompressed_document_too() {
        assert_eq!(
            text_from_bytes(&document("plain")).as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn accepts_the_base64_cloudkit_actually_sends() {
        let encoded = B64.encode(gzipped(&document("Milk")));
        assert_eq!(text_from_base64(&encoded).as_deref(), Some("Milk"));
    }

    /// Anything malformed must yield None. These bytes come off the network, so a panic here
    /// would be a crash triggered by a remote value.
    #[test]
    fn malformed_input_is_none_not_a_panic() {
        let cases: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xff; 32],
            vec![0x1f, 0x8b, 0x08],       // gzip header, no body
            vec![0x12, 0xff, 0xff, 0xff], // length far beyond the buffer
            vec![0x80; 64],               // varint continuation forever
            field(2, &[0x12, 0x7f]),      // nested length overruns
            document("").to_vec(),        // present but empty
            gzipped(b"not a protobuf at all"),
        ];
        for bytes in cases {
            let _ = text_from_bytes(&bytes);
        }
        assert!(text_from_bytes(&[0x80; 64]).is_none());
        assert!(
            text_from_bytes(&document("")).is_none(),
            "empty text is not text"
        );
    }

    /// The decisive property: anything written must read back identically, or a reminder
    /// created here shows up blank on the phone.
    #[test]
    fn text_survives_a_write_then_read() {
        for text in [
            "Milk",
            "🏃 Move more",
            "Café — déjà vu",
            "日本語のリマインダー",
            "a line\nand another",
            "  leading and trailing  ",
            &"long ".repeat(400),
        ] {
            let encoded = text_to_base64(text);
            assert_eq!(
                text_from_base64(&encoded).as_deref(),
                Some(text.trim()),
                "round trip failed for {text:?}"
            );
        }
    }

    /// What we produce must be the same shape we parse out of Apple's own payloads.
    #[test]
    fn an_encoded_document_is_gzip_at_the_expected_path() {
        let raw = B64.decode(text_to_base64("Milk")).unwrap();
        assert_eq!(&raw[..3], &[0x1f, 0x8b, 0x08], "Apple's payloads are gzip");
        let plain = inflate(&raw).unwrap();
        assert_eq!(field_at(&plain, TEXT_PATH), Some(&b"Milk"[..]));
    }

    /// Note and folder titles arrive as bare UTF-8 rather than wrapped in the document
    /// protobuf, so one decoder has to serve both shapes.
    #[test]
    fn reads_bare_text_that_was_never_wrapped() {
        assert_eq!(
            text_from_bytes(b"Recently Deleted").as_deref(),
            Some("Recently Deleted")
        );
        assert_eq!(text_from_bytes(b"Notes").as_deref(), Some("Notes"));
        assert_eq!(
            text_from_bytes(&gzipped(b"Shopping")).as_deref(),
            Some("Shopping")
        );
    }

    /// The wrapped form must still win, or a document would be returned as its raw wire bytes.
    #[test]
    fn the_wrapped_form_takes_precedence() {
        let payload = gzipped(&document("Real title"));
        assert_eq!(text_from_bytes(&payload).as_deref(), Some("Real title"));
    }

    /// Binary that happens to be valid UTF-8 must not be mistaken for text.
    #[test]
    fn control_characters_mean_it_is_not_text() {
        assert!(text_from_bytes(&[0x01, 0x02, 0x03]).is_none());
        assert!(text_from_bytes(&[0x00; 16]).is_none());
    }

    #[test]
    fn a_document_without_the_text_field_is_none() {
        // Right nesting, wrong final tag — and binary enough that the plain-text fallback
        // does not rescue it either.
        let wrong = field(2, &field(3, &field(9, &[0x01, 0x02, 0x03])));
        assert!(text_from_bytes(&wrong).is_none());
    }
}

#[cfg(test)]
mod rewrite_tests {
    use super::*;

    fn varint(tag: u64, value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        write_varint(tag << 3, &mut out);
        write_varint(value, &mut out);
        out
    }

    /// The shape a real note body has: the text at 2 -> 3 -> 2, with version-vector
    /// messages and stray scalars around it that a save must not disturb.
    fn document(text: &str) -> Vec<u8> {
        let mut note = Vec::new();
        note.extend(length_delimited(2, text.as_bytes()));
        // Two of Apple's merge records, which carry no length describing the text.
        for index in 1..=2u64 {
            let mut run = Vec::new();
            run.extend(length_delimited(
                1,
                &[varint(1, 1), varint(2, 144_927)].concat(),
            ));
            run.extend(varint(2, 34_947));
            run.extend(varint(5, index));
            note.extend(length_delimited(3, &run));
        }
        note.extend(length_delimited(4, &length_delimited(1, &[0xAB; 28])));
        // The attribute run that makes the text renderable, carrying styling alongside its
        // length exactly as Apple's own runs do.
        note.extend(length_delimited(
            5,
            &[
                varint(1, utf16_len(text)),
                length_delimited(2, &[0x4a, 0x02, 0xAA, 0xBB]),
            ]
            .concat(),
        ));

        let mut inner = Vec::new();
        inner.extend(varint(1, 0));
        inner.extend(varint(2, 0));
        inner.extend(length_delimited(3, &note));

        let mut doc = varint(1, 0);
        doc.extend(length_delimited(2, &inner));
        doc
    }

    #[test]
    fn the_text_is_swapped_and_nothing_else_moves() {
        let before = document("the original body");
        let after = replace_text(&before, "a longer body than the one it replaced").unwrap();

        assert_eq!(
            text_from_bytes(&after).as_deref(),
            Some("a longer body than the one it replaced")
        );
        // Everything that is not the text is still there, byte for byte.
        assert_eq!(
            replace_text(&after, "the original body").unwrap(),
            before,
            "putting the old text back must give the original document"
        );
    }

    /// The bug both of these fix: Apple's clients map styling onto the text through
    /// attribute runs, and a run whose length no longer matches leaves the note unrenderable
    /// — a note edited here simply never changed on the phone.
    #[test]
    fn the_runs_always_add_up_to_the_text_they_describe() {
        for (from, to) in [
            ("short", "a considerably longer body"),
            ("a considerably longer body", "short"),
            ("same size", "SAME SIZE"),
            ("plain", "with an emoji 🔴 in it"),
            ("with an emoji 🔴 in it", "plain"),
            ("anything", ""),
        ] {
            let after = replace_text(&document(from), to).unwrap();
            let lengths = debug_attribute_lengths(&after);
            let text = text_from_bytes(&after).unwrap_or_default();
            assert_eq!(
                lengths.iter().sum::<u64>(),
                text.encode_utf16().count() as u64,
                "{from:?} -> {to:?}: runs {lengths:?} must cover the text exactly"
            );
        }
    }

    /// A document written from scratch needs a run too — without one, everything created
    /// from here arrived blank.
    #[test]
    fn a_created_document_carries_a_run_covering_its_text() {
        for text in ["hello", "🔴 emoji counts as two", ""] {
            let encoded = text_to_base64(text);
            let raw = B64.decode(&encoded).unwrap();
            let lengths = debug_attribute_lengths(&raw);
            assert_eq!(
                lengths.iter().sum::<u64>(),
                text.encode_utf16().count() as u64,
                "{text:?} must be covered by its runs"
            );
            assert_eq!(text_from_base64(&encoded).as_deref().unwrap_or(""), text);
        }
    }

    /// An edit in the middle must leave the runs on either side of it alone — that is what
    /// keeps a heading still a heading after you change a word underneath it.
    #[test]
    fn an_edit_only_disturbs_the_runs_it_touches() {
        // Three runs of ten units each.
        let lengths = [10, 10, 10];
        // Replace two units in the middle run with five.
        let adjusted = adjust_run_lengths(&lengths, 14, 2, 5);
        assert_eq!(adjusted, vec![10, 13, 10], "only the middle run moves");
        assert_eq!(adjusted.iter().sum::<u64>(), 33);
    }

    #[test]
    fn a_run_left_covering_nothing_is_dropped() {
        // Remove the whole middle run.
        assert_eq!(adjust_run_lengths(&[10, 10, 10], 10, 10, 0), vec![10, 10]);
    }

    /// Repairing a note whose runs were already wrong must not cost it its formatting.
    #[test]
    fn reconciling_takes_the_difference_at_the_end() {
        // Short by 169, as one real note on the account was: 16612 covered, 16781 there.
        assert_eq!(
            reconcile_run_lengths(vec![10, 20, 16_582], 16_781),
            vec![10, 20, 16_751],
            "the shortfall is added to the trailing run, not spread over the styled ones"
        );
        // Over by 29, as another was: 167 claimed over 138 of text.
        assert_eq!(
            reconcile_run_lengths(vec![100, 37, 30], 138),
            vec![100, 37, 1],
            "the excess is given back from the end"
        );
        // Over by enough to swallow whole trailing runs.
        assert_eq!(reconcile_run_lengths(vec![10, 20, 30], 10), vec![10]);
        // Already right: untouched.
        assert_eq!(reconcile_run_lengths(vec![10, 20], 30), vec![10, 20]);
        // Nothing at all to work with.
        assert_eq!(reconcile_run_lengths(Vec::new(), 42), vec![42]);
        assert_eq!(reconcile_run_lengths(Vec::new(), 0), Vec::<u64>::new());
    }

    /// Repair must not touch the text, including whitespace a reader would trim off. This
    /// is the guarantee that makes it safe to run over someone's real notes.
    #[test]
    fn repairing_leaves_the_text_byte_for_byte() {
        // Text with whitespace at both ends, which reading it back would have trimmed.
        let text = "\n  a note with padding around it  \n";
        let mut note = length_delimited(2, text.as_bytes());
        note.extend(length_delimited(5, &varint(1, 9_999))); // a nonsense length
        let mut inner = varint(1, 0);
        inner.extend(length_delimited(3, &note));
        let mut broken = varint(1, 0);
        broken.extend(length_delimited(2, &inner));

        let repaired = repair_runs(&broken).expect("it needed repair");
        assert_eq!(
            field_at(&repaired, TEXT_PATH),
            Some(text.as_bytes()),
            "the text must come through untouched, padding and all"
        );
        assert_eq!(
            debug_attribute_lengths(&repaired).iter().sum::<u64>(),
            text.encode_utf16().count() as u64
        );
    }

    /// A note that is already correct is left completely alone, so a repair pass can tell
    /// what it actually changed.
    #[test]
    fn repairing_a_healthy_note_is_a_no_op() {
        assert!(
            repair_runs(&document("already fine")).is_none(),
            "nothing to do, and it says so"
        );
    }

    /// A note that arrived with no runs — one of ours, written before any of this was
    /// understood — is repaired simply by saving it again.
    #[test]
    fn a_note_with_no_runs_is_repaired_by_an_edit() {
        // A document with text but no attribute run at all, as our own writes produced.
        let mut note = length_delimited(2, b"some text that was never covered");
        note.extend(length_delimited(4, &length_delimited(1, &[0xAB; 4])));
        let mut inner = varint(1, 0);
        inner.extend(length_delimited(3, &note));
        let mut broken = varint(1, 0);
        broken.extend(length_delimited(2, &inner));

        assert_eq!(debug_attribute_lengths(&broken), Vec::<u64>::new());

        let text = text_from_bytes(&broken).unwrap();
        let repaired = replace_text(&broken, &text).unwrap();
        assert_eq!(
            debug_attribute_lengths(&repaired).iter().sum::<u64>(),
            text.encode_utf16().count() as u64,
            "saving it unchanged is enough to put it right"
        );
    }

    #[test]
    fn appending_extends_the_last_run() {
        assert_eq!(adjust_run_lengths(&[10, 10], 20, 0, 5), vec![10, 15]);
    }

    /// The change is described in UTF-16 units, because that is what the runs count in.
    #[test]
    fn a_change_is_measured_the_way_the_runs_measure() {
        // An emoji is one char but two UTF-16 units.
        assert_eq!(utf16_change("ab", "a🔴b"), (1, 0, 2));
        assert_eq!(utf16_change("a🔴b", "ab"), (1, 2, 0));
        assert_eq!(utf16_change("same", "same"), (4, 0, 0));
        assert_eq!(utf16_change("", "new"), (0, 0, 3));
    }

    /// The whole point of rewriting in place rather than re-encoding. An edit of the same
    /// length must leave the document byte-identical apart from the text itself, which is
    /// the strongest statement available that nothing else was touched.
    #[test]
    fn a_same_length_edit_changes_only_the_text() {
        let before = document("first");
        let after = replace_text(&before, "FIRST").unwrap();
        assert_eq!(before.len(), after.len());

        let differing: Vec<usize> = before
            .iter()
            .zip(&after)
            .enumerate()
            .filter_map(|(i, (a, b))| (a != b).then_some(i))
            .collect();
        assert_eq!(differing.len(), 5, "only the five text bytes moved");
        // And the run still covers it, unchanged, because the length did not change.
        assert_eq!(debug_attribute_lengths(&after), vec![5]);
    }
    /// And when the length does change, the merge records Apple uses to reconcile edits
    /// across devices must come through byte for byte. Those are what a phone consults to
    /// decide whether our version is newer than its own.
    #[test]
    fn the_version_vectors_survive_a_save() {
        let before = document("first");
        let after = replace_text(&before, "second").unwrap();

        // Field 3 of the note is Apple's merge state. Pull it out of both and compare.
        let merge_records = |bytes: &[u8]| -> Vec<Vec<u8>> {
            let plain = inflate(bytes).unwrap();
            let note = field_at(&plain, &TEXT_PATH[..2]).unwrap();
            let mut found = Vec::new();
            let mut reader = Reader::new(note);
            while !reader.done() {
                let key = reader.varint().unwrap();
                let (field, wire) = (key >> 3, key & 7);
                if field == 3 && wire == WIRE_LEN {
                    let n = usize::try_from(reader.varint().unwrap()).unwrap();
                    found.push(reader.take(n).unwrap().to_vec());
                } else {
                    reader.skip(wire).unwrap();
                }
            }
            found
        };

        assert_eq!(
            merge_records(&after),
            merge_records(&before),
            "the merge state must survive an edit untouched"
        );
        assert!(!merge_records(&before).is_empty(), "the fixture has some");
    }

    #[test]
    fn an_empty_body_is_a_legitimate_edit() {
        let after = replace_text(&document("something"), "").unwrap();
        assert_eq!(field_at(&after, TEXT_PATH), Some(&[][..]));
    }

    /// Growing and shrinking both rewrite the enclosing lengths, which is where a naive
    /// splice corrupts the message.
    #[test]
    fn lengths_are_rewritten_in_both_directions() {
        for (from, to) in [
            ("short", &"x".repeat(5_000)[..]),
            (&"y".repeat(5_000), "tiny"),
        ] {
            let after = replace_text(&document(from), to).unwrap();
            assert_eq!(text_from_bytes(&after).as_deref(), Some(to));
            assert!(looks_like_message(&after), "still a valid message");
        }
    }

    /// A document without the path is not ours to rewrite, and saying so is the safe answer.
    #[test]
    fn a_document_we_do_not_recognise_is_refused() {
        assert!(replace_text(&varint(1, 0), "anything").is_none());
        assert!(replace_text(&[0xFF, 0xFF, 0xFF], "anything").is_none());
        assert!(replace_text(&[], "anything").is_none());
    }

    /// Real payloads arrive compressed, and a save has to survive the round trip.
    #[test]
    fn a_gzipped_document_round_trips() {
        let encoded = B64.encode(gzip(&document("before")));
        let saved = replace_text_in_base64(&encoded, "after").unwrap();
        assert_eq!(text_from_base64(&saved).as_deref(), Some("after"));
    }

    /// Unicode is measured in bytes on the wire, not characters.
    #[test]
    fn emoji_and_accents_survive() {
        let text = "🔴 UNIT 2 — Prüfung · 测试\nsecond line";
        let after = replace_text(&document("x"), text).unwrap();
        assert_eq!(text_from_bytes(&after).as_deref(), Some(text));
    }
}

/// Documents written before this app knew Apple's reader wanted a version on them.
#[cfg(test)]
mod version_tests {
    use super::*;

    /// Exactly the shape this app used to write: straight to the note, no version anywhere.
    /// Our own reader never minded, which is why it survived — the reader that mattered was
    /// Apple's, and such a record arrived on a phone with nothing in it.
    fn written_the_old_way(text: &str) -> Vec<u8> {
        let mut note = length_delimited(TEXT_PATH[2], text.as_bytes());
        note.extend(length_delimited(
            ATTRIBUTE_RUN,
            &varint_field(1, utf16_len(text)),
        ));
        let document = length_delimited(TEXT_PATH[1], &note);
        length_delimited(TEXT_PATH[0], &document)
    }

    fn shape(plain: &[u8]) -> (Vec<u64>, Vec<u64>) {
        let tags = |m: &[u8]| {
            let mut reader = Reader::new(m);
            let mut out = Vec::new();
            while !reader.done() {
                let Some(key) = reader.varint() else { break };
                out.push(key >> 3);
                if reader.skip(key & 7).is_none() {
                    break;
                }
            }
            out
        };
        let document = field_at(plain, &TEXT_PATH[..1]).unwrap_or_default();
        (tags(plain), tags(document))
    }

    /// What this app writes today is what the phone writes: a version at the top, and two on
    /// the document inside, before the note.
    #[test]
    fn a_new_document_carries_the_versions_apple_writes() {
        let (top, document) = shape(&document_bytes("Buy milk"));
        assert_eq!(top, vec![1, 2]);
        assert_eq!(document, vec![1, 2, 3]);
    }

    /// And one written the old way is put right the first time it is edited, rather than
    /// staying broken for as long as it exists.
    #[test]
    fn editing_a_document_written_the_old_way_puts_the_versions_back() {
        let old = written_the_old_way("Buy milk");
        assert_eq!(
            shape(&old),
            (vec![2], vec![3]),
            "the fixture is the old shape"
        );

        let edited = replace_text(&gzip(&old), "Buy oat milk").expect("still rewritable");
        assert_eq!(shape(&edited), (vec![1, 2], vec![1, 2, 3]));
        assert_eq!(
            text_from_bytes(&edited).as_deref(),
            Some("Buy oat milk"),
            "and the edit itself landed"
        );
    }

    /// Repair notices a missing version even where the runs are already right, because the
    /// text renders from the run and the record is read from the version.
    #[test]
    fn repair_puts_the_versions_back_on_its_own() {
        let old = written_the_old_way("Buy milk");
        let repaired = repair_runs(&gzip(&old)).expect("a missing version is worth repairing");
        assert_eq!(shape(&repaired), (vec![1, 2], vec![1, 2, 3]));
        assert_eq!(text_from_bytes(&repaired).as_deref(), Some("Buy milk"));
    }

    /// A document that is already right is left alone — repair must be able to say "nothing
    /// to do" or every sync rewrites every note.
    #[test]
    fn a_sound_document_is_not_rewritten() {
        assert_eq!(repair_runs(&gzip(&document_bytes("Buy milk"))), None);
    }

    /// A version Apple set to something other than zero is not ours to overwrite.
    #[test]
    fn an_existing_version_is_left_exactly_as_it_was() {
        let mut note = length_delimited(TEXT_PATH[2], b"Buy milk");
        note.extend(length_delimited(ATTRIBUTE_RUN, &varint_field(1, 8)));
        let mut document = varint_field(1, 7);
        document.extend(varint_field(2, 3));
        document.extend(length_delimited(TEXT_PATH[1], &note));
        let mut store = varint_field(1, 9);
        store.extend(length_delimited(TEXT_PATH[0], &document));

        let out = ensure_versions(&store).unwrap();
        assert_eq!(out, store, "nothing was missing, so nothing changed");
    }
}
