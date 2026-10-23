//! The session's cookies, sealed on disk with a key the system keyring keeps.
//!
//! The cookies are as good as the session for as long as they last: anything that could read
//! `cookies.json` could be signed in to the person's iCloud with it. The file was readable by
//! this user alone, which kept other users out and nothing else — a backup, a synced folder, a
//! copy of the data folder carried them in the clear. Now the file holds them sealed with
//! AES-256-GCM, under a key made for this data folder and kept in the keyring beside the
//! session, so the file on its own opens nothing.
//!
//! The file is a header and the sealed jar:
//!
//! ```text
//! "SKCK"  version (1)  nonce (12 bytes, fresh every write)  ciphertext and tag
//! ```
//!
//! The header is bound in as associated data, so a changed version byte fails to open just as
//! a changed byte of the cookies does.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::STANDARD as B64, Engine};

/// What every sealed file starts with. A jar from before sealing is JSON, which never does.
pub(crate) const MAGIC: &[u8; 4] = b"SKCK";
/// The one layout there is. A file of any other version is not guessed at.
pub(crate) const VERSION: u8 = 1;
const HEADER: usize = MAGIC.len() + 1;
const NONCE: usize = 12;
/// The smallest a sealed file can be: header, nonce and a tag over nothing.
const SMALLEST: usize = HEADER + NONCE + 16;

/// 256 random bits, one per data folder.
pub(crate) type Key = [u8; 32];

/// Why a sealed file did not open. Either way the cookies in it are lost to us: set aside, and
/// the session carries on from the tokens the keyring holds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unopened {
    /// Written by a layout this build does not know.
    Version(u8),
    /// Cut short, changed, or sealed with another key.
    Damaged,
}

/// Whether `bytes` are a sealed jar, rather than one from before sealing.
pub(crate) fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// A new key, from the operating system's own randomness.
pub(crate) fn new_key() -> std::io::Result<Key> {
    let mut key = [0u8; 32];
    fill(&mut key)?;
    Ok(key)
}

fn fill(bytes: &mut [u8]) -> std::io::Result<()> {
    use rand::TryRngCore;
    rand::rngs::OsRng
        .try_fill_bytes(bytes)
        .map_err(std::io::Error::other)
}

/// The key as the keyring keeps it: text, 44 characters, well inside any platform's limit.
pub(crate) fn encode_key(key: &Key) -> String {
    B64.encode(key)
}

/// A key read back from the keyring; anything that is not exactly one is no key.
pub(crate) fn decode_key(text: &str) -> Option<Key> {
    B64.decode(text.trim()).ok()?.try_into().ok()
}

/// `plain` sealed under `key`, header first, with a nonce of its own.
pub(crate) fn seal(key: &Key, plain: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE];
    fill(&mut nonce)?;
    let mut out = Vec::with_capacity(SMALLEST + plain.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    let sealed = Aes256Gcm::new(key.into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &out[..HEADER],
            },
        )
        .map_err(|_| std::io::Error::other("couldn't seal the cookies"))?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// What [`seal`] wrote, opened — only if every byte of it is as it was written.
pub(crate) fn open(key: &Key, bytes: &[u8]) -> Result<Vec<u8>, Unopened> {
    if !is_sealed(bytes) || bytes.len() < HEADER {
        return Err(Unopened::Damaged);
    }
    if bytes[MAGIC.len()] != VERSION {
        return Err(Unopened::Version(bytes[MAGIC.len()]));
    }
    if bytes.len() < SMALLEST {
        return Err(Unopened::Damaged);
    }
    let (header, rest) = bytes.split_at(HEADER);
    let (nonce, sealed) = rest.split_at(NONCE);
    Aes256Gcm::new(key.into())
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: header,
            },
        )
        .map_err(|_| Unopened::Damaged)
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAR: &[u8] = br#"[{"raw_cookie":"X-APPLE-WEBAUTH-USER=secret-value"}]"#;

    #[test]
    fn what_is_sealed_opens_with_its_key_and_says_nothing_in_the_clear() {
        let key = new_key().unwrap();
        let sealed = seal(&key, JAR).unwrap();
        assert!(is_sealed(&sealed));
        assert_eq!(sealed[4], VERSION);
        assert!(
            !sealed.windows(12).any(|w| w == b"secret-value"),
            "the cookie is not in the file"
        );
        assert_eq!(open(&key, &sealed).unwrap(), JAR);
        // A fresh nonce every write: the same jar never seals to the same bytes.
        assert_ne!(seal(&key, JAR).unwrap(), sealed);
    }

    /// Any byte changed — header, nonce, cookies or tag — and nothing opens.
    #[test]
    fn a_changed_byte_anywhere_is_caught() {
        let key = new_key().unwrap();
        let sealed = seal(&key, JAR).unwrap();
        for i in MAGIC.len() + 1..sealed.len() {
            let mut bent = sealed.clone();
            bent[i] ^= 0x01;
            assert_eq!(open(&key, &bent), Err(Unopened::Damaged), "byte {i}");
        }
        assert_eq!(
            open(&key, &sealed[..sealed.len() - 1]),
            Err(Unopened::Damaged)
        );
        assert_eq!(open(&key, &sealed[..HEADER]), Err(Unopened::Damaged));
        assert_eq!(
            open(&new_key().unwrap(), &sealed),
            Err(Unopened::Damaged),
            "another folder's key"
        );
    }

    #[test]
    fn a_version_this_build_does_not_know_is_not_guessed_at() {
        let key = new_key().unwrap();
        let mut sealed = seal(&key, JAR).unwrap();
        sealed[4] = 2;
        assert_eq!(open(&key, &sealed), Err(Unopened::Version(2)));
        sealed[4] = 0;
        assert_eq!(open(&key, &sealed), Err(Unopened::Version(0)));
    }

    #[test]
    fn a_jar_from_before_sealing_is_told_apart() {
        assert!(!is_sealed(JAR));
        assert!(!is_sealed(b""));
        assert!(!is_sealed(b"[\n  {}\n]"));
    }

    #[test]
    fn a_key_survives_the_keyring_as_text_and_nothing_else_reads_as_one() {
        let key = new_key().unwrap();
        let text = encode_key(&key);
        assert_eq!(text.len(), 44);
        assert_eq!(decode_key(&text), Some(key));
        assert_eq!(decode_key("not a key"), None);
        assert_eq!(decode_key(&B64.encode([7u8; 16])), None, "too short");
        assert_ne!(new_key().unwrap(), key);
    }
}
