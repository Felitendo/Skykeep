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
    pub fn notes_in(&self, folder_guid: Option<&str>) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, folder_guid, title, snippet, body, locked, pinned,
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
                body: row.get(4)?,
                locked: row.get::<_, i64>(5)? != 0,
                pinned: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
                modified_at: row.get(8)?,
                editable: row.get::<_, i64>(9)? != 0,
                etag: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
