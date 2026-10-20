//! iCloud Drive as a folder on this computer (quince-vfs), served by the app itself: its
//! signed-in session, its library, and what it learns kept in the library for the window too.
//! Off until it is turned on in Settings. Everything a folder does: files written (kept here,
//! then sent), folders made, renames, moves, deletes into Recently Deleted. See
//! docs/mount-design.md.

use crate::{Auth, Data};
use quince_core::drive::DriveItem;
use quince_vfs::{Fut, Remote, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// Whether the folder is wanted, kept in the library.
const ON_KEY: &str = "drive_folder_on";
/// Where it is mounted, when moved from the default.
const PATH_KEY: &str = "drive_folder_path";
/// How long a file's download link is used before asking Apple for a fresh one.
const LINK_LIFE: Duration = Duration::from_secs(20 * 60);

/// The folders while they are up; dropping one lets its folder go.
#[cfg(target_os = "linux")]
static SESSIONS: Mutex<Vec<(Which, PathBuf, fuser::BackgroundSession)>> = Mutex::new(Vec::new());

/// What the iCloud Drive folder has written and not yet sent, while it is up.
static WRITES: Mutex<Option<Arc<quince_vfs::writes::Writes>>> = Mutex::new(None);

struct AppRemote {
    app: AppHandle,
    links: Arc<Mutex<HashMap<String, (String, Instant)>>>,
}

/// The session's address for one Apple service, with the auth lock let go first.
async fn context(
    app: &AppHandle,
    service: &str,
) -> Result<(reqwest::Client, String, Vec<(String, String)>), String> {
    let auth = app.state::<Auth>();
    let client = auth.client.lock().await;
    let root = client
        .session()
        .service_url(service)
        .ok_or("Apple hasn't offered iCloud Drive for this account.")?;
    Ok((client.http().clone(), root, client.service_params()))
}

impl Remote for AppRemote {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let (app, folder) = (self.app.clone(), folder.to_string());
        Box::pin(async move {
            let (http, root, params) = context(&app, "drivews").await?;
            let _one = crate::DRIVE_PASS.lock().await;
            let items = quince_icloud::drive::fetch_folder(&http, &root, params, &folder)
                .await
                .map_err(|e| e.to_string())?;
            // Kept, so the window's Drive knows it too and the next look needs no network.
            let data = app.state::<Data>();
            let mut library = data.0.lock().await;
            quince_icloud::drive::store_folder(&mut library, &folder, &items)
                .map_err(|e| e.to_string())?;
            Ok(items)
        })
    }

    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        let (app, parent, name) = (self.app.clone(), parent.drivewsid.clone(), name.to_string());
        Box::pin(async move {
            let (http, root, params) = context(&app, "drivews").await?;
            quince_icloud::drive::create_folder(&http, &root, params, &parent, &name)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn rename(&self, item: &DriveItem, name: &str) -> Fut<()> {
        let (app, item, name) = (self.app.clone(), item.clone(), name.to_string());
        Box::pin(async move {
            let (http, root, params) = context(&app, "drivews").await?;
            quince_icloud::drive::rename(&http, &root, params, &item, &name)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn move_to(&self, item: &DriveItem, folder: &DriveItem) -> Fut<()> {
        let (app, item, to) = (self.app.clone(), item.clone(), folder.drivewsid.clone());
        Box::pin(async move {
            let (http, root, params) = context(&app, "drivews").await?;
            quince_icloud::drive::move_to(&http, &root, params, &item, &to)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn upload(&self, folder: &DriveItem, path: &std::path::Path) -> Fut<DriveItem> {
        let (app, folder, path) = (
            self.app.clone(),
            folder.drivewsid.clone(),
            path.to_path_buf(),
        );
        Box::pin(async move {
            let (http, docws, params) = context(&app, "docws").await?;
            let drivews = context(&app, "drivews").await?.1;
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            quince_icloud::drive::upload(&http, &docws, &drivews, params, &folder, &path, progress)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn space(&self) -> Fut<(u64, u64)> {
        let app = self.app.clone();
        Box::pin(async move {
            let (http, setup, params) = {
                let auth = app.state::<Auth>();
                let client = auth.client.lock().await;
                (
                    client.http().clone(),
                    client.setup_url().to_string(),
                    client.service_params(),
                )
            };
            let s = quince_icloud::drive::storage(&http, &setup, params)
                .await
                .map_err(|e| e.to_string())?;
            Ok((s.used_bytes, s.total_bytes))
        })
    }

    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let (app, item) = (self.app.clone(), item.clone());
        Box::pin(async move {
            let (http, root, params) = context(&app, "drivews").await?;
            quince_icloud::drive::delete(&http, &root, params, &item)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn thumbnails(&self, folder: &str) -> Fut<HashMap<String, Vec<u8>>> {
        let (app, folder) = (self.app.clone(), folder.to_string());
        Box::pin(async move { crate::folder_thumbnails(&app, &folder).await })
    }

    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let (app, links, item) = (self.app.clone(), Arc::clone(&self.links), item.clone());
        Box::pin(async move {
            let key = format!("{}~{}", item.drivewsid, item.etag.as_deref().unwrap_or(""));
            let known = links
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .filter(|(_, at)| at.elapsed() < LINK_LIFE)
                .map(|(u, _)| u.clone());
            // A link Apple no longer honours is asked for again, once.
            for fresh in [known.is_none(), true] {
                let url = match (&known, fresh) {
                    (Some(u), false) => u.clone(),
                    _ => {
                        let (http, docws, params) = context(&app, "docws").await?;
                        let u = quince_icloud::drive::download_url(&http, &docws, params, &item)
                            .await
                            .map_err(|e| e.to_string())?;
                        links
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(key.clone(), (u.clone(), Instant::now()));
                        u
                    }
                };
                let http = app.state::<Auth>().client.lock().await.http().clone();
                let r = http
                    .get(&url)
                    .header("Range", format!("bytes={offset}-{}", offset + len - 1))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = r.status().as_u16();
                if matches!(status, 401 | 403 | 404 | 410) && !fresh {
                    continue;
                }
                let whole = status == 200;
                if status != 206 && !whole {
                    return Err(format!("Apple answered {status}"));
                }
                let bytes = r.bytes().await.map_err(|e| e.to_string())?;
                // A server that ignores the range sends the whole file: the part is cut from it.
                return Ok(if whole {
                    let from = usize::try_from(offset)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len());
                    let to = usize::try_from(offset + len)
                        .unwrap_or(usize::MAX)
                        .min(bytes.len());
                    bytes[from..to].to_vec()
                } else {
                    bytes.to_vec()
                });
            }
            Err("Apple would not send the file".into())
        })
    }
}

struct AppStore(AppHandle);

impl Store for AppStore {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        // Called on the folder's own threads, never the async runtime's: waiting is allowed.
        let data = self.0.state::<Data>();
        let library = data.0.blocking_lock();
        library.drive_children(folder).ok()
    }
}

/// The two folders: iCloud Drive and iCloud Photos, each its own switch and place, each
/// needing only its own service (someone with iCloud Photos off still has Drive).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    Drive,
    Photos,
}

impl Which {
    fn on_key(self) -> &'static str {
        match self {
            Which::Drive => ON_KEY,
            Which::Photos => "photos_folder_on",
        }
    }
    fn path_key(self) -> &'static str {
        match self {
            Which::Drive => PATH_KEY,
            Which::Photos => "photos_folder_path",
        }
    }
    /// Its folder's name in the home folder, as iCloud for Windows names them.
    fn folder(self) -> &'static str {
        match self {
            Which::Drive => "iCloud Drive",
            Which::Photos => "iCloud Photos",
        }
    }
}

/// Where a folder goes: as moved, or its own name in the home folder.
fn place(app: &AppHandle, which: Which) -> PathBuf {
    let data = app.state::<Data>();
    let library = data.0.blocking_lock();
    library
        .state(which.path_key())
        .ok()
        .flatten()
        .map(PathBuf::from)
        .or_else(|| app.path().home_dir().ok().map(|h| h.join(which.folder())))
        .unwrap_or_else(|| PathBuf::from(which.folder()))
}

/// What Settings shows: whether it is up, where, and what stopped it if anything did.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FolderStatus {
    pub on: bool,
    pub path: String,
    pub problem: Option<String>,
    /// Files written into it and not yet at Apple (the Drive folder).
    pub waiting: Option<quince_vfs::writes::Waiting>,
}

#[cfg(target_os = "linux")]
fn mounted(which: Which) -> Option<PathBuf> {
    SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .find(|(w, ..)| *w == which)
        .map(|(_, p, _)| p.clone())
}

/// Where every file manager looks for thumbnails (~/.cache/thumbnails).
fn thumbnail_cache(app: &AppHandle) -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| app.path().home_dir().ok().map(|h| h.join(".cache")))
        .map(|c| c.join("thumbnails"))
}

/// A folder brought up at `at`. Never over a folder with something in it: a mount would hide
/// what is there.
#[cfg(target_os = "linux")]
fn start(app: &AppHandle, which: Which, at: &std::path::Path) -> Result<(), String> {
    if mounted(which).is_some() {
        return Ok(());
    }
    if !std::path::Path::new("/dev/fuse").exists() {
        return Err("This computer has no FUSE, which the folder needs. Install your system's fuse3 package.".into());
    }
    if let Ok(mut entries) = std::fs::read_dir(at) {
        if entries.next().is_some() {
            return Err(format!(
                "{} already has files in it. Choose an empty folder.",
                at.display()
            ));
        }
    }
    let rt = tauri::async_runtime::handle().inner().clone();
    let mut fs = match which {
        Which::Drive => {
            let cache = crate::state_dir(app).join("drive-cache");
            let cache = Arc::new(quince_vfs::Cache::new(&cache).map_err(|e| e.to_string())?);
            let remote = Arc::new(AppRemote {
                app: app.clone(),
                links: Arc::new(Mutex::new(HashMap::new())),
            });
            // What is written here waits beside the library (owner-only) until Apple has it.
            quince_vfs::fuse::DriveFs::new(remote, Arc::new(AppStore(app.clone())), cache, rt)
                .with_writes(&crate::state_dir(app).join("drive-writes"))
                .map_err(|e| e.to_string())?
        }
        Which::Photos => {
            // Read through the app's own photo cache; the folder keeps no second copy.
            let unused = crate::state_dir(app).join("photos-folder-cache");
            let cache = Arc::new(quince_vfs::Cache::new(&unused).map_err(|e| e.to_string())?);
            quince_vfs::fuse::DriveFs::new(
                Arc::new(crate::mount_photos::PhotosRemote(app.clone())),
                Arc::new(crate::mount_photos::PhotosStore(app.clone())),
                cache,
                rt,
            )
            .read_only()
        }
    };
    // Pictures go where every file manager looks for thumbnails, so none of them downloads a
    // file to draw one.
    if let Some(thumbs) = thumbnail_cache(app) {
        fs = fs.with_thumbnails(at.to_path_buf(), thumbs);
    }
    if which == Which::Drive {
        *WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = fs.writes();
    }
    let session = quince_vfs::fuse::mount(fs, at)
        .map_err(|e| format!("The folder could not be mounted: {e}"))?;
    SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((which, at.to_path_buf(), session));
    Ok(())
}

#[cfg(target_os = "linux")]
fn stop(which: Which) {
    let gone: Vec<_> = {
        let mut sessions = SESSIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (go, keep): (Vec<_>, Vec<_>) = sessions.drain(..).partition(|(w, ..)| *w == which);
        *sessions = keep;
        go
    };
    if which == Which::Drive {
        *WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    for (_, at, session) in gone {
        drop(session);
        // The empty folder the mount stood on is put away too, as it was made for it.
        let _ = std::fs::remove_dir(at);
    }
}

#[cfg(not(target_os = "linux"))]
fn start(_app: &AppHandle, _which: Which, _at: &std::path::Path) -> Result<(), String> {
    Err("These folders are not on this system yet.".into())
}
#[cfg(not(target_os = "linux"))]
fn stop(_which: Which) {}
#[cfg(not(target_os = "linux"))]
fn mounted(_which: Which) -> Option<PathBuf> {
    None
}

fn status(app: &AppHandle, which: Which, problem: Option<String>) -> FolderStatus {
    let at = mounted(which).unwrap_or_else(|| place(app, which));
    let on = mounted(which).is_some();
    let waiting = if which == Which::Drive && on {
        WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|w| w.waiting())
    } else {
        None
    };
    FolderStatus {
        on,
        path: at.to_string_lossy().into_owned(),
        problem,
        waiting,
    }
}

/// At start: each brought up if it was on.
pub fn resume(app: &AppHandle) {
    for which in [Which::Drive, Which::Photos] {
        let wanted = {
            let data = app.state::<Data>();
            let library = data.0.blocking_lock();
            library.state(which.on_key()).ok().flatten().as_deref() == Some("1")
        };
        if wanted {
            if let Err(e) = start(app, which, &place(app, which)) {
                quiet!("a folder could not come up: {e}");
            }
        }
    }
}

/// When signing out or quitting: both folders go with the session.
pub fn release() {
    stop(Which::Drive);
    stop(Which::Photos);
}

async fn set(app: AppHandle, which: Which, on: bool) -> Result<FolderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let problem = if on {
            start(&app, which, &place(&app, which)).err()
        } else {
            stop(which);
            None
        };
        let data = app.state::<Data>();
        let mut library = data.0.blocking_lock();
        let _ = library.set_state(
            which.on_key(),
            if on && problem.is_none() { "1" } else { "0" },
        );
        drop(library);
        status(&app, which, problem)
    })
    .await
    .map_err(|e| e.to_string())
}

fn open(app: &AppHandle, which: Which) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let at = mounted(which).ok_or("That folder is off.")?;
    app.opener()
        .open_path(at.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn drive_folder_status(app: AppHandle) -> Result<FolderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || status(&app, Which::Drive, None))
        .await
        .map_err(|e| e.to_string())
}

/// Turn the iCloud Drive folder on or off, remembered for the next start.
#[tauri::command]
pub async fn drive_folder_set(on: bool, app: AppHandle) -> Result<FolderStatus, String> {
    set(app, Which::Drive, on).await
}

/// The iCloud Drive folder opened in the system's file manager.
#[tauri::command]
pub async fn drive_folder_open(app: AppHandle) -> Result<(), String> {
    open(&app, Which::Drive)
}

#[tauri::command]
pub async fn photos_folder_status(app: AppHandle) -> Result<FolderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || status(&app, Which::Photos, None))
        .await
        .map_err(|e| e.to_string())
}

/// Turn the iCloud Photos folder on or off, remembered for the next start.
#[tauri::command]
pub async fn photos_folder_set(on: bool, app: AppHandle) -> Result<FolderStatus, String> {
    set(app, Which::Photos, on).await
}

/// The iCloud Photos folder opened in the system's file manager.
#[tauri::command]
pub async fn photos_folder_open(app: AppHandle) -> Result<(), String> {
    open(&app, Which::Photos)
}
