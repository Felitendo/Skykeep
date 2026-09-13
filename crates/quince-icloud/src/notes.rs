//! Apple Notes, read from its CloudKit zone.
//!
//! The zone mixes several record types. `Note` holds the content; `Folder` holds a name; and
//! `Note_UserSpecific` carries per-device state — crucially including whether a note was
//! deleted, which is *not* on the note itself.
//!
//! Text arrives in two shapes. Bodies are wrapped in Apple's document protobuf; titles and
//! folder names are bare UTF-8. One decoder handles both.

use crate::cloudkit::{field, ZoneClient};
use crate::{Error, Result};
use quince_core::notes::{Note, NoteFolder};
use serde_json::Value;

const CONTAINER: &str = "com.apple.notes";
const ZONE: &str = "Notes";

fn folder_from_record(record: &Value) -> Option<NoteFolder> {
    Some(NoteFolder {
        guid: field::record_name(record)?,
        title: field::document_text(record, "TitleEncrypted").unwrap_or_else(|| "Notes".into()),
        sort_order: 0,
    })
}

/// Whether this note's document can be rewritten without losing anything.
///
/// Apple's body is not only text: version vectors, folder links and per-device merge state
/// sit around it. The test is exact — take the text out, put the same text back, and the
/// bytes must come back identical. When they do, an edit is a surgical swap and everything
/// else survives. When they do not, there is something in this document the code does not
/// model, and saving would flatten it, so the note stays read-only.
fn can_rewrite(record: &Value) -> bool {
    let Some(encoded) = record
        .pointer("/fields/TextDataEncrypted/value")
        .and_then(Value::as_str)
    else {
        return false;
    };
    // Compares the compressed payload's own bytes, not a decoded string: the check has to
    // cover whatever whitespace the note really ends with, which reading it as text would
    // quietly trim away.
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded.trim())
        .ok()
        .is_some_and(|raw| crate::ckdocument::is_rewritable(&raw))
}

fn note_from_record(record: &Value) -> Option<Note> {
    let guid = field::record_name(record)?;
    let title = field::document_text(record, "TitleEncrypted");
    let body = field::document_text(record, "TextDataEncrypted");
    let editable = body.is_some() && can_rewrite(record);

    Some(Note {
        guid,
        folder_guid: field::reference(record, "Folder"),
        title: title.unwrap_or_else(|| "Untitled".into()),
        snippet: field::document_text(record, "SnippetEncrypted"),
        body,
        // Set from the PasswordProtectedNote records below. Inferring it from a body that
        // failed to decode would label our own parsing failures as "locked", which is a
        // claim we would not be able to stand behind.
        locked: false,
        pinned: false, // carried on the per-user record, applied below
        created_at: field::timestamp_seconds(record, "CreationDate"),
        modified_at: field::timestamp_seconds(record, "ModificationDate"),
        editable,
        etag: record
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

/// Read notes from CloudKit into the local library.
///
/// Pass `since` to fetch only what changed; as with reminders, absence in a delta means
/// unchanged rather than deleted.
pub async fn sync_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    since: Option<&str>,
) -> Result<(usize, usize, Option<String>)> {
    let fetched = fetch_cloudkit(http, ckdatabasews_root, params, since).await?;
    apply_cloudkit(library, fetched)
}

/// One pass over the zone, with nothing stored yet.
///
/// Split from storing so a refresh does the slow half without holding the library open —
/// every screen reads that library, and locking it for the length of a network walk is
/// what makes an app feel stuck.
pub struct NotesFetch {
    records: Vec<Value>,
    token: Option<String>,
    incremental: bool,
}

/// Ask Apple what changed. Touches no local state.
pub async fn fetch_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    since: Option<&str>,
) -> Result<NotesFetch> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let walk = client.all_records(ZONE, since).await?;
    Ok(NotesFetch {
        records: walk.records,
        token: walk.token,
        // A walk cut short by the page cap is not the whole truth, whatever it was asked
        // for — pruning against a partial view would delete records that are perfectly fine.
        incremental: since.is_some() || !walk.complete,
    })
}

/// Store a fetched pass. Touches no network.
pub fn apply_cloudkit(
    library: &mut quince_core::Library,
    fetched: NotesFetch,
) -> Result<(usize, usize, Option<String>)> {
    let NotesFetch {
        records,
        token,
        incremental,
    } = fetched;

    let mut folders: Vec<NoteFolder> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    // Deletion and pinning live on the per-user record, keyed by a reference to the note.
    let mut deleted_notes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pinned_notes: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Apple names this record type outright, so locking is something we read rather than guess.
    let mut locked_notes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for record in &records {
        let removed = record
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || field::flag(record, "Deleted");

        let Some(kind) = field::record_type(record) else {
            // A tombstone has a name and nothing else. Skipping them left notes deleted
            // elsewhere on screen here, and saving one of those wrote it back to Apple —
            // which is exactly how a deleted note reappears.
            if removed {
                if let Some(name) = field::record_name(record) {
                    gone.push(name);
                }
            }
            continue;
        };

        match kind.as_str() {
            "Note" => {
                let Some(name) = field::record_name(record) else {
                    continue;
                };
                if removed {
                    gone.push(name);
                } else {
                    notes.extend(note_from_record(record));
                }
            }
            "Folder" => folders.extend(folder_from_record(record)),
            // Per-user state. The note it refers to is what matters, not this record's name.
            "Note_UserSpecific" | "PasswordProtectedNote_UserSpecific" => {
                if let Some(target) = field::reference(record, "Note") {
                    if kind == "PasswordProtectedNote_UserSpecific" {
                        locked_notes.insert(target.clone());
                    }
                    if removed {
                        deleted_notes.insert(target);
                    } else if field::flag(record, "IsPinned") {
                        pinned_notes.insert(target);
                    }
                }
            }
            _ => {}
        }
    }

    // A note the owner deleted is still present as a Note record; only the per-user record
    // says so. Without this, deleted notes would keep showing.
    notes.retain(|n| !deleted_notes.contains(&n.guid));
    gone.extend(deleted_notes);
    for note in &mut notes {
        note.pinned = pinned_notes.contains(&note.guid);
        note.locked = locked_notes.contains(&note.guid);
    }

    // A note may reference a folder that this page did not carry; keeping the reference
    // would break the foreign key, so drop it and leave the note unfiled.
    let known: std::collections::HashSet<&str> = folders.iter().map(|f| f.guid.as_str()).collect();
    let stored: std::collections::HashSet<String> = library
        .note_folders()?
        .into_iter()
        .map(|f| f.guid)
        .collect();
    for note in &mut notes {
        if let Some(folder) = &note.folder_guid {
            if !known.contains(folder.as_str()) && !stored.contains(folder) {
                note.folder_guid = None;
            }
        }
    }

    library.put_note_folders(&folders)?;
    library.put_notes(&notes)?;
    if !gone.is_empty() {
        library.forget_notes(&gone)?;
    }
    // A full pass is the whole truth; an incremental one only carries changes, where
    // absence means unchanged rather than deleted.
    if !incremental {
        let keep: Vec<String> = notes.iter().map(|n| n.guid.clone()).collect();
        library.retain_notes(&keep)?;
    }
    library.mark_synced("notes", token.as_deref())?;
    Ok((folders.len(), notes.len(), token))
}

/// Milliseconds since the epoch, which is how CloudKit writes timestamps.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Apple's own rule for a note's title and its preview: the first line is the title, and
/// what follows it is the snippet.
///
/// Deriving both here rather than asking the interface for them is what keeps a note saved
/// from this app looking, in the list on a phone, exactly like one typed on the phone.
#[must_use]
pub fn title_and_snippet(body: &str) -> (String, String) {
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or_default().to_string();
    let snippet = lines.next().unwrap_or_default().to_string();
    (title, snippet)
}

/// Save a new body onto an existing note.
///
/// The document is rewritten in place against whatever Apple currently holds, not against
/// the local copy: the note may have moved on since the last sync, and the version vectors
/// and merge state inside it belong to that newer version. Refuses outright rather than
/// writing a document it cannot reproduce, because the failure mode of guessing here is
/// losing the note's history.
pub async fn edit_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    body: &str,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);

    let current = client
        .lookup_record(ZONE, &note.guid)
        .await?
        .ok_or_else(|| Error::Protocol("Apple no longer has that note.".into()))?;

    let encoded = current
        .pointer("/fields/TextDataEncrypted/value")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("That note has no body Apple will let us read.".into()))?;

    let rewritten = crate::ckdocument::replace_text_in_base64(encoded, body).ok_or_else(|| {
        Error::Protocol(
            "This note holds formatting Quince can't rewrite safely, so it was left unchanged."
                .into(),
        )
    })?;

    let (title, snippet) = title_and_snippet(body);
    let tag = current
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .or(note.etag.as_deref());

    client
        .update_record(
            ZONE,
            &note.guid,
            "Note",
            tag,
            serde_json::json!({
                "TextDataEncrypted": { "type": "ENCRYPTED_BYTES", "value": rewritten },
                "TitleEncrypted": {
                    "type": "ENCRYPTED_BYTES",
                    "value": crate::ckdocument::bare_text_to_base64(&title),
                },
                "SnippetEncrypted": {
                    "type": "ENCRYPTED_BYTES",
                    "value": crate::ckdocument::bare_text_to_base64(&snippet),
                },
                "ModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
            }),
        )
        .await?;
    Ok(())
}

/// Write a new note, optionally into a folder.
///
/// Returns the record name Apple will know it by, so the caller can open it without waiting
/// for the next sync.
pub async fn create_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    folder_guid: Option<&str>,
    body: &str,
) -> Result<String> {
    if body.trim().is_empty() {
        return Err(Error::Protocol("a note needs something in it".into()));
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record_name = format!("Note/{}", uuid::Uuid::new_v4().to_string().to_uppercase());
    let stamp = now_millis();
    let (title, snippet) = title_and_snippet(body);

    let mut fields = serde_json::json!({
        "TextDataEncrypted": {
            "type": "ENCRYPTED_BYTES",
            "value": crate::ckdocument::text_to_base64(body),
        },
        "TitleEncrypted": {
            "type": "ENCRYPTED_BYTES",
            "value": crate::ckdocument::bare_text_to_base64(&title),
        },
        "SnippetEncrypted": {
            "type": "ENCRYPTED_BYTES",
            "value": crate::ckdocument::bare_text_to_base64(&snippet),
        },
        "CreationDate": { "type": "TIMESTAMP", "value": stamp },
        "ModificationDate": { "type": "TIMESTAMP", "value": stamp },
    });
    if let Some(folder) = folder_guid {
        fields["Folder"] = client.reference_to(ZONE, folder);
    }

    client
        .create_record(ZONE, &record_name, "Note", fields)
        .await?;
    Ok(record_name)
}

/// Move a note into a folder, or out of every folder.
pub async fn move_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    folder_guid: Option<&str>,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let folder = match folder_guid {
        Some(guid) => client.reference_to(ZONE, guid),
        // CloudKit clears a reference by writing an explicit null, not by omitting it.
        None => Value::Null,
    };
    client
        .update_record(
            ZONE,
            &note.guid,
            "Note",
            note.etag.as_deref(),
            serde_json::json!({
                "Folder": folder,
                "ModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
            }),
        )
        .await?;
    Ok(())
}

/// Delete a note.
///
/// Apple's own clients mark a per-user record instead, but that record's name is built from
/// the account id in a shape CloudKit validates strictly, and inventing one is refused. A
/// direct delete of the note record is what this can do correctly, and it is what the
/// person asked for: the note goes, on every device.
pub async fn delete_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    client.delete_record(ZONE, &note.guid).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::Library;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn bare(text: &str) -> String {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        B64.encode(text.as_bytes())
    }

    fn note_record(name: &str, title: &str, body: Option<&str>, folder: &str) -> Value {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "TitleEncrypted".into(),
            json!({ "type": "ENCRYPTED_BYTES", "value": bare(title) }),
        );
        fields.insert(
            "TextDataEncrypted".into(),
            json!({ "type": "ENCRYPTED_BYTES",
                    "value": body.map_or_else(|| bare("\u{1}\u{2}"), bare) }),
        );
        fields.insert(
            "Folder".into(),
            json!({ "type": "REFERENCE", "value": { "recordName": folder } }),
        );
        fields.insert(
            "ModificationDate".into(),
            json!({ "type": "TIMESTAMP", "value": 1_700_000_000_000_i64 }),
        );
        json!({ "recordName": name, "recordType": "Note", "fields": fields })
    }

    fn folder_record(name: &str, title: &str) -> Value {
        json!({
            "recordName": name, "recordType": "Folder",
            "fields": { "TitleEncrypted": { "type": "ENCRYPTED_BYTES", "value": bare(title) } }
        })
    }

    async fn serve(body: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn notes_and_folders_land_with_their_titles() {
        let server = serve(json!({ "zones": [{
            "records": [
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "UNIT 1: Testing", Some("the body"), "Folder/f1"),
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        let (folders, notes, _) = sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        assert_eq!((folders, notes), (1, 1));

        let stored = library.notes_in(None).unwrap();
        assert_eq!(stored[0].title, "UNIT 1: Testing");
        assert_eq!(stored[0].body.as_deref(), Some("the body"));
        assert_eq!(library.note_folders().unwrap()[0].title, "Notes");
    }

    /// Deletion lives on the per-user record, not the note. Missing that leaves deleted
    /// notes on screen forever.
    #[tokio::test]
    async fn a_note_deleted_via_its_user_record_disappears() {
        let server = serve(json!({ "zones": [{
            "records": [
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Gone", Some("x"), "Folder/f1"),
                { "recordName": "NoteUS/1", "recordType": "Note_UserSpecific",
                  "fields": {
                      "Deleted": { "type": "INT64", "value": 1 },
                      "Note": { "type": "REFERENCE", "value": { "recordName": "Note/n1" } }
                  }}
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        assert!(library.notes_in(None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn pinning_comes_from_the_per_user_record() {
        let server = serve(json!({ "zones": [{
            "records": [
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Pinned one", Some("x"), "Folder/f1"),
                { "recordName": "NoteUS/1", "recordType": "Note_UserSpecific",
                  "fields": {
                      "IsPinned": { "type": "INT64", "value": 1 },
                      "Note": { "type": "REFERENCE", "value": { "recordName": "Note/n1" } }
                  }}
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        assert!(library.notes_in(None).unwrap()[0].pinned);
    }

    /// Locking is read from Apple's own record type, not inferred from a body that failed
    /// to decode — otherwise our parsing failures would reach the user as "locked", a claim
    /// we could not stand behind.
    #[tokio::test]
    async fn locking_comes_from_apples_own_record() {
        let server = serve(json!({ "zones": [{
            "records": [
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Private", Some("x"), "Folder/f1"),
                { "recordName": "PPN/1", "recordType": "PasswordProtectedNote_UserSpecific",
                  "fields": { "Note": { "type": "REFERENCE",
                                        "value": { "recordName": "Note/n1" } } } }
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        let stored = library.notes_in(None).unwrap();
        assert_eq!(stored.len(), 1, "a locked note still exists");
        assert!(stored[0].locked);
        assert_eq!(stored[0].title, "Private", "its title is not secret");
    }

    /// A body that simply would not decode must NOT be reported as locked.
    #[tokio::test]
    async fn an_undecodable_body_is_not_called_locked() {
        let server = serve(json!({ "zones": [{
            "records": [
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Odd one", None, "Folder/f1"),
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        let stored = library.notes_in(None).unwrap();
        assert!(
            !stored[0].locked,
            "we must not claim Apple locked something it did not"
        );
        assert!(stored[0].body.is_none());
    }

    /// A note pointing at a folder we have never seen must not break the foreign key.
    #[tokio::test]
    async fn an_unknown_folder_leaves_the_note_unfiled() {
        let server = serve(json!({ "zones": [{
            "records": [note_record("Note/n1", "Orphan", Some("x"), "Folder/missing")],
            "syncToken": "t1", "moreComing": false }] }))
        .await;

        let mut library = Library::in_memory().unwrap();
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        let stored = library.notes_in(None).unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].folder_guid.is_none());
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn note(guid: &str) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: Some("Folder/a".into()),
            title: "Curtains".into(),
            snippet: None,
            body: Some("Curtains\nmeasure the bay window".into()),
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: None,
            editable: true,
            etag: Some("tag".into()),
        }
    }

    /// A document shaped like the real ones: text surrounded by Apple's merge state.
    fn document(text: &str) -> String {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        use std::io::Write;
        fn framed(tag: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = vec![(tag << 3) | 2];
            let mut len = payload.len();
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
        // Version-vector siblings alongside the text, which a save must not disturb.
        let mut note = framed(2, text.as_bytes());
        note.extend(framed(3, &[0x08, 0x01, 0x10, 0x83, 0x8d, 0x02]));
        let proto = framed(2, &framed(3, &note));

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&proto).unwrap();
        B64.encode(gz.finish().unwrap())
    }

    async fn server(existing: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{
                    "recordName": "Note/1",
                    "recordType": "Note",
                    "recordChangeTag": "fresh",
                    "fields": {
                        "TextDataEncrypted": { "type": "ENCRYPTED_BYTES", "value": document(existing) }
                    }
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "Note/1" }] })),
            )
            .mount(&server)
            .await;
        server
    }

    async fn sent(server: &MockServer) -> Value {
        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/records/modify"))
            .expect("a modify was sent");
        serde_json::from_slice::<Value>(&modify.body).unwrap()["operations"][0]["record"].clone()
    }

    #[tokio::test]
    async fn an_edit_rewrites_the_body_in_place() {
        let server = server("Curtains\nmeasure the bay window").await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            "Curtains\nmeasure the bay window\norder the rail",
        )
        .await
        .unwrap();

        let record = sent(&server).await;
        let written = record["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap();
        assert_eq!(
            crate::ckdocument::text_from_base64(written).as_deref(),
            Some("Curtains\nmeasure the bay window\norder the rail")
        );
        // The version vector alongside the text has to have come through the save.
        let raw = crate::ckdocument::debug_inflate(written).unwrap();
        assert!(
            crate::ckdocument::debug_tree(&raw, 6)
                .iter()
                .any(|line| line.contains("3: message")),
            "Apple's merge state must survive an edit"
        );
    }

    /// The change tag comes from what Apple holds now, not from the local copy, so an edit
    /// applies to the current version rather than racing a change made on a phone.
    #[tokio::test]
    async fn an_edit_uses_the_change_tag_apple_just_gave_us() {
        let server = server("before").await;
        edit_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            "after",
        )
        .await
        .unwrap();
        assert_eq!(sent(&server).await["recordChangeTag"], "fresh");
    }

    /// Apple's list shows the first line as the title and the next as the preview.
    #[test]
    fn the_first_line_is_the_title() {
        assert_eq!(
            title_and_snippet("Shopping\n\nmilk and bread\neggs"),
            ("Shopping".into(), "milk and bread".into())
        );
        assert_eq!(
            title_and_snippet("  just one line  "),
            ("just one line".into(), String::new())
        );
        assert_eq!(title_and_snippet(""), (String::new(), String::new()));
    }

    #[tokio::test]
    async fn an_empty_note_is_refused_before_anything_is_sent() {
        let server = server("x").await;
        let outcome = create_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            None,
            "   \n  ",
        )
        .await;
        assert!(outcome.is_err());
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing should reach Apple"
        );
    }

    #[tokio::test]
    async fn a_new_note_carries_its_folder_and_a_readable_body() {
        let server = server("x").await;
        let name = create_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            Some("Folder/a"),
            "Packing\npassport",
        )
        .await
        .unwrap();
        assert!(name.starts_with("Note/"));

        let record = sent(&server).await;
        assert_eq!(
            record["fields"]["Folder"]["value"]["recordName"],
            "Folder/a"
        );
        assert_eq!(
            crate::ckdocument::text_from_base64(
                record["fields"]["TextDataEncrypted"]["value"]
                    .as_str()
                    .unwrap()
            )
            .as_deref(),
            Some("Packing\npassport")
        );
        // The title Apple shows in its list is bare text, not a wrapped document.
        assert_eq!(
            crate::ckdocument::text_from_base64(
                record["fields"]["TitleEncrypted"]["value"]
                    .as_str()
                    .unwrap()
            )
            .as_deref(),
            Some("Packing")
        );
    }

    /// Apple validates a per-user record's name against a shape built from the account id,
    /// and refused an invented one outright — measured against the real service, not
    /// assumed. Deleting the note record is what this can do correctly.
    #[tokio::test]
    async fn deleting_removes_the_note_record() {
        let server = server("x").await;
        delete_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operations"][0]["operationType"], "forceDelete");
        assert_eq!(body["operations"][0]["record"]["recordName"], "Note/1");
    }

    #[tokio::test]
    async fn moving_out_of_every_folder_clears_the_reference() {
        let server = server("x").await;
        move_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(sent(&server).await["fields"]["Folder"], Value::Null);
    }
}
