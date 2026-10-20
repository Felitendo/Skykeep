//! iCloud Photos in the local library.
//!
//! The whole library is mirrored through CloudKit's change feed, so every view (Library,
//! Favorites, Videos, Recently Deleted) is a query here and draws at once. Only the photos'
//! descriptions are kept; the images themselves are cached on disk by the app as they are
//! seen, and their download links, which expire, are never stored.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// What a photo is, as Photos sorts it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Image,
    Video,
    /// A still with its moving part beside it.
    Live,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Video => "video",
            Kind::Live => "live",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "video" => Kind::Video,
            "live" => Kind::Live,
            _ => Kind::Image,
        }
    }
}

/// One photo or video: an asset (what the person sees, with its edits and flags) and the
/// master it shows (the original file).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    /// The asset's record name: what writes are addressed to.
    pub id: String,
    pub master: String,
    /// When it was taken, unix seconds.
    pub taken_at: i64,
    /// When it came into the library, unix seconds.
    pub added_at: Option<i64>,
    pub kind: Kind,
    pub width: i64,
    pub height: i64,
    /// Seconds, for a video.
    pub duration: Option<f64>,
    pub filename: Option<String>,
    /// The original's size in bytes.
    pub size: Option<i64>,
    /// The original's type, as a UTI (`public.jpeg`, `public.mpeg-4` …).
    pub uti: Option<String>,
    pub favorite: bool,
    pub hidden: bool,
    /// In Recently Deleted.
    pub deleted: bool,
    /// When it was deleted: Photos counts its 30 days from here.
    pub deleted_at: Option<i64>,
    /// In Recently Deleted: when Apple deletes it for good, unix seconds (`dateExpunged`).
    #[serde(default)]
    pub expunge_at: Option<i64>,
    /// Photos' subtype: 1 and 3 mark screenshots and the like (`assetSubtypeV2`).
    pub subtype: i64,
    /// The asset's change tag, for writing back to it.
    pub change_tag: Option<String>,
}

/// An album the person made.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub change_tag: Option<String>,
}

/// A file on its way into Photos: waiting, sending, verifying (Apple making the photo), done,
/// failed (with why) or cancelled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub id: String,
    pub batch: String,
    pub path: String,
    pub name: String,
    pub state: String,
    pub error: Option<String>,
    pub asset: Option<String>,
    pub added_at: i64,
}

/// Which photos a view shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum View {
    Library,
    Favorites,
    Videos,
    Screenshots,
    /// Newest added first.
    Recents,
    /// Hidden from every other view, as Photos' Hidden album.
    Hidden,
    RecentlyDeleted,
}

const COLUMNS: &str = "id, master, taken_at, added_at, kind, width, height, duration, filename, \
                       size, uti, favorite, hidden, deleted, deleted_at, subtype, change_tag, expunge_at";

fn row_to_photo(r: &rusqlite::Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: r.get(0)?,
        master: r.get(1)?,
        taken_at: r.get(2)?,
        added_at: r.get(3)?,
        kind: Kind::parse(&r.get::<_, String>(4)?),
        width: r.get(5)?,
        height: r.get(6)?,
        duration: r.get(7)?,
        filename: r.get(8)?,
        size: r.get(9)?,
        uti: r.get(10)?,
        favorite: r.get::<_, i64>(11)? != 0,
        hidden: r.get::<_, i64>(12)? != 0,
        deleted: r.get::<_, i64>(13)? != 0,
        deleted_at: r.get(14)?,
        subtype: r.get(15)?,
        change_tag: r.get(16)?,
        expunge_at: r.get(17)?,
    })
}

impl Library {
    /// Photos written or replaced, by asset id.
    pub fn put_photos(&mut self, photos: &[Photo]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO photos (id, master, taken_at, added_at, kind, width, height, duration,
                                     filename, size, uti, favorite, hidden, deleted, deleted_at,
                                     subtype, change_tag, expunge_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(id) DO UPDATE SET
                   master = excluded.master, taken_at = excluded.taken_at,
                   added_at = excluded.added_at, kind = excluded.kind, width = excluded.width,
                   height = excluded.height, duration = excluded.duration,
                   filename = excluded.filename, size = excluded.size, uti = excluded.uti,
                   favorite = excluded.favorite, hidden = excluded.hidden,
                   deleted = excluded.deleted, deleted_at = excluded.deleted_at,
                   subtype = excluded.subtype, change_tag = excluded.change_tag,
                   expunge_at = excluded.expunge_at",
            )?;
            for p in photos {
                stmt.execute(params![
                    p.id,
                    p.master,
                    p.taken_at,
                    p.added_at,
                    p.kind.as_str(),
                    p.width,
                    p.height,
                    p.duration,
                    p.filename,
                    p.size,
                    p.uti,
                    i64::from(p.favorite),
                    i64::from(p.hidden),
                    i64::from(p.deleted),
                    p.deleted_at,
                    p.subtype,
                    p.change_tag,
                    p.expunge_at
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Photos gone from iCloud for good (expunged), by asset id.
    pub fn forget_photos(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM photos WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every photo, replaced by `photos`: a full pass, where anything not in it is gone.
    pub fn replace_photos(&mut self, photos: &[Photo]) -> Result<()> {
        self.conn.execute("DELETE FROM photos", [])?;
        self.put_photos(photos)
    }

    /// A view's photos, newest first.
    pub fn photos(&self, view: View) -> Result<Vec<Photo>> {
        let (filter, order) = Self::view_query(view);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM photos WHERE {filter} ORDER BY {order}, id"
        ))?;
        let rows = stmt.query_map([], row_to_photo)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Which photos a view holds, and in what order.
    fn view_query(view: View) -> (&'static str, &'static str) {
        match view {
            View::Library => ("deleted = 0 AND hidden = 0", "taken_at DESC"),
            View::Favorites => (
                "deleted = 0 AND hidden = 0 AND favorite = 1",
                "taken_at DESC",
            ),
            View::Videos => (
                "deleted = 0 AND hidden = 0 AND kind = 'video'",
                "taken_at DESC",
            ),
            View::Screenshots => (
                "deleted = 0 AND hidden = 0 AND (subtype IN (1, 3) OR uti = 'public.png')",
                "taken_at DESC",
            ),
            View::Recents => (
                "deleted = 0 AND hidden = 0",
                "COALESCE(added_at, taken_at) DESC",
            ),
            View::Hidden => ("deleted = 0 AND hidden = 1", "taken_at DESC"),
            View::RecentlyDeleted => ("deleted = 1", "COALESCE(deleted_at, taken_at) DESC"),
        }
    }

    /// One photo, by asset id, wherever it is (Recently Deleted and hidden included).
    pub fn photo(&self, id: &str) -> Result<Option<Photo>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM photos WHERE id = ?1"))?;
        let mut rows = stmt.query_map(params![id], row_to_photo)?;
        Ok(rows.next().transpose()?)
    }

    /// How many photos each view holds.
    pub fn photo_counts(&self) -> Result<Vec<(View, i64)>> {
        let mut out = Vec::new();
        for view in [
            View::Library,
            View::Favorites,
            View::Videos,
            View::Screenshots,
            View::Hidden,
            View::RecentlyDeleted,
        ] {
            // Counted where the rows are: a library of twenty thousand is not read seven times
            // over to say how many each view holds.
            let (filter, _) = Self::view_query(view);
            let n: i64 = self.conn.query_row(
                &format!("SELECT COUNT(*) FROM photos WHERE {filter}"),
                [],
                |r| r.get(0),
            )?;
            out.push((view, n));
        }
        Ok(out)
    }

    /// Files added to the upload queue, waiting.
    pub fn queue_uploads(&mut self, uploads: &[Upload]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for u in uploads {
            tx.execute(
                "INSERT OR IGNORE INTO photo_uploads (id, batch, path, name, state, error, asset, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![u.id, u.batch, u.path, u.name, u.state, u.error, u.asset, u.added_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// An upload's state set, with why it failed or the photo it made.
    pub fn set_upload(
        &self,
        id: &str,
        state: &str,
        error: Option<&str>,
        asset: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE photo_uploads SET state = ?2, error = ?3, asset = COALESCE(?4, asset) WHERE id = ?1",
            params![id, state, error, asset],
        )?;
        Ok(())
    }

    /// Uploads not yet finished, oldest first: what a launch resumes.
    pub fn unfinished_uploads(&self) -> Result<Vec<Upload>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, batch, path, name, state, error, asset, added_at FROM photo_uploads
             WHERE state IN ('waiting', 'sending', 'verifying') ORDER BY added_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Upload {
                id: r.get(0)?,
                batch: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                state: r.get(4)?,
                error: r.get(5)?,
                asset: r.get(6)?,
                added_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Finished uploads forgotten once their batch is: what the tray no longer shows.
    pub fn forget_finished_uploads(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM photo_uploads WHERE state IN ('done', 'failed', 'cancelled')",
            [],
        )?;
        Ok(())
    }

    /// The albums, replaced by `albums`.
    pub fn replace_albums(&mut self, albums: &[Album]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM photo_albums", [])?;
        for a in albums {
            tx.execute(
                "INSERT INTO photo_albums (id, name, position, change_tag) VALUES (?1, ?2, ?3, ?4)",
                params![a.id, a.name, a.position, a.change_tag],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Which photos are in which album: `(album, asset, position)`, added or moved.
    pub fn put_album_items(&mut self, items: &[(String, String, i64)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (album, asset, position) in items {
            tx.execute(
                "INSERT INTO photo_album_items (album, asset, position) VALUES (?1, ?2, ?3)
                 ON CONFLICT(album, asset) DO UPDATE SET position = excluded.position",
                params![album, asset, position],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Photos taken out of albums: `(album, asset)`.
    pub fn forget_album_items(&mut self, items: &[(String, String)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (album, asset) in items {
            tx.execute(
                "DELETE FROM photo_album_items WHERE album = ?1 AND asset = ?2",
                params![album, asset],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every album's photos, replaced: a full pass.
    pub fn replace_album_items(&mut self, items: &[(String, String, i64)]) -> Result<()> {
        self.conn.execute("DELETE FROM photo_album_items", [])?;
        self.put_album_items(items)
    }

    /// An album's photos, in the album's order; none deleted or hidden.
    pub fn album_photos(&self, album: &str) -> Result<Vec<Photo>> {
        let columns = COLUMNS
            .split(", ")
            .map(|c| format!("p.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {columns} FROM photo_album_items i JOIN photos p ON p.id = i.asset
             WHERE i.album = ?1 AND p.deleted = 0 AND p.hidden = 0
             ORDER BY i.position, p.taken_at DESC, p.id"
        ))?;
        let rows = stmt.query_map(params![album], row_to_photo)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One album added or changed, as Apple answered a write.
    pub fn put_album(&mut self, a: &Album) -> Result<()> {
        self.conn.execute(
            "INSERT INTO photo_albums (id, name, position, change_tag) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, position = excluded.position,
             change_tag = excluded.change_tag",
            params![a.id, a.name, a.position, a.change_tag],
        )?;
        Ok(())
    }

    /// An album gone, and which photos were in it with it. Apple leaves a deleted album's
    /// relation records standing, so they are dropped here.
    pub fn forget_album(&mut self, id: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM photo_albums WHERE id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM photo_album_items WHERE album = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Every album with how many photos it shows (deleted and hidden ones not counted), in
    /// the order the albums are listed.
    pub fn albums_counted(&self) -> Result<Vec<(Album, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.position, a.change_tag,
                    (SELECT COUNT(*) FROM photo_album_items i JOIN photos p ON p.id = i.asset
                      WHERE i.album = a.id AND p.deleted = 0 AND p.hidden = 0)
             FROM photo_albums a ORDER BY a.position, a.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                Album {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    position: r.get(2)?,
                    change_tag: r.get(3)?,
                },
                r.get(4)?,
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Where the album's last photo sits, so what is added goes after it.
    pub fn album_last_position(&self, album: &str) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT MAX(position) FROM photo_album_items WHERE album = ?1",
            params![album],
            |r| r.get(0),
        )?)
    }

    pub fn albums(&self) -> Result<Vec<Album>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, position, change_tag FROM photo_albums ORDER BY position, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Album {
                id: r.get(0)?,
                name: r.get(1)?,
                position: r.get(2)?,
                change_tag: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(id: &str, taken: i64) -> Photo {
        Photo {
            id: id.into(),
            master: format!("m-{id}"),
            taken_at: taken,
            added_at: Some(taken),
            kind: Kind::Image,
            width: 4032,
            height: 3024,
            duration: None,
            filename: Some(format!("{id}.jpg")),
            size: Some(1_000_000),
            uti: Some("public.jpeg".into()),
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            subtype: 0,
            change_tag: None,
            expunge_at: None,
        }
    }

    /// Each view is the right photos, newest first; Recently Deleted holds only the deleted.
    #[test]
    fn views_sort_and_filter() {
        let mut lib = Library::in_memory().unwrap();
        let mut fav = photo("b", 200);
        fav.favorite = true;
        let mut video = photo("c", 300);
        video.kind = Kind::Video;
        let mut gone = photo("d", 400);
        gone.deleted = true;
        let mut hidden = photo("e", 500);
        hidden.hidden = true;
        lib.put_photos(&[photo("a", 100), fav, video, gone, hidden])
            .unwrap();
        let ids = |lib: &Library, v| {
            lib.photos(v)
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&lib, View::Library), ["c", "b", "a"]);
        assert_eq!(ids(&lib, View::Favorites), ["b"]);
        assert_eq!(ids(&lib, View::Videos), ["c"]);
        assert_eq!(ids(&lib, View::RecentlyDeleted), ["d"]);
        // Each view's count is how many it shows; the sidebar never reads a view to count it.
        for (view, n) in lib.photo_counts().unwrap() {
            assert_eq!(n as usize, lib.photos(view).unwrap().len(), "{view:?}");
        }
        assert_eq!(
            ids(&lib, View::Hidden),
            ["e"],
            "the hidden one, and only there"
        );
        // An album holds what was put in it, in its order; a deleted photo leaves it.
        lib.put_photos(&[photo("a", 100)]).unwrap();
        lib.put_album_items(&[
            ("AL".into(), "b".into(), 2048),
            ("AL".into(), "a".into(), 1024),
            ("AL".into(), "d".into(), 0),
        ])
        .unwrap();
        let in_album: Vec<String> = lib
            .album_photos("AL")
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(
            in_album,
            ["a", "b"],
            "in position order, the deleted one not shown"
        );
        lib.replace_albums(&[Album {
            id: "AL".into(),
            name: "Trip".into(),
            position: 0,
            change_tag: None,
        }])
        .unwrap();
        assert_eq!(
            lib.albums_counted().unwrap()[0].1,
            2,
            "an album's count is what it shows"
        );
        lib.forget_album_items(&[("AL".into(), "a".into())])
            .unwrap();
        assert_eq!(lib.album_photos("AL").unwrap().len(), 1);
        lib.forget_photos(&["a".into()]).unwrap();
        assert_eq!(ids(&lib, View::Library), ["c", "b"]);
    }
}
