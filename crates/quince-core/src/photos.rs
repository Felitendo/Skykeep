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
    /// A Live Photo's moving part, its original `.MOV`, in bytes.
    #[serde(default)]
    pub live_size: Option<i64>,
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
                       keywords, latitude, longitude, rendered, burst, burst_pick, live_size";

/// A burst shows as its chosen photo, as Photos shows it (Apple's library says as much: its
/// `burstsExcludedFromPhotosCount` is every burst's photos less one each). A burst whose chosen
/// photo is not in the library shows whole rather than not at all.
pub(crate) const ONE_PER_BURST: &str =
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
        live_size: r.get(27)?,
    })
}

/// What is typed, as SQL: one condition per word (each its own parameter — what is typed is
/// never part of the SQL), every word to match the file's name, title, caption or a keyword, or
/// the year or month it was taken. `None` when nothing is typed.
fn search_terms(query: &str) -> Option<(String, Vec<String>)> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty())
        .take(8)
        .collect();
    if words.is_empty() {
        return None;
    }
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
    let patterns = words
        .iter()
        .map(|w| {
            let escaped = w
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("%{escaped}%")
        })
        .collect();
    Some((conditions.join(" AND "), patterns))
}

impl Library {
    /// Photos written or replaced, by asset id.
    pub fn put_photos(&mut self, photos: &[Photo]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO photos (id, master, taken_at, added_at, kind, width, height, duration,
                                     filename, size, uti, favorite, hidden, deleted, deleted_at,
                                     subtype, change_tag, expunge_at, title, caption, rating,
                                     keywords, latitude, longitude, rendered, burst, burst_pick,
                                     live_size)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                         ?22, ?23, ?24, ?25, ?26, ?27, ?28)
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
                   burst = excluded.burst, burst_pick = excluded.burst_pick,
                   live_size = excluded.live_size",
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
                    i64::from(p.burst_pick),
                    p.live_size
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Photos gone from iCloud for good (expunged), by asset id.
    pub fn forget_photos(&mut self, ids: &[String]) -> Result<()> {
        let tx = self.conn.savepoint()?;
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

    /// Every photo id the library holds, in any view: what a grid picture on disk may still
    /// be drawn for.
    pub fn photo_ids(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM photos")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
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
        // Counted for the bursts here only: a page of two hundred never counts the library's.
        let mut stmt = self
            .conn
            .prepare_cached("SELECT COUNT(*) FROM photos WHERE burst = ?1 AND deleted = 0")?;
        let mut sizes: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for b in photos.iter().filter_map(|p| p.burst.as_ref()) {
            if !sizes.contains_key(b) {
                let n: i64 = stmt.query_row(params![b], |r| r.get(0))?;
                sizes.insert(b.clone(), n);
            }
        }
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
        let Some((conditions, patterns)) = search_terms(query) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {COLUMNS} FROM photos WHERE deleted = 0 AND hidden = 0 AND {ONE_PER_BURST} AND {conditions}
             ORDER BY taken_at DESC, id LIMIT {}",
            limit.min(50_000)
        );
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
        let tx = self.conn.savepoint()?;
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

    /// The latest upload of the file at `path`, whatever its state: a file added through the
    /// Photos folder is sent once, even when the folder sends it again after a restart.
    pub fn upload_at(&self, path: &str) -> Result<Option<Upload>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM photo_uploads WHERE path = ?1 ORDER BY added_at DESC, rowid DESC LIMIT 1",
                params![path],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.upload(&id),
            None => Ok(None),
        }
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
        let tx = self.conn.savepoint()?;
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
        let tx = self.conn.savepoint()?;
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
        let tx = self.conn.savepoint()?;
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
        let tx = self.conn.savepoint()?;
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

/// Where the photos a grid shows come from: a view, an album, a search, or one burst.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    View(View),
    Album(String),
    Query(String),
    Burst(String),
}

/// How a view the person can sort is sorted: by when taken or when added, newest or oldest
/// first. Albums, searches, bursts, Rated and Recently Deleted keep their own order.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub by: OrderBy,
    pub oldest: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OrderBy {
    Taken,
    Added,
}

/// What a grid needs of every photo it shows, before any photo itself: enough to lay out every
/// month, select, move and count — a few bytes a photo, where the photos are a kilobyte each.
/// The photos themselves come a page at a time ([`Library::photo_page`]), in the same order.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhotoIndex {
    pub ids: Vec<String>,
    /// What each is grouped by: when it was taken, or added when sorted by that.
    pub keys: Vec<i64>,
    /// Each one's kind and marks, packed: bits 0–1 the kind (0 image, 1 video, 2 Live), bit 2
    /// a favorite, bits 3–5 its stars.
    pub marks: Vec<u8>,
    /// The earliest and latest it holds were taken, unix seconds.
    pub first: Option<i64>,
    pub last: Option<i64>,
}

/// One packed mark (see [`PhotoIndex::marks`]).
fn mark(kind: &str, favorite: bool, rating: i64) -> u8 {
    let k = match Kind::parse(kind) {
        Kind::Image => 0,
        Kind::Video => 1,
        Kind::Live => 2,
    };
    k | (u8::from(favorite) << 2) | ((rating.clamp(0, 5) as u8) << 3)
}

/// A source's photos as SQL: what is selected from, which, in what order, with its parameters
/// — one place for the index and the pages, so their orders never differ. `pre` qualifies the
/// photos' columns where another table is joined (an album's items).
struct Query {
    from: &'static str,
    pre: &'static str,
    filter: String,
    order: String,
    key: &'static str,
    params: Vec<String>,
}

impl Library {
    fn source_query(source: &Source, order: Option<Order>) -> Query {
        const TAKEN: &str = "taken_at";
        const ADDED: &str = "COALESCE(added_at, taken_at)";
        let plain = |filter: String, order: String, key, params| Query {
            from: "photos",
            pre: "",
            filter,
            order,
            key,
            params,
        };
        match source {
            Source::View(view) => {
                let (filter, own) = Self::view_query(*view);
                let bursts = if *view == View::RecentlyDeleted {
                    "1"
                } else {
                    ONE_PER_BURST
                };
                let sortable = !matches!(view, View::RecentlyDeleted | View::Rated);
                let (order, key) = match order.filter(|_| sortable) {
                    Some(o) => {
                        let key = if o.by == OrderBy::Added { ADDED } else { TAKEN };
                        let dir = if o.oldest { "ASC" } else { "DESC" };
                        (format!("{key} {dir}, id"), key)
                    }
                    // Grouped by what it is sorted by: Recents by when added.
                    None => (
                        format!("{own}, id"),
                        if *view == View::Recents { ADDED } else { TAKEN },
                    ),
                };
                plain(format!("{filter} AND {bursts}"), order, key, Vec::new())
            }
            Source::Album(album) => Query {
                from: "photo_album_items i JOIN photos p ON p.id = i.asset",
                pre: "p.",
                filter: "i.album = ?1 AND p.deleted = 0 AND p.hidden = 0".into(),
                order: "i.position, p.taken_at DESC, p.id".into(),
                key: "p.taken_at",
                params: vec![album.clone()],
            },
            Source::Query(typed) => match search_terms(typed) {
                Some((conditions, patterns)) => plain(
                    format!("deleted = 0 AND hidden = 0 AND {ONE_PER_BURST} AND {conditions}"),
                    "taken_at DESC, id".into(),
                    TAKEN,
                    patterns,
                ),
                None => plain("0".into(), "id".into(), TAKEN, Vec::new()),
            },
            Source::Burst(burst) => plain(
                "burst = ?1 AND deleted = 0".into(),
                "taken_at, id".into(),
                TAKEN,
                vec![burst.clone()],
            ),
        }
    }

    /// Every photo a source shows, as a grid lays them out (see [`PhotoIndex`]).
    pub fn photo_index(&self, source: &Source, order: Option<Order>) -> Result<PhotoIndex> {
        let q = Self::source_query(source, order);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {p}id, {key}, {p}kind, {p}favorite, {p}rating, {p}taken_at FROM {from}
             WHERE {filter} ORDER BY {order}",
            p = q.pre,
            key = q.key,
            from = q.from,
            filter = q.filter,
            order = q.order,
        ))?;
        let mut rows = stmt.query(rusqlite::params_from_iter(q.params.iter()))?;
        let mut out = PhotoIndex::default();
        while let Some(r) = rows.next()? {
            out.ids.push(r.get(0)?);
            out.keys.push(r.get(1)?);
            out.marks.push(mark(
                &r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? != 0,
                r.get(4)?,
            ));
            let taken: i64 = r.get(5)?;
            out.first = Some(out.first.map_or(taken, |f| f.min(taken)));
            out.last = Some(out.last.map_or(taken, |l| l.max(taken)));
        }
        Ok(out)
    }

    /// `limit` photos of a source from `offset`, in the order its index has them.
    pub fn photo_page(
        &self,
        source: &Source,
        order: Option<Order>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Photo>> {
        let q = Self::source_query(source, order);
        let columns = COLUMNS
            .split(", ")
            .map(|c| format!("{}{}", q.pre, c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {columns} FROM {from} WHERE {filter} ORDER BY {order} LIMIT {limit} OFFSET {offset}",
            from = q.from,
            filter = q.filter,
            order = q.order,
            limit = limit.min(5_000),
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(q.params.iter()), row_to_photo)?;
        self.with_bursts(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Photos by asset id, in the order asked; those gone are left out.
    pub fn photos_by_ids(&self, ids: &[String]) -> Result<Vec<Photo>> {
        let mut out = Vec::with_capacity(ids.len());
        let mut stmt = self
            .conn
            .prepare_cached(&format!("SELECT {COLUMNS} FROM photos WHERE id = ?1"))?;
        for id in ids {
            if let Some(p) = stmt.query_row(params![id], row_to_photo).optional()? {
                out.push(p);
            }
        }
        self.with_bursts(out)
    }

    /// The library's months, newest first, each with how many it holds and the photos that stand
    /// for it (see [`PhotoMonth`]): what the Years and Months views draw. Counted and chosen here,
    /// a month at a time over the date index, so a library of fifty thousand answers in a few
    /// hundred rows — the window never walks its photos to find them.
    ///
    /// Months are this machine's own calendar months, the same months the grid groups its photos
    /// into, so a month chosen here is a month the grid has. Their edges are worked out here
    /// rather than in the query: dating every photo in SQLite (`strftime … 'localtime'`) was
    /// most of the time on a large library, where a range over the index costs next to nothing.
    pub fn photo_months(&self) -> Result<Vec<PhotoMonth>> {
        use jiff::{civil::Date, Span, Timestamp};
        let (filter, _) = Self::view_query(View::Library);
        let shown = format!("{filter} AND {ONE_PER_BURST} AND taken_at >= ?1 AND taken_at < ?2");
        let (first, last): (Option<i64>, Option<i64>) = self.conn.query_row(
            &format!("SELECT MIN(taken_at), MAX(taken_at) FROM photos WHERE {filter}"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (Some(first), Some(last)) = (first, last) else {
            return Ok(Vec::new());
        };
        let zone = self.local_zone();
        let month_of = |t: i64| -> Option<Date> {
            Some(
                Timestamp::from_second(t)
                    .ok()?
                    .to_zoned(zone.clone())
                    .date()
                    .first_of_month(),
            )
        };
        let start_of = |d: Date| -> Option<i64> {
            Some(d.to_zoned(zone.clone()).ok()?.timestamp().as_second())
        };
        let (Some(oldest), Some(mut month)) = (month_of(first), month_of(last)) else {
            return Ok(Vec::new());
        };
        let mut tally = self.conn.prepare_cached(&format!(
            "SELECT COUNT(*), COALESCE(SUM(kind = 'video'), 0) FROM photos WHERE {shown}"
        ))?;
        // The n-th taken in the month, oldest first.
        let mut nth = self.conn.prepare_cached(&format!(
            "SELECT id, taken_at FROM photos WHERE {shown} ORDER BY taken_at, id LIMIT 1 OFFSET ?3"
        ))?;
        // The key: a favourite first, a still before a video, then the nearest the middle photo
        // (SQLite gives the other columns of the row MIN chose).
        let mut key = self.conn.prepare_cached(&format!(
            "SELECT id, favorite, MIN((favorite = 0) * 4000000000000 + (kind = 'video') * 2000000000000
                                     + ABS(taken_at - ?3))
             FROM photos WHERE {shown}"
        ))?;
        let mut out = Vec::new();
        while month >= oldest {
            let next = month.checked_add(Span::new().months(1)).ok();
            let (Some(from), Some(to)) = (start_of(month), next.and_then(start_of)) else {
                break;
            };
            let (count, videos): (i64, i64) =
                tally.query_row(params![from, to], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if count > 0 {
                let mut at = |n: i64| -> rusqlite::Result<(String, i64)> {
                    nth.query_row(params![from, to, n], |r| Ok((r.get(0)?, r.get(1)?)))
                };
                let (middle, mid) = at(count / 2)?;
                let (chosen, favorite): (String, bool) =
                    key.query_row(params![from, to, mid], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let mut more = Vec::new();
                if count >= 4 {
                    for n in [(3 * count) / 4, count / 2, count / 4] {
                        let id = if n == count / 2 {
                            middle.clone()
                        } else {
                            at(n)?.0
                        };
                        // The key, when it is one of these too, is shown once: as the large one.
                        if id != chosen && !more.contains(&id) {
                            more.push(id);
                        }
                    }
                }
                out.push(PhotoMonth {
                    year: i32::from(month.year()),
                    month: month.month() as u32,
                    count,
                    videos,
                    key: chosen,
                    key_favorite: favorite,
                    more,
                });
            }
            match month.checked_sub(Span::new().months(1)) {
                Ok(m) => month = m,
                Err(_) => break,
            }
        }
        Ok(out)
    }
}

/// One month of the library as the Years and Months views show it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhotoMonth {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// Photos and videos it holds, a burst counted once (as the library shows it).
    pub count: i64,
    pub videos: i64,
    /// The photo that stands for it: a favourite if it has one, a still before a video, and the
    /// one taken nearest the middle of the month among those.
    pub key: String,
    pub key_favorite: bool,
    /// Up to three more, spread through the month in the order taken, newest first; none for a
    /// month of fewer than four.
    pub more: Vec<String>,
}

/// A whole read of Photos that takes more than one pass.
///
/// A large library is more change feed than one pass reads, so a whole read goes on across
/// passes from where the last one stopped. What each pass brings is stored at once, so the
/// library fills as it goes; what the read has seen is noted, so that when it reaches the end,
/// and only then, whatever it never saw is let go. The position it carries on from is kept
/// here too, and a pass is only taken as part of the read when it began at exactly that
/// position: one from anywhere else knows nothing of what the read saw before it.
///
/// Records that arrive without their other half at the end of an unfinished pass (an asset
/// whose master is in the next page, or a master whose asset is) are held until it comes.
const WALK_TABLES: &str = "
    CREATE TABLE IF NOT EXISTS photo_walk (
        kind TEXT NOT NULL,
        id   TEXT NOT NULL,
        PRIMARY KEY (kind, id)
    ) STRICT, WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS photo_walk_held (
        id     TEXT PRIMARY KEY,
        record TEXT NOT NULL
    ) STRICT;
";

impl Library {
    fn walk_tables(&self) -> Result<()> {
        self.conn.execute_batch(WALK_TABLES)?;
        Ok(())
    }

    /// A whole read starting from the beginning: anything an earlier one noted or held is
    /// dropped.
    pub fn begin_photo_walk(&mut self) -> Result<()> {
        self.walk_tables()?;
        self.conn
            .execute_batch("DELETE FROM photo_walk; DELETE FROM photo_walk_held;")?;
        Ok(())
    }

    /// Where the whole read underway carries on from, if one is.
    pub fn photo_walk_cursor(&self) -> Result<Option<String>> {
        self.walk_tables()?;
        Ok(self
            .conn
            .query_row("SELECT id FROM photo_walk WHERE kind = 'cursor'", [], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// The whole read underway, forgotten without letting anything go.
    pub fn forget_photo_walk(&mut self) -> Result<()> {
        self.walk_tables()?;
        self.conn.execute("DELETE FROM photo_walk", [])?;
        Ok(())
    }

    /// What a pass of the whole read stored, and where the next pass carries on from (none:
    /// the read starts over).
    pub fn note_photo_walk(
        &mut self,
        photos: &[String],
        albums: &[String],
        items: &[(String, String)],
        cursor: Option<&str>,
    ) -> Result<()> {
        self.walk_tables()?;
        let tx = self.conn.savepoint()?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO photo_walk (kind, id) VALUES (?1, ?2)")?;
            for id in photos {
                stmt.execute(params!["photo", id])?;
            }
            for id in albums {
                stmt.execute(params!["album", id])?;
            }
            for (album, asset) in items {
                stmt.execute(params!["item", format!("{album}\u{1f}{asset}")])?;
            }
        }
        tx.execute("DELETE FROM photo_walk WHERE kind = 'cursor'", [])?;
        if let Some(cursor) = cursor {
            tx.execute(
                "INSERT INTO photo_walk (kind, id) VALUES ('cursor', ?1)",
                params![cursor],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The whole read reached the end: every photo, album and album's photo it never saw is
    /// gone from iCloud, and goes here.
    pub fn finish_photo_walk(&mut self) -> Result<()> {
        self.walk_tables()?;
        let tx = self.conn.savepoint()?;
        tx.execute_batch(
            "DELETE FROM photos
               WHERE id NOT IN (SELECT id FROM photo_walk WHERE kind = 'photo');
             DELETE FROM photo_albums
               WHERE id NOT IN (SELECT id FROM photo_walk WHERE kind = 'album');
             DELETE FROM photo_album_items
               WHERE album || char(31) || asset
                     NOT IN (SELECT id FROM photo_walk WHERE kind = 'item');
             DELETE FROM photo_walk;",
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records held from an unfinished pass, as JSON, for the next pass to read first.
    pub fn photo_walk_held(&self) -> Result<Vec<String>> {
        self.walk_tables()?;
        let mut stmt = self
            .conn
            .prepare("SELECT record FROM photo_walk_held ORDER BY rowid")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The records held for the next pass, by record name, in place of those held before.
    pub fn hold_for_photo_walk(&mut self, records: &[(String, String)]) -> Result<()> {
        self.walk_tables()?;
        let tx = self.conn.savepoint()?;
        tx.execute("DELETE FROM photo_walk_held", [])?;
        for (id, record) in records {
            tx.execute(
                "INSERT OR REPLACE INTO photo_walk_held (id, record) VALUES (?1, ?2)",
                params![id, record],
            )?;
        }
        tx.commit()?;
        Ok(())
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
            live_size: None,
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

    /// The index and its pages are the whole list's read, cut up: the same photos in the same
    /// order for every view, album, search and burst — sorted any way — whatever the page size,
    /// ties on the same second broken by id as the window breaks them.
    #[test]
    fn the_index_and_its_pages_are_the_list_in_order() {
        let mut lib = Library::in_memory().unwrap();
        let mut all = Vec::new();
        for i in 0..40i64 {
            let mut p = photo(&format!("p{i:02}"), 1_000 + (i % 7) * 10);
            p.added_at = Some(5_000 - i * 3);
            p.favorite = i % 5 == 0;
            p.rating = i % 6;
            p.hidden = i % 11 == 3;
            p.deleted = i % 13 == 4;
            p.kind = if i % 9 == 0 { Kind::Video } else { Kind::Image };
            p.subtype = if i % 8 == 1 { 3 } else { 0 };
            if (20..24).contains(&i) {
                p.burst = Some("B".into());
                p.burst_pick = i == 22;
            }
            p.filename = Some(format!(
                "{}{i}.jpg",
                if i % 2 == 0 { "beach" } else { "city" }
            ));
            all.push(p);
        }
        lib.put_photos(&all).unwrap();
        lib.put_album_items(&[
            ("AL".into(), "p05".into(), 3),
            ("AL".into(), "p01".into(), 1),
            ("AL".into(), "p30".into(), 2),
        ])
        .unwrap();
        let pages = |source: &Source, order: Option<Order>, size: usize| {
            let mut out = Vec::new();
            for at in (0..).step_by(size) {
                let page = lib.photo_page(source, order, at, size).unwrap();
                let n = page.len();
                out.extend(page);
                if n < size {
                    break;
                }
            }
            out
        };
        let check = |source: Source, order: Option<Order>, want: Vec<Photo>| {
            let index = lib.photo_index(&source, order).unwrap();
            let ids: Vec<&str> = want.iter().map(|p| p.id.as_str()).collect();
            assert_eq!(index.ids, ids, "{source:?} {order:?}");
            for size in [1, 3, 7, 500] {
                assert_eq!(
                    pages(&source, order, size),
                    want,
                    "{source:?} {order:?} by {size}"
                );
            }
            for (i, p) in want.iter().enumerate() {
                let m = index.marks[i];
                assert_eq!(m & 3, if p.kind == Kind::Video { 1 } else { 0 });
                assert_eq!(m & 4 != 0, p.favorite);
                assert_eq!(i64::from(m >> 3), p.rating);
            }
            if !want.is_empty() {
                assert_eq!(index.first, want.iter().map(|p| p.taken_at).min());
                assert_eq!(index.last, want.iter().map(|p| p.taken_at).max());
            }
        };
        for view in [
            View::Library,
            View::Favorites,
            View::Videos,
            View::Screenshots,
            View::Recents,
            View::Hidden,
            View::Rated,
            View::RecentlyDeleted,
        ] {
            check(Source::View(view), None, lib.photos(view).unwrap());
        }
        // Sorted as the window sorts: by the key, then by id; Rated and Recently Deleted keep
        // their own order whatever is asked.
        for by in [OrderBy::Taken, OrderBy::Added] {
            for oldest in [false, true] {
                let order = Some(Order { by, oldest });
                let mut want = lib.photos(View::Library).unwrap();
                let key = |p: &Photo| match by {
                    OrderBy::Taken => p.taken_at,
                    OrderBy::Added => p.added_at.unwrap_or(p.taken_at),
                };
                want.sort_by(|a, b| {
                    let k = if oldest {
                        key(a).cmp(&key(b))
                    } else {
                        key(b).cmp(&key(a))
                    };
                    k.then_with(|| a.id.cmp(&b.id))
                });
                check(Source::View(View::Library), order, want);
                check(
                    Source::View(View::Rated),
                    order,
                    lib.photos(View::Rated).unwrap(),
                );
                let index = lib
                    .photo_index(&Source::View(View::Library), order)
                    .unwrap();
                let keyed: Vec<i64> = lib
                    .photos_by_ids(&index.ids)
                    .unwrap()
                    .iter()
                    .map(key)
                    .collect();
                assert_eq!(index.keys, keyed, "grouped by what it is sorted by");
            }
        }
        check(
            Source::Album("AL".into()),
            None,
            lib.album_photos("AL").unwrap(),
        );
        check(Source::Album("none".into()), None, Vec::new());
        check(
            Source::Query("beach".into()),
            None,
            lib.search_photos("beach", 1000).unwrap(),
        );
        check(Source::Query("  ".into()), None, Vec::new());
        check(
            Source::Burst("B".into()),
            None,
            lib.burst_photos("B").unwrap(),
        );
        // Asked for by id: in the order asked, the gone left out, burst sizes filled.
        let got = lib
            .photos_by_ids(&["p22".into(), "gone".into(), "p01".into()])
            .unwrap();
        assert_eq!(
            got.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["p22", "p01"]
        );
        assert_eq!(got[0].burst_count, 4);
    }

    /// Media types as Apple decides them (read from its smart albums on his library): a
    /// screenshot is assetSubtypeV2 3 and a panorama 1 — not every PNG.
    #[test]
    fn the_months_count_and_choose_their_key_photos() {
        let mut lib = Library::in_memory().unwrap();
        // Mid-month at noon UTC, so no time zone moves one into the next month.
        const OCT: i64 = 1_791_028_800; // 3 Oct 2026
        const SEP: i64 = 1_789_473_600; // 15 Sep 2026
        const DEC: i64 = 1_765_800_000; // 15 Dec 2025
        let mut all: Vec<Photo> = (0..9)
            .map(|i| photo(&format!("o{i}"), OCT + i * 2 * 86_400))
            .collect();
        all.extend((0..3).map(|i| Photo {
            favorite: i == 0,
            ..photo(&format!("s{i}"), SEP + i * 3_600)
        }));
        // The middle of December is a video: the still is chosen.
        all.push(photo("d0", DEC));
        all.push(Photo {
            kind: Kind::Video,
            ..photo("d1", DEC + 3_600)
        });
        all.push(Photo {
            hidden: true,
            favorite: true,
            ..photo("dh", DEC + 60)
        });
        all.push(Photo {
            deleted: true,
            ..photo("dx", DEC + 120)
        });
        // A burst of three counts once, as its chosen photo.
        all.extend((0..3).map(|i| Photo {
            burst: Some("B".into()),
            burst_pick: i == 1,
            ..photo(&format!("b{i}"), DEC + 7_200 + i)
        }));
        lib.put_photos(&all).unwrap();

        let months = lib.photo_months().unwrap();
        let at = |y, m| months.iter().find(|x| x.year == y && x.month == m).unwrap();
        assert_eq!(
            months.iter().map(|m| (m.year, m.month)).collect::<Vec<_>>(),
            vec![(2026, 10), (2026, 9), (2025, 12)]
        );
        // No favourite: the one in the middle, with three spread through it (the key not twice).
        let oct = at(2026, 10);
        assert_eq!(
            (oct.count, oct.key.as_str(), oct.key_favorite),
            (9, "o4", false)
        );
        assert_eq!(oct.more, vec!["o6", "o2"]);
        // A favourite stands for its month; too few for more.
        let sep = at(2026, 9);
        assert_eq!(
            (sep.count, sep.key.as_str(), sep.key_favorite),
            (3, "s0", true)
        );
        assert!(sep.more.is_empty());
        // The hidden favourite and the deleted one are not the library's.
        let dec = at(2025, 12);
        assert_eq!((dec.count, dec.videos), (3, 1));
        assert_eq!(dec.key, "d0");

        // Months are this machine's own: 20:00 UTC on 31 October is 1 November in Kolkata.
        lib.put_photos(&[photo("late", 1_793_476_800)]).unwrap();
        lib.set_zone(Some(jiff::tz::TimeZone::get("Asia/Kolkata").unwrap()));
        let months = lib.photo_months().unwrap();
        assert_eq!(
            (months[0].year, months[0].month, months[0].key.as_str()),
            (2026, 11, "late")
        );
        lib.set_zone(Some(jiff::tz::TimeZone::UTC));
        let months = lib.photo_months().unwrap();
        assert_eq!(
            (months[0].year, months[0].month, months[0].count),
            (2026, 10, 10)
        );
    }

    /// Fifty thousand photos over twenty years: the months come in one quick query.
    #[test]
    fn the_months_of_a_large_library_are_quick() {
        let mut lib = Library::in_memory().unwrap();
        let all: Vec<Photo> = (0..50_000i64)
            .map(|i| Photo {
                favorite: i % 997 == 0,
                ..photo(&format!("q{i}"), 1_792_000_000 - i * 12_600)
            })
            .collect();
        lib.put_photos(&all).unwrap();
        let t = std::time::Instant::now();
        let months = lib.photo_months().unwrap();
        let took = t.elapsed();
        eprintln!(
            "photo_months over 50,000: {took:?}, {} months",
            months.len()
        );
        assert_eq!(months.iter().map(|m| m.count).sum::<i64>(), 50_000);
        assert!(months.len() > 200 && months.len() < 245);
        assert!(took.as_millis() < 500, "{took:?}");
    }

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

    /// The whole library over IPC against the index and a first page, on a copy of a real
    /// library (never the real one): `QUINCE_DB_COPY=<dir holding a copy of library.db> cargo
    /// test -p quince-core --release ipc_paging_on_a_copy -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a copy of a real library"]
    fn ipc_paging_on_a_copy() {
        let dir = std::path::PathBuf::from(std::env::var("QUINCE_DB_COPY").unwrap());
        let lib = Library::open(&dir).unwrap();
        let time = |f: &dyn Fn() -> usize| {
            let mut runs: Vec<(std::time::Duration, usize)> = (0..5)
                .map(|_| {
                    let t = std::time::Instant::now();
                    let n = f();
                    (t.elapsed(), n)
                })
                .collect();
            runs.sort();
            runs[2]
        };
        let source = Source::View(View::Library);
        let whole = time(&|| {
            serde_json::to_vec(&lib.photos(View::Library).unwrap())
                .unwrap()
                .len()
        });
        let index = time(&|| {
            serde_json::to_vec(&lib.photo_index(&source, None).unwrap())
                .unwrap()
                .len()
        });
        let first = time(&|| {
            serde_json::to_vec(&lib.photo_page(&source, None, 0, 240).unwrap())
                .unwrap()
                .len()
        });
        let ids = lib.photo_index(&source, None).unwrap().ids;
        let mid = &ids[ids.len() / 2..(ids.len() / 2 + 240).min(ids.len())];
        let deep = time(&|| {
            serde_json::to_vec(&lib.photos_by_ids(mid).unwrap())
                .unwrap()
                .len()
        });
        println!("{} photos", ids.len());
        println!("whole list:      {:>10} bytes {:?}", whole.1, whole.0);
        println!("index:           {:>10} bytes {:?}", index.1, index.0);
        println!("first page:      {:>10} bytes {:?}", first.1, first.0);
        println!("page by id (mid):{:>10} bytes {:?}", deep.1, deep.0);
    }
}
