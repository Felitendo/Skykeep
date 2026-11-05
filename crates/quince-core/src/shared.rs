//! Shared Albums: what each album is, and its photos, kept apart from the library's own (see
//! migration 51). An album is replaced whole when Apple says it changed, in one transaction:
//! never half an old read and half a new one.

use crate::photos::Photo;
use crate::{Library, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// One shared album, as the window lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedAlbum {
    /// Its zone in Apple's Photos container: what it is known by.
    pub zone: String,
    /// `private` for an album this account made, `shared` for one it was invited to.
    pub database: String,
    /// The account that owns it, for one shared with this account.
    pub owner: Option<String>,
    pub title: Option<String>,
    /// When Apple ends it, unix seconds.
    pub expires_at: Option<i64>,
    /// How many photos it holds.
    #[serde(default)]
    pub count: i64,
    /// Its newest photo's id, for the album's picture.
    #[serde(default)]
    pub cover: Option<String>,
}

impl SharedAlbum {
    /// Made by this account, rather than joined.
    #[must_use]
    pub fn owned(&self) -> bool {
        self.database == "private"
    }
}

/// Where a shared photo lives, to ask Apple for its pictures again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedPlace {
    pub zone: String,
    pub database: String,
    pub owner: Option<String>,
}

/// A photo that wouldn't turn to or from its stored JSON: reported as the library's own error.
fn json_error(e: serde_json::Error) -> crate::Error {
    crate::Error::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

impl Library {
    /// An album stored as Apple has it now: what it is, its photos (all of them — any it held
    /// before and no longer does are gone), and how far its change feed was read.
    pub fn put_shared_album(
        &mut self,
        album: &SharedAlbum,
        photos: &[Photo],
        token: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        tx.execute(
            "INSERT INTO shared_albums (zone, database, owner, title, expires_at, token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(zone) DO UPDATE SET database = excluded.database,
               owner = excluded.owner, title = excluded.title,
               expires_at = excluded.expires_at, token = excluded.token",
            params![
                album.zone,
                album.database,
                album.owner,
                album.title,
                album.expires_at,
                token
            ],
        )?;
        tx.execute(
            "DELETE FROM shared_album_photos WHERE zone = ?1",
            params![album.zone],
        )?;
        {
            let mut put = tx.prepare(
                "INSERT INTO shared_album_photos (zone, id, taken_at, photo) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(zone, id) DO UPDATE SET taken_at = excluded.taken_at, photo = excluded.photo",
            )?;
            for p in photos {
                let json = serde_json::to_string(p).map_err(json_error)?;
                put.execute(params![album.zone, p.id, p.taken_at, json])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// How far an album's change feed was read, when it has been.
    pub fn shared_album_token(&self, zone: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT token FROM shared_albums WHERE zone = ?1",
                params![zone],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Every shared album but `kept`'s let go of, with their photos: albums deleted, left or
    /// expired on Apple's side.
    pub fn keep_shared_albums(&mut self, kept: &[&str]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM shared_albums WHERE zone NOT IN (SELECT value FROM json_each(?1))",
            params![crate::id_list(kept)],
        )?;
        Ok(())
    }

    /// Every shared album, newest made first by title for a steady order: with how many photos
    /// each holds and its newest photo.
    pub fn shared_albums(&self) -> Result<Vec<SharedAlbum>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.zone, a.database, a.owner, a.title, a.expires_at,
                    (SELECT COUNT(*) FROM shared_album_photos p WHERE p.zone = a.zone),
                    (SELECT p.id FROM shared_album_photos p WHERE p.zone = a.zone
                      ORDER BY p.taken_at DESC, p.id LIMIT 1)
               FROM shared_albums a
              ORDER BY a.database, COALESCE(a.title, '') COLLATE NOCASE, a.zone",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SharedAlbum {
                zone: r.get(0)?,
                database: r.get(1)?,
                owner: r.get(2)?,
                title: r.get(3)?,
                expires_at: r.get(4)?,
                count: r.get(5)?,
                cover: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// An album's photos, newest first.
    pub fn shared_album_photos(&self, zone: &str) -> Result<Vec<Photo>> {
        let mut stmt = self.conn.prepare(
            "SELECT photo FROM shared_album_photos WHERE zone = ?1 ORDER BY taken_at DESC, id",
        )?;
        let rows = stmt.query_map(params![zone], |r| r.get::<_, String>(0))?;
        rows.map(|json| serde_json::from_str(&json?).map_err(json_error))
            .collect()
    }

    /// Every shared album's photos as the grid lays them out (for their pictures to be made
    /// ahead), newest first.
    pub fn shared_photo_shapes(&self) -> Result<Vec<crate::photos::PhotoShape>> {
        let mut stmt = self
            .conn
            .prepare("SELECT photo FROM shared_album_photos ORDER BY taken_at DESC, id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for json in rows {
            let p: Photo = serde_json::from_str(&json?).map_err(json_error)?;
            out.push(crate::photos::PhotoShape {
                id: p.id,
                width: p.width,
                height: p.height,
                kind: p.kind,
                taken_at: p.taken_at,
            });
        }
        Ok(out)
    }

    /// A photo to look at, copy or save: the library's own, or one in a shared album.
    pub fn photo_to_view(&self, id: &str) -> Result<Option<Photo>> {
        if let Some(p) = self.photo(id)? {
            return Ok(Some(p));
        }
        Ok(self.shared_photo(id)?.map(|(p, _)| p))
    }

    /// A shared photo by its id, and where it lives: for its pictures.
    pub fn shared_photo(&self, id: &str) -> Result<Option<(Photo, SharedPlace)>> {
        let row: Option<(String, String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT p.photo, a.zone, a.database, a.owner
                   FROM shared_album_photos p JOIN shared_albums a ON a.zone = p.zone
                  WHERE p.id = ?1 LIMIT 1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((json, zone, database, owner)) = row else {
            return Ok(None);
        };
        let photo = serde_json::from_str(&json).map_err(json_error)?;
        Ok(Some((
            photo,
            SharedPlace {
                zone,
                database,
                owner,
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::{Kind, View};

    fn photo(id: &str, taken: i64) -> Photo {
        Photo {
            id: id.into(),
            master: format!("m-{id}"),
            taken_at: taken,
            added_at: Some(taken),
            kind: Kind::Image,
            width: 1200,
            height: 2600,
            duration: None,
            filename: Some(format!("{id}.JPG")),
            size: None,
            uti: Some("public.jpeg".into()),
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            subtype: 0,
            change_tag: None,
            expunge_at: None,
            title: None,
            caption: None,
            rating: 0,
            keywords: Vec::new(),
            latitude: None,
            longitude: None,
            rendered: 0,
            burst: None,
            burst_pick: false,
            burst_count: 0,
            live_size: None,
        }
    }

    fn album(zone: &str, title: &str) -> SharedAlbum {
        SharedAlbum {
            zone: zone.into(),
            database: "private".into(),
            owner: None,
            title: Some(title.into()),
            expires_at: Some(1_795_498_444),
            count: 0,
            cover: None,
        }
    }

    /// An album stored again is what Apple has now: a photo removed there is gone here, its
    /// count and cover follow, and where it lives is known for its pictures.
    #[test]
    fn an_album_is_replaced_whole() {
        let mut lib = Library::in_memory().unwrap();
        let a = album("SharedCollection-1", "Wallpapers");
        lib.put_shared_album(&a, &[photo("p1", 10), photo("p2", 20)], Some("t1"))
            .unwrap();
        lib.put_shared_album(&a, &[photo("p1", 10)], Some("t2"))
            .unwrap();
        let listed = lib.shared_albums().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].count, listed[0].cover.as_deref()),
            (1, Some("p1"))
        );
        assert_eq!(
            lib.shared_album_token("SharedCollection-1")
                .unwrap()
                .as_deref(),
            Some("t2")
        );
        let ids: Vec<String> = lib
            .shared_album_photos("SharedCollection-1")
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["p1"]);
        assert!(lib.shared_photo("p2").unwrap().is_none());
        let (p, place) = lib.shared_photo("p1").unwrap().unwrap();
        assert_eq!(
            (p.filename.as_deref(), place.zone.as_str()),
            (Some("p1.JPG"), "SharedCollection-1")
        );
    }

    /// An album gone on Apple's side (deleted, left, expired) leaves with every photo in it.
    #[test]
    fn an_album_let_go_of_takes_its_photos() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_shared_album(&album("SC-a", "A"), &[photo("a1", 1)], None)
            .unwrap();
        lib.put_shared_album(&album("SC-b", "B"), &[photo("b1", 1)], None)
            .unwrap();
        lib.keep_shared_albums(&["SC-b"]).unwrap();
        let zones: Vec<String> = lib
            .shared_albums()
            .unwrap()
            .into_iter()
            .map(|a| a.zone)
            .collect();
        assert_eq!(zones, ["SC-b"]);
        assert!(lib.shared_photo("a1").unwrap().is_none());
        assert!(lib.shared_album_photos("SC-a").unwrap().is_empty());
    }

    /// What others share never shows in the person's own Library, its counts or search; and
    /// signing out takes it with the rest of the account.
    #[test]
    fn shared_photos_stay_out_of_the_library_and_leave_with_the_account() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_shared_album(&album("SC-a", "A"), &[photo("s1", 1)], None)
            .unwrap();
        assert!(lib.photo("s1").unwrap().is_none());
        let library = lib
            .photo_counts()
            .unwrap()
            .into_iter()
            .find(|(v, _)| *v == View::Library)
            .map(|(_, n)| n);
        assert_eq!(library, Some(0));
        lib.forget_account().unwrap();
        assert!(lib.shared_albums().unwrap().is_empty());
    }
}
