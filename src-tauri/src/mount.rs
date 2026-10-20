//! iCloud Drive as a folder on this computer (quince-vfs), served by the app itself: its
//! signed-in session, its library, and what it learns kept in the library for the window too.
//! Off until it is turned on in Settings; read-only for now. See docs/mount-design.md.

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

/// The mount while it is up; dropping it lets the folder go.
#[cfg(target_os = "linux")]
static SESSION: Mutex<Option<(PathBuf, fuser::BackgroundSession)>> = Mutex::new(None);

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

/// Where the folder goes: as moved, or "iCloud Drive" in the home folder, where iCloud for
/// Windows puts it in a user's folder.
fn place(app: &AppHandle) -> PathBuf {
    let data = app.state::<Data>();
    let library = data.0.blocking_lock();
    library
        .state(PATH_KEY)
        .ok()
        .flatten()
        .map(PathBuf::from)
        .or_else(|| app.path().home_dir().ok().map(|h| h.join("iCloud Drive")))
        .unwrap_or_else(|| PathBuf::from("iCloud Drive"))
}

/// What Settings shows: whether it is up, where, and what stopped it if anything did.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FolderStatus {
    pub on: bool,
    pub path: String,
    pub problem: Option<String>,
}

#[cfg(target_os = "linux")]
fn mounted() -> Option<PathBuf> {
    SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|(p, _)| p.clone())
}

/// The folder brought up at `at`. Never over a folder with something in it: a mount would
/// hide what is there.
#[cfg(target_os = "linux")]
fn start(app: &AppHandle, at: &std::path::Path) -> Result<(), String> {
    if mounted().is_some() {
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
    let cache = crate::state_dir(app).join("drive-cache");
    let cache = Arc::new(quince_vfs::Cache::new(&cache).map_err(|e| e.to_string())?);
    let remote = Arc::new(AppRemote {
        app: app.clone(),
        links: Arc::new(Mutex::new(HashMap::new())),
    });
    let store = Arc::new(AppStore(app.clone()));
    let rt = tauri::async_runtime::handle().inner().clone();
    let fs = quince_vfs::fuse::DriveFs::new(remote, store, cache, rt);
    let session = quince_vfs::fuse::mount(fs, at)
        .map_err(|e| format!("The folder could not be mounted: {e}"))?;
    *SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((at.to_path_buf(), session));
    Ok(())
}

#[cfg(target_os = "linux")]
fn stop() {
    let gone = SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some((at, session)) = gone {
        drop(session);
        // The empty folder the mount stood on is put away too, as it was made for it.
        let _ = std::fs::remove_dir(at);
    }
}

#[cfg(not(target_os = "linux"))]
fn start(_app: &AppHandle, _at: &std::path::Path) -> Result<(), String> {
    Err("The iCloud Drive folder is not on this system yet.".into())
}
#[cfg(not(target_os = "linux"))]
fn stop() {}
#[cfg(not(target_os = "linux"))]
fn mounted() -> Option<PathBuf> {
    None
}

fn status(app: &AppHandle, problem: Option<String>) -> FolderStatus {
    let at = mounted().unwrap_or_else(|| place(app));
    FolderStatus {
        on: mounted().is_some(),
        path: at.to_string_lossy().into_owned(),
        problem,
    }
}

/// At start: brought up if it was on.
pub fn resume(app: &AppHandle) {
    let wanted = {
        let data = app.state::<Data>();
        let library = data.0.blocking_lock();
        library.state(ON_KEY).ok().flatten().as_deref() == Some("1")
    };
    if wanted {
        if let Err(e) = start(app, &place(app)) {
            quiet!("the iCloud Drive folder could not come up: {e}");
        }
    }
}

/// When signing out: the folder goes with the session.
pub fn release() {
    stop();
}

#[tauri::command]
pub async fn drive_folder_status(app: AppHandle) -> Result<FolderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || status(&app, None))
        .await
        .map_err(|e| e.to_string())
}

/// Turn the folder on or off, remembered for the next start.
#[tauri::command]
pub async fn drive_folder_set(on: bool, app: AppHandle) -> Result<FolderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let problem = if on {
            start(&app, &place(&app)).err()
        } else {
            stop();
            None
        };
        let data = app.state::<Data>();
        let mut library = data.0.blocking_lock();
        let _ = library.set_state(ON_KEY, if on && problem.is_none() { "1" } else { "0" });
        drop(library);
        status(&app, problem)
    })
    .await
    .map_err(|e| e.to_string())
}

/// The folder opened in the system's file manager.
#[tauri::command]
pub async fn drive_folder_open(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let at = mounted().ok_or("The iCloud Drive folder is off.")?;
    app.opener()
        .open_path(at.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}
