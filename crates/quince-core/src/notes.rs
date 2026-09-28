//! Notes and their folders in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteFolder {
    pub guid: String,
    pub title: String,
    pub sort_order: i64,
    /// The folder this one sits inside, where it sits inside one at all.
    #[serde(default)]
    pub parent_guid: Option<String>,
}

/// Apple's identifier for the folder deleted notes go to.
///
/// The record name, not the title: the title is translated, so matching on "Recently
/// Deleted" would work in English and quietly fail everywhere else.
pub const TRASH_FOLDER: &str = "TrashFolder-CloudKit";

impl NoteFolder {
    /// Whether this is where deleted notes wait before they are really gone.
    #[must_use]
    pub fn is_trash(&self) -> bool {
        self.guid == TRASH_FOLDER
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub guid: String,
    pub folder_guid: Option<String>,
    pub title: String,
    pub snippet: Option<String>,
    pub body: Option<String>,
    /// Locked by its owner, so Apple holds it encrypted and there is nothing to show.
    pub locked: bool,
    pub pinned: bool,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    /// Whether saving an edit would preserve the note.
    ///
    /// Apple's document carries version vectors and merge state around the text. Where that
    /// document can be reproduced byte for byte, an edit is a safe surgical swap; where it
    /// cannot, writing would flatten everything else in it, so the note stays read-only and
    /// the interface says why.
    #[serde(default = "yes")]
    pub editable: bool,
    pub etag: Option<String>,
}

fn yes() -> bool {
    true
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    pub fn put_note_folders(&mut self, folders: &[NoteFolder]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO note_folders (guid, title, sort_order, parent_guid, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, sort_order = excluded.sort_order,
                   parent_guid = excluded.parent_guid,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for f in folders {
                stmt.execute(params![f.guid, f.title, f.sort_order, f.parent_guid, stamp])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn put_notes(&mut self, notes: &[Note]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO notes (guid, folder_guid, title, snippet, body, locked, pinned,
                                    created_at, modified_at, editable, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(guid) DO UPDATE SET
                   folder_guid = excluded.folder_guid, title = excluded.title,
                   snippet = excluded.snippet, body = excluded.body,
                   locked = excluded.locked, pinned = excluded.pinned,
                   created_at = excluded.created_at, modified_at = excluded.modified_at,
                   editable = excluded.editable,
                   etag = excluded.etag, updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for n in notes {
                stmt.execute(params![
                    n.guid,
                    n.folder_guid,
                    n.title,
                    n.snippet,
                    n.body,
                    i64::from(n.locked),
                    i64::from(n.pinned),
                    n.created_at,
                    n.modified_at,
                    i64::from(n.editable),
                    n.etag,
                    stamp
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep only these notes, dropping everything else.
    ///
    /// The same floor a full reminder sync has: a tombstone arrives once, so a sync that
    /// consumed one before it was understood would leave a deleted note on screen forever —
    /// and saving one of those writes it back to iCloud.
    pub fn retain_notes(&mut self, keep: &[String]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let removed = {
            let mut known: std::collections::HashSet<&str> =
                keep.iter().map(String::as_str).collect();
            let mut stmt = tx.prepare("SELECT guid FROM notes")?;
            let all: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut delete = tx.prepare("DELETE FROM notes WHERE guid = ?1")?;
            let mut count = 0;
            for guid in &all {
                if !known.remove(guid.as_str()) {
                    delete.execute(params![guid])?;
                    count += 1;
                }
            }
            count
        };
        tx.commit()?;
        Ok(removed)
    }

    pub fn forget_notes(&mut self, note_guids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM notes WHERE guid = ?1")?;
            for guid in note_guids {
                stmt.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One note's text, whole.
    ///
    /// Fetched for the note being read rather than for every note in the list, which is what
    /// keeps opening a folder of five thousand cheap.
    pub fn note_body(&self, guid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body, title, snippet FROM notes WHERE guid = ?1")?;
        let mut rows = stmt.query(params![guid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let body: Option<String> = row.get(0)?;
        if body.is_some() {
            return Ok(body);
        }

        // No body stored. That is either a note with nothing in it, or one whose text could
        // not be read — and the two must not be confused, because an editor opened over the
        // second shows an empty box, and the first keystroke saves that emptiness over
        // whatever was really there. A five-line note became a single letter that way.
        //
        // A note with a name or a line of preview has something in it, so no body means it
        // was not read. Answering "not known" keeps the editor shut.
        let title: Option<String> = row.get(1)?;
        let snippet: Option<String> = row.get(2)?;
        let has_content = [title, snippet]
            .into_iter()
            .flatten()
            .any(|s| !s.trim().is_empty());
        Ok(if has_content {
            None
        } else {
            Some(String::new())
        })
    }

    pub fn note_folders(&self) -> Result<Vec<NoteFolder>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, sort_order, parent_guid
             FROM note_folders ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NoteFolder {
                guid: row.get(0)?,
                title: row.get(1)?,
                sort_order: row.get(2)?,
                parent_guid: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Notes in a folder, pinned first and then most recently changed — Apple's order.
    ///
    /// Passing `None` is the "All Notes" view, which in Apple's app means every note that
    /// has not been deleted. Including the deleted ones put them in both places at once.
    /// The notes in a folder, without their bodies.
    ///
    /// A list of notes shows a title, a line of preview and a date. Carrying every body along
    /// with that is what the plan calls an unbounded payload: five thousand ordinary notes
    /// come to nearly ten megabytes, and all of it crosses into the window on every refresh to
    /// draw text nobody reads. The body of the one note being read is fetched on its own.
    ///
    /// The preview is a truncation, so it is put in `snippet` — which is only ever displayed —
    /// and never in `body`, which is what an editor would save back. A body that came back
    /// short and got written would take the rest of the note with it.
    pub fn notes_in(&self, folder_guid: Option<&str>) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, folder_guid, title,
                    coalesce(snippet, substr(body, 1, 200)), locked, pinned,
                    created_at, modified_at, editable, etag
             FROM notes
             WHERE (?1 IS NULL AND (folder_guid IS NULL OR folder_guid != ?2))
                OR folder_guid = ?1
             ORDER BY pinned DESC, modified_at DESC, title",
        )?;
        let rows = stmt.query_map(params![folder_guid, TRASH_FOLDER], |row| {
            Ok(Note {
                guid: row.get(0)?,
                folder_guid: row.get(1)?,
                title: row.get(2)?,
                snippet: row.get(3)?,
                // Never a truncation. Absent, so nothing can save a short one back.
                body: None,
                locked: row.get::<_, i64>(4)? != 0,
                pinned: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                modified_at: row.get(7)?,
                editable: row.get::<_, i64>(8)? != 0,
                etag: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Notes whose title or text holds every word of `query`, in the list's own order.
    ///
    /// Matched here rather than in SQL, because SQLite folds case for ASCII alone: a search
    /// for "Café" would miss "café", and so would anything written in a script with case of
    /// its own. Every word has to appear, in any order, which is how Apple's search behaves.
    /// Recently Deleted is left out, as it is on the phone. Bodies are read to match against
    /// and never returned — the results cross into the window like any other list.
    pub fn search_notes(&self, query: &str) -> Result<Vec<Note>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT guid, folder_guid, title,
                    coalesce(snippet, substr(body, 1, 200)), locked, pinned,
                    created_at, modified_at, editable, etag, coalesce(body, '')
             FROM notes
             WHERE folder_guid IS NULL OR folder_guid != ?1
             ORDER BY pinned DESC, modified_at DESC, title",
        )?;
        let rows = stmt.query_map(params![TRASH_FOLDER], |row| {
            let body: String = row.get(10)?;
            Ok((
                Note {
                    guid: row.get(0)?,
                    folder_guid: row.get(1)?,
                    title: row.get(2)?,
                    snippet: row.get(3)?,
                    body: None,
                    locked: row.get::<_, i64>(4)? != 0,
                    pinned: row.get::<_, i64>(5)? != 0,
                    created_at: row.get(6)?,
                    modified_at: row.get(7)?,
                    editable: row.get::<_, i64>(8)? != 0,
                    etag: row.get(9)?,
                },
                body,
            ))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (note, body) = row?;
            let haystack = format!("{}\n{}", note.title, body).to_lowercase();
            if words.iter().all(|w| haystack.contains(w.as_str())) {
                found.push(note);
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn note(guid: &str, folder: &str, title: &str, body: &str) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: Some(folder.into()),
            title: title.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            editable: true,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(1),
            etag: None,
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[
            NoteFolder {
                guid: "F".into(),
                title: "Notes".into(),
                sort_order: 0,
                parent_guid: None,
            },
            NoteFolder {
                guid: TRASH_FOLDER.into(),
                title: "Recently Deleted".into(),
                sort_order: 1,
                parent_guid: None,
            },
        ])
        .unwrap();
        lib.put_notes(&[
            note("A", "F", "Curtains", "Bathroom accessories like mirrors"),
            note("B", "F", "Café list", "Flat white, then a croissant"),
            note("C", "F", "తెలుగు", "Notes written in Telugu"),
            note(
                "D",
                TRASH_FOLDER,
                "Curtains old",
                "deleted draft about mirrors",
            ),
        ])
        .unwrap();
        lib
    }

    fn found(lib: &Library, q: &str) -> Vec<String> {
        lib.search_notes(q)
            .unwrap()
            .into_iter()
            .map(|n| n.guid)
            .collect()
    }

    #[test]
    fn a_word_in_the_body_finds_the_note() {
        assert_eq!(found(&library(), "mirrors"), vec!["A"]);
    }

    /// SQLite folds ASCII only; this is the case that made matching live in Rust.
    #[test]
    fn case_is_folded_beyond_ascii() {
        assert_eq!(found(&library(), "CAFÉ"), vec!["B"]);
        assert_eq!(found(&library(), "తెలుగు"), vec!["C"]);
    }

    /// Every word must be there, in any order.
    #[test]
    fn every_word_has_to_appear() {
        assert_eq!(found(&library(), "croissant flat"), vec!["B"]);
        assert!(found(&library(), "croissant mirrors").is_empty());
    }

    /// Recently Deleted is not searched, as on the phone.
    #[test]
    fn deleted_notes_are_not_found() {
        assert!(!found(&library(), "draft").contains(&"D".to_string()));
    }

    #[test]
    fn nothing_typed_finds_nothing_and_nothing_carries_a_body() {
        assert!(found(&library(), "   ").is_empty());
        assert!(library()
            .search_notes("mirrors")
            .unwrap()
            .iter()
            .all(|n| n.body.is_none()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(guid: &str, title: &str) -> NoteFolder {
        NoteFolder {
            guid: guid.into(),
            title: title.into(),
            sort_order: 0,
            parent_guid: None,
        }
    }

    /// Apple Notes nests folders, and the link has to survive a round trip through the
    /// library or somebody's filing comes back as one flat list.
    #[test]
    fn a_folder_remembers_the_one_it_sits_inside() {
        let mut lib = Library::in_memory().unwrap();
        let mut inner = folder("f2", "Semester 5");
        inner.parent_guid = Some("f1".into());
        lib.put_note_folders(&[folder("f1", "University"), inner])
            .unwrap();

        let stored = lib.note_folders().unwrap();
        let nested = stored.iter().find(|f| f.guid == "f2").unwrap();
        assert_eq!(nested.parent_guid.as_deref(), Some("f1"));
        assert_eq!(
            stored.iter().find(|f| f.guid == "f1").unwrap().parent_guid,
            None,
            "a top-level folder sits inside nothing"
        );
    }

    /// A folder moved out to the top level must not keep its old parent.
    #[test]
    fn a_folder_moved_to_the_top_stops_naming_a_parent() {
        let mut lib = Library::in_memory().unwrap();
        let mut inner = folder("f2", "Semester 5");
        inner.parent_guid = Some("f1".into());
        lib.put_note_folders(&[folder("f1", "University"), inner.clone()])
            .unwrap();

        inner.parent_guid = None;
        lib.put_note_folders(&[folder("f1", "University"), inner])
            .unwrap();
        assert_eq!(
            lib.note_folders()
                .unwrap()
                .iter()
                .find(|f| f.guid == "f2")
                .unwrap()
                .parent_guid,
            None
        );
    }

    fn note(guid: &str, folder: Option<&str>, pinned: bool, modified: i64) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: folder.map(ToString::to_string),
            title: guid.into(),
            snippet: None,
            body: Some("body".into()),
            locked: false,
            pinned,
            created_at: Some(1),
            modified_at: Some(modified),
            editable: true,
            etag: None,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[folder("f1", "Notes"), folder("f2", "Recently Deleted")])
            .unwrap();
        lib
    }

    /// Apple pins to the top, then orders by most recently changed.
    #[test]
    fn pinned_notes_come_first_then_most_recent() {
        let mut lib = seeded();
        lib.put_notes(&[
            note("old", Some("f1"), false, 100),
            note("newest", Some("f1"), false, 300),
            note("pinned-old", Some("f1"), true, 50),
        ])
        .unwrap();

        let order: Vec<String> = lib
            .notes_in(Some("f1"))
            .unwrap()
            .into_iter()
            .map(|n| n.guid)
            .collect();
        assert_eq!(order, vec!["pinned-old", "newest", "old"]);
    }

    #[test]
    fn every_note_is_returned_when_no_folder_is_given() {
        let mut lib = seeded();
        lib.put_notes(&[
            note("a", Some("f1"), false, 1),
            note("b", Some("f2"), false, 2),
        ])
        .unwrap();
        assert_eq!(lib.notes_in(None).unwrap().len(), 2);
        assert_eq!(lib.notes_in(Some("f1")).unwrap().len(), 1);
    }

    /// A note whose folder disappears must survive, unfiled — losing the note would be worse.
    #[test]
    fn a_note_outlives_its_folder() {
        let mut lib = seeded();
        lib.put_notes(&[note("a", Some("f1"), false, 1)]).unwrap();
        lib.conn()
            .execute("DELETE FROM note_folders WHERE guid = 'f1'", [])
            .unwrap();

        let all = lib.notes_in(None).unwrap();
        assert_eq!(all.len(), 1, "the note must not go with the folder");
        assert!(all[0].folder_guid.is_none(), "it becomes unfiled");
    }

    #[test]
    fn re_syncing_updates_rather_than_duplicates() {
        let mut lib = seeded();
        lib.put_notes(&[note("a", Some("f1"), false, 1)]).unwrap();
        let mut changed = note("a", Some("f1"), false, 2);
        changed.title = "Renamed".into();
        lib.put_notes(&[changed]).unwrap();

        let all = lib.notes_in(None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Renamed");
    }

    #[test]
    fn a_locked_note_is_recorded_as_locked() {
        let mut lib = seeded();
        let mut locked = note("secret", Some("f1"), false, 1);
        locked.locked = true;
        locked.body = None;
        lib.put_notes(&[locked]).unwrap();
        assert!(lib.notes_in(None).unwrap()[0].locked);
    }
}

/// Reading a lot of notes without carrying every word of them.
#[cfg(test)]
mod body_tests {
    use super::*;

    fn stored() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[NoteFolder {
            guid: "F1".into(),
            title: "Notes".into(),
            sort_order: 0,
            parent_guid: None,
        }])
        .unwrap();
        lib.put_notes(&[
            Note {
                guid: "N1".into(),
                folder_guid: Some("F1".into()),
                title: "Long one".into(),
                snippet: None,
                body: Some("x".repeat(5_000)),
                locked: false,
                editable: true,
                pinned: false,
                created_at: Some(1),
                modified_at: Some(2),
                etag: None,
            },
            Note {
                guid: "N2".into(),
                // A note with nothing in it has no name either — a name with no body means
                // the text was not read, which is a different thing and answered differently.
                folder_guid: Some("F1".into()),
                title: String::new(),
                snippet: None,
                body: None,
                locked: false,
                editable: true,
                pinned: false,
                created_at: Some(1),
                modified_at: Some(1),
                etag: None,
            },
        ])
        .unwrap();
        lib
    }

    /// The danger this guards: a body that came back short and got saved would take the rest
    /// of the note with it. The list carries no body at all, so nothing can.
    #[test]
    fn a_listed_note_carries_no_body_to_save_back() {
        for note in stored().notes_in(None).unwrap() {
            assert_eq!(
                note.body, None,
                "{} must not carry a body into a list",
                note.title
            );
        }
    }

    /// A preview still has to appear, and it is a truncation — so it goes where only display
    /// reads it.
    #[test]
    fn a_note_with_no_snippet_still_previews() {
        let listed = stored().notes_in(None).unwrap();
        let long = listed.iter().find(|n| n.guid == "N1").unwrap();
        assert_eq!(long.snippet.as_deref().map(str::len), Some(200));
    }

    /// The note being read gets its text whole.
    #[test]
    fn the_body_is_fetched_in_full_for_one_note() {
        let lib = stored();
        assert_eq!(lib.note_body("N1").unwrap().map(|b| b.len()), Some(5_000));
    }

    /// A note with nothing in it is not the same as no such note, and the difference decides
    /// whether an editor may open at all.
    #[test]
    fn an_empty_note_and_a_missing_one_are_told_apart() {
        let lib = stored();
        assert_eq!(lib.note_body("N2").unwrap(), Some(String::new()));
        assert_eq!(lib.note_body("nothing-like-this").unwrap(), None);
    }
}

/// Telling an empty note from one whose text could not be read.
#[cfg(test)]
mod unreadable_tests {
    use super::*;

    fn note(guid: &str, title: &str, snippet: Option<&str>, body: Option<&str>) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: None,
            title: title.into(),
            snippet: snippet.map(ToString::to_string),
            body: body.map(ToString::to_string),
            locked: false,
            editable: true,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(1),
            etag: None,
        }
    }

    /// The difference decides whether an editor opens. Over a note whose text was not read,
    /// an empty box is shown and the first keystroke saves that emptiness over whatever was
    /// really there — which is how a five-line note became a single letter.
    #[test]
    fn a_note_whose_text_was_not_read_answers_that_it_is_not_known() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_notes(&[
            note("EMPTY", "", None, None),
            note("UNREAD", "Shopping list", Some("Milk"), None),
            note(
                "READ",
                "Shopping list",
                Some("Milk"),
                Some("Shopping list\nMilk"),
            ),
        ])
        .unwrap();

        assert_eq!(
            lib.note_body("EMPTY").unwrap(),
            Some(String::new()),
            "a note with nothing in it is empty, and may be typed into"
        );
        assert_eq!(
            lib.note_body("UNREAD").unwrap(),
            None,
            "a note with a name but no text was not read, and must not be typed into"
        );
        assert_eq!(
            lib.note_body("READ").unwrap().as_deref(),
            Some("Shopping list\nMilk")
        );
        assert_eq!(lib.note_body("no-such-note").unwrap(), None);
    }
}
