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

/// A Shared Library (iOS 16+): the family photo library this account owns or was invited to,
/// in a `SharedSync-<id>` zone. Its photos are the library's own rows, marked with this zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLibrary {
    pub zone: String,
    /// `private` when this account set it up, `shared` when it joined someone else's.
    pub database: String,
    /// The account that owns it, for one this account joined.
    pub owner: Option<String>,
    pub title: Option<String>,
}

impl Library {
    /// A Shared Library as Apple lists it now.
    pub fn put_shared_library(&mut self, library: &SharedLibrary) -> Result<()> {
        self.conn.execute(
            "INSERT INTO shared_libraries (zone, database, owner, title) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(zone) DO UPDATE SET database = excluded.database,
               owner = excluded.owner, title = excluded.title",
            params![library.zone, library.database, library.owner, library.title],
        )?;
        Ok(())
    }

    /// Only these Shared Libraries are left: one this account left or that was ended goes,
    /// with its photos and its read's place, so none of them lingers in the grid.
    pub fn keep_shared_libraries(&mut self, kept: &[&str]) -> Result<()> {
        let list = crate::id_list(kept);
        let tx = self.conn.savepoint()?;
        let gone: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT zone FROM shared_libraries WHERE zone NOT IN (SELECT value FROM json_each(?1))",
            )?;
            let rows = stmt.query_map(params![list], |r| r.get(0))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        for zone in &gone {
            tx.execute("DELETE FROM photos WHERE zone = ?1", params![zone])?;
            tx.execute(
                "DELETE FROM sync_state WHERE service = ?1",
                params![format!("photos:{zone}")],
            )?;
            tx.execute(
                "DELETE FROM shared_libraries WHERE zone = ?1",
                params![zone],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every Shared Library this account is in.
    pub fn shared_libraries(&self) -> Result<Vec<SharedLibrary>> {
        let mut stmt = self.conn.prepare(
            "SELECT zone, database, owner, title FROM shared_libraries ORDER BY database, zone",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SharedLibrary {
                zone: r.get(0)?,
                database: r.get(1)?,
                owner: r.get(2)?,
                title: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Where a photo of a Shared Library lives in Apple's databases: none for a photo of the
    /// personal library (or one not kept).
    pub fn shared_library_of(&self, photo: &str) -> Result<Option<SharedLibrary>> {
        Ok(self
            .conn
            .query_row(
                "SELECT l.zone, l.database, l.owner, l.title
                   FROM photos p JOIN shared_libraries l ON l.zone = p.zone
                  WHERE p.id = ?1",
                params![photo],
                |r| {
                    Ok(SharedLibrary {
                        zone: r.get(0)?,
                        database: r.get(1)?,
                        owner: r.get(2)?,
                        title: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {

    /// A Shared Library left or ended goes with its photos and its place in the change feed;
    /// the personal library's photos and the libraries kept stay.
    #[test]
    fn leaving_a_shared_library_lets_go_of_its_photos() {
        use crate::photos::{Kind, Photo};
        let shot = |id: &str| Photo {
            id: id.into(),
            master: format!("m-{id}"),
            taken_at: 1,
            added_at: None,
            kind: Kind::Image,
            width: 1,
            height: 1,
            duration: None,
            filename: None,
            size: None,
            uti: None,
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            expunge_at: None,
            title: None,
            caption: None,
            rating: 0,
            keywords: Vec::new(),
            latitude: None,
            longitude: None,
            rendered: 0,
            subtype: 0,
            burst: None,
            burst_pick: false,
            burst_count: 0,
            live_size: None,
            change_tag: None,
        };
        let mut lib = Library::in_memory().unwrap();
        for zone in ["SharedSync-A", "SharedSync-B"] {
            lib.put_shared_library(&SharedLibrary {
                zone: zone.into(),
                database: "shared".into(),
                owner: Some("_owner".into()),
                title: None,
            })
            .unwrap();
        }
        lib.put_photos_in(None, &[shot("mine")]).unwrap();
        lib.put_photos_in(Some("SharedSync-A"), &[shot("a")])
            .unwrap();
        lib.put_photos_in(Some("SharedSync-B"), &[shot("b")])
            .unwrap();
        lib.mark_synced("photos:SharedSync-A", Some("t")).unwrap();
        assert_eq!(
            lib.shared_library_of("a")
                .unwrap()
                .map(|l| l.zone)
                .as_deref(),
            Some("SharedSync-A")
        );
        assert_eq!(lib.shared_library_of("mine").unwrap(), None);

        lib.keep_shared_libraries(&["SharedSync-B"]).unwrap();
        let ids = lib.photo_ids().unwrap();
        assert!(ids.contains("mine") && ids.contains("b") && !ids.contains("a"));
        assert_eq!(lib.sync_token("photos:SharedSync-A").unwrap(), None);
        assert_eq!(lib.shared_libraries().unwrap().len(), 1);
    }
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

    /// Every shared photo, from every album, newest first, laid out as the grid lays it out:
    /// what their pictures are made ahead from.
    #[test]
    fn every_shared_photo_is_laid_out_for_its_pictures() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_shared_album(&album("SC-a", "A"), &[photo("a1", 10)], None)
            .unwrap();
        lib.put_shared_album(
            &album("SC-b", "B"),
            &[photo("b1", 30), photo("b2", 20)],
            None,
        )
        .unwrap();
        let shapes = lib.shared_photo_shapes().unwrap();
        let ids: Vec<&str> = shapes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["b1", "b2", "a1"]);
        assert_eq!((shapes[0].width, shapes[0].height), (1200, 2600));
    }

    /// The grid asks for photos by id as it scrolls: a shared album's are found too (left
    /// unfound, the grid asked for them again every frame and its tiles flickered).
    #[test]
    fn photos_by_id_finds_shared_ones_too() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_photos(&[photo("own", 5)]).unwrap();
        lib.put_shared_album(
            &album("SC-a", "A"),
            &[photo("s1", 10), photo("s2", 9)],
            None,
        )
        .unwrap();
        let ids: Vec<String> = ["own", "s2", "nope", "s1"].map(String::from).to_vec();
        let found: Vec<String> = lib
            .photos_by_ids(&ids)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(found, ["own", "s2", "s1"]);
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
