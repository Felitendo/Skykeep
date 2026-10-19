//! Photos in the app: the library's commands, the pictures served to the window from a disk
//! cache, and the download links Apple hands out (which expire) kept in memory only.
//!
//! A picture is fetched from Apple once, the first time it is drawn, and kept under the
//! app's state directory by size; after that the grid is drawn from disk. A link older than
//! LINK_LIFE, or one this run never saw, is asked of Apple again before a fetch.

use crate::{Auth, Data, UiResult};
use quince_core::photos::{Photo, View};
use quince_icloud::photos::Links;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

/// How long a download link from Apple is trusted before it is asked for again.
const LINK_LIFE: Duration = Duration::from_secs(45 * 60);

/// Every master's links seen this run, and when.
static LINKS: Mutex<Option<HashMap<String, (Links, Instant)>>> = Mutex::new(None);

/// Links a pass brought, kept for the pictures it will be asked for.
pub fn remember(links: HashMap<String, Links>) {
    let now = Instant::now();
    let mut kept = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = kept.get_or_insert_with(HashMap::new);
    for (master, l) in links {
        map.insert(master, (l, now));
    }
}

fn fresh_links(master: &str) -> Option<Links> {
    let kept = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    kept.as_ref()?
        .get(master)
        .filter(|(_, at)| at.elapsed() < LINK_LIFE)
        .map(|(l, _)| l.clone())
}

/// Which of a photo's pictures is wanted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Thumb,
    Medium,
    Original,
    Video,
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Size::Thumb => "thumb",
            Size::Medium => "medium",
            Size::Original => "original",
            Size::Video => "video",
        }
    }
    fn pick(self, l: &Links) -> Option<String> {
        match self {
            Size::Thumb => l.thumb.clone(),
            Size::Medium => l.medium.clone().or_else(|| l.thumb.clone()),
            Size::Original => l.original.clone(),
            Size::Video => l.video.clone(),
        }
    }
}

/// The photo and picture a `quince://` request asks for: `pic/<asset id>/<size>`, escaped as
/// Tauri escapes it.
pub fn requested(path: &str) -> Option<(String, Size)> {
    let path = crate::percent_decode(path.trim_start_matches('/'));
    let rest = path.strip_prefix("pic/")?;
    let (id, size) = rest.rsplit_once('/')?;
    let size = match size {
        "thumb" => Size::Thumb,
        "medium" => Size::Medium,
        "original" => Size::Original,
        "video" => Size::Video,
        _ => return None,
    };
    // An asset id is a record name: letters, digits and dashes only, never a path.
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some((id.to_string(), size))
}

fn cache_path(app: &AppHandle, id: &str, size: Size) -> PathBuf {
    crate::state_dir(app)
        .join("photos")
        .join(size.name())
        .join(id)
}

/// One picture of a photo: from the disk cache, or fetched from Apple once and kept.
pub async fn picture(app: &AppHandle, id: &str, size: Size) -> Result<Vec<u8>, String> {
    let path = cache_path(app, id, size);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(bytes);
    }
    let photo = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library
            .photo(id)
            .map_err(|e| e.to_string())?
            .ok_or("That photo is no longer in the library.")?
    };
    let auth = app.state::<Auth>();
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    // A cached link first; a fresh one from Apple when it has none or Apple refuses the old.
    let fetch = |url: String| {
        let http = http.clone();
        async move {
            let r = http.get(&url).send().await.map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err(format!("Apple answered {}", r.status()));
            }
            r.bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| e.to_string())
        }
    };
    let cached = fresh_links(&photo.master).and_then(|l| size.pick(&l));
    let bytes = match cached {
        Some(url) => match fetch(url).await {
            Ok(b) => b,
            Err(_) => fetch(new_link(&http, &root, params, &photo, size).await?).await?,
        },
        None => fetch(new_link(&http, &root, params, &photo, size).await?).await?,
    };
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    // Written beside and moved into place, so a half-written picture is never served.
    let partial = path.with_extension("part");
    if tokio::fs::write(&partial, &bytes).await.is_ok() {
        tokio::fs::rename(&partial, &path).await.ok();
    }
    Ok(bytes)
}

async fn new_link(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    photo: &Photo,
    size: Size,
) -> Result<String, String> {
    let links = quince_icloud::photos::links(http, root, params, &photo.master)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Apple no longer has that photo.")?;
    remember(HashMap::from([(photo.master.clone(), links.clone())]));
    size.pick(&links)
        .ok_or_else(|| "That photo has no picture of that size.".into())
}

/// What a picture's bytes are, for the window to draw them.
pub fn content_type(bytes: &[u8], size: Size) -> &'static str {
    if size == Size::Video || bytes.get(4..8) == Some(b"ftyp") {
        return "video/mp4";
    }
    crate::image_type(bytes)
}

/// A view's photos, newest first.
#[tauri::command]
pub async fn photos_list(view: View, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0.lock().await.photos(view).map_err(|e| e.to_string())
}

/// The albums the person made, each with how many photos it holds.
#[tauri::command]
pub async fn photos_albums(
    data: State<'_, Data>,
) -> UiResult<Vec<(quince_core::photos::Album, usize)>> {
    let library = data.0.lock().await;
    let albums = library.albums().map_err(|e| e.to_string())?;
    albums
        .into_iter()
        .map(|a| {
            let n = library
                .album_photos(&a.id)
                .map_err(|e| e.to_string())?
                .len();
            Ok((a, n))
        })
        .collect()
}

/// One album's photos, in its order.
#[tauri::command]
pub async fn photos_in_album(album: String, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0
        .lock()
        .await
        .album_photos(&album)
        .map_err(|e| e.to_string())
}

/// Favorite, Delete (to Recently Deleted, never for good) or Recover a photo, as icloud.com
/// does it: the asset's one field updated against its change tag. The library takes Apple's
/// answer at once, so every view shows it without waiting for the next pass.
#[tauri::command]
pub async fn photo_set_flag(
    id: String,
    flag: quince_icloud::photos::Flag,
    on: bool,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    let _one = crate::PHOTOS_PASS.lock().await;
    let photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let answer = quince_icloud::photos::set_flag(
        &http,
        &root,
        params,
        &id,
        photo.change_tag.as_deref(),
        flag,
        on,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut updated = photo;
    match flag {
        quince_icloud::photos::Flag::Favorite => updated.favorite = on,
        quince_icloud::photos::Flag::Hidden => updated.hidden = on,
        quince_icloud::photos::Flag::Deleted => {
            updated.deleted = on;
            updated.deleted_at = on.then(|| jiff::Timestamp::now().as_second());
        }
    }
    updated.change_tag = answer
        .get("recordChangeTag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(updated.change_tag);
    data.0
        .lock()
        .await
        .put_photos(std::slice::from_ref(&updated))
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

/// A video opened in this computer's own player. WebKitGTK plays nothing without GStreamer's
/// codec plugins, which many systems (his among them) do not have, so a video is fetched once,
/// kept beside the other pictures under its own name, and handed to the system's player.
#[tauri::command]
pub async fn photo_play(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<()> {
    let photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let size = if photo.kind == quince_core::photos::Kind::Video {
        Size::Original
    } else {
        Size::Video
    };
    let bytes = picture(&app, &id, size).await?;
    let name = photo
        .filename
        .filter(|n| !n.contains('/') && !n.starts_with('.'))
        .unwrap_or_else(|| format!("{id}.mp4"));
    let dir = crate::state_dir(&app).join("photos").join("open").join(&id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let path = dir.join(name);
    if tokio::fs::metadata(&path).await.is_err() {
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| e.to_string())?;
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("No app on this computer opens that video: {e}"))
}

/// How many photos each view holds.
#[tauri::command]
pub async fn photos_counts(data: State<'_, Data>) -> UiResult<Vec<(View, i64)>> {
    data.0
        .lock()
        .await
        .photo_counts()
        .map_err(|e| e.to_string())
}

/// A photo's original saved to Downloads under its own name; answers where.
#[tauri::command]
pub async fn photo_download(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<String> {
    let photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let bytes = picture(&app, &id, Size::Original).await?;
    let dir = app
        .path()
        .download_dir()
        .map_err(|_| "There is no Downloads folder to save to.")?;
    let name = photo
        .filename
        .filter(|n| !n.contains('/') && !n.starts_with('.'))
        .unwrap_or_else(|| format!("Photo {}", &id[..id.len().min(8)]));
    let path = crate::free_path(&dir, &name);
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
