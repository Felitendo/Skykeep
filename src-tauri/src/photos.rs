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
    /// The whole picture, displayable: for a photo zoomed in.
    Full,
    Video,
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Size::Thumb => "thumb",
            Size::Medium => "medium",
            Size::Original => "original",
            Size::Full => "full",
            Size::Video => "video",
        }
    }
    fn pick(self, l: &Links) -> Option<String> {
        match self {
            Size::Thumb => l.thumb.clone(),
            Size::Medium => l.medium.clone().or_else(|| l.thumb.clone()),
            Size::Original => l.original.clone(),
            Size::Full => l.full.clone().or_else(|| l.medium.clone()),
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
        "full" => Size::Full,
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
    let counted = data
        .0
        .lock()
        .await
        .albums_counted()
        .map_err(|e| e.to_string())?;
    Ok(counted
        .into_iter()
        .map(|(a, n)| (a, usize::try_from(n).unwrap_or(0)))
        .collect())
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

/// A new album, first in the list as Photos puts one. Answers it as Apple made it.
#[tauri::command]
pub async fn photos_album_create(
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<quince_core::photos::Album> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let first = data
        .0
        .lock()
        .await
        .albums()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|a| a.position)
        .min();
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let album = quince_icloud::photos::create_album(&http, &root, params, name, first)
        .await
        .map_err(|e| e.to_string())?;
    data.0
        .lock()
        .await
        .put_album(&album)
        .map_err(|e| e.to_string())?;
    Ok(album)
}

/// An album renamed, against the change tag last seen.
#[tauri::command]
pub async fn photos_album_rename(
    id: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut album = album_of(&data, &id).await?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let tag = quince_icloud::photos::rename_album(
        &http,
        &root,
        params,
        &id,
        album.change_tag.as_deref(),
        name,
    )
    .await
    .map_err(|e| e.to_string())?;
    album.name = name.to_string();
    album.change_tag = tag.or(album.change_tag);
    data.0
        .lock()
        .await
        .put_album(&album)
        .map_err(|e| e.to_string())
}

/// An album deleted. Its photos stay in the library.
#[tauri::command]
pub async fn photos_album_delete(
    id: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    let album = album_of(&data, &id).await?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    quince_icloud::photos::delete_album(&http, &root, params, &id, album.change_tag.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    data.0
        .lock()
        .await
        .forget_album(&id)
        .map_err(|e| e.to_string())
}

/// Photos put in an album after what is there. One already in it is not added twice.
#[tauri::command]
pub async fn photos_album_add(
    album: String,
    ids: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    album_of(&data, &album).await?;
    let (fresh, last) = {
        let library = data.0.lock().await;
        let inside: std::collections::HashSet<String> = library
            .album_photos(&album)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let fresh: Vec<String> = ids.into_iter().filter(|i| !inside.contains(i)).collect();
        let last = library
            .album_last_position(&album)
            .map_err(|e| e.to_string())?;
        (fresh, last)
    };
    if fresh.is_empty() {
        return Ok(());
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let added =
        quince_icloud::photos::add_to_album(&http, &root, params, &album, &fresh, last).await;
    // What went in before a failure is in the album on Apple; the library says so too.
    let placed = match &added {
        Ok(a) => a.clone(),
        Err(_) => Vec::new(),
    };
    data.0
        .lock()
        .await
        .put_album_items(
            &placed
                .into_iter()
                .map(|(asset, at)| (album.clone(), asset, at))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
    added.map(|_| ()).map_err(|e| e.to_string())
}

/// Photos taken out of an album. They stay in the library.
#[tauri::command]
pub async fn photos_album_remove(
    album: String,
    ids: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    quince_icloud::photos::remove_from_album(&http, &root, params, &album, &ids)
        .await
        .map_err(|e| e.to_string())?;
    data.0
        .lock()
        .await
        .forget_album_items(
            &ids.into_iter()
                .map(|i| (album.clone(), i))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())
}

async fn album_of(data: &State<'_, Data>, id: &str) -> UiResult<quince_core::photos::Album> {
    data.0
        .lock()
        .await
        .albums()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "That album is no longer there.".into())
}

/// Favorite, Hide, Delete (to Recently Deleted), Recover, or — from Recently Deleted only —
/// delete for good, as icloud.com does it: the asset's one field updated against its change tag. The library takes Apple's
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
    // Deleting for good is only ever from Recently Deleted, and only ever on: there is no
    // taking it back, so nothing else may reach it.
    if flag == quince_icloud::photos::Flag::Expunged && (!on || !photo.deleted) {
        return Err("Only a photo in Recently Deleted can be deleted for good.".into());
    }
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
            // Apple's own date for when it goes for good, or its thirty days until it says.
            updated.expunge_at = on.then(|| {
                answer
                    .pointer("/fields/dateExpunged/value")
                    .and_then(serde_json::Value::as_i64)
                    .map_or(jiff::Timestamp::now().as_second() + 30 * 86_400, |ms| {
                        ms / 1000
                    })
            });
        }
        quince_icloud::photos::Flag::Expunged => {
            data.0
                .lock()
                .await
                .forget_photos(std::slice::from_ref(&id))
                .map_err(|e| e.to_string())?;
            return Ok(updated);
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

/// How many photos each view holds. Asked after every sync, which is also when an upload queue
/// paused for a session coming back starts again.
#[tauri::command]
pub async fn photos_counts(app: AppHandle, data: State<'_, Data>) -> UiResult<Vec<(View, i64)>> {
    work(app);
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

/// Where Photos uploads tell the window how they are going: the same shape Drive's tray reads.
pub const PHOTO_UPLOAD: &str = "quince://photo-upload";

#[derive(Clone, serde::Serialize)]
struct UploadEvent {
    id: String,
    batch: String,
    name: String,
    within: String,
    sent: u64,
    total: u64,
    state: &'static str,
    folder: bool,
    error: Option<String>,
}

fn say(
    app: &AppHandle,
    u: &quince_core::photos::Upload,
    state: &'static str,
    sent: u64,
    total: u64,
    error: Option<String>,
) {
    use tauri::Emitter;
    app.emit(
        PHOTO_UPLOAD,
        UploadEvent {
            id: u.id.clone(),
            batch: u.batch.clone(),
            name: u.name.clone(),
            within: String::new(),
            sent,
            total,
            state,
            folder: false,
            error,
        },
    )
    .ok();
}

/// One worker at a time drains the queue, as icloud.com uploads one file at a time.
static WORKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Batches, and single uploads, asked to stop.
static CANCELLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The upload on its way, so Cancel can stop it mid-file rather than after it lands.
static CURRENT: Mutex<Option<(String, String, futures_util::future::AbortHandle)>> =
    Mutex::new(None);

fn cancelled(u: &quince_core::photos::Upload) -> bool {
    CANCELLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|b| *b == u.batch || *b == u.id)
}

/// Files (and the photos and videos inside folders) added to Photos. Each is checked against
/// Photos' limits first; a file Photos would refuse is answered at once with why, and never
/// sent. Answers the batch, whose progress comes as `quince://photo-upload` events.
#[tauri::command]
pub async fn photos_upload(
    paths: Vec<String>,
    app: AppHandle,
    data: State<'_, Data>,
) -> UiResult<String> {
    let batch = uuid::Uuid::new_v4().to_string();
    let mut files = Vec::new();
    for p in paths {
        let path = PathBuf::from(&p);
        if path.is_dir() {
            let mut stack = vec![path];
            while let Some(dir) = stack.pop() {
                let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                    continue;
                };
                while let Ok(Some(e)) = entries.next_entry().await {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        files.push(p);
                    }
                }
            }
        } else {
            files.push(path);
        }
    }
    files.sort();
    let now = jiff::Timestamp::now().as_second();
    let mut queued = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut u = quince_core::photos::Upload {
            id: uuid::Uuid::new_v4().to_string(),
            batch: batch.clone(),
            path: path.to_string_lossy().into_owned(),
            name: name.clone(),
            state: "waiting".into(),
            error: None,
            asset: None,
            added_at: now,
        };
        if let Some(r) = quince_icloud::photos_upload::refusal(&name, size) {
            u.state = "failed".into();
            u.error = Some(r.words());
        }
        queued.push((u, size));
    }
    if queued.is_empty() {
        return Err("There are no photos or videos there to add.".into());
    }
    data.0
        .lock()
        .await
        .queue_uploads(&queued.iter().map(|(u, _)| u.clone()).collect::<Vec<_>>())
        .map_err(|e| e.to_string())?;
    for (u, size) in &queued {
        say(
            &app,
            u,
            if u.state == "failed" {
                "failed"
            } else {
                "waiting"
            },
            0,
            *size,
            u.error.clone(),
        );
    }
    if let Some((u, _)) = queued.first() {
        say(&app, u, "planned", 0, 0, None);
    }
    work(app);
    Ok(batch)
}

/// Stop uploads, by id or a whole batch: what is waiting is not sent, and the file on its way
/// is abandoned mid-send. A photo Apple has already made stays (it is in the library, and can
/// be deleted there).
#[tauri::command]
pub async fn photos_upload_cancel(
    ids: Vec<String>,
    batch: Option<String>,
    app: AppHandle,
    data: State<'_, Data>,
) -> UiResult<()> {
    let wanted = |id: &str, b: &str| ids.iter().any(|i| i == id) || batch.as_deref() == Some(b);
    CANCELLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .extend(ids.iter().cloned().chain(batch.clone()));
    if let Some((id, b, handle)) = CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        if wanted(id, b) {
            handle.abort();
        }
    }
    let library = data.0.lock().await;
    for u in library.unfinished_uploads().map_err(|e| e.to_string())? {
        if wanted(&u.id, &u.batch) && u.state == "waiting" {
            library
                .set_upload(&u.id, "cancelled", None, None)
                .map_err(|e| e.to_string())?;
            say(&app, &u, "cancelled", 0, 0, None);
        }
    }
    Ok(())
}

/// The queue drained, one file at a time; started on demand and at launch to resume what a
/// quit left half done.
pub fn work(app: AppHandle) {
    if WORKING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let next = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library
                    .unfinished_uploads()
                    .ok()
                    .and_then(|u| u.into_iter().next())
            };
            let Some(u) = next else { break };
            if !one(&app, u).await {
                break;
            }
        }
        WORKING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// One upload, to its end. False when the queue must pause (the session lapsed).
async fn one(app: &AppHandle, u: quince_core::photos::Upload) -> bool {
    let set = |state: &str, error: Option<&str>, asset: Option<&str>| {
        let data = app.state::<Data>();
        let (id, state, error, asset) = (
            u.id.clone(),
            state.to_string(),
            error.map(str::to_string),
            asset.map(str::to_string),
        );
        async move {
            data.0
                .lock()
                .await
                .set_upload(&id, &state, error.as_deref(), asset.as_deref())
                .ok();
        }
    };
    if cancelled(&u) {
        set("cancelled", None, None).await;
        say(app, &u, "cancelled", 0, 0, None);
        return true;
    }
    let auth = app.state::<Auth>();
    let (http, ck, params, upload_root) = {
        let client = auth.client.lock().await;
        let s = client.session();
        (
            client.http().clone(),
            s.service_url("ckdatabasews"),
            client.service_params(),
            s.service_url("photosupload"),
        )
    };
    // Not signed in yet (a launch resumes before the session is back): the queue waits, and
    // is started again by the next sync.
    let (Some(ck), Some(upload_root)) = (ck, upload_root) else {
        return false;
    };
    // Resumed after a quit: a photo Apple had already named is not made twice.
    if let Some(asset) = u.asset.as_deref() {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            &http,
            &ck,
            quince_icloud::photos::CONTAINER,
            params.clone(),
        );
        if matches!(
            zc.lookup_record(quince_icloud::photos::ZONE, asset).await,
            Ok(Some(_))
        ) {
            set("done", None, Some(asset)).await;
            say(app, &u, "done", 1, 1, None);
            refresh_photos(app).await;
            return true;
        }
    }
    let path = PathBuf::from(&u.path);
    let Ok(meta) = tokio::fs::metadata(&path).await else {
        set("failed", Some("That file is no longer there."), None).await;
        say(
            app,
            &u,
            "failed",
            0,
            0,
            Some("That file is no longer there.".into()),
        );
        return true;
    };
    let size = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    let zone = jiff::tz::TimeZone::system();
    let zone_name = zone.iana_name().unwrap_or("UTC").to_string();
    let offset = -i64::from(jiff::Zoned::now().offset().seconds()) / 60;
    let dsid = params
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    set("sending", None, None).await;
    say(app, &u, "sending", 0, size, None);

    let mut attempt = 0;
    loop {
        attempt += 1;
        // Cancelled while waiting to try again.
        if attempt > 1 && cancelled(&u) {
            set("cancelled", None, None).await;
            say(app, &u, "cancelled", 0, size, None);
            return true;
        }
        let app2 = app.clone();
        let u2 = u.clone();
        let last = std::sync::Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
        let progress: std::sync::Arc<dyn Fn(quince_icloud::photos_upload::Stage) + Send + Sync> =
            std::sync::Arc::new(move |stage| {
                use quince_icloud::photos_upload::Stage;
                match stage {
                    Stage::Sending(sent, total) => {
                        // A few times a second at most: every piece would flood the window.
                        let mut at = last
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if sent == total || at.elapsed() > Duration::from_millis(120) {
                            *at = Instant::now();
                            say(&app2, &u2, "sending", sent, total, None);
                        }
                    }
                    Stage::Named(made) => {
                        let (app3, id) = (app2.clone(), u2.id.clone());
                        tauri::async_runtime::spawn(async move {
                            let data = app3.state::<Data>();
                            data.0
                                .lock()
                                .await
                                .set_upload(&id, "verifying", None, Some(&made.asset))
                                .ok();
                        });
                    }
                    Stage::Processing(done) => {
                        say(
                            &app2,
                            &u2,
                            "verifying",
                            (done.clamp(0.0, 100.0) as u64) * size / 100,
                            size,
                            None,
                        );
                    }
                }
            });
        let (handle, registration) = futures_util::future::AbortHandle::new_pair();
        *CURRENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((u.id.clone(), u.batch.clone(), handle));
        let result = futures_util::future::Abortable::new(
            quince_icloud::photos_upload::upload(
                &http,
                &upload_root,
                &dsid,
                &path,
                &u.name,
                modified,
                &zone_name,
                offset,
                &u.batch,
                progress,
            ),
            registration,
        )
        .await;
        CURRENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Ok(result) = result else {
            set("cancelled", None, None).await;
            say(app, &u, "cancelled", 0, size, None);
            // Apple may already have made it; the library shows what is true.
            refresh_photos(app).await;
            return true;
        };
        use quince_icloud::photos_upload::{Outcome, Refusal};
        match result {
            Ok(Outcome::Uploaded(made)) => {
                set("done", None, Some(&made.asset)).await;
                say(app, &u, "done", size, size, None);
                refresh_photos(app).await;
                return true;
            }
            Ok(Outcome::Refused(Refusal::Duplicate)) => {
                set("done", Some("Already in your library."), None).await;
                say(app, &u, "done", size, size, None);
                return true;
            }
            Ok(Outcome::Refused(r)) => {
                set("failed", Some(&r.words()), None).await;
                say(app, &u, "failed", 0, size, Some(r.words()));
                return true;
            }
            Err(quince_icloud::Error::ReauthRequired) => {
                // Not a failure of this file: the queue waits for the session to come back.
                set("waiting", None, None).await;
                say(
                    app,
                    &u,
                    "waiting",
                    0,
                    size,
                    Some("Waiting for iCloud to sign back in.".into()),
                );
                return false;
            }
            Err(e) if attempt < 3 && !cancelled(&u) => {
                eprintln!("photos upload of {}: try {attempt} failed: {e}", u.name);
                tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
            }
            Err(e) => {
                let words = e.to_string();
                set("failed", Some(&words), None).await;
                say(app, &u, "failed", 0, size, Some(words));
                return true;
            }
        }
    }
}

async fn refresh_photos(app: &AppHandle) {
    use tauri::Emitter;
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    crate::refresh::some(&auth.client, &data.0, &["photos"]).await;
    app.emit(crate::SYNCED, crate::refresh::status(&data.0).await)
        .ok();
}
