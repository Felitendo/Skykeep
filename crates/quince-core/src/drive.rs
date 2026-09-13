//! iCloud Drive in the local library.
//!
//! Apple exposes Drive as a tree addressed by `drivewsid`, an identifier that encodes both
//! the kind and the zone. Folders are fetched one level at a time, so the library holds
//! whatever has been visited rather than a full mirror.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// The root every Drive browse starts from.
pub const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub drivewsid: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub extension: Option<String>,
    /// `FOLDER`, `APP_LIBRARY` or `FILE`.
    pub kind: String,
    pub size: Option<i64>,
    pub child_count: Option<i64>,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub etag: Option<String>,
}

impl DriveItem {
    /// True for anything that can be opened to reveal more items.
    #[must_use]
    pub fn is_container(&self) -> bool {
        self.kind == "FOLDER" || self.kind == "APP_LIBRARY"
    }

    /// The name as a person reads it, with the extension Apple keeps separate.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self.extension.as_deref().filter(|e| !e.is_empty()) {
            Some(ext) if !self.is_container() => format!("{}.{}", self.name, ext),
            _ => self.name.clone(),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Replace the contents of one folder.
    ///
    /// Scoped to the folder rather than the whole tree: Drive is fetched a level at a time,
    /// so anything outside this folder is simply not part of this answer.
    pub fn put_drive_children(&mut self, parent_id: &str, items: &[DriveItem]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO drive_items (drivewsid, parent_id, name, extension, kind, size,
                                          child_count, created_at, modified_at, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(drivewsid) DO UPDATE SET
                   parent_id = excluded.parent_id, name = excluded.name,
                   extension = excluded.extension, kind = excluded.kind,
                   size = excluded.size, child_count = excluded.child_count,
                   created_at = excluded.created_at, modified_at = excluded.modified_at,
                   etag = excluded.etag, updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for item in items {
                stmt.execute(params![
                    item.drivewsid,
                    parent_id,
                    item.name,
                    item.extension,
                    item.kind,
                    item.size,
                    item.child_count,
                    item.created_at,
                    item.modified_at,
                    item.etag,
                    stamp
                ])?;
            }

            // Anything still filed under this folder that Apple no longer lists was moved or
            // deleted elsewhere.
            let keep: Vec<&str> = items.iter().map(|i| i.drivewsid.as_str()).collect();
            if keep.is_empty() {
                tx.execute(
                    "DELETE FROM drive_items WHERE parent_id = ?1",
                    params![parent_id],
                )?;
            } else {
                let placeholders = vec!["?"; keep.len()].join(",");
                let mut args: Vec<&str> = vec![parent_id];
                args.extend(keep);
                tx.execute(
                    &format!(
                        "DELETE FROM drive_items WHERE parent_id = ? AND drivewsid NOT IN ({placeholders})"
                    ),
                    rusqlite::params_from_iter(args),
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// What is inside a folder, folders first and then by name — how a file browser reads.
    pub fn drive_children(&self, parent_id: &str) -> Result<Vec<DriveItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT drivewsid, parent_id, name, extension, kind, size, child_count,
                    created_at, modified_at, etag
             FROM drive_items
             WHERE parent_id = ?1
             ORDER BY kind = 'FILE', lower(name)",
        )?;
        let rows = stmt.query_map(params![parent_id], |row| {
            Ok(DriveItem {
                drivewsid: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                extension: row.get(3)?,
                kind: row.get(4)?,
                size: row.get(5)?,
                child_count: row.get(6)?,
                created_at: row.get(7)?,
                modified_at: row.get(8)?,
                etag: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One item by its identifier, for building a breadcrumb trail.
    pub fn drive_item(&self, drivewsid: &str) -> Result<Option<DriveItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT drivewsid, parent_id, name, extension, kind, size, child_count,
                    created_at, modified_at, etag
             FROM drive_items WHERE drivewsid = ?1",
        )?;
        let mut rows = stmt.query_map(params![drivewsid], |row| {
            Ok(DriveItem {
                drivewsid: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                extension: row.get(3)?,
                kind: row.get(4)?,
                size: row.get(5)?,
                child_count: row.get(6)?,
                created_at: row.get(7)?,
                modified_at: row.get(8)?,
                etag: row.get(9)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str, kind: &str) -> DriveItem {
        DriveItem {
            drivewsid: id.into(),
            parent_id: None,
            name: name.into(),
            extension: if kind == "FILE" {
                Some("pdf".into())
            } else {
                None
            },
            kind: kind.into(),
            size: if kind == "FILE" { Some(1024) } else { None },
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
        }
    }

    #[test]
    fn folders_come_before_files_then_alphabetical() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            ROOT,
            &[
                item("f3", "notes", "FILE"),
                item("f1", "Zebra", "FOLDER"),
                item("f2", "apples", "FOLDER"),
                item("f4", "Archive", "FILE"),
            ],
        )
        .unwrap();

        let names: Vec<String> = lib
            .drive_children(ROOT)
            .unwrap()
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(names, vec!["apples", "Zebra", "Archive", "notes"]);
    }

    #[test]
    fn an_extension_shows_on_files_but_not_folders() {
        assert_eq!(item("a", "report", "FILE").display_name(), "report.pdf");
        assert_eq!(item("b", "Downloads", "FOLDER").display_name(), "Downloads");
    }

    #[test]
    fn app_libraries_are_containers_too() {
        assert!(item("a", "Shortcuts", "APP_LIBRARY").is_container());
        assert!(item("b", "Downloads", "FOLDER").is_container());
        assert!(!item("c", "report", "FILE").is_container());
    }

    /// Refreshing one folder must not disturb another; Drive is fetched a level at a time.
    #[test]
    fn refreshing_a_folder_leaves_other_folders_alone() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        lib.put_drive_children("a", &[item("a1", "inside", "FILE")])
            .unwrap();

        // Re-fetch the root; the child folder's contents must survive.
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        assert_eq!(lib.drive_children("a").unwrap().len(), 1);
    }

    #[test]
    fn an_item_removed_from_a_folder_disappears() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "One", "FILE"), item("b", "Two", "FILE")])
            .unwrap();
        lib.put_drive_children(ROOT, &[item("a", "One", "FILE")])
            .unwrap();
        assert_eq!(lib.drive_children(ROOT).unwrap().len(), 1);
    }

    #[test]
    fn an_item_can_be_looked_up_for_a_breadcrumb() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        let found = lib.drive_item("a").unwrap().unwrap();
        assert_eq!(found.name, "Downloads");
        assert_eq!(found.parent_id.as_deref(), Some(ROOT));
        assert!(lib.drive_item("nope").unwrap().is_none());
    }
}
