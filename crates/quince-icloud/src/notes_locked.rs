//! Password-locked notes (the Notes password, on an account without Advanced Data Protection),
//! opened read-only with the password the person types.
//!
//! The scheme is the one apple_cloud_notes_parser documents for NoteStore and icloud.com uses
//! in the browser: the password and the note's own salt through PBKDF2-HMAC-SHA256 (the
//! note's iteration count, 16 bytes out) give a key-encryption key; that unwraps (AES key
//! wrap, RFC 3394) the note's wrapped key; and that key opens the note's body with AES-GCM,
//! the note's initialization vector (16 bytes — not GCM's usual 12) and its tag. What comes
//! out is the same gzipped document an unlocked note keeps in `TextDataEncrypted`.
//!
//! What was measured, and what was not. On his account two `PasswordProtectedNote_UserSpecific`
//! records exist, and `AccountData` carries `CryptoPassphraseVerifier` and
//! `LockedNotesMode` = 2 — but both notes those records point at are gone (tombstones in the
//! zone walk, not found on lookup), so no live locked note was there to read. The field names
//! below are therefore the documented ones, each also accepted without its `Encrypted`
//! suffix, and a note carrying none of them is said to be locked in a way this cannot open —
//! never guessed at.
//!
//! Read only. Nothing here writes a locked note, and the password is never stored: it is used
//! for the one derivation and dropped (the app keeps it in memory for the session only).

use crate::cloudkit::ZoneClient;
use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Serialize;
use serde_json::Value;

const CONTAINER: &str = "com.apple.notes";
const ZONE: &str = "Notes";

/// The fields a locked note's record carries, each tried with and without `Encrypted`.
const SALT: &str = "CryptoSalt";
const ITERATIONS: &str = "CryptoIterationCount";
const WRAPPED_KEY: &str = "CryptoWrappedKey";
const IV: &str = "CryptoInitializationVector";
const TAG: &str = "CryptoTag";
const HINT: &str = "PasswordHint";
const BODY: &str = "TextDataEncrypted";

/// Bounds on the iteration count a record may ask for: Apple's are in the tens of thousands;
/// a record asking for billions is not one to spend a minute of CPU on.
const MOST_ITERATIONS: u32 = 10_000_000;

/// What a locked note's record holds to open it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub salt: Vec<u8>,
    pub iterations: u32,
    pub wrapped_key: Vec<u8>,
    pub iv: Vec<u8>,
    pub tag: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// A locked note, opened: its text and formatting, as an unlocked note's are read.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Unlocked {
    pub body: String,
    pub runs: Vec<crate::ckdocument::Run>,
}

/// A bytes field, as `name` + `Encrypted` or as `name`.
fn bytes_of(record: &Value, name: &str) -> Option<Vec<u8>> {
    [format!("{name}Encrypted"), name.to_string()]
        .iter()
        .find_map(|k| record.pointer(&format!("/fields/{k}/value"))?.as_str())
        .and_then(|v| B64.decode(v.trim()).ok())
        .filter(|b| !b.is_empty())
}

fn number_of(record: &Value, name: &str) -> Option<i64> {
    [format!("{name}Encrypted"), name.to_string()]
        .iter()
        .find_map(|k| record.pointer(&format!("/fields/{k}/value"))?.as_i64())
}

/// Whether a note record carries the fields a password-locked note has.
#[must_use]
pub fn is_sealed(record: &Value) -> bool {
    bytes_of(record, WRAPPED_KEY).is_some() && bytes_of(record, SALT).is_some()
}

/// The sealed parts of a locked note's record, or why it cannot be opened here.
pub fn sealed(record: &Value) -> Result<Sealed> {
    let not_here = || {
        Error::Refused(
            "This note is locked in a way Skykeep can’t open — open it on an Apple device.".into(),
        )
    };
    if record.pointer(&format!("/fields/{BODY}")).is_none()
        && record.pointer("/fields/TextDataAsset").is_some()
    {
        return Err(Error::Refused(
            "This locked note is too long for Skykeep to open yet.".into(),
        ));
    }
    let iterations = number_of(record, ITERATIONS)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| (1..=MOST_ITERATIONS).contains(n))
        .ok_or_else(not_here)?;
    Ok(Sealed {
        salt: bytes_of(record, SALT).ok_or_else(not_here)?,
        iterations,
        wrapped_key: bytes_of(record, WRAPPED_KEY).ok_or_else(not_here)?,
        iv: bytes_of(record, IV).ok_or_else(not_here)?,
        tag: bytes_of(record, TAG).ok_or_else(not_here)?,
        ciphertext: record
            .pointer(&format!("/fields/{BODY}/value"))
            .and_then(Value::as_str)
            .and_then(|v| B64.decode(v.trim()).ok())
            .ok_or_else(not_here)?,
    })
}

/// The hint the person left with the password, if there is one.
#[must_use]
pub fn hint(record: &Value) -> Option<String> {
    let bytes = bytes_of(record, HINT)?;
    let text = String::from_utf8(bytes).ok()?;
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn wrong_password() -> Error {
    Error::Refused("That password didn’t open this note.".into())
}

/// The note's key, unwrapped with the key the password derives.
fn note_key(sealed: &Sealed, password: &str) -> Result<Vec<u8>> {
    let mut kek = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(
        password.as_bytes(),
        &sealed.salt,
        sealed.iterations,
        &mut kek,
    );
    let wrapped = &sealed.wrapped_key;
    // RFC 3394 output is the key plus 8 bytes; a 16- or 32-byte key is all AES-GCM takes.
    if !matches!(wrapped.len(), 24 | 40) {
        return Err(Error::Protocol(format!(
            "a locked note's wrapped key was {} bytes",
            wrapped.len()
        )));
    }
    let mut key = vec![0u8; wrapped.len() - 8];
    let unwrapped = aes_kw::KekAes128::from(kek).unwrap(wrapped, &mut key);
    kek.fill(0);
    // The unwrap carries its own integrity check: a wrong password fails here.
    unwrapped.map_err(|_| wrong_password())?;
    Ok(key)
}

/// Open the body: the plaintext document (as `TextDataEncrypted` carries an unlocked one).
pub fn decrypt(sealed: &Sealed, password: &str) -> Result<Vec<u8>> {
    use aes_gcm::aead::consts::U16;
    use aes_gcm::aead::{generic_array::GenericArray, AeadInPlace, KeyInit};
    if sealed.iv.len() != 16 || sealed.tag.len() != 16 {
        return Err(Error::Protocol(format!(
            "a locked note's IV/tag were {}/{} bytes, not 16/16",
            sealed.iv.len(),
            sealed.tag.len()
        )));
    }
    let mut key = note_key(sealed, password)?;
    let nonce = GenericArray::from_slice(&sealed.iv);
    let tag = GenericArray::from_slice(&sealed.tag);
    let mut buffer = sealed.ciphertext.clone();
    let opened = match key.len() {
        16 => aes_gcm::AesGcm::<aes::Aes128, U16>::new_from_slice(&key)
            .map_err(|_| wrong_password())?
            .decrypt_in_place_detached(nonce, b"", &mut buffer, tag),
        _ => aes_gcm::AesGcm::<aes::Aes256, U16>::new_from_slice(&key)
            .map_err(|_| wrong_password())?
            .decrypt_in_place_detached(nonce, b"", &mut buffer, tag),
    };
    key.fill(0);
    opened.map_err(|_| wrong_password())?;
    Ok(buffer)
}

/// A locked note's record opened with `password`: its text and formatting.
pub fn unlock(record: &Value, password: &str) -> Result<Unlocked> {
    let plain = decrypt(&sealed(record)?, password)?;
    let encoded = B64.encode(&plain);
    // Its lines as the phone draws them, the runs moved with them.
    let body = crate::ckdocument::shown_text_from_base64(&encoded)
        .or_else(|| {
            crate::ckdocument::text_from_bytes(&plain)
                .map(|t| crate::ckdocument::apple_line_breaks(&t).into_owned())
        })
        .ok_or_else(|| {
            Error::Protocol("the note opened, but its text is in a shape this can’t read".into())
        })?;
    Ok(Unlocked {
        body,
        runs: crate::ckdocument::shown_runs_from_base64(&encoded),
    })
}

async fn record_of(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
) -> Result<Value> {
    ZoneClient::new(http, ckdatabasews_root, CONTAINER, params)
        .lookup_record(ZONE, guid)
        .await?
        .ok_or_else(|| Error::Refused("Apple no longer has that note.".into()))
}

/// Open a locked note as Apple holds it now. Reads only.
pub async fn unlock_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    password: &str,
) -> Result<Unlocked> {
    let record = record_of(http, ckdatabasews_root, params, guid).await?;
    unlock(&record, password)
}

/// A locked note's password hint, as Apple holds it now.
pub async fn hint_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
) -> Result<Option<String>> {
    let record = record_of(http, ckdatabasews_root, params, guid).await?;
    Ok(hint(&record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::consts::U16;
    use aes_gcm::aead::{generic_array::GenericArray, AeadInPlace, KeyInit};
    use serde_json::json;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// RFC 3394 §4.1: 128-bit key data with a 128-bit KEK.
    #[test]
    fn key_unwrap_matches_rfc_3394() {
        let kek: [u8; 16] = hex("000102030405060708090A0B0C0D0E0F").try_into().unwrap();
        let wrapped = hex("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5");
        let mut out = [0u8; 16];
        aes_kw::KekAes128::from(kek)
            .unwrap(&wrapped, &mut out)
            .unwrap();
        assert_eq!(out.to_vec(), hex("00112233445566778899AABBCCDDEEFF"));
    }

    /// RFC 7914 §11: PBKDF2-HMAC-SHA256("passwd", "salt", 1).
    #[test]
    fn pbkdf2_sha256_matches_rfc_7914() {
        let mut out = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(b"passwd", b"salt", 1, &mut out);
        assert_eq!(out.to_vec(), hex("55ac046e56e3089fec1691c22544b605"));
    }

    /// A locked note built the way Apple builds one: a random note key wrapped under the
    /// password's key, the document sealed with AES-GCM and a 16-byte IV.
    fn seal(password: &str, document: &[u8], iterations: u32) -> Value {
        let salt = b"0123456789abcdef".to_vec();
        let mut kek = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, iterations, &mut kek);
        let note_key = hex("8899aabbccddeeff0011223344556677");
        let mut wrapped = [0u8; 24];
        aes_kw::KekAes128::from(kek)
            .wrap(&note_key, &mut wrapped)
            .unwrap();
        let iv = b"IV-sixteen-bytes".to_vec();
        let mut body = document.to_vec();
        let tag = aes_gcm::AesGcm::<aes::Aes128, U16>::new_from_slice(&note_key)
            .unwrap()
            .encrypt_in_place_detached(GenericArray::from_slice(&iv), b"", &mut body)
            .unwrap();
        json!({ "recordName": "N", "recordType": "Note", "fields": {
            "CryptoSaltEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode(&salt) },
            "CryptoIterationCount": { "type": "INT64", "value": iterations },
            "CryptoWrappedKeyEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode(wrapped) },
            "CryptoInitializationVectorEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode(&iv) },
            "CryptoTagEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode(tag) },
            "PasswordHintEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode("the dog") },
            "TextDataEncrypted": { "type": "ENCRYPTED_BYTES", "value": B64.encode(&body) },
        } })
    }

    #[test]
    fn the_right_password_opens_the_note() {
        let document = B64
            .decode(crate::ckdocument::text_to_base64("Secret plans\nline two"))
            .unwrap();
        let record = seal("hunter2", &document, 2000);
        assert!(is_sealed(&record));
        let opened = unlock(&record, "hunter2").unwrap();
        assert_eq!(opened.body, "Secret plans\nline two");
        assert_eq!(opened.runs.iter().map(|r| r.length).sum::<u64>(), 21);
        assert_eq!(hint(&record).as_deref(), Some("the dog"));
    }

    #[test]
    fn a_wrong_password_is_refused_in_words() {
        let document = B64.decode(crate::ckdocument::text_to_base64("x")).unwrap();
        let record = seal("hunter2", &document, 1000);
        let err = unlock(&record, "hunter3").unwrap_err().to_string();
        assert!(err.contains("password"), "{err}");
    }

    #[test]
    fn a_tampered_body_is_refused() {
        let document = B64
            .decode(crate::ckdocument::text_to_base64("abc"))
            .unwrap();
        let mut record = seal("pw", &document, 1000);
        let mut body = B64
            .decode(
                record["fields"]["TextDataEncrypted"]["value"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        body[0] ^= 1;
        record["fields"]["TextDataEncrypted"]["value"] = json!(B64.encode(body));
        assert!(unlock(&record, "pw").is_err());
    }

    /// The field names without `Encrypted` are read too.
    #[test]
    fn plain_field_names_are_read_too() {
        let document = B64.decode(crate::ckdocument::text_to_base64("hi")).unwrap();
        let mut record = seal("pw", &document, 1000);
        let fields = record["fields"].as_object_mut().unwrap();
        for name in [SALT, WRAPPED_KEY, IV, TAG] {
            let v = fields.remove(&format!("{name}Encrypted")).unwrap();
            fields.insert(name.to_string(), v);
        }
        assert_eq!(unlock(&record, "pw").unwrap().body, "hi");
    }

    /// A note with none of the fields (a passcode-locked note, or an unlocked one) is never
    /// guessed at.
    #[test]
    fn a_note_without_the_fields_is_not_guessed_at() {
        let record = json!({ "fields": { "TextDataEncrypted": { "value": "AAAA" } } });
        assert!(!is_sealed(&record));
        assert!(matches!(unlock(&record, "pw"), Err(Error::Refused(_))));
        let mut huge = seal("pw", b"x", 1000);
        huge["fields"]["CryptoIterationCount"]["value"] = json!(4_000_000_000_u64);
        assert!(matches!(unlock(&huge, "pw"), Err(Error::Refused(_))));
    }
}
