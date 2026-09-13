//! Apple Notes, read from its CloudKit zone.
//!
//! The zone mixes several record types. `Note` holds the content; `Folder` holds a name; and
//! `Note_UserSpecific` carries per-device state — crucially including whether a note was
//! deleted, which is *not* on the note itself.
//!
//! Text arrives in two shapes. Bodies are wrapped in Apple's document protobuf; titles and
//! folder names are bare UTF-8. One decoder handles both.

use crate::cloudkit::{field, ZoneClient};
use crate::Result;
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

fn note_from_record(record: &Value) -> Option<Note> {
    let guid = field::record_name(record)?;
    let title = field::document_text(record, "TitleEncrypted");
    let body = field::document_text(record, "TextDataEncrypted");

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
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let (records, token) = client.all_records(ZONE, since).await?;

    let mut folders: Vec<NoteFolder> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    // Deletion and pinning live on the per-user record, keyed by a reference to the note.
    let mut deleted_notes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pinned_notes: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Apple names this record type outright, so locking is something we read rather than guess.
    let mut locked_notes: std::collections::HashSet<String> = std::collections::HashSet::new();

    for record in &records {
        let Some(kind) = field::record_type(record) else {
            continue;
        };
        let removed = record
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || field::flag(record, "Deleted");

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
    library.mark_synced("notes", token.as_deref())?;
    Ok((folders.len(), notes.len(), token))
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
