//! iCloud Photos on the phone: the library read from Apple a few pages at a time into the phone's
//! own library (`quince_sync::photos`, the desktop's own steps), a grid read from it as one
//! compact index (ids, dates, marks: what a list of a million needs), and each picture fetched
//! once, at the size shown, into the app's cache, from the link Apple gave (asked again when old).

use crate::{failed, runtime, Account, EngineError};
use quince_core::photos::{Kind, Source, View};
use quince_icloud::photos::Links;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long Apple's download link is trusted before it is asked for again (as the desktop's).
const LINK_LIFE: Duration = Duration::from_secs(45 * 60);

/// Apple's download links by master id, with when they came.
static LINKS: Mutex<Option<HashMap<String, (Links, Instant)>>> = Mutex::new(None);

fn remember(links: HashMap<String, Links>) {
    let mut held = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = held.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    map.retain(|_, (_, at)| at.elapsed() < LINK_LIFE);
    for (master, l) in links {
        map.insert(master, (l, now));
    }
}

fn fresh(master: &str) -> Option<Links> {
    let held = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    held.as_ref()?
        .get(master)
        .filter(|(_, at)| at.elapsed() < LINK_LIFE)
        .map(|(l, _)| l.clone())
}

/// Which photos a grid shows.
#[derive(uniffi::Enum)]
pub enum PView {
    Library,
    Favorites,
    Videos,
    Screenshots,
    Recents,
    RecentlyDeleted,
    Album { id: String },
}

/// A grid's photos as one compact index: ids in order, each one's date (unix seconds), and its
/// marks packed (bits 0–1 the kind: 0 image, 1 video, 2 Live; bit 2 a favorite).
#[derive(uniffi::Record)]
pub struct PIndex {
    pub ids: Vec<String>,
    pub taken: Vec<i64>,
    pub marks: Vec<u8>,
}

/// One photo's details, for the viewer.
#[derive(uniffi::Record)]
pub struct PItem {
    pub id: String,
    pub taken_at: i64,
    pub width: i64,
    pub height: i64,
    pub video: bool,
    pub live: bool,
    pub duration: Option<f64>,
    pub filename: Option<String>,
    pub size: Option<i64>,
    pub favorite: bool,
}

/// An album.
#[derive(uniffi::Record)]
pub struct PAlbum {
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub parent: Option<String>,
}

/// Which picture of a photo: the grid's, the viewer's, the whole one for zooming, or its film.
#[derive(uniffi::Enum)]
pub enum PSize {
    Thumb,
    Medium,
    Full,
    Video,
}

/// What a pass from Apple came to.
#[derive(uniffi::Record)]
pub struct PSync {
    pub records: u32,
    /// Apple has more: the next pass carries on (a large library is read over several).
    pub more: bool,
    /// Apple is still preparing the library: nothing read yet.
    pub preparing: bool,
    pub photos: u32,
}

fn source(v: PView) -> Source {
    match v {
        PView::Library => Source::View(View::Library),
        PView::Favorites => Source::View(View::Favorites),
        PView::Videos => Source::View(View::Videos),
        PView::Screenshots => Source::View(View::Screenshots),
        PView::Recents => Source::View(View::Recents),
        PView::RecentlyDeleted => Source::View(View::RecentlyDeleted),
        PView::Album { id } => Source::Album(id),
    }
}

/// What a call to Apple's Photos needs: the client, CloudKit's address and the session's parameters.
type Context = (reqwest::Client, String, Vec<(String, String)>);

impl Account {
    fn photos_context(&self) -> Result<Context, EngineError> {
        let c = self.client.lock().map_err(failed)?;
        let root = c
            .session()
            .service_url("ckdatabasews")
            .ok_or_else(|| failed("Apple hasn't offered Photos for this account."))?;
        Ok((c.http().clone(), root, c.service_params()))
    }
}

#[uniffi::export]
impl Account {
    /// A pass from Apple's library into the phone's, as the desktop reads it.
    pub fn photos_sync(&self) -> Result<PSync, EngineError> {
        let (lock, _) = self.library()?;
        let (http, root, params) = self.photos_context()?;
        let pass = runtime()
            .block_on(quince_sync::photos::refresh(&lock, &http, &root, params))
            .map_err(failed)?;
        remember(pass.links);
        let photos = runtime()
            .block_on(async {
                lock.lock()
                    .await
                    .photo_index(&Source::View(View::Library), None)
            })
            .map_err(failed)?
            .ids
            .len();
        Ok(PSync {
            records: u32::try_from(pass.records).unwrap_or(u32::MAX),
            more: pass.more,
            preparing: pass.preparing,
            photos: u32::try_from(photos).unwrap_or(u32::MAX),
        })
    }

    /// A grid's photos, newest first, as one compact index.
    pub fn photo_index(&self, view: PView) -> Result<PIndex, EngineError> {
        let (lock, _) = self.library()?;
        let index = runtime()
            .block_on(async { lock.lock().await.photo_index(&source(view), None) })
            .map_err(failed)?;
        Ok(PIndex {
            ids: index.ids,
            taken: index.keys,
            marks: index.marks,
        })
    }

    /// One photo's details.
    pub fn photo_item(&self, id: String) -> Result<Option<PItem>, EngineError> {
        let (lock, _) = self.library()?;
        let found = runtime()
            .block_on(async { lock.lock().await.photo(&id) })
            .map_err(failed)?;
        Ok(found.map(|p| PItem {
            video: p.kind == Kind::Video,
            live: p.kind == Kind::Live,
            id: p.id,
            taken_at: p.taken_at,
            width: p.width,
            height: p.height,
            duration: p.duration,
            filename: p.filename,
            size: p.size,
            favorite: p.favorite,
        }))
    }

    /// The albums, in Apple's order.
    pub fn photo_albums(&self) -> Result<Vec<PAlbum>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.albums() })
            .map_err(failed)?
            .into_iter()
            .map(|a| PAlbum {
                id: a.id,
                name: a.name,
                folder: a.folder,
                parent: a.parent,
            })
            .collect())
    }

    /// A photo's picture at `size` fetched to `path` (the app's cache), once: a path that has
    /// it already is left as it is. False when Apple has no picture of that size.
    pub fn photo_file(&self, id: String, size: PSize, path: String) -> Result<bool, EngineError> {
        let target = std::path::PathBuf::from(&path);
        if target.exists() {
            return Ok(true);
        }
        let (lock, _) = self.library()?;
        let photo = runtime()
            .block_on(async { lock.lock().await.photo(&id) })
            .map_err(failed)?
            .ok_or_else(|| failed("That photo is no longer in the library."))?;
        let (http, root, params) = self.photos_context()?;
        let pick = |l: &Links| match size {
            PSize::Thumb => l.thumb.clone().or_else(|| l.medium.clone()),
            PSize::Medium => l.medium.clone().or_else(|| l.full.clone()),
            PSize::Full => l.full.clone().or_else(|| l.medium.clone()),
            PSize::Video => l.video.clone(),
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        let part = target.with_extension("part");
        runtime().block_on(async {
            // A link kept from the pass first; a fresh one from Apple when there is none or it
            // is refused.
            if let Some(url) = fresh(&photo.master).as_ref().and_then(pick) {
                if quince_icloud::photos::fetch_to_file(&http, &url, &part)
                    .await
                    .unwrap_or(false)
                {
                    return std::fs::rename(&part, &target)
                        .map(|()| true)
                        .map_err(failed);
                }
            }
            let links =
                quince_icloud::photos::links(&http, &root, params, None, &photo.master, &photo.id)
                    .await
                    .map_err(failed)?
                    .ok_or_else(|| failed("Apple no longer has that photo."))?;
            remember(HashMap::from([(photo.master.clone(), links.clone())]));
            let Some(url) = pick(&links) else {
                return Ok(false);
            };
            if !quince_icloud::photos::fetch_to_file(&http, &url, &part)
                .await
                .map_err(failed)?
            {
                return Ok(false);
            }
            std::fs::rename(&part, &target)
                .map(|()| true)
                .map_err(failed)
        })
    }
}
