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
use quince_core::notes::{Note, NoteFolder, TRASH_FOLDER};
use serde_json::Value;

const CONTAINER: &str = "com.apple.notes";
const ZONE: &str = "Notes";

/// The first folder in a note's `Folders` list, where it has one.
fn first_folder(record: &Value) -> Option<String> {
    record
        .pointer("/fields/Folders/value")?
        .as_array()?
        .iter()
        .find_map(|entry| entry.get("recordName")?.as_str().map(ToString::to_string))
}

fn folder_from_record(record: &Value) -> Option<NoteFolder> {
    Some(NoteFolder {
        guid: field::record_name(record)?,
        title: field::document_text(record, "TitleEncrypted").unwrap_or_else(|| "Notes".into()),
        sort_order: 0,
        // Apple Notes nests folders and keeps the link here. Neither folder Apple makes for
        // itself has one, so this account never shows it — but a folder created with one is
        // accepted and handed straight back, which is how it was confirmed. Without it,
        // somebody's filing comes out as one long list with every level jumbled together.
        parent_guid: field::reference(record, "ParentFolder"),
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
        // `Folders` first, `Folder` only if there is no list.
        //
        // Apple carries both, and on one note on this account they disagree: Curtains has
        // Folder pointing at Recently Deleted and Folders naming the real one. The phone
        // shows it where Folders says, so the list is what counts and the singular is a
        // leftover. Reading the leftover put a note the owner had taken back out of the
        // trash straight back into it here — and hid it, because deleted notes are filtered
        // from every folder view.
        folder_guid: first_folder(record).or_else(|| field::reference(record, "Folder")),
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
/// Where the document a new note is shaped from is kept.
pub const TEMPLATE_KEY: &str = "notes.document_template";

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
    let mut template: Option<String> = None;
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
                    // A document Apple wrote is worth keeping the shape of. A note created
                    // here is built from it, because one built from nothing shows on a phone
                    // as an empty note. Noticed during sync rather than fetched on demand:
                    // notes sit far enough into the zone walk that the first page of a hundred
                    // and thirty-nine records held none of them.
                    if template.is_none() {
                        template = record
                            .pointer("/fields/TextDataEncrypted/value")
                            .and_then(Value::as_str)
                            .filter(|d| crate::ckdocument::has_merge_records(d))
                            .map(ToString::to_string);
                    }
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
    if let Some(document) = template {
        library.set_state(TEMPLATE_KEY, &document)?;
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
    // The first line exactly as written, blank or not, and the next line that says something.
    //
    // Skipping the blank ones meant a note that opens with an empty line was titled here and
    // untitled on the phone, which takes the first line as it finds it. Two apps disagreeing
    // about what a note is called is worse than an untitled note.
    let mut lines = body.lines();
    let title = lines.next().unwrap_or_default().trim().to_string();
    let snippet = lines
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
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
    template: Option<&str>,
) -> Result<String> {
    if body.trim().is_empty() {
        return Err(Error::Protocol("a note needs something in it".into()));
    }

    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    // Apple names a note record with a bare uppercase UUID and nothing else. Every note on a
    // real account is shaped that way; the four that were not are the ones this app made.
    //
    // The shape is load-bearing, not cosmetic. Apple derives a second record from it —
    // `Note_UserSpecific::_<account>/<this name>` — which is where per-person state like
    // pinning lives, and the server validates that derived name. A record called `Note/<uuid>`
    // produced `Note_UserSpecific::_<account>/Note/<uuid>`, which Apple rejected outright: it
    // is why deleting a note written here used to need a forced delete, and why a note made
    // here was never quite the same kind of thing as a note made on the phone.
    let record_name = uuid::Uuid::new_v4().to_string().to_uppercase();
    let stamp = now_millis();
    let (title, snippet) = title_and_snippet(body);

    // A document Apple's own reader will show. See `document_like_apples`.
    let document = document_like_apples(template, body);

    let mut fields = serde_json::json!({
        "TextDataEncrypted": {
            "type": "ENCRYPTED_BYTES",
            "value": document,
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
        // Every note Apple made carries these four and no note written here ever did. They
        // are not decoration: a client reading a note with no minimum version has nothing to
        // decide against, and Apple's own reader is stricter than this one. INT64 rather than
        // NUMBER_INT64 — the notes zone spells it the short way, and the reminders zone does
        // not, which is the sort of thing only reading the real records tells you.
        "MinimumSupportedNotesVersion": { "type": "INT64", "value": 0 },
        "AttachmentViewType": { "type": "INT64", "value": 0 },
        "PaperStyleType": { "type": "INT64", "value": 0 },
        "Deleted": { "type": "INT64", "value": 0 },
    });
    if let Some(folder) = folder_guid {
        fields["Folder"] = client.reference_to(ZONE, folder);
    }
    client
        .create_record(ZONE, &record_name, "Note", fields)
        .await?;
    Ok(record_name)
}

/// A note's text in a document Apple's own reader will show.
///
/// A document built from nothing — text, one attribute run, the version fields — is read back
/// here perfectly and shows on a phone as an empty note. Laying one beside a note Apple wrote
/// says why: inside the note message Apple keeps a handful of merge records, and ours had none.
///
/// ```text
/// Apple's   note [ text, 3,3,3,3,3,3,3, 4, 5 ]
/// ours      note [ text,                   5 ]
/// ```
///
/// Those records say which writer produced which stretch of text, and field 4 names the writer
/// with a UUID. Synthesising a plausible-looking set was tried and the phone still showed
/// nothing; what works is taking a document Apple already wrote and swapping only the text —
/// the same operation an edit performs, which has always worked.
///
/// Copying them is not the sleight of hand it first looks like. Field 4 identifies the writer,
/// and a phone stamps every note it makes with the same one, so notes sharing a writer is the
/// normal case rather than a collision. Each note's records are its own and reference nothing
/// outside it.
///
/// An account with no note to learn from — a new one — gets a document built from scratch.
/// That is the shape that does not render, and there is nothing better to be had until the
/// account has one note on it; the alternative is refusing to create a note at all.
///
/// Tried and rejected along the way: copying `ReplicaIDToUserIDEncrypted` and
/// `ReplicaIDToNotesVersionDataEncrypted`, which are identical on every note on an account and
/// so looked like a registry worth stamping onto a new one. Apple drops them silently on a
/// write. They are the server's to keep, not a client's to set — and notes that lack them
/// render perfectly, so they were never the reason.
fn document_like_apples(template: Option<&str>, body: &str) -> String {
    template
        .filter(|document| crate::ckdocument::has_merge_records(document))
        // The borrowed note's history goes; the writer stays. Carrying another note's record
        // of which stretch of text came from where made a phone show a short note as the same
        // word repeated, with gaps where the arithmetic ran off the end of it.
        .and_then(crate::ckdocument::without_history)
        .and_then(|document| crate::ckdocument::replace_text_in_base64(&document, body))
        .unwrap_or_else(|| crate::ckdocument::text_to_base64(body))
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
    client
        .update_record(
            ZONE,
            &note.guid,
            "Note",
            note.etag.as_deref(),
            filed_in(&client, folder_guid.unwrap_or(DEFAULT_FOLDER)),
        )
        .await?;
    Ok(())
}

/// Apple's own "Notes" folder, where a note filed nowhere in particular lives.
pub const DEFAULT_FOLDER: &str = "DefaultFolder-CloudKit";

/// The fields that put a note in a folder, as the phone writes them.
///
/// The phone reads `Folders`, a list; `Folder` is an older single reference it keeps beside
/// it. Only `Folder` was written here, so a note moved in this app stayed where it was on the
/// phone — and a note "deleted" here by moving it would have too. Both are written, with the
/// date the phone stamps on a move. A note is always in some folder: Apple has no "none".
fn filed_in(client: &ZoneClient<'_>, folder: &str) -> Value {
    let reference = client.reference_to(ZONE, folder);
    serde_json::json!({
        "Folder": reference,
        "Folders": { "type": "REFERENCE_LIST", "value": [reference["value"].clone()] },
        "FoldersModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
        "ModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
    })
}
/// Delete a note the way the phone does.
///
/// This used to remove the record outright, which on iCloud is final: no Recently Deleted, no
/// thirty days, nothing on any device to recover — several notes were lost that way. Measured
/// on the account, the phone never does that. Deleting files a note in `TrashFolder-CloudKit`
/// with `Deleted` still 0, and that is Recently Deleted; deleting it again from there sets
/// `Deleted` to 1 and leaves the record for Apple to clear. Nothing here removes a record.
pub async fn delete_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let fields = if note.folder_guid.as_deref() == Some(TRASH_FOLDER) {
        // Already in Recently Deleted: this is "Delete Now". Apple marks the record rather
        // than removing it, and so does this.
        serde_json::json!({
            "Deleted": { "type": "INT64", "value": 1 },
            "ModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
        })
    } else {
        // Delete means Recently Deleted, where it waits thirty days to be recovered.
        filed_in(&client, TRASH_FOLDER)
    };
    client
        .update_record(ZONE, &note.guid, "Note", note.etag.as_deref(), fields)
        .await?;
    Ok(())
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
        // A listed note carries no body — the text is stored, and asked for on its own when
        // the note is opened. Carrying it here is what made a folder of thousands slow, and
        // a body that came back short and got saved would take the rest of the note with it.
        assert_eq!(stored[0].body, None);
        assert_eq!(
            library.note_body(&stored[0].guid).unwrap().as_deref(),
            Some("the body"),
            "the text arrived and was stored"
        );
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

    /// A document shaped like the real ones: text with a phone's merge records around it.
    fn document(text: &str) -> String {
        crate::ckdocument::add_merge_records_for_test(&crate::ckdocument::text_to_base64(text))
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
        // The records a phone merges on came through the save, describing the new text.
        assert_eq!(
            crate::ckdocument::debug_records(written),
            Some(utf16("Curtains\nmeasure the bay window\norder the rail"))
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
            None,
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
            None,
        )
        .await
        .unwrap();
        // Apple names a note with a bare uppercase UUID and nothing else. The shape is
        // load-bearing: Apple derives `Note_UserSpecific::_<account>/<name>` from it and
        // validates that, so a name with a prefix in it made a note the phone treated as a
        // different kind of thing and would not let us delete normally.
        assert!(
            !name.contains('/'),
            "a bare name, the way Apple's are: {name}"
        );
        assert_eq!(name.len(), 36, "a UUID: {name}");
        assert_eq!(name, name.to_uppercase(), "uppercase, the way Apple's are");

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

    /// The operations sent to Apple, every one.
    async fn operations(server: &MockServer) -> Vec<Value> {
        let mut out = Vec::new();
        for r in server.received_requests().await.unwrap() {
            if let Ok(body) = serde_json::from_slice::<Value>(&r.body) {
                out.extend(body["operations"].as_array().cloned().unwrap_or_default());
            }
        }
        out
    }

    /// Delete is Recently Deleted, as on the phone. Removing the record — what this did —
    /// is final on iCloud: notes deleted here were gone from every device with nothing left
    /// to recover.
    #[tokio::test]
    async fn deleting_files_the_note_in_recently_deleted() {
        let server = server("x").await;
        delete_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
        )
        .await
        .unwrap();
        let ops = operations(&server).await;
        assert!(
            ops.iter()
                .all(|op| op["operationType"] != "forceDelete" && op["operationType"] != "delete"),
            "nothing is ever removed: {ops:?}"
        );
        let fields = &ops[0]["record"]["fields"];
        assert_eq!(fields["Folders"]["value"][0]["recordName"], TRASH_FOLDER);
        assert_eq!(fields["Folder"]["value"]["recordName"], TRASH_FOLDER);
    }

    /// Deleting again from Recently Deleted marks it, as the phone does; still no removal.
    #[tokio::test]
    async fn deleting_from_recently_deleted_marks_the_record() {
        let server = server("x").await;
        let mut trashed = note("Note/1");
        trashed.folder_guid = Some(TRASH_FOLDER.into());
        delete_cloudkit(&reqwest::Client::new(), &server.uri(), vec![], &trashed)
            .await
            .unwrap();
        let ops = operations(&server).await;
        assert!(ops
            .iter()
            .all(|op| op["operationType"] != "forceDelete" && op["operationType"] != "delete"));
        assert_eq!(ops[0]["record"]["fields"]["Deleted"]["value"], 1);
    }

    /// The phone reads `Folders`; writing only `Folder` left a moved note where it was.
    #[tokio::test]
    async fn moving_writes_the_folder_the_phone_reads() {
        let server = server("x").await;
        move_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            Some("Folder/a"),
        )
        .await
        .unwrap();
        let fields = &sent(&server).await["fields"];
        assert_eq!(fields["Folders"]["value"][0]["recordName"], "Folder/a");
        assert_eq!(fields["Folder"]["value"]["recordName"], "Folder/a");
    }

    /// Apple has no "no folder": a note filed nowhere in particular is in Notes.
    #[tokio::test]
    async fn filing_nowhere_is_filing_in_notes() {
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
        let fields = &sent(&server).await["fields"];
        assert_eq!(fields["Folders"]["value"][0]["recordName"], DEFAULT_FOLDER);
    }
}

/// Folders other people have, that this account does not.
///
/// This account holds only the two Apple makes for itself, neither nested. A folder created
/// with a `ParentFolder` reference was accepted and handed straight back, which is how the
/// field was confirmed — these carry that shape.
#[cfg(test)]
mod folder_shape_tests {
    use super::*;
    use serde_json::json;

    fn folder(name: &str, title: &str, parent: Option<&str>) -> Value {
        let mut fields = json!({
            "TitleEncrypted": {
                "type": "ENCRYPTED_BYTES",
                "value": crate::ckdocument::bare_text_to_base64(title),
            },
        });
        if let Some(parent) = parent {
            fields["ParentFolder"] = json!({
                "type": "REFERENCE",
                "value": { "action": "VALIDATE", "recordName": parent },
            });
        }
        json!({ "recordName": name, "recordType": "Folder", "fields": fields })
    }

    #[test]
    fn a_nested_folder_keeps_the_one_it_sits_in() {
        let outer = folder_from_record(&folder("F1", "University", None)).unwrap();
        let inner = folder_from_record(&folder("F2", "Semester 5", Some("F1"))).unwrap();
        assert_eq!(outer.parent_guid, None);
        assert_eq!(inner.parent_guid.as_deref(), Some("F1"));
        assert_eq!(inner.title, "Semester 5");
    }

    /// The two Apple makes for itself have no parent and must not gain one.
    #[test]
    fn apples_own_folders_stay_at_the_top() {
        for name in ["DefaultFolder-CloudKit", "TrashFolder-CloudKit"] {
            let parsed = folder_from_record(&json!({
                "recordName": name,
                "recordType": "Folder",
                "fields": {
                    "MinimumSupportedNotesVersion": { "type": "INT64", "value": 0 },
                    "TitleEncrypted": { "type": "ENCRYPTED_BYTES", "value": "Tm90ZXM=" },
                }
            }))
            .unwrap();
            assert_eq!(parsed.parent_guid, None);
            assert_eq!(parsed.title, "Notes", "plain base64, which Apple also uses");
        }
    }

    /// A folder whose parent has been deleted must still appear, at the top rather than
    /// nowhere — a folder that references nothing is not a folder that does not exist.
    #[test]
    fn a_folder_whose_parent_is_gone_is_still_a_folder() {
        let orphan = folder_from_record(&folder("F3", "Odds and ends", Some("GONE"))).unwrap();
        assert_eq!(orphan.title, "Odds and ends");
        assert_eq!(orphan.parent_guid.as_deref(), Some("GONE"));
    }
}

#[cfg(test)]
fn utf16(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

/// Shaping a new note like one Apple wrote.
#[cfg(test)]
mod template_tests {
    use super::*;

    /// A document with a phone's merge records, to stand in for one Apple wrote.
    fn apples_shape(text: &str) -> String {
        crate::ckdocument::add_merge_records_for_test(&crate::ckdocument::text_to_base64(text))
    }

    /// A new note's records are its own: none of the history of the note it was shaped by,
    /// and the text it holds written by this app.
    ///
    /// The history is lengths measured against text that is not in this note. Carried over, a
    /// fifteen-character note came out on a phone as the same word four times with gaps in
    /// between. Carrying none at all left it blank.
    #[test]
    fn a_new_note_has_records_of_its_own() {
        let template = apples_shape("Something Apple wrote");
        let made = document_like_apples(Some(&template), "Something written here");
        assert_eq!(
            crate::ckdocument::text_from_base64(&made).as_deref(),
            Some("Something written here")
        );
        assert_eq!(
            crate::ckdocument::debug_records(&made),
            Some(utf16("Something written here")),
            "records that describe exactly this text"
        );
        assert_eq!(
            crate::ckdocument::debug_writers(&made),
            1,
            "this app alone wrote it; the template's writer did not"
        );
    }

    /// A note's own history is its own, and an edit adds to it rather than replacing it.
    #[test]
    fn editing_a_note_keeps_the_notes_own_history() {
        let mine = apples_shape("What I wrote before");
        let edited =
            crate::ckdocument::replace_text_in_base64(&mine, "What I wrote after").unwrap();
        assert_eq!(
            crate::ckdocument::debug_records(&edited),
            Some(utf16("What I wrote after"))
        );
        assert_eq!(
            crate::ckdocument::debug_writers(&edited),
            2,
            "the phone, then this app"
        );
    }

    /// A new account has no note to learn from. Building from nothing is the only option, and
    /// refusing to create a note at all would be worse.
    #[test]
    fn with_nothing_to_learn_from_it_still_makes_a_note() {
        let made = document_like_apples(None, "The first note on this account");
        assert_eq!(
            crate::ckdocument::text_from_base64(&made).as_deref(),
            Some("The first note on this account")
        );
    }

    /// A template that is itself the shape that does not render is no use as a template.
    #[test]
    fn a_template_without_the_records_is_not_used_as_one() {
        let useless = crate::ckdocument::text_to_base64("no merge records here");
        let made = document_like_apples(Some(&useless), "New text");
        assert_eq!(
            crate::ckdocument::text_from_base64(&made).as_deref(),
            Some("New text"),
            "the note is still made"
        );
    }

    /// Rubbish stored as a template must not take a note down with it.
    #[test]
    fn rubbish_stored_as_a_template_is_ignored() {
        for bad in ["", "not base64 !!", "aGVsbG8="] {
            let made = document_like_apples(Some(bad), "Still fine");
            assert_eq!(
                crate::ckdocument::text_from_base64(&made).as_deref(),
                Some("Still fine")
            );
        }
    }
}

/// What a note is called, and what shows under the name.
#[cfg(test)]
mod title_tests {
    use super::*;

    /// A phone takes the first line as it finds it. Skipping blank ones meant a note that
    /// opens with an empty line read as titled here and untitled there — two apps disagreeing
    /// about what the same note is called.
    #[test]
    fn the_title_is_the_first_line_even_when_it_is_empty() {
        assert_eq!(title_and_snippet("\nhello").0, "");
        assert_eq!(title_and_snippet("\n\n\nhello").0, "");
        assert_eq!(title_and_snippet("hello\nworld").0, "hello");
        assert_eq!(title_and_snippet("  hello  \nworld").0, "hello");
    }

    /// The line under it is the next one that says something, so a note separated by a blank
    /// line still previews.
    #[test]
    fn the_preview_is_the_next_line_with_something_on_it() {
        assert_eq!(title_and_snippet("Shopping\n\n\nMilk").1, "Milk");
        assert_eq!(title_and_snippet("Shopping\nMilk").1, "Milk");
        assert_eq!(title_and_snippet("Shopping").1, "");
        assert_eq!(title_and_snippet("").1, "");
    }

    /// A note of nothing but blank lines has no name and nothing to preview, which is what it
    /// is rather than an error.
    #[test]
    fn a_note_of_blank_lines_has_neither() {
        assert_eq!(title_and_snippet("\n\n\n"), (String::new(), String::new()));
    }
}

/// Which folder a note is in, when Apple's two answers disagree.
#[cfg(test)]
mod folder_of_tests {
    use super::*;
    use serde_json::json;

    fn note_in(folder: Option<&str>, folders: Option<&[&str]>) -> Value {
        let mut fields = json!({
            "TextDataEncrypted": {
                "type": "ENCRYPTED_BYTES",
                "value": crate::ckdocument::text_to_base64("Curtains\nBathroom accessories"),
            },
        });
        if let Some(one) = folder {
            fields["Folder"] = json!({
                "type": "REFERENCE",
                "value": { "action": "NONE", "recordName": one },
            });
        }
        if let Some(list) = folders {
            fields["Folders"] = json!({
                "type": "REFERENCE_LIST",
                "value": list.iter().map(|n| json!({ "action": "NONE", "recordName": n }))
                    .collect::<Vec<_>>(),
            });
        }
        json!({ "recordName": "N1", "recordType": "Note", "fields": fields })
    }

    /// The case this was found on: one note where the two fields disagree. The phone shows it
    /// where the list says, so the list is what counts. Believing the other one put a note the
    /// owner had taken back out of the trash straight back into it — and hid it, because
    /// deleted notes are filtered out of every folder view.
    #[test]
    fn the_list_wins_over_the_leftover() {
        let parsed = note_from_record(&note_in(
            Some("TrashFolder-CloudKit"),
            Some(&["DefaultFolder-CloudKit"]),
        ))
        .unwrap();
        assert_eq!(
            parsed.folder_guid.as_deref(),
            Some("DefaultFolder-CloudKit")
        );
    }

    /// Where they agree, nothing changes.
    #[test]
    fn agreement_is_left_alone() {
        let parsed = note_from_record(&note_in(
            Some("DefaultFolder-CloudKit"),
            Some(&["DefaultFolder-CloudKit"]),
        ))
        .unwrap();
        assert_eq!(
            parsed.folder_guid.as_deref(),
            Some("DefaultFolder-CloudKit")
        );
    }

    /// A note with no list still has to be filed somewhere.
    #[test]
    fn without_a_list_the_single_field_is_used() {
        let parsed = note_from_record(&note_in(Some("TrashFolder-CloudKit"), None)).unwrap();
        assert_eq!(parsed.folder_guid.as_deref(), Some("TrashFolder-CloudKit"));
    }

    /// And a note with neither is in no folder rather than in a wrong one.
    #[test]
    fn with_neither_it_is_in_no_folder() {
        assert_eq!(
            note_from_record(&note_in(None, None)).unwrap().folder_guid,
            None
        );
    }

    /// An empty list is not an answer; the other field still is.
    #[test]
    fn an_empty_list_falls_back() {
        let parsed = note_from_record(&note_in(Some("DefaultFolder-CloudKit"), Some(&[]))).unwrap();
        assert_eq!(
            parsed.folder_guid.as_deref(),
            Some("DefaultFolder-CloudKit")
        );
    }
}
