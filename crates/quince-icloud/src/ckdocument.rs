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

/// Ungzip a CloudKit document payload, refusing anything implausibly large.
fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    // Not gzip: some payloads are stored plainly.
    if compressed.len() < 3 || compressed[0] != 0x1f || compressed[1] != 0x8b {
        return Some(compressed.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(compressed)
        .take(MAX_DECOMPRESSED)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
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
