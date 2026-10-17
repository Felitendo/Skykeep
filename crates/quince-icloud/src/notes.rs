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
    // A note kept as a file is read from the file, but writing one back means sending a file,
    // which this does not do yet — so it is shown and not offered for editing.
    let from_asset = record.pointer(&format!("/fields/{FROM_ASSET}")).is_some();
    let editable = body.is_some() && can_rewrite(record) && !from_asset;

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
        // The stored title where there is one; otherwise the phone's rule applied to the text,
        // and "New Note" — Apple's word, not ours — only when there is no text at all.
        title: title
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                body.as_deref()
                    .map(|b| title_and_snippet(b).0)
                    .filter(|t| !t.is_empty())
            })
            .unwrap_or_else(|| "New Note".into()),
        snippet: field::document_text(record, "SnippetEncrypted"),
        body,
        runs: record
            .pointer("/fields/TextDataEncrypted/value")
            .and_then(Value::as_str)
            .map(crate::ckdocument::runs_from_base64)
            .filter(|r| !r.is_empty())
            .and_then(|r| serde_json::to_string(&r).ok()),
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
    let mut walk = client.all_records(ZONE, since).await?;
    // Long notes' files, eight at a time: one after another, a first pass over an account
    // with many long notes waited on each download in turn.
    let wanted: Vec<(usize, String)> = walk
        .records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| asset_url(r).map(|u| (i, u)))
        .collect();
    use futures_util::StreamExt;
    let fetched: Vec<(usize, Option<Vec<u8>>)> = futures_util::stream::iter(wanted)
        .map(|(i, url)| async move { (i, download(http, &url).await) })
        .buffer_unordered(8)
        .collect()
        .await;
    for (i, bytes) in fetched {
        if let (Some(bytes), Some(record)) = (bytes, walk.records.get_mut(i)) {
            put_asset_body(record, &bytes);
        }
    }
    Ok(NotesFetch {
        records: walk.records,
        token: walk.token,
        // A walk cut short by the page cap is not the whole truth, whatever it was asked
        // for — pruning against a partial view would delete records that are perfectly fine.
        incremental: since.is_some() || !walk.complete,
    })
}

/// The most document a note record carries inline. CloudKit limits a whole record to about a
/// megabyte; past that Apple keeps the document as a file, which this does not write yet.
const MOST_INLINE: usize = 700 * 1024;

/// Refuse a document too large to send inline, before anything is sent — the text stays on
/// screen, where a failed request midway would have left it in doubt.
fn fits_inline(document: &str) -> Result<()> {
    if document.len() > MOST_INLINE {
        return Err(Error::Protocol(
            "This note is longer than Quince can save to iCloud yet. Nothing was changed.".into(),
        ));
    }
    Ok(())
}

/// Marks a record whose text was fetched from its `TextDataAsset` rather than read inline.
const FROM_ASSET: &str = "_QuinceTextFromAsset";

/// Fetch a long note's text from the file Apple keeps it in.
///
/// Measured on the account: a note of about a million characters has no `TextDataEncrypted`
/// at all. The phone moved its document into `TextDataAsset`, a downloadable file holding the
/// same gzipped document, and a note read only from the inline field came out empty and
/// read-only. The file is fetched here and put where the inline text would be, so everything
/// after reads it the same way, and the record is marked so nothing tries to write it back
/// inline. A failed download leaves the note as it was: unread, never emptied.
/// One record's long text, fetched and put in place — the pass does many of these at once.
#[cfg(test)]
async fn read_asset_body(http: &reqwest::Client, record: &mut Value) {
    if let Some(url) = asset_url(record) {
        if let Some(bytes) = download(http, &url).await {
            put_asset_body(record, &bytes);
        }
    }
}

fn asset_url(record: &Value) -> Option<String> {
    if field::record_type(record).as_deref() != Some("Note")
        || record.pointer("/fields/TextDataEncrypted").is_some()
    {
        return None;
    }
    record
        .pointer("/fields/TextDataAsset/value/downloadURL")
        .and_then(Value::as_str)
        .map(|u| u.replace("${f}", "note"))
}

async fn download(http: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.bytes().await.ok().map(|b| b.to_vec())
}

fn put_asset_body(record: &mut Value, bytes: &[u8]) {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    if let Some(fields) = record.get_mut("fields").and_then(Value::as_object_mut) {
        fields.insert(
            "TextDataEncrypted".into(),
            serde_json::json!({ "type": "ENCRYPTED_BYTES", "value": B64.encode(bytes) }),
        );
        fields.insert(FROM_ASSET.into(), Value::Bool(true));
    }
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
    // Folders Apple has deleted: removed outright, or — as icloud.com's own delete leaves
    // them on the way — flagged `Deleted`, which its folder list filters out.
    let mut gone_folders: Vec<String> = Vec::new();
    let mut template: Option<String> = None;
    let mut notes: Vec<Note> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    // Deletion and pinning live on the per-user record, keyed by a reference to the note.
    let mut deleted_notes: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pins: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
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
                    // A tombstone does not say what it was: a folder's name is tried as a
                    // folder too — removing one this library does not have does nothing — or
                    // a folder deleted on a phone stayed in the sidebar here for good.
                    gone_folders.push(name.clone());
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
                    if template.is_none()
                        && record.pointer(&format!("/fields/{FROM_ASSET}")).is_none()
                    {
                        template = record
                            .pointer("/fields/TextDataEncrypted/value")
                            .and_then(Value::as_str)
                            .filter(|d| crate::ckdocument::has_merge_records(d))
                            .map(ToString::to_string);
                    }
                    notes.extend(note_from_record(record));
                }
            }
            "Folder" => {
                if removed || field::flag(record, "Deleted") {
                    gone_folders.extend(field::record_name(record));
                } else {
                    folders.extend(folder_from_record(record));
                }
            }
            // Per-user state. The note it refers to is what matters, not this record's name.
            "Note_UserSpecific" | "PasswordProtectedNote_UserSpecific" => {
                if let Some(target) = field::reference(record, "Note") {
                    if kind == "PasswordProtectedNote_UserSpecific" {
                        locked_notes.insert(target.clone());
                    }
                    if removed {
                        deleted_notes.insert(target);
                    } else {
                        // Pinned or not, as this record says: an unpin is as much news as a pin.
                        pins.insert(target, field::flag(record, "IsPinned"));
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
    // Pinning lives on its own record, and a pass carries only what changed: pinning a note
    // changes that record and not the note, and editing a note changes the note and not that
    // record. Read together only when both came in the same pass, a pin made here never showed
    // and an edited pinned note came back unpinned. So each half stands on its own: a note in
    // this pass keeps the pin already stored unless this pass says otherwise, and a pin in
    // this pass reaches its note whether or not the note came too.
    let stored = if incremental {
        library.note_flags()?
    } else {
        std::collections::HashMap::new()
    };
    for note in &mut notes {
        let (was_pinned, was_locked) = stored.get(&note.guid).copied().unwrap_or((false, false));
        note.pinned = pins.get(&note.guid).copied().unwrap_or(was_pinned);
        // Locked when Apple says so in this pass, or when it was and still has no text to read.
        // `|| was_locked` alone meant a note unlocked on a phone stayed locked here for good.
        note.locked = locked_notes.contains(&note.guid) || (was_locked && note.body.is_none());
    }
    let in_pass: std::collections::HashSet<String> = notes.iter().map(|n| n.guid.clone()).collect();

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
    if !gone_folders.is_empty() {
        library.remove_note_folders(&gone_folders)?;
    }
    for (guid, pinned) in &pins {
        if !in_pass.contains(guid) {
            library.set_note_pinned(guid, *pinned)?;
        }
    }
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

/// Apple's own rule for a note's title and its preview: the first line with anything on it
/// is the title, and the next such line is the snippet.
///
/// Measured on the account: a note whose text opens with a blank line was given, by the
/// phone, the title of its first line with words on it ("Your iCloud") and the next line as
/// its preview. This had taken the first line as written, blank or not — so such a note was
/// "Untitled" here and titled on the phone, and an empty title was written for it. Deriving
/// both the phone's way is what keeps a note saved here looking, in the list on a phone,
/// exactly like one typed there.
#[must_use]
pub fn title_and_snippet(body: &str) -> (String, String) {
    // The first line with words on it, as the phone takes it — not the first line that is
    // merely not blank. A pasted note opening with a rule of box-drawing characters
    // (`━━━━`) was sent with that rule as its title, and the phone showed none.
    let mut lines = body
        .lines()
        .map(str::trim)
        .filter(|l| l.chars().any(char::is_alphanumeric));
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
    edit_cloudkit_from(http, ckdatabasews_root, params, note, None, body)
        .await
        .map(|_| ())
}

/// Save `body` over a note that read `base` when the editing began, and answer with the text
/// Apple now holds.
///
/// If the note has changed on another device since — Apple's text is no longer `base` — the
/// two edits are combined when they touch different parts of the note, as two people
/// editing apart would expect, and refused when they touch the same part. Writing `body`
/// regardless replaced the other device's words with ours and recorded them as deleted, so
/// they went from every device. The write itself is exact: a conflict at that moment reads
/// the note again and decides again, never resending a document built from the old one.
pub async fn edit_cloudkit_from(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    base: Option<&str>,
    body: &str,
) -> Result<String> {
    edit_rich_cloudkit_from(http, ckdatabasews_root, params, note, base, body, None).await
}

/// [`edit_cloudkit_from`], with the note's formatting as an editor shows it: `runs` describe
/// `body`, and are laid over the text Apple ends up holding — moved along with another
/// device's edit when one is merged in.
pub async fn edit_rich_cloudkit_from(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    base: Option<&str>,
    body: &str,
    runs: Option<&[crate::ckdocument::Run]>,
) -> Result<String> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    for _ in 0..3 {
        let current = client
            .lookup_record(ZONE, &note.guid)
            .await?
            .ok_or_else(|| Error::Refused("Apple no longer has that note.".into()))?;
        let encoded = current
            .pointer("/fields/TextDataEncrypted/value")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Refused("That note has no body Apple will let us read.".into())
            })?;
        let theirs = crate::ckdocument::text_from_base64(encoded).unwrap_or_default();
        let target = match base {
            Some(base) if base != theirs => merge_edits(base, body, &theirs).ok_or_else(|| {
                Error::Refused(
                    "This note was changed on another device while you were writing, in the same place. Nothing was saved; your text is still here."
                        .into(),
                )
            })?,
            _ => body.to_string(),
        };
        let rewritten = match runs {
            Some(runs) => {
                let laid = if target == body { runs.to_vec() } else { crate::ckdocument::carry_runs(body, runs, &target) };
                crate::ckdocument::replace_rich_in_base64(encoded, &target, &laid)
            }
            None => crate::ckdocument::replace_text_in_base64(encoded, &target),
        }
        .ok_or_else(|| {
            Error::Refused(
                "Quince can’t safely save changes to this note yet, so nothing was changed. Editing it on an Apple device is safe."
                    .into(),
            )
        })?;
        fits_inline(&rewritten)?;
        let (title, snippet) = title_and_snippet(&target);
        let Some(tag) = current.get("recordChangeTag").and_then(Value::as_str) else {
            return Err(Error::Refused(
                "Apple did not say which version of the note it holds.".into(),
            ));
        };
        match client
            .update_record_exact(
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
            .await
        {
            Ok(_) => return Ok(target),
            Err(Error::Conflict) => {} // changed under us: read it again and decide again
            Err(e) => return Err(e),
        }
    }
    Err(Error::Refused(
        "This note keeps changing on another device, so nothing was saved yet; your text is still here.".into(),
    ))
}

/// One edit of `base`, character by character: which of its characters it removed, and what
/// it put in each gap between them (gap `i` is just before character `i`).
struct Script {
    removed: Vec<bool>,
    added: Vec<Vec<char>>,
}

/// The edit that turns `base` into `other`, as removals and additions. The unchanged start
/// and end are matched directly; the stretch between by longest common subsequence, so an
/// edit in two places reads as two edits — or, when that stretch is too large to compare
/// cheaply, as one replacement of it.
fn script(base: &[char], other: &[char]) -> Script {
    let mut removed = vec![false; base.len()];
    let mut added = vec![Vec::new(); base.len() + 1];
    let pre = base.iter().zip(other).take_while(|(a, b)| a == b).count();
    let max_suf = base.len().min(other.len()) - pre;
    let suf = base
        .iter()
        .rev()
        .zip(other.iter().rev())
        .take(max_suf)
        .take_while(|(a, b)| a == b)
        .count();
    let (b, o) = (&base[pre..base.len() - suf], &other[pre..other.len() - suf]);
    if b.len().saturating_mul(o.len()) <= 4_000_000 {
        // lcs[i][j]: the longest common subsequence of b[i..] and o[j..].
        let w = o.len() + 1;
        let mut lcs = vec![0u32; (b.len() + 1) * w];
        for i in (0..b.len()).rev() {
            for j in (0..o.len()).rev() {
                lcs[i * w + j] = if b[i] == o[j] {
                    lcs[(i + 1) * w + j + 1] + 1
                } else {
                    lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < b.len() || j < o.len() {
            if i < b.len() && j < o.len() && b[i] == o[j] {
                i += 1;
                j += 1;
            } else if j < o.len() && (i == b.len() || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j]) {
                added[pre + i].push(o[j]);
                j += 1;
            } else {
                removed[pre + i] = true;
                i += 1;
            }
        }
    } else {
        for r in &mut removed[pre..pre + b.len()] {
            *r = true;
        }
        added[pre + b.len()] = o.to_vec();
    }
    Script { removed, added }
}

/// Combine two edits of `base` — `mine` and `theirs` — the way Notes' own merge does: nothing
/// either side wrote is lost. What either removed goes; what either added stays, in place;
/// two additions at the same point are both kept, the other device's first, and an addition
/// both made is kept once.
///
/// It used to refuse when the two touched the same place — and a refusal was a save that did
/// not happen, with "changed on another device" over a note nobody else was really editing.
pub fn merge_edits(base: &str, mine: &str, theirs: &str) -> Option<String> {
    if mine == theirs || base == theirs {
        return Some(mine.to_string());
    }
    if base == mine {
        return Some(theirs.to_string());
    }
    let b: Vec<char> = base.chars().collect();
    let m = script(&b, &mine.chars().collect::<Vec<_>>());
    let t = script(&b, &theirs.chars().collect::<Vec<_>>());
    let mut out = String::with_capacity(mine.len().max(theirs.len()) + 16);
    for gap in 0..=b.len() {
        let (ta, ma) = (&t.added[gap], &m.added[gap]);
        out.extend(ta.iter());
        if ma != ta {
            out.extend(ma.iter());
        }
        if gap < b.len() && !m.removed[gap] && !t.removed[gap] {
            out.push(b[gap]);
        }
    }
    Some(out)
}

/// Set one paragraph of a note — a style from the Aa menu, or a checklist tick — written
/// over what Apple holds now, with the text and everything else in the document unchanged.
pub async fn set_paragraph_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    start: u64,
    change: crate::ckdocument::ParagraphChange,
    expect: Option<&str>,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let current = client
        .lookup_record(ZONE, &note.guid)
        .await?
        .ok_or_else(|| Error::Refused("Apple no longer has that note.".into()))?;
    let encoded = current
        .pointer("/fields/TextDataEncrypted/value")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Refused("That note has no body Apple will let us read.".into()))?;
    let rewritten = crate::ckdocument::set_paragraph_in_base64(encoded, start, change, expect).ok_or_else(|| {
        Error::Refused("That line has changed since it was shown, so nothing was changed. Try again in a moment.".into())
    })?;
    fits_inline(&rewritten)?;
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
    fits_inline(&document)?;

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
    // Filed as a move files it — `Folders`, which the phone reads, beside the older `Folder`
    // — and in Apple's own "Notes" when no folder is chosen, since Apple has no "none".
    // `Folder` alone left a note made here in a different folder on the phone than here.
    if let Some(filing) = filed_in(&client, folder_guid.unwrap_or(DEFAULT_FOLDER)).as_object() {
        for key in ["Folder", "Folders", "FoldersModificationDate"] {
            if let Some(v) = filing.get(key) {
                fields[key] = v.clone();
            }
        }
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

/// A folder's title as icloud.com writes it: the UTF-8 bytes, base64, nothing else
/// (`FolderEncodings` in its bundle) — not the gzip a note's title is sent in.
fn plain_title(title: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(title.as_bytes())
}

/// Make a folder, as icloud.com's New Folder does: a `Folder` record with its title (plain
/// UTF-8, base64 — "encrypted" in name only) and, when it sits inside another, a
/// `ParentFolder` reference. Returns the record name, a client-made UUID as icloud.com uses.
pub async fn create_folder_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    title: &str,
    parent: Option<&str>,
) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Refused("A folder needs a name.".into()));
    }
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let name = uuid::Uuid::new_v4().to_string();
    // As every folder on a real account carries it, Apple's own two included.
    let mut fields = serde_json::json!({
        "TitleEncrypted": { "type": "ENCRYPTED_BYTES", "value": plain_title(title) },
        "MinimumSupportedNotesVersion": { "type": "INT64", "value": 0 },
    });
    if let Some(parent) = parent {
        fields["ParentFolder"] = client.reference_to(ZONE, parent);
    }
    client.create_record(ZONE, &name, "Folder", fields).await?;
    Ok(name)
}

/// Rename a folder: its title, written over what Apple holds now. Apple's own two cannot be.
pub async fn rename_folder_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    title: &str,
) -> Result<()> {
    let title = title.trim();
    if guid == DEFAULT_FOLDER || guid == quince_core::notes::TRASH_FOLDER {
        return Err(Error::Refused(
            "Apple's own folders keep their names.".into(),
        ));
    }
    if title.is_empty() {
        return Err(Error::Refused("A folder needs a name.".into()));
    }
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let current = client
        .lookup_record(ZONE, guid)
        .await?
        .ok_or_else(|| Error::Refused("That folder is no longer in iCloud.".into()))?;
    let tag = current.get("recordChangeTag").and_then(Value::as_str);
    client
        .update_record(
            ZONE,
            guid,
            "Folder",
            tag,
            serde_json::json!({
                "TitleEncrypted": { "type": "ENCRYPTED_BYTES", "value": plain_title(title) },
                "TitleModificationDate": { "type": "TIMESTAMP", "value": now_millis() },
            }),
        )
        .await?;
    Ok(())
}

/// Delete a folder as icloud.com does: every note in it — and in every folder inside it —
/// goes to Recently Deleted first, then the folder records go, deepest first. The folder
/// itself has to be removed outright: measured on a real account, CloudKit refuses a
/// `Deleted` flag on a folder ("Field Deleted not found in Folder"), and icloud.com's own
/// delete ends the same way. Nothing a person wrote is in a folder record — its notes are
/// all safe in Recently Deleted before it goes — and a failure part-way loses nothing.
pub async fn delete_folder_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    all: &[NoteFolder],
    notes: &[Note],
    guid: &str,
) -> Result<()> {
    if guid == DEFAULT_FOLDER || guid == quince_core::notes::TRASH_FOLDER {
        return Err(Error::Refused(
            "Apple's own folders can't be deleted.".into(),
        ));
    }
    // This folder, then every folder under it, deepest last.
    let mut doomed = vec![guid.to_string()];
    let mut i = 0;
    while i < doomed.len() {
        let parent = doomed[i].clone();
        doomed.extend(
            all.iter()
                .filter(|f| f.parent_guid.as_deref() == Some(parent.as_str()))
                .map(|f| f.guid.clone()),
        );
        i += 1;
    }
    for note in notes.iter().filter(|n| {
        n.folder_guid
            .as_deref()
            .is_some_and(|f| doomed.iter().any(|d| d == f))
    }) {
        move_cloudkit(
            http,
            ckdatabasews_root,
            params.clone(),
            note,
            Some(quince_core::notes::TRASH_FOLDER),
        )
        .await?;
    }
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    for folder in doomed.iter().rev() {
        if client.lookup_record(ZONE, folder).await?.is_some() {
            client.delete_record(ZONE, folder).await?;
        }
    }
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
/// The name of the record that holds one person's state for a note — whether it is pinned.
///
/// Measured on the account: `Note_UserSpecific::<owner>/Private::Notes::<owner>::<note>`,
/// where the owner is the zone's. Apple validates it, so it is built exactly, never guessed.
fn user_specific_name(owner: &str, note_guid: &str) -> String {
    format!("Note_UserSpecific::{owner}/Private::Notes::{owner}::{note_guid}")
}

/// Pin a note, or unpin it, the way the phone does.
///
/// Pinning is not on the note: it is `IsPinned` on the per-user record beside it. That
/// record is updated where Apple already made one, and made — under the name Apple would
/// give it — for a note that has none yet. The note itself is not touched.
pub async fn pin_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    note: &Note,
    pinned: bool,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record = client
        .lookup_record(ZONE, &note.guid)
        .await?
        .ok_or_else(|| Error::Protocol("that note is no longer in iCloud".into()))?;
    let owner = record
        .pointer("/zoneID/ownerRecordName")
        .or_else(|| record.pointer("/created/userRecordName"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("iCloud did not say whose note this is".into()))?
        .to_string();
    let name = user_specific_name(&owner, &note.guid);
    let flag = serde_json::json!({ "IsPinned": { "type": "INT64", "value": i64::from(pinned) } });

    if let Some(existing) = client.lookup_record(ZONE, &name).await? {
        let tag = existing.get("recordChangeTag").and_then(Value::as_str);
        client
            .update_record(ZONE, &name, "Note_UserSpecific", tag, flag)
            .await?;
    } else {
        let mut fields = flag;
        fields["Note"] = client.reference_to(ZONE, &note.guid);
        fields["Note"]["value"]["action"] = Value::from("NONE");
        fields["Folder"] =
            client.reference_to(ZONE, note.folder_guid.as_deref().unwrap_or(DEFAULT_FOLDER));
        fields["Folder"]["value"]["action"] = Value::from("NONE");
        fields["Deleted"] = serde_json::json!({ "type": "INT64", "value": 0 });
        fields["IsSystemPaper"] = serde_json::json!({ "type": "INT64", "value": 0 });
        client
            .create_record(ZONE, &name, "Note_UserSpecific", fields)
            .await?;
    }
    Ok(())
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

    #[test]
    fn a_title_is_the_first_line_with_words_on_it() {
        let pasted = "━━━━━━━━━━━━━━━━\n\n𝗨𝗡𝗜𝗧 𝟭: 𝗜𝗡𝗧𝗥𝗢\n•  ──\nQuantum states";
        assert_eq!(
            title_and_snippet(pasted),
            ("𝗨𝗡𝗜𝗧 𝟭: 𝗜𝗡𝗧𝗥𝗢".to_string(), "Quantum states".to_string())
        );
    }

    #[test]
    fn a_notes_own_record_is_named_as_apple_names_it() {
        // Copied from a pinned note on the account.
        assert_eq!(
            user_specific_name(
                "_909a7c35ff0feea469084c60822e0cfd",
                "C454914D-6726-4216-ADDC-E441B9BB8952"
            ),
            "Note_UserSpecific::_909a7c35ff0feea469084c60822e0cfd/Private::Notes::\
             _909a7c35ff0feea469084c60822e0cfd::C454914D-6726-4216-ADDC-E441B9BB8952"
        );
    }
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

    /// A folder deleted elsewhere — flagged `Deleted` on its way out, as icloud.com does it —
    /// leaves the list, and a note still filed in it is kept, unfiled, not lost.
    /// A folder deleted on a phone arrives as a bare tombstone — a name, no type — and still
    /// leaves the sidebar.
    #[tokio::test]
    async fn a_folder_tombstone_removes_the_folder() {
        let mut library = Library::in_memory().unwrap();
        let first = serve(json!({ "zones": [{
            "records": [folder_record("F-gone", "Old")], "syncToken": "t1", "moreComing": false }] }))
        .await;
        sync_cloudkit(
            &reqwest::Client::new(),
            &first.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        let second = serve(json!({ "zones": [{
            "records": [{ "recordName": "F-gone", "deleted": true }], "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(
            &reqwest::Client::new(),
            &second.uri(),
            vec![],
            &mut library,
            Some("t1"),
        )
        .await
        .unwrap();
        assert!(library.note_folders().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_folder_flagged_deleted_leaves_the_list() {
        let mut library = Library::in_memory().unwrap();
        let first = serve(json!({ "zones": [{
            "records": [
                folder_record("F-keep", "Keep"),
                folder_record("F-gone", "Old"),
                note_record("Note/n1", "Kept note", Some("body"), "F-gone"),
            ],
            "syncToken": "t1", "moreComing": false }] }))
        .await;
        sync_cloudkit(
            &reqwest::Client::new(),
            &first.uri(),
            vec![],
            &mut library,
            None,
        )
        .await
        .unwrap();
        assert_eq!(library.note_folders().unwrap().len(), 2);

        let mut flagged = folder_record("F-gone", "Old");
        flagged["fields"]["Deleted"] = json!({ "type": "INT64", "value": 1 });
        let second = serve(json!({ "zones": [{
            "records": [flagged], "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(
            &reqwest::Client::new(),
            &second.uri(),
            vec![],
            &mut library,
            Some("t1"),
        )
        .await
        .unwrap();
        let titles: Vec<String> = library
            .note_folders()
            .unwrap()
            .into_iter()
            .map(|f| f.title)
            .collect();
        assert_eq!(titles, ["Keep"]);
        assert_eq!(
            library.notes_in(None).unwrap().len(),
            1,
            "the note survives"
        );
    }

    /// New Folder writes icloud.com's shape: a title, and a parent reference when nested.
    #[tokio::test]
    async fn a_new_folder_is_a_titled_record_inside_its_parent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "records": [{ "recordName": "x" }] })),
            )
            .mount(&server)
            .await;
        create_folder_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            " Recipes ",
            Some("F-parent"),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let record = serde_json::from_slice::<Value>(&requests[0].body).unwrap()["operations"][0]
            ["record"]
            .clone();
        assert_eq!(record["recordType"], "Folder");
        assert_eq!(record["fields"]["TitleEncrypted"]["value"], bare("Recipes"));
        assert_eq!(
            record["fields"]["ParentFolder"]["value"]["recordName"],
            "F-parent"
        );
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

    fn pin_record(pinned: i64) -> Value {
        json!({ "recordName": "NoteUS/1", "recordType": "Note_UserSpecific",
        "fields": {
            "IsPinned": { "type": "INT64", "value": pinned },
            "Note": { "type": "REFERENCE", "value": { "recordName": "Note/n1" } }
        }})
    }

    async fn pass(library: &mut Library, records: Vec<Value>, since: Option<&str>) {
        let server = serve(json!({ "zones": [{
            "records": records, "syncToken": "t2", "moreComing": false }] }))
        .await;
        sync_cloudkit(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            library,
            since,
        )
        .await
        .unwrap();
    }

    /// Pinning here writes the per-user record and not the note, so the pass after it carries
    /// that record alone. The pin was only ever applied to notes in the same pass, so it was
    /// dropped and the note never showed as pinned — and unpinning, likewise.
    #[tokio::test]
    async fn a_pin_reaches_its_note_without_the_note_in_the_pass() {
        let mut library = Library::in_memory().unwrap();
        pass(
            &mut library,
            vec![
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Curtains", Some("x"), "Folder/f1"),
            ],
            None,
        )
        .await;
        assert!(!library.notes_in(None).unwrap()[0].pinned);

        pass(&mut library, vec![pin_record(1)], Some("t1")).await;
        assert!(library.notes_in(None).unwrap()[0].pinned, "pinned");
        pass(&mut library, vec![pin_record(0)], Some("t2")).await;
        assert!(!library.notes_in(None).unwrap()[0].pinned, "and unpinned");
    }

    /// And the other way round: editing a pinned note changes the note and not its per-user
    /// record, and the edit came back unpinned.
    #[tokio::test]
    async fn an_edited_pinned_note_stays_pinned() {
        let mut library = Library::in_memory().unwrap();
        pass(
            &mut library,
            vec![
                folder_record("Folder/f1", "Notes"),
                note_record("Note/n1", "Curtains", Some("x"), "Folder/f1"),
                pin_record(1),
            ],
            None,
        )
        .await;
        pass(
            &mut library,
            vec![note_record(
                "Note/n1",
                "Curtains",
                Some("x and more"),
                "Folder/f1",
            )],
            Some("t1"),
        )
        .await;
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
            runs: None,
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

    /// The phone changed the very words being edited here: nothing is written, and the
    /// phone's version stands until the person decides.
    #[tokio::test]
    async fn an_edit_over_a_phone_edit_in_the_same_place_writes_nothing() {
        let server = server("Curtains\nmeasure the bay window twice").await;
        let err = edit_cloudkit_from(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            Some("Curtains\nmeasure the bay window"),
            "Curtains\nmeasure the bay window once",
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("changed on another device"),
            "{err}"
        );
        let wrote = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/records/modify"));
        assert!(!wrote, "nothing may be written over the phone's edit");
    }

    /// Apart, both survive: the phone's line and ours.
    #[tokio::test]
    async fn an_edit_apart_from_a_phone_edit_keeps_both() {
        let server = server("Curtains!\nmeasure the bay window").await;
        let saved = edit_cloudkit_from(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &note("Note/1"),
            Some("Curtains\nmeasure the bay window"),
            "Curtains\nmeasure the bay window\norder the rail",
        )
        .await
        .unwrap();
        assert_eq!(saved, "Curtains!\nmeasure the bay window\norder the rail");
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
        // What the phone did with a note that opens on a blank line: the first line with
        // words on it is the title, the next the preview.
        assert_eq!(
            title_and_snippet("\nYour iCloud\n\nPhotos\n\nCalendar"),
            ("Your iCloud".into(), "Photos".into())
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

    /// The phone names a note by its first line with words on it. Measured: a note opening on
    /// a blank line was titled "Your iCloud" by the phone, its first line with text. Taking
    /// the blank line — what this did — left it "Untitled" here and titled there.
    #[test]
    fn the_title_is_the_first_line_with_words_on_it() {
        assert_eq!(title_and_snippet("\nhello").0, "hello");
        assert_eq!(title_and_snippet("\n\n\nhello").0, "hello");
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

/// A long note, kept by Apple as a file rather than inline, is read from the file.
#[cfg(test)]
mod asset_body_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn a_note_kept_as_a_file_is_read_and_not_offered_for_editing() {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let server = MockServer::start().await;
        let document = B64
            .decode(crate::ckdocument::text_to_base64(
                "A very long note\nand more",
            ))
            .unwrap();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(document))
            .mount(&server)
            .await;
        let mut record = json!({
            "recordName": "N1", "recordType": "Note",
            "fields": { "TextDataAsset": { "type": "ASSETID", "value": {
                "downloadURL": format!("{}/B/abc/${{f}}?o=x", server.uri()) } } }
        });
        read_asset_body(&reqwest::Client::new(), &mut record).await;
        let note = note_from_record(&record).unwrap();
        assert_eq!(note.body.as_deref(), Some("A very long note\nand more"));
        assert!(
            !note.editable,
            "writing one back means sending a file, which is not built"
        );
    }

    #[test]
    fn a_document_too_large_to_send_inline_is_refused_before_sending() {
        assert!(fits_inline(&"x".repeat(MOST_INLINE + 1)).is_err());
        assert!(fits_inline(&"x".repeat(1000)).is_ok());
    }
}

/// A note with no stored title is named from its text, the phone's way.
#[cfg(test)]
mod merge_tests {
    use super::merge_edits;

    /// Edits in different places both survive — the phone's added line is not erased by
    /// a save made here from before it.
    #[test]
    fn edits_apart_are_combined() {
        let base = "Groceries\nmilk\neggs";
        let mine = "Groceries\nmilk\neggs\nbread";
        let theirs = "Groceries!\nmilk\neggs";
        assert_eq!(
            merge_edits(base, mine, theirs).as_deref(),
            Some("Groceries!\nmilk\neggs\nbread")
        );
    }

    /// Edits in the same place both survive, as Notes merges them: nothing typed is lost.
    #[test]
    fn edits_in_one_place_both_survive() {
        assert_eq!(
            merge_edits("a b c", "a X c", "a Y c").as_deref(),
            Some("a YX c")
        );
        assert_eq!(
            merge_edits("ab", "aXb", "aYb").as_deref(),
            Some("aYXb"),
            "two inserts at one point"
        );
        // Both typed at the end: the phone's line, then this one's.
        assert_eq!(
            merge_edits("Tea filter\n", "Tea filter\nSpoon", "Tea filter\nKnife").as_deref(),
            Some("Tea filter\nKnifeSpoon")
        );
        // One deleted a word the other edited beside: the deletion and the edit both hold.
        assert_eq!(
            merge_edits("one two three", "one three", "one two three!").as_deref(),
            Some("one three!")
        );
    }

    /// Edits in two places on one side, and another on the other, all survive.
    #[test]
    fn several_edits_on_each_side_combine() {
        let base = "milk\neggs\nbread\njam";
        let mine = "milk!\neggs\nbread\njam?";
        let theirs = "milk\neggs\nbutter\njam";
        assert_eq!(
            merge_edits(base, mine, theirs).as_deref(),
            Some("milk!\neggs\nbutter\njam?")
        );
    }

    #[test]
    fn the_same_edit_or_none_is_trivial() {
        assert_eq!(merge_edits("a", "ab", "ab").as_deref(), Some("ab"));
        assert_eq!(merge_edits("a", "a", "ab").as_deref(), Some("ab"));
        assert_eq!(merge_edits("a", "ab", "a").as_deref(), Some("ab"));
    }

    #[test]
    fn text_beyond_ascii_is_cut_by_character() {
        assert_eq!(merge_edits("ü 𝐔", "ü 𝐔 x", "Ü 𝐔").as_deref(), Some("Ü 𝐔 x"));
    }
}

#[cfg(test)]
mod derived_title_tests {
    use super::*;
    use serde_json::json;

    fn record(text: Option<&str>) -> Value {
        let mut fields = json!({});
        if let Some(t) = text {
            fields["TextDataEncrypted"] =
                json!({ "type": "ENCRYPTED_BYTES", "value": crate::ckdocument::text_to_base64(t) });
        }
        json!({ "recordName": "N", "recordType": "Note", "fields": fields })
    }

    /// Measured: the phone left one note with no TitleEncrypted at all, its text "hi".
    #[test]
    fn a_missing_title_comes_from_the_text() {
        assert_eq!(
            note_from_record(&record(Some("\nhi\nthere")))
                .unwrap()
                .title,
            "hi"
        );
        assert_eq!(
            note_from_record(&record(Some(""))).unwrap().title,
            "New Note"
        );
        assert_eq!(note_from_record(&record(None)).unwrap().title, "New Note");
    }
}
