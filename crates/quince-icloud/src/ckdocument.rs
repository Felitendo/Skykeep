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

/// The minimal document Apple accepts: the text nested at [`TEXT_PATH`].
fn document_bytes(text: &str) -> Vec<u8> {
    let mut nested = length_delimited(TEXT_PATH[2], text.as_bytes());
    for tag in TEXT_PATH[..2].iter().rev() {
        nested = length_delimited(*tag, &nested);
    }
    nested
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to an in-memory buffer cannot fail.
    let _ = encoder.write_all(bytes);
    encoder.finish().unwrap_or_default()
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

    rewrite(&plain, TEXT_PATH, text.as_bytes())
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
    }

    /// And when the length does change, the only fields that change are the lengths on the
    /// path to the text. The version vectors keep their exact values.
    #[test]
    fn the_version_vectors_survive_a_save() {
        let before = document("first");
        let after = replace_text(&before, "second").unwrap();
        // Sizes shift on the path by construction; the fields and their values must not.
        let shape = |bytes: &[u8]| {
            debug_tree(bytes, 6)
                .into_iter()
                .filter(|line| !line.contains("text "))
                .map(|line| {
                    line.split_whitespace()
                        .filter(|token| !token.ends_with('B'))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(&after),
            shape(&before),
            "every field but the text must survive unchanged"
        );
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
