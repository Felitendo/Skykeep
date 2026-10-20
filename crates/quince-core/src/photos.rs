//! iCloud Photos in the local library.
//!
//! The whole library is mirrored through CloudKit's change feed, so every view (Library,
//! Favorites, Videos, Recently Deleted) is a query here and draws at once. Only the photos'
//! descriptions are kept; the images themselves are cached on disk by the app as they are
//! seen, and their download links, which expire, are never stored.

use crate::{Library, Result};
use rusqlite::{params, OptionalExtension};
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
    /// Its title, as typed in Photos' Info (`captionEnc` — Apple's name for the title).
    #[serde(default)]
    pub title: Option<String>,
    /// Its caption, the longer words under the title (`extendedDescEnc`).
    #[serde(default)]
    pub caption: Option<String>,
    /// Its star rating, 0 (none) to 5.
    #[serde(default)]
    pub rating: i64,
    /// The words it is filed under (`keywordsEnc`).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Where it was taken, in degrees, when Apple has it (`locationEnc`).
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    /// How it was rendered (Apple's customRenderedValue): 8 is a Portrait, with depth.
    #[serde(default)]
    pub rendered: i64,
    /// Photos' subtype: 1 and 3 mark screenshots and the like (`assetSubtypeV2`).
    pub subtype: i64,
    /// The burst it was shot in (`burstId`): a burst shows as its one chosen photo.
    #[serde(default)]
    pub burst: Option<String>,
    /// The burst's chosen photo — the one Photos shows for it (`burstFlags` 16).
    #[serde(default)]
    pub burst_pick: bool,
    /// How many photos its burst holds (filled when read; not stored).
    #[serde(default)]
    pub burst_count: i64,
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
    /// A folder of albums (Apple's albumType 3), not an album of photos.
    #[serde(default)]
    pub folder: bool,
    /// The folder it sits in; none at the top.
    #[serde(default)]
    pub parent: Option<String>,
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
    /// The album open when it was added: the photo goes into it too, as in Photos.
    #[serde(default)]
    pub album: Option<String>,
}

/// Which photos a view shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum View {
    Library,
    Favorites,
    Videos,
    Screenshots,
    /// Wide photos made by sweeping: assetSubtypeV2 1.
    Panoramas,
    /// Portraits, with depth: customRenderedValue 8.
    Portrait,
    /// Newest added first.
    Recents,
    /// Hidden from every other view, as Photos' Hidden album.
    Hidden,
    /// Those given stars, the most first.
    Rated,
    RecentlyDeleted,
}

const COLUMNS: &str = "id, master, taken_at, added_at, kind, width, height, duration, filename, \
                       size, uti, favorite, hidden, deleted, deleted_at, subtype, change_tag, expunge_at, title, caption, rating, \
                       keywords, latitude, longitude, rendered, burst, burst_pick";

/// A burst shows as its chosen photo, as Photos shows it (Apple's library says as much: its
/// `burstsExcludedFromPhotosCount` is every burst's photos less one each). A burst whose chosen
/// photo is not in the library shows whole rather than not at all.
const ONE_PER_BURST: &str =
    "(burst IS NULL OR burst_pick = 1 OR NOT EXISTS (SELECT 1 FROM photos k \
                             WHERE k.burst = photos.burst AND k.burst_pick = 1 AND k.deleted = 0))";

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
        title: r.get(18)?,
        caption: r.get(19)?,
        rating: r.get(20)?,
        keywords: r
            .get::<_, Option<String>>(21)?
            .and_then(|k| serde_json::from_str(&k).ok())
            .unwrap_or_default(),
        latitude: r.get(22)?,
        longitude: r.get(23)?,
        rendered: r.get(24)?,
        burst: r.get(25)?,
        burst_pick: r.get::<_, i64>(26)? != 0,
        burst_count: 0,
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
                                     subtype, change_tag, expunge_at, title, caption, rating,
                                     keywords, latitude, longitude, rendered, burst, burst_pick)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                         ?22, ?23, ?24, ?25, ?26, ?27)
                 ON CONFLICT(id) DO UPDATE SET
                   master = excluded.master, taken_at = excluded.taken_at,
                   added_at = excluded.added_at, kind = excluded.kind, width = excluded.width,
                   height = excluded.height, duration = excluded.duration,
                   filename = excluded.filename, size = excluded.size, uti = excluded.uti,
                   favorite = excluded.favorite, hidden = excluded.hidden,
                   deleted = excluded.deleted, deleted_at = excluded.deleted_at,
                   subtype = excluded.subtype, change_tag = excluded.change_tag,
                   expunge_at = excluded.expunge_at, title = excluded.title,
                   caption = excluded.caption, rating = excluded.rating,
                   keywords = excluded.keywords, latitude = excluded.latitude,
                   longitude = excluded.longitude, rendered = excluded.rendered,
                   burst = excluded.burst, burst_pick = excluded.burst_pick",
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
                    p.expunge_at,
                    p.title,
                    p.caption,
                    p.rating,
                    (!p.keywords.is_empty())
                        .then(|| serde_json::to_string(&p.keywords).unwrap_or_default()),
                    p.latitude,
                    p.longitude,
                    p.rendered,
                    p.burst,
                    i64::from(p.burst_pick)
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
        let bursts = if view == View::RecentlyDeleted {
            "1"
        } else {
            ONE_PER_BURST
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM photos WHERE {filter} AND {bursts} ORDER BY {order}, id"
        ))?;
        let rows = stmt.query_map([], row_to_photo)?;
        self.with_bursts(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Each photo's burst size, from one count over the library.
    fn with_bursts(&self, mut photos: Vec<Photo>) -> Result<Vec<Photo>> {
        if !photos.iter().any(|p| p.burst.is_some()) {
            return Ok(photos);
        }
        let mut stmt = self.conn.prepare(
            "SELECT burst, COUNT(*) FROM photos WHERE burst IS NOT NULL AND deleted = 0 GROUP BY burst",
        )?;
        let sizes: std::collections::HashMap<String, i64> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        for p in &mut photos {
            if let Some(b) = &p.burst {
                p.burst_count = sizes.get(b).copied().unwrap_or(1);
            }
        }
        Ok(photos)
    }

    /// Every photo of one burst, in the order they were shot.
    pub fn burst_photos(&self, burst: &str) -> Result<Vec<Photo>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM photos WHERE burst = ?1 AND deleted = 0 ORDER BY taken_at, id"
        ))?;
        let rows = stmt.query_map(params![burst], row_to_photo)?;
        self.with_bursts(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Photos found by what is typed, as Photos' search finds them: every word must match the
    /// file's name, title, caption or a keyword, or the year or month it was taken ("2025",
    /// "march"). Deleted and hidden photos are not searched. Newest first, at most `limit`.
    pub fn search_photos(&self, query: &str, limit: usize) -> Result<Vec<Photo>> {
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| !w.is_empty())
            .take(8)
            .collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        // One condition per word, each its own parameter: what is typed is never part of the SQL.
        let one = "(LOWER(COALESCE(filename, '')) LIKE ?{n} ESCAPE '\\'
              OR LOWER(COALESCE(title, '')) LIKE ?{n} ESCAPE '\\'
              OR LOWER(COALESCE(caption, '')) LIKE ?{n} ESCAPE '\\'
              OR LOWER(COALESCE(keywords, '')) LIKE ?{n} ESCAPE '\\'
              OR strftime('%Y', taken_at, 'unixepoch', 'localtime') LIKE ?{n} ESCAPE '\\'
              OR (CASE strftime('%m', taken_at, 'unixepoch', 'localtime')
                    WHEN '01' THEN 'january' WHEN '02' THEN 'february' WHEN '03' THEN 'march'
                    WHEN '04' THEN 'april' WHEN '05' THEN 'may' WHEN '06' THEN 'june'
                    WHEN '07' THEN 'july' WHEN '08' THEN 'august' WHEN '09' THEN 'september'
                    WHEN '10' THEN 'october' WHEN '11' THEN 'november' ELSE 'december' END)
                  LIKE ?{n} ESCAPE '\\')";
        let conditions: Vec<String> = (1..=words.len())
            .map(|n| one.replace("{n}", &n.to_string()))
            .collect();
        let sql = format!(
            "SELECT {COLUMNS} FROM photos WHERE deleted = 0 AND hidden = 0 AND {ONE_PER_BURST} AND {}
             ORDER BY taken_at DESC, id LIMIT {}",
            conditions.join(" AND "),
            limit.min(50_000)
        );
        let patterns: Vec<String> = words
            .iter()
            .map(|w| {
                let escaped = w
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                format!("%{escaped}%")
            })
            .collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(patterns.iter()), row_to_photo)?;
        self.with_bursts(rows.collect::<std::result::Result<_, _>>()?)
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
            // Apple's own rule, read from its smart albums on his library: assetSubtypeV2 3 is a
            // screenshot, 1 a panorama. (Counting any PNG, and 1, as screenshots put 43 there
            // against Apple's 4.)
            View::Screenshots => (
                "deleted = 0 AND hidden = 0 AND subtype = 3",
                "taken_at DESC",
            ),
            View::Panoramas => (
                "deleted = 0 AND hidden = 0 AND subtype = 1",
                "taken_at DESC",
            ),
            // Apple's DEPTH smart album, read on his library: customRenderedValue 8.
            View::Portrait => (
                "deleted = 0 AND hidden = 0 AND rendered = 8",
                "taken_at DESC",
            ),
            View::Recents => (
                "deleted = 0 AND hidden = 0",
                "COALESCE(added_at, taken_at) DESC",
            ),
            View::Hidden => ("deleted = 0 AND hidden = 1", "taken_at DESC"),
            View::Rated => (
                "deleted = 0 AND hidden = 0 AND rating > 0",
                "rating DESC, taken_at DESC",
            ),
            View::RecentlyDeleted => ("deleted = 1", "COALESCE(deleted_at, taken_at) DESC"),
        }
    }

    /// One photo, by asset id, wherever it is (Recently Deleted and hidden included).
    pub fn photo(&self, id: &str) -> Result<Option<Photo>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM photos WHERE id = ?1"))?;
        let mut rows = stmt.query_map(params![id], row_to_photo)?;
        let one: Option<Photo> = rows.next().transpose()?;
        Ok(match one {
            Some(p) => self.with_bursts(vec![p])?.pop(),
            None => None,
        })
    }

    /// How many photos each view holds.
    pub fn photo_counts(&self) -> Result<Vec<(View, i64)>> {
        let mut out = Vec::new();
        for view in [
            View::Library,
            View::Favorites,
            View::Videos,
            View::Screenshots,
            View::Panoramas,
            View::Portrait,
            View::Hidden,
            View::Rated,
            View::RecentlyDeleted,
        ] {
            // Counted where the rows are: a library of twenty thousand is not read seven times
            // over to say how many each view holds.
            let (filter, _) = Self::view_query(view);
            let bursts = if view == View::RecentlyDeleted {
                "1"
            } else {
                ONE_PER_BURST
            };
            let n: i64 = self.conn.query_row(
                &format!("SELECT COUNT(*) FROM photos WHERE {filter} AND {bursts}"),
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
                "INSERT OR IGNORE INTO photo_uploads (id, batch, path, name, state, error, asset, added_at, album)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![u.id, u.batch, u.path, u.name, u.state, u.error, u.asset, u.added_at, u.album],
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
            "SELECT id, batch, path, name, state, error, asset, added_at, album FROM photo_uploads
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
                album: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One upload, by id, whatever its state: what the upload panel shows a picture of.
    pub fn upload(&self, id: &str) -> Result<Option<Upload>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, batch, path, name, state, error, asset, added_at, album FROM photo_uploads
                 WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Upload {
                        id: r.get(0)?,
                        batch: r.get(1)?,
                        path: r.get(2)?,
                        name: r.get(3)?,
                        state: r.get(4)?,
                        error: r.get(5)?,
                        asset: r.get(6)?,
                        added_at: r.get(7)?,
                        album: r.get(8)?,
                    })
                },
            )
            .optional()?)
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
                "INSERT INTO photo_albums (id, name, position, change_tag, folder, parent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    a.id,
                    a.name,
                    a.position,
                    a.change_tag,
                    i64::from(a.folder),
                    a.parent
                ],
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
        self.with_bursts(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One album added or changed, as Apple answered a write.
    pub fn put_album(&mut self, a: &Album) -> Result<()> {
        self.conn.execute(
            "INSERT INTO photo_albums (id, name, position, change_tag, folder, parent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, position = excluded.position,
             change_tag = excluded.change_tag, folder = excluded.folder, parent = excluded.parent",
            params![
                a.id,
                a.name,
                a.position,
                a.change_tag,
                i64::from(a.folder),
                a.parent
            ],
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

    /// Every album with how many photos it shows (deleted and hidden ones not counted) and
    /// the first of them — its cover, as the sidebar and the albums page show it — in the
    /// order the albums are listed.
    pub fn albums_counted(&self) -> Result<Vec<(Album, i64, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.position, a.change_tag, a.folder, a.parent,
                    (SELECT COUNT(*) FROM photo_album_items i JOIN photos p ON p.id = i.asset
                      WHERE i.album = a.id AND p.deleted = 0 AND p.hidden = 0),
                    (SELECT i.asset FROM photo_album_items i JOIN photos p ON p.id = i.asset
                      WHERE i.album = a.id AND p.deleted = 0 AND p.hidden = 0
                      ORDER BY i.position, p.taken_at DESC, p.id LIMIT 1)
             FROM photo_albums a ORDER BY a.position, a.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                Album {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    position: r.get(2)?,
                    change_tag: r.get(3)?,
                    folder: r.get::<_, i64>(4)? != 0,
                    parent: r.get(5)?,
                },
                r.get(6)?,
                r.get(7)?,
            ))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Albums whose folder is gone, gone with it — and theirs in turn. Apple keeps an album in a
    /// deleted folder (checked live), where every Photos shows it as deleted with the folder.
    pub fn forget_orphan_albums(&mut self) -> Result<()> {
        loop {
            let orphans: Vec<String> = {
                let mut stmt = self.conn.prepare(
                    "SELECT id FROM photo_albums
                     WHERE parent IS NOT NULL AND parent NOT IN (SELECT id FROM photo_albums)",
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect::<std::result::Result<_, _>>()?
            };
            if orphans.is_empty() {
                return Ok(());
            }
            for id in orphans {
                self.forget_album(&id)?;
            }
        }
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
            "SELECT id, name, position, change_tag, folder, parent FROM photo_albums
             ORDER BY position, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Album {
                id: r.get(0)?,
                name: r.get(1)?,
                position: r.get(2)?,
                change_tag: r.get(3)?,
                folder: r.get::<_, i64>(4)? != 0,
                parent: r.get(5)?,
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
        }
    }

    /// A burst shows as its chosen photo in every view and count, carrying how many it holds;
    /// all of it is there to look through; with its chosen photo gone it shows whole.
    #[test]
    fn a_burst_shows_as_its_chosen_photo() {
        let mut lib = Library::in_memory().unwrap();
        let mut shots: Vec<Photo> = (0..5)
            .map(|i| Photo {
                burst: Some("B".into()),
                burst_pick: i == 2,
                ..photo(&format!("b{i}"), 100 + i)
            })
            .collect();
        shots.push(photo("alone", 50));
        lib.put_photos(&shots).unwrap();
        let shown = lib.photos(View::Library).unwrap();
        assert_eq!(
            shown.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["b2", "alone"]
        );
        assert_eq!(shown[0].burst_count, 5);
        let counts = lib.photo_counts().unwrap();
        assert_eq!(
            counts.iter().find(|(v, _)| *v == View::Library).unwrap().1,
            2
        );
        assert_eq!(lib.burst_photos("B").unwrap().len(), 5);
        // The chosen one deleted: the rest show, rather than the burst vanishing.
        let mut gone = lib.photo("b2").unwrap().unwrap();
        gone.deleted = true;
        lib.put_photos(&[gone]).unwrap();
        assert_eq!(lib.photos(View::Library).unwrap().len(), 5);
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
        // Search finds by name and by month, every word, and leaves the hidden and deleted out.
        let found = |q: &str| {
            lib.search_photos(q, 100)
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(found("B.JPG"), ["b"]);
        assert_eq!(
            found("jpg january 1970"),
            ["c", "b", "a"],
            "every word, the newest first"
        );
        assert!(found("e.jpg").is_empty(), "the hidden one is not found");
        assert!(
            found("100%").is_empty(),
            "what is typed is matched as text, never as a pattern"
        );
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
            folder: false,
            parent: None,
        }])
        .unwrap();
        assert_eq!(
            lib.albums_counted().unwrap()[0].1,
            2,
            "an album's count is what it shows"
        );
        assert_eq!(
            lib.albums_counted().unwrap()[0].2.as_deref(),
            Some("a"),
            "and its cover is the first it shows"
        );
        lib.forget_album_items(&[("AL".into(), "a".into())])
            .unwrap();
        assert_eq!(lib.album_photos("AL").unwrap().len(), 1);
        lib.forget_photos(&["a".into()]).unwrap();
        assert_eq!(ids(&lib, View::Library), ["c", "b"]);
    }

    /// Media types as Apple decides them (read from its smart albums on his library): a
    /// screenshot is assetSubtypeV2 3 and a panorama 1 — not every PNG.
    #[test]
    fn media_types_follow_apple() {
        let mut lib = Library::in_memory().unwrap();
        let mut png = photo("png", 100);
        png.uti = Some("public.png".into());
        let mut shot = photo("shot", 200);
        shot.subtype = 3;
        let mut wide = photo("wide", 300);
        wide.subtype = 1;
        lib.put_photos(&[png, shot, wide]).unwrap();
        let ids = |v| {
            lib.photos(v)
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(View::Screenshots), ["shot"]);
        assert_eq!(ids(View::Panoramas), ["wide"]);
        let mut portrait = photo("depth", 400);
        portrait.rendered = 8;
        lib.put_photos(&[portrait]).unwrap();
        assert_eq!(
            lib.photos(View::Portrait)
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            ["depth"]
        );
    }
}
