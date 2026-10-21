//! A note's document is whatever its record holds — a shared note's, anyone's. Malformed bytes
//! make a note unreadable, never the app panic: random bytes, and a real document cut and
//! flipped, go through every reader.

use quince_icloud::{ckdocument, mergeable, topotext};

/// A small, seeded generator: the same run every time.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n.max(1) as u64).unwrap_or(0)
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn every_reader(bytes: &[u8]) {
    let _ = ckdocument::text_from_bytes(bytes);
    let _ = ckdocument::set_runs(bytes, &[]);
    let _ = ckdocument::replace_text(bytes, "hello\nworld");
    let _ = ckdocument::repair_runs(bytes);
    let _ = mergeable::ordered_strings(bytes);
    let _ = topotext::debug_graph(bytes);
    let text = b64(bytes);
    let _ = ckdocument::text_from_base64(&text);
    let _ = ckdocument::stored_text_from_base64(&text);
    let _ = ckdocument::has_attachments_in_base64(&text);
    let _ = ckdocument::repair_runs_in_base64(&text);
    let _ = ckdocument::replace_rich_in_base64(&text, "x", &[]);
}

#[test]
fn malformed_note_documents_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let real = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(ckdocument::text_to_base64(
                "A title\nSome body text, with a list:\n• one",
            ))
            .unwrap()
    };
    // The protobuf itself, out of its gzip: mutations then reach the parser, not the inflater.
    let real = {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&real[..])
            .read_to_end(&mut out)
            .unwrap();
        out
    };
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for _ in 0..3000 {
        let len = rng.below(256);
        cases.push((0..len).map(|_| rng.next() as u8).collect());
    }
    for _ in 0..3000 {
        let mut doc = real.clone();
        for _ in 0..=rng.below(6) {
            match rng.below(4) {
                0 if !doc.is_empty() => {
                    let at = rng.below(doc.len());
                    doc[at] = rng.next() as u8;
                }
                1 => doc.truncate(rng.below(doc.len() + 1)),
                2 => {
                    let at = rng.below(doc.len() + 1);
                    // A length or a field number past anything real.
                    doc.splice(
                        at..at,
                        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01],
                    );
                }
                _ => {
                    let at = rng.below(doc.len() + 1);
                    doc.insert(at, rng.next() as u8);
                }
            }
        }
        // And as Apple sends it, gzipped, too.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &doc).unwrap();
        cases.push(gz.finish().unwrap());
        cases.push(doc);
    }
    let mut panics = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        if std::panic::catch_unwind(|| every_reader(case)).is_err() {
            panics.push(i);
        }
    }
    assert!(
        panics.is_empty(),
        "{} of {} panicked, first: {:02x?}",
        panics.len(),
        cases.len(),
        panics.first().map(|&i| &cases[i])
    );
}
