//! Decompressing someone else's bytes, with a ceiling. A shared note's table, a feed or a
//! document arrives compressed, and ten megabytes of zeros inflate to gigabytes: read whole,
//! that was enough to take the computer's memory (and on Linux, the whole desktop with it).
//! Every inflate of remote data reads through here, and anything past its ceiling is refused —
//! never cut short and read as if whole.

use std::io::Read;

/// The most any one remote payload inflates to: far beyond a real note, table or calendar.
pub const MOST_INFLATED: u64 = 64 * 1024 * 1024;

/// All of `reader`, or `None` when it fails or holds more than `most` bytes.
pub fn read_capped(reader: impl Read, most: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .take(most.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 <= most).then_some(out)
}

/// A gzip stream inflated, within `most`.
pub fn gunzip(data: &[u8], most: u64) -> Option<Vec<u8>> {
    read_capped(flate2::read::GzDecoder::new(data), most)
}

/// A zlib stream inflated, within `most`.
pub fn unzlib(data: &[u8], most: u64) -> Option<Vec<u8>> {
    read_capped(flate2::read::ZlibDecoder::new(data), most)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn a_bomb_is_refused_and_a_real_payload_reads_whole() {
        let bomb = gz(&vec![0u8; 4 * 1024 * 1024]);
        assert!(bomb.len() < 64 * 1024, "{}", bomb.len());
        assert_eq!(gunzip(&bomb, 1024 * 1024), None);
        assert_eq!(
            gunzip(&bomb, 4 * 1024 * 1024).map(|v| v.len()),
            Some(4 * 1024 * 1024)
        );
        assert_eq!(gunzip(&gz(b"hello"), 5).as_deref(), Some(&b"hello"[..]));
        assert_eq!(gunzip(b"not gzip", 100), None);
    }
}
