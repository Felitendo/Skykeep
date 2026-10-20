//! iCloud Photos as a folder (quince-vfs, read-only for now), laid out as people look for
//! photos: by when they were taken, in their albums, the favourites, the videos.
//!
//!   Library/2026/10 October/IMG_1234.HEIC
//!   Albums/Trips/Goa/IMG_0042.JPG
//!   Favorites/…   Videos/…
//!
//! Each file is the original, under its own name and dated when it was taken. Its bytes are
//! read from the app's own photo cache (fetched once if they are not there), never copied a
//! second time; thumbnails are the app's own, so a file manager browsing downloads nothing.
//! Hidden and Recently Deleted are never shown.

use crate::photos::{cache_path, cached, Size};
use crate::Data;
use quince_core::drive::DriveItem;
use quince_core::photos::{Photo, View};
use quince_vfs::{Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const LIBRARY: &str = "PH::library";
const ALBUMS: &str = "PH::albums";
const FAVORITES: &str = "PH::favorites";
const VIDEOS: &str = "PH::videos";
const FILE: &str = "PHOTO::";

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn folder(id: String, parent: &str, name: String) -> DriveItem {
    DriveItem {
        drivewsid: id,
        parent_id: Some(parent.to_string()),
        name,
        extension: None,
        kind: "FOLDER".into(),
        size: None,
        child_count: None,
        created_at: None,
        modified_at: None,
        etag: None,
    }
}

/// A photo as a file: its own name (or one made from its id and kind), its size, the time it
/// was taken.
fn file(p: &Photo, parent: &str) -> DriveItem {
    let name = p.filename.clone().unwrap_or_else(|| {
        let ext = match p.kind {
            quince_core::photos::Kind::Video => "MOV",
            _ => "JPG",
        };
        format!("{}.{ext}", p.id)
    });
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
        _ => (name, None),
    };
    DriveItem {
        drivewsid: format!("{FILE}{}", p.id),
        parent_id: Some(parent.to_string()),
        name: stem,
        extension: ext,
        kind: "FILE".into(),
        size: p.size,
        child_count: None,
        created_at: p.added_at,
        modified_at: Some(p.taken_at),
        etag: p.change_tag.clone(),
    }
}

/// (year, month 1–12) a photo was taken in, in this computer's time zone.
fn taken(p: &Photo) -> (i16, i8) {
    let zone = jiff::tz::TimeZone::system();
    jiff::Timestamp::from_second(p.taken_at)
        .map(|t| t.to_zoned(zone).date())
        .map_or((1970, 1), |d| (d.year(), d.month()))
}

/// What a folder holds, read from the library (the app keeps it in step with Apple).
fn children(app: &AppHandle, id: &str) -> Option<Vec<DriveItem>> {
    let data = app.state::<Data>();
    let library = data.0.blocking_lock();
    let photos = |view: View| library.photos(view).unwrap_or_default();
    Some(match id {
        ROOT => vec![
            folder(LIBRARY.into(), ROOT, "Library".into()),
            folder(ALBUMS.into(), ROOT, "Albums".into()),
            folder(FAVORITES.into(), ROOT, "Favorites".into()),
            folder(VIDEOS.into(), ROOT, "Videos".into()),
        ],
        LIBRARY => {
            let mut years: Vec<i16> = photos(View::Library).iter().map(|p| taken(p).0).collect();
            years.sort_unstable();
            years.dedup();
            years
                .into_iter()
                .rev()
                .map(|y| folder(format!("PH::y:{y}"), LIBRARY, y.to_string()))
                .collect()
        }
        FAVORITES => photos(View::Favorites)
            .iter()
            .map(|p| file(p, FAVORITES))
            .collect(),
        VIDEOS => photos(View::Videos)
            .iter()
            .map(|p| file(p, VIDEOS))
            .collect(),
        ALBUMS => albums_in(&library, None, ALBUMS),
        _ => {
            if let Some(y) = id.strip_prefix("PH::y:") {
                let y: i16 = y.parse().ok()?;
                let mut months: Vec<i8> = photos(View::Library)
                    .iter()
                    .filter(|p| taken(p).0 == y)
                    .map(|p| taken(p).1)
                    .collect();
                months.sort_unstable();
                months.dedup();
                months
                    .into_iter()
                    .map(|m| {
                        let name =
                            format!("{m:02} {}", MONTHS[usize::try_from(m - 1).unwrap_or(0)]);
                        folder(format!("PH::m:{y}-{m}"), id, name)
                    })
                    .collect()
            } else if let Some(ym) = id.strip_prefix("PH::m:") {
                let (y, m) = ym.split_once('-')?;
                let (y, m): (i16, i8) = (y.parse().ok()?, m.parse().ok()?);
                photos(View::Library)
                    .iter()
                    .filter(|p| taken(p) == (y, m))
                    .map(|p| file(p, id))
                    .collect()
            } else {
                let album = id.strip_prefix("PH::album:")?;
                let is_folder = library
                    .albums()
                    .ok()?
                    .iter()
                    .any(|a| a.id == album && a.folder);
                if is_folder {
                    albums_in(&library, Some(album), id)
                } else {
                    library
                        .album_photos(album)
                        .ok()?
                        .iter()
                        .filter(|p| !p.hidden && !p.deleted)
                        .map(|p| file(p, id))
                        .collect()
                }
            }
        }
    })
}

/// The albums (and album folders) directly inside `parent`, in Photos' own order.
fn albums_in(library: &quince_core::Library, parent: Option<&str>, at: &str) -> Vec<DriveItem> {
    let mut albums: Vec<_> = library
        .albums()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.parent.as_deref() == parent)
        .collect();
    albums.sort_by_key(|a| a.position);
    albums
        .into_iter()
        .map(|a| folder(format!("PH::album:{}", a.id), at, a.name))
        .collect()
}

pub struct PhotosStore(pub AppHandle);

impl Store for PhotosStore {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        children(&self.0, folder)
    }
}

pub struct PhotosRemote(pub AppHandle);

impl Remote for PhotosRemote {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        // Nothing to ask Apple: the library is kept in step by the app's own sync.
        let (app, folder) = (self.0.clone(), folder.to_string());
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                children(&app, &folder).unwrap_or_default()
            })
            .await
            .map_err(|e| e.to_string())
        })
    }

    fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
        // Every photo is read from the app's cache (file): never through the folder's own.
        Box::pin(async { Err("read through the photo cache".to_string()) })
    }

    fn file(&self, item: &DriveItem) -> Option<Fut<PathBuf>> {
        let id = item.drivewsid.strip_prefix(FILE)?.to_string();
        let app = self.0.clone();
        Some(Box::pin(
            async move { cached(&app, &id, Size::Original).await },
        ))
    }

    fn is_local(&self, item: &DriveItem) -> bool {
        item.drivewsid
            .strip_prefix(FILE)
            .is_some_and(|id| cache_path(&self.0, id, Size::Original).exists())
    }

    fn thumbnails(&self, folder: &str) -> Fut<HashMap<String, Vec<u8>>> {
        let (app, folder) = (self.0.clone(), folder.to_string());
        Box::pin(async move {
            let (a, f) = (app.clone(), folder.clone());
            let files =
                tauri::async_runtime::spawn_blocking(move || children(&a, &f).unwrap_or_default())
                    .await
                    .map_err(|e| e.to_string())?;
            let mut out = HashMap::new();
            for item in files.iter().filter(|i| i.kind == "FILE") {
                let Some(id) = item.drivewsid.strip_prefix(FILE) else {
                    continue;
                };
                if let Ok(path) = cached(&app, id, Size::Thumb).await {
                    if let Ok(bytes) = tokio::fs::read(&path).await {
                        out.insert(item.drivewsid.clone(), bytes);
                    }
                }
            }
            Ok(out)
        })
    }
}
