// Quince — desktop shell.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// What went wrong, said on the terminal in a development build only, and never with anything
/// of the person's in it — no names, record ids, addresses or Apple's answers. A release build
/// says nothing at all: Quince keeps no diagnostics and sends none anywhere.
macro_rules! quiet {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($t)*);
        }
    };
}

mod alerts;
mod cpu;
mod login;
mod mount;
mod mount_photos;
mod outbox;
mod photos;
#[cfg(target_os = "linux")]
mod poster;
#[cfg(not(target_os = "linux"))]
mod poster {
    //! Elsewhere than Linux there is no GStreamer to read a frame with: Apple's own poster stands.
    pub fn first_seen_frame(_path: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}
mod push;
mod refresh;
mod video;

use quince_auth::{Client, SignInOutcome, TwoFactorOptions};
use quince_core::calendar::{Calendar, Event};
use quince_core::contacts::Contact;
use quince_core::drive::DriveItem;
use quince_core::notes::{Note, NoteFolder};
use quince_core::reminders::{Reminder, ReminderList, SmartList};
use quince_core::Library;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

struct Auth {
    client: Arc<Mutex<Client>>,
    /// A sign-in is in flight. The mutex would serialise a second click rather than reject
    /// it, so it would run a whole second SRP exchange and trigger another code push.
    signing_in: AtomicBool,
}

/// Clears the in-flight flag however the guarded block exits.
struct InFlight<'a>(&'a AtomicBool);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// A stored session exists and is worth trying to resume.
    has_stored_session: bool,
    /// Apple's service map is loaded, i.e. we are actually signed in right now.
    signed_in: bool,
}

struct Data(Mutex<Library>);

/// Errors cross to the UI as plain sentences a person can act on.
type UiResult<T> = Result<T, String>;

/// The HTTP client, CloudKit root and query parameters, with the auth lock released before
/// any network work so a slow sync cannot block the heartbeat or a sign-out.
/// Re-read reminders after a write, so what is on screen is what Apple stored.
///
/// A list change moves records other than the one written, so this is a full pass rather
/// than a delta.
async fn resync_reminders(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
    refresh_reminders(http, root, params, data, true).await
}

/// Ask Apple what changed in Reminders and store it — without holding the library while
/// Apple answers.
///
/// Every screen reads that library. A refresh that held it across the network made every
/// screen wait on Apple: when CloudKit was slow or overloaded (503, retry in 10s), Reminders,
/// Notes and the rest drew nothing at all until it gave up. The fetch runs unlocked; the
/// library is held only to store what came back.
/// One reminders pass at a time, for the reason [`NOTES_PASS`] gives: an older pass storing
/// after a newer write put the reminder back as it was before the write.
pub static REMINDERS_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One calendar pass or write at a time. A refresh that read Apple before a save and stored
/// after it pruned the new event, put an edited one back with its old etag — so the next
/// edit failed as a conflict — and brought a just-deleted one back.
pub static CALENDAR_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One contacts pass or write at a time, for the reason [`CALENDAR_PASS`] gives: a refresh
/// that read Apple before a save and stored after it put the old card back, and the next
/// edit, made from it, was refused as a conflict.
pub static CONTACTS_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One Drive folder fetched and stored at a time. A folder opened just before a rename is
/// fetched with the old name; finishing after the rename's own re-read, it stored that old
/// listing last and the rename looked undone. Held from fetch to store, the re-read after a
/// write — which starts only once the write is made — always lands last.
pub static DRIVE_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// One Photos pass at a time, from reading its sync position through storing what came back.
pub static PHOTOS_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn refresh_reminders(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
    whole: bool,
) -> Result<(), String> {
    let _one = REMINDERS_PASS.lock().await;
    let since = if whole {
        None
    } else {
        data.0
            .lock()
            .await
            .sync_token("reminders")
            .map_err(|e| e.to_string())?
    };
    let fetched = quince_icloud::reminders::fetch_cloudkit(http, root, params, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    {
        let mut library = data.0.lock().await;
        quince_icloud::reminders::apply_cloudkit(&mut library, fetched)
            .map_err(|e| e.to_string())?;
        // Changes still on their way to Apple stay as made here.
        outbox::replay_reminders(&mut library).map_err(|e| e.to_string())?;
    }
    alerts::changed();
    Ok(())
}

/// The same for Notes.
/// One notes pass at a time, held from reading the sync position through storing the answer.
///
/// Passes fetch without holding the library, so two could overlap — the timer's and the one
/// after a save — and the slower, older one would store last: text from before the save put
/// back over it, and a note just made here pruned as unknown. This is not the library's lock;
/// every other screen keeps reading while a pass is out.
pub static NOTES_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn refresh_notes(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
    let _one = NOTES_PASS.lock().await;
    let since = data
        .0
        .lock()
        .await
        .sync_token("notes")
        .map_err(|e| e.to_string())?;
    let fetched = quince_icloud::notes::fetch_cloudkit(http, root, params, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    quince_icloud::notes::apply_cloudkit(&mut library, fetched)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn cloudkit_context(
    auth: &State<'_, Auth>,
) -> Result<(reqwest::Client, String, Vec<(String, String)>), String> {
    let client = auth.client.lock().await;
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or("Apple hasn't offered CloudKit for this account.")?;
    Ok((client.http().clone(), root, client.service_params()))
}

/// Undo percent-encoding in a path segment. Enough for the ids this scheme carries.
/// The note attachment, and which of its pictures, a `quince://` request asks for:
/// `note/<attachment id>/preview` or `note/<attachment id>/file`, escaped as Tauri escapes it.
fn note_part(path: &str) -> Option<(String, quince_icloud::note_media::Part)> {
    let path = percent_decode(path.trim_start_matches('/'));
    let rest = path.strip_prefix("note/")?;
    let (id, part) = rest.rsplit_once('/')?;
    let part = match part {
        "preview" => quince_icloud::note_media::Part::Preview,
        "file" => quince_icloud::note_media::Part::File,
        "drawing" | "drawing-dark" => quince_icloud::note_media::Part::Drawing,
        _ => return None,
    };
    (!id.is_empty()).then(|| (id.to_string(), part))
}

/// A note attachment's picture, from disk when it has been fetched before, from Apple when
/// not — kept, so a note opens on its pictures at once the next time.
async fn note_media_cached(
    app: &tauri::AppHandle,
    id: &str,
    part: quince_icloud::note_media::Part,
) -> Result<Vec<u8>, String> {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = match part {
        quince_icloud::note_media::Part::Preview => "preview",
        quince_icloud::note_media::Part::File => "file",
        quince_icloud::note_media::Part::Drawing => "drawing",
    };
    let path = state_dir(app)
        .join("note-media")
        .join(format!("{safe}-{name}"));
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(bytes);
    }
    let (http, root, params) = {
        let auth = app.state::<Auth>();
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("ckdatabasews")
            .ok_or("no CloudKit")?;
        (client.http().clone(), root, client.service_params())
    };
    let bytes = quince_icloud::note_media::bytes(&http, &root, params, id, part)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    tokio::fs::write(&path, &bytes).await.ok();
    Ok(bytes)
}

/// A note's picture as the window can draw it: a HEIC original (what an iPhone takes) made a
/// JPEG on this computer once and kept beside it — the engine draws no HEIC, and the picture
/// fell back to a file chip. Opening it elsewhere still hands over the original.
///
/// A drawing is shown as its ink alone, for the page's look (`dark`): see
/// `quince_media::drawing_ink`.
async fn note_media_shown(
    app: &tauri::AppHandle,
    id: &str,
    part: quince_icloud::note_media::Part,
    dark: bool,
) -> Result<Vec<u8>, String> {
    let bytes = note_media_cached(app, id, part).await?;
    let drawing = part == quince_icloud::note_media::Part::Drawing;
    if !drawing && !quince_media::is_heif(&bytes) {
        return Ok(bytes);
    }
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = match (drawing, dark) {
        (true, true) => format!("{safe}-ink-dark.png"),
        (true, false) => format!("{safe}-ink.png"),
        _ => format!("{safe}-shown.jpg"),
    };
    let shown = state_dir(app).join("note-media").join(name);
    if let Ok(made) = tokio::fs::read(&shown).await {
        return Ok(made);
    }
    let jpeg = crate::cpu::run(move || {
        if drawing {
            quince_media::drawing_ink(&bytes, dark)
        } else {
            quince_media::heic_to_jpeg(&bytes)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    let part_path = shown.with_extension("part");
    if tokio::fs::write(&part_path, &jpeg).await.is_ok() {
        let _ = tokio::fs::rename(&part_path, &shown).await;
    }
    Ok(jpeg)
}

/// A plain report of how this install is doing, for a bug report: versions, the system, and
/// each service's last sync and last error — never a name, an address, or anything from a
/// note, card or file. Written to Downloads for the person to read and send themselves;
/// Quince sends nothing anywhere.
#[tauri::command]
async fn diagnostics_save(app: tauri::AppHandle, data: State<'_, Data>) -> UiResult<String> {
    let health = {
        let library = data.0.lock().await;
        library.sync_health().map_err(|e| e.to_string())?
    };
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| {
            t.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                l.trim_start_matches("PRETTY_NAME=")
                    .trim_matches('"')
                    .to_string()
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    // Addresses and ids that an error might quote are taken out.
    let scrub = |t: &str| -> String {
        t.split_whitespace()
            .map(|w| {
                if w.contains('@') || w.len() > 24 && w.chars().any(|c| c.is_ascii_digit()) {
                    "[…]"
                } else {
                    w
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut out = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(out, "Quince diagnostic report");
    let _ = writeln!(
        out,
        "Quince {} · {} · {} {}",
        app.package_info().version,
        os,
        std::env::consts::ARCH,
        kernel.trim()
    );
    let _ = writeln!(
        out,
        "Desktop: {} ({})",
        if desktop.is_empty() {
            "unknown"
        } else {
            &desktop
        },
        if session.is_empty() {
            "unknown"
        } else {
            &session
        }
    );
    let _ = writeln!(out, "Written: {}", jiff::Timestamp::now());
    let _ = writeln!(out, "\nSync, by service:");
    for h in &health {
        let when = h
            .last_synced_at
            .and_then(|t| jiff::Timestamp::from_second(t).ok())
            .map_or_else(|| "never".into(), |t| t.to_string());
        let _ = writeln!(
            out,
            "  {:10} last synced {when}{}",
            h.service,
            h.last_error
                .as_deref()
                .map(|e| format!(" — last error: {}", scrub(e)))
                .unwrap_or_default()
        );
    }
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "Quince diagnostics {}.txt",
        jiff::Zoned::now().strftime("%Y-%m-%d %H.%M")
    ));
    tokio::fs::write(&path, out)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

/// What a note's attachment is, to draw it where it sits.
#[tauri::command]
async fn note_attachment_info(
    id: String,
    auth: State<'_, Auth>,
) -> UiResult<quince_icloud::note_media::AttachmentInfo> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::note_media::info(&http, &root, params, &id)
        .await
        .map_err(|e| e.to_string())
}

/// Open a note's attachment in the app this computer uses for it: the file is fetched,
/// saved beside the others under its own name, and handed over.
#[tauri::command]
async fn note_attachment_open(
    id: String,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let info = quince_icloud::note_media::info(&http, &root, params, &id)
        .await
        .map_err(|e| e.to_string())?;
    let bytes = note_media_cached(&app, &id, quince_icloud::note_media::Part::File).await?;
    let name = info
        .filename
        .filter(|n| !n.contains('/') && !n.starts_with('.'))
        .unwrap_or_else(|| format!("Attachment {}", &id[..id.len().min(8)]));
    let dir = state_dir(&app).join("note-files").join(
        id.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
    );
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let path = dir.join(name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| e.to_string())?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// The contact a `quince://` request asks for the photo of.
///
/// WebKitGTK reads `quince://photo/<id>` with "photo" as the host, while Tauri's own helper
/// builds `quince://localhost/photo%2F<id>` — the whole path escaped, slash and all. The path
/// is decoded before it is read: matched undecoded, no request ever found its photo, and every
/// contact showed initials with nothing to say why.
fn photo_guid(host: Option<&str>, path: &str) -> Option<String> {
    let path = percent_decode(path.trim_start_matches('/'));
    let guid = if host == Some("photo") {
        path
    } else {
        path.strip_prefix("photo/")?.to_string()
    };
    (!guid.is_empty()).then_some(guid)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Everything Quince keeps — the library, every photo, the session's cookies — in a folder only
/// its owner can open. Made with the system's usual permissions it was readable by every user
/// of the computer, and the cookies are a signed-in iCloud session. Everything inside is covered
/// at once: nothing in a folder can be reached without opening the folder.
fn keep_private(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
}

fn state_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".").join("quince"))
}

#[tauri::command]
async fn auth_status(auth: State<'_, Auth>) -> UiResult<Status> {
    let client = auth.client.lock().await;
    Ok(Status {
        has_stored_session: client.has_stored_session(),
        signed_in: client.session().webservices.is_some(),
    })
}

/// Try the stored session. `false` just means a sign-in is needed — not an error.
#[tauri::command]
async fn auth_resume(app: tauri::AppHandle, auth: State<'_, Auth>) -> UiResult<&'static str> {
    let outcome = {
        let mut client = auth.client.lock().await;
        let outcome = resume_session(&mut client).await?;
        if outcome == "library" {
            signed_in(&app, &mut client).await?;
        }
        outcome
    };
    if outcome == "library" {
        folders_up(&app).await;
    }
    Ok(outcome)
}

async fn resume_session(client: &mut Client) -> UiResult<&'static str> {
    let resumed = match client.resume().await {
        Ok(resumed) => resumed,
        // Offline with a session kept: the library opens on what it holds, as a local-first
        // app should, and the next refresh — on the timer, or when the network is back —
        // signs back in.
        Err(quince_auth::Error::Network(_)) if client.has_stored_session() => true,
        Err(e) => return Err(e.to_string()),
    };
    // Back in; or back in but for the code Apple wants before trusting this computer again;
    // or out, with nothing kept to sign in with.
    Ok(if resumed {
        "library"
    } else if client.awaiting_code() {
        "code"
    } else {
        "signin"
    })
}

/// The folders on this computer brought up, once the library is the signed-in account's: they
/// are served by the session, so they are never up without one.
async fn folders_up(app: &tauri::AppHandle) {
    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || mount::resume(&app)).await;
}

/// Emitted when a lapsed session signed itself back in only as far as Apple's code.
const NEEDS_CODE: &str = "quince://needs-code";

/// Make sure the session is alive before a refresh uses it, signing back in if it lapsed.
///
/// A window left open past the life of Apple's cookies, or of its session token, had every
/// refresh failing quietly from then on. Checked here, a lapse is mended before it is felt;
/// one that needs a code brings the code screen up instead.
async fn keep_signed_in(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>) {
    let mut guard = client.lock().await;
    if guard.session().webservices.is_none() {
        return;
    }
    if matches!(guard.heartbeat().await, Ok(false)) && guard.awaiting_code() {
        app.emit(NEEDS_CODE, ()).ok();
    }
}

/// Whose data the library holds: a hash of Apple's account id, never the id itself.
pub const ACCOUNT_KEY: &str = "account";

/// What of an account lives in the data folder beside the session: its library, and every
/// cache and queue made from it. Moved as one when another account signs in.
const ACCOUNT_FILES: [&str; 12] = [
    "library.db",
    "library.db-wal",
    "library.db-shm",
    "photos",
    "photos-folder-cache",
    "drive-cache",
    "drive-writes",
    "photos-folder-writes",
    "thumbnails",
    "note-files",
    "note-media",
    "faces",
];

/// Where the accounts not signed in are kept, each under its own tag.
const SET_ASIDE: &str = "accounts";

/// An account as the library names it.
pub fn account_tag(dsid: &str) -> String {
    use sha2::Digest;
    format!(
        "{:x}",
        sha2::Sha256::digest(format!("quince-account:{dsid}"))
    )
}

/// The library made the signed-in account's, before anything else can use the session: called
/// with the auth lock still held, so no sync, send or upload ever sees one account's session
/// beside another's library.
///
/// Someone else's library here — a sign-out kept a copy, then another Apple ID signed in — is
/// set aside whole under `accounts/`, never deleted and never mixed, and this account's own,
/// if it was set aside before, comes back where it was: its records, its caches, what it had
/// still to send. A library from before accounts were told apart is simply claimed.
async fn claim_library(app: &tauri::AppHandle, client: &Client) -> Result<(), String> {
    let Some(dsid) = client.session().dsid.as_deref() else {
        return Ok(());
    };
    let tag = account_tag(dsid);
    let data = app.state::<Data>();
    let mut library = data.0.lock().await;
    let known = library.state(ACCOUNT_KEY).map_err(|e| e.to_string())?;
    if known.as_deref() == Some(tag.as_str()) {
        return Ok(());
    }
    let dir = state_dir(app);
    let home = dir.join(SET_ASIDE).join(&tag);
    if known.is_some() || home.exists() {
        let now = jiff::Timestamp::now().as_second();
        let away = known.unwrap_or_else(|| format!("unclaimed-{now}"));
        switch_library(&mut library, &dir, &away, &home)?;
        photos::forget_links();
    }
    library
        .set_state(ACCOUNT_KEY, &tag)
        .map_err(|e| e.to_string())
}

/// One account's library and caches set aside under `away`, and the ones at `home` (if any)
/// brought back in their place. The library is let go of while its files move and opened
/// again after, whatever happened; this computer's settings stay with the computer.
fn switch_library(
    library: &mut Library,
    dir: &std::path::Path,
    away: &str,
    home: &std::path::Path,
) -> Result<(), String> {
    let kept = library.computer_state().map_err(|e| e.to_string())?;
    let empty = !library.holds_account().map_err(|e| e.to_string())?;
    let shelf = dir.join(SET_ASIDE).join(away);
    *library = Library::in_memory().map_err(|e| e.to_string())?;
    let moved = move_account(dir, &shelf).and_then(|()| {
        if home.exists() {
            move_account(home, dir)?;
            std::fs::remove_dir(home)?;
        }
        Ok(())
    });
    *library = Library::open(dir).map_err(|e| e.to_string())?;
    for (key, value) in kept {
        library.set_state(&key, &value).map_err(|e| e.to_string())?;
    }
    moved.map_err(|e| format!("The last account's data could not be set aside: {e}"))?;
    // A library that held nothing — one emptied by a sign-out that kept nothing — is not kept.
    if empty && away.starts_with("unclaimed-") {
        let _ = std::fs::remove_dir_all(&shelf);
    }
    Ok(())
}

/// An account's files moved from one folder to another, all or none: whatever had moved goes
/// back if one cannot, and nothing is moved over something already there.
fn move_account(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    if let Some(taken) = ACCOUNT_FILES.iter().find(|n| to.join(n).exists()) {
        return Err(std::io::Error::other(format!("{taken} is already there")));
    }
    let mut done = Vec::new();
    for name in ACCOUNT_FILES {
        if !from.join(name).exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(from.join(name), to.join(name)) {
            for back in done {
                let _ = std::fs::rename(to.join(back), from.join(back));
            }
            return Err(e);
        }
        done.push(name);
    }
    Ok(())
}

/// Whether the session and the library are the same account's — asked by everything that sends
/// what the library holds to Apple, so that nothing of one account is ever sent with another's
/// session (a switch that failed half-way, a session left from a crash).
pub async fn same_account(app: &tauri::AppHandle) -> bool {
    let session = app
        .state::<Auth>()
        .client
        .lock()
        .await
        .session()
        .dsid
        .as_deref()
        .map(account_tag);
    let owner = app
        .state::<Data>()
        .0
        .lock()
        .await
        .state(ACCOUNT_KEY)
        .ok()
        .flatten();
    session.is_some() && session == owner
}

/// The account the session is signed in to now, as the library records its owner.
async fn account_now(auth: &Auth) -> Option<String> {
    auth.client
        .lock()
        .await
        .session()
        .dsid
        .as_deref()
        .map(account_tag)
}

/// Whether the library still belongs to `account`, taken when a command began: a result that
/// came back after a sign-out, or another account's sign-in, is never stored over its data.
fn still_owned(library: &quince_core::Library, account: Option<&str>) -> Result<(), String> {
    let owner = library.state(ACCOUNT_KEY).ok().flatten();
    if account.is_some() && owner.as_deref() == account {
        Ok(())
    } else {
        Err("Signed out while this was on its way, so nothing was stored.".into())
    }
}

/// Signed in, then the library made this account's; if it could not be, signed out again with
/// the reason, never left signed in over someone else's data.
async fn signed_in(app: &tauri::AppHandle, client: &mut Client) -> Result<(), String> {
    if let Err(e) = claim_library(app, client).await {
        let _ = client.sign_out();
        return Err(e);
    }
    Ok(())
}

#[tauri::command]
async fn auth_sign_in(
    apple_id: String,
    password: String,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
) -> UiResult<&'static str> {
    // Reject rather than queue. A second attempt would mean a second code sent to the
    // user's devices and another step toward Apple's rate limit.
    if auth.signing_in.swap(true, Ordering::AcqRel) {
        return Err("Already signing in.".into());
    }
    let _guard = InFlight(&auth.signing_in);

    let mut client = auth.client.lock().await;
    match client.sign_in(&apple_id, &password).await {
        Ok(SignInOutcome::Complete) => {
            signed_in(&app, &mut client).await?;
            drop(client);
            folders_up(&app).await;
            Ok("complete")
        }
        Ok(SignInOutcome::NeedsTwoFactor) => Ok("needs_two_factor"),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn auth_submit_code(
    code: String,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client
        .submit_two_factor(&code)
        .await
        .map_err(|e| e.to_string())?;
    signed_in(&app, &mut client).await?;
    drop(client);
    folders_up(&app).await;
    Ok(())
}

/// Where Apple can send a code, and whether it already pushed one.
/// A `trustedDeviceCount` of zero means nothing was sent and waiting is pointless.
#[tauri::command]
async fn auth_two_factor_options(auth: State<'_, Auth>) -> UiResult<TwoFactorOptions> {
    let mut client = auth.client.lock().await;
    client.two_factor_options().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_resend_code(auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client.resend_to_devices().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_send_sms(phone_id: u64, auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client.send_sms(phone_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_submit_sms_code(
    phone_id: u64,
    code: String,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client
        .submit_sms_code(phone_id, &code)
        .await
        .map_err(|e| e.to_string())?;
    signed_in(&app, &mut client).await?;
    drop(client);
    folders_up(&app).await;
    Ok(())
}

/// One search across every app's library — notes, reminders, events, contacts, files, photos —
/// the first few of each and how many in all. Offline: read from the library alone.
#[tauri::command]
async fn search_everything(
    query: String,
    each: Option<usize>,
    data: State<'_, Data>,
) -> UiResult<quince_core::search::Everything> {
    let library = data.0.lock().await;
    // A few of each kind, or more of one when its section is shown whole.
    library
        .search_everything(&query, each.unwrap_or(5).clamp(1, 200))
        .map_err(|e| e.to_string())
}

/// What of the signed-in account's has not reached Apple yet: asked before a sign-out, so a
/// choice to keep nothing on this computer is made knowing what it loses.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Unsent {
    /// Changes queued for Apple (a reminder ticked offline, say).
    changes: usize,
    /// Files written in the iCloud Drive folder and not yet uploaded.
    files: usize,
    /// Photos added and not yet uploaded.
    photos: usize,
}

#[tauri::command]
async fn account_unsent(app: tauri::AppHandle, data: State<'_, Data>) -> UiResult<Unsent> {
    let (changes, photos) = {
        let library = data.0.lock().await;
        (
            library.outbox_count().map_err(|e| e.to_string())?,
            library
                .unfinished_uploads()
                .map_err(|e| e.to_string())?
                .len(),
        )
    };
    Ok(Unsent {
        changes,
        photos,
        files: mount::unsent_files(&state_dir(&app)),
    })
}

/// Signed out, keeping a copy of the account here or not, as the person chose.
///
/// Kept, everything stays as it was — the library, its caches, what waits to be sent — and is
/// there again when the account signs back in; another account signing in first finds it set
/// aside, not mixed in (see `claim_library`). Not kept, every record and cached file of the
/// account is removed from this computer. Either way nothing is deleted from iCloud.
#[tauri::command]
async fn auth_sign_out(keep: bool, app: tauri::AppHandle, auth: State<'_, Auth>) -> UiResult<()> {
    // The folders go with the session they were served by — before the auth lock is taken,
    // since what they are still answering may be waiting on it.
    tauri::async_runtime::spawn_blocking(mount::release)
        .await
        .map_err(|e| e.to_string())?;
    let mut client = auth.client.lock().await;
    client.sign_out().map_err(|e| e.to_string())?;
    if !keep {
        let data = app.state::<Data>();
        let mut library = data.0.lock().await;
        library.forget_account().map_err(|e| e.to_string())?;
        let dir = state_dir(&app);
        for name in ACCOUNT_FILES
            .iter()
            .filter(|n| !n.starts_with("library.db"))
        {
            let _ = std::fs::remove_dir_all(dir.join(name));
        }
        photos::forget_links();
        drop(library);
        drop(client);
        let app = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || mount::forget_thumbnails(&app)).await;
    }
    Ok(())
}

/// Services Apple advertises for this account, for the sidebar.
/// Who is signed in, as Apple names them: for the account in the sidebar and in Settings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountInfo {
    name: Option<String>,
    email: Option<String>,
    /// `false` when the account keeps its data from the web: nothing can be read until
    /// the person turns on "Access iCloud Data on the Web".
    web_access_allowed: Option<bool>,
    /// Advanced Data Protection.
    data_protection: Option<bool>,
    /// The person's own card, when it has a photo: theirs to show as the account's picture,
    /// served like any contact's.
    photo_contact: Option<String>,
}

#[tauri::command]
async fn account_info(auth: State<'_, Auth>, data: State<'_, Data>) -> UiResult<AccountInfo> {
    // The picture is the person's own card's — the card Contacts marks as theirs.
    let photo_contact = {
        let library = data.0.lock().await;
        library
            .state(quince_icloud::contacts::ME_KEY)
            .ok()
            .flatten()
            .filter(|guid| !guid.is_empty())
            .filter(|guid| library.contact_photo_source(guid).ok().flatten().is_some())
    };
    let client = auth.client.lock().await;
    let s = client.session();
    Ok(AccountInfo {
        name: s.account_name.clone(),
        email: s.account_email.clone().or_else(|| s.apple_id.clone()),
        web_access_allowed: s.web_access_allowed,
        data_protection: s.data_protection,
        photo_contact,
    })
}

/// Settings' switches, as the backend obeys them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    reminder_alerts: bool,
    event_alerts: bool,
}

#[tauri::command]
async fn settings_get(data: State<'_, Data>) -> UiResult<Settings> {
    let library = data.0.lock().await;
    Ok(Settings {
        reminder_alerts: alerts::allowed(&library, alerts::REMINDER_ALERTS),
        event_alerts: alerts::allowed(&library, alerts::EVENT_ALERTS),
    })
}

/// Turn a kind of alert on or off. Only the switches Settings shows can be set.
#[tauri::command]
async fn settings_set(key: String, on: bool, data: State<'_, Data>) -> UiResult<()> {
    let state = match key.as_str() {
        "reminderAlerts" => alerts::REMINDER_ALERTS,
        "eventAlerts" => alerts::EVENT_ALERTS,
        _ => return Err(format!("There is no setting called {key}.")),
    };
    let mut library = data.0.lock().await;
    library
        .set_state(state, if on { "on" } else { "off" })
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_services(auth: State<'_, Auth>) -> UiResult<Vec<String>> {
    let client = auth.client.lock().await;
    Ok(client
        .session()
        .webservices
        .as_ref()
        .and_then(|w| w.as_object())
        .map(|o| {
            let mut keys: Vec<String> = o.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default())
}

/// Keep the session warm.
///
/// Apple resets auth cookies after a long idle gap, so a session left untouched can die well
/// before its nominal life. Roughly hourly contact is what keeps it alive in practice; 45
/// minutes leaves margin without being chatty enough to look like polling.
fn spawn_heartbeat(client: Arc<Mutex<Client>>) {
    tauri::async_runtime::spawn(async move {
        const INTERVAL: Duration = Duration::from_secs(45 * 60);
        loop {
            tokio::time::sleep(INTERVAL).await;
            let mut guard = client.lock().await;
            // Only meaningful once signed in; before that there is nothing to keep warm.
            if guard.session().webservices.is_none() {
                continue;
            }
            // A lapsed session is expected roughly every two months. The UI discovers it on
            // next use and asks for a code; nothing to do here but stop pinging a dead one.
            // A transient error is not proof of that, so only a clean `false` ends the loop.
            if matches!(guard.heartbeat().await, Ok(false)) {
                break;
            }
        }
    });
}

/// Refresh the calendar window `[from, to]` (both `YYYY-MM-DD`) from Apple.
///
/// The interface does not wait on this to render: it reads the library, which still holds
/// the last good data while a refresh is in flight or failing.
#[tauri::command]
async fn calendar_sync(
    from: String,
    to: String,
    timezone: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<quince_icloud::sync::SyncSummary> {
    // Take what the sync needs and release the auth lock before the network work, so a slow
    // refresh cannot block the heartbeat or a sign-out.
    let account = account_now(&auth).await;
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("calendar")
            .ok_or("Apple hasn't offered a calendar service for this account.")?;
        (client.http().clone(), root, client.service_params())
    };

    // Asked of Apple unlocked; the library is held only to store the answer.
    let _one = CALENDAR_PASS.lock().await;
    let fetched = quince_icloud::sync::fetch_calendar(&http, &root, params, &from, &to, &timezone)
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    still_owned(&library, account.as_deref())?;
    quince_icloud::sync::apply_calendar(&mut library, fetched).map_err(|e| e.to_string())
}

/// What the interface sends to create or change an event.
///
/// The guid is what decides which it is: absent means a new event, present means saving
/// changes to one Apple already has.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventDraft {
    guid: Option<String>,
    calendar_guid: String,
    title: String,
    location: Option<String>,
    starts_at: i64,
    ends_at: i64,
    all_day: bool,
    timezone: Option<String>,
    notes: Option<String>,
    url: Option<String>,
    /// Set the alerts to exactly these, minutes before the start (`0` at the time); absent,
    /// the event's alerts are left as Apple holds them.
    #[serde(default)]
    alerts: Option<Vec<i64>>,
    /// For one occurrence of a repeating event: which events the change is for.
    #[serde(default)]
    scope: Option<quince_icloud::calendar::Scope>,
    /// Set the repeat to this — `None` for never — when `repeat_changed`; otherwise the event
    /// keeps the repeat Apple holds.
    #[serde(default)]
    repeat: Option<quince_icloud::calendar::EventRepeat>,
    #[serde(default)]
    repeat_changed: bool,
}

/// Everything an event write needs from the session, with the auth lock released first.
async fn calendar_context(
    auth: &State<'_, Auth>,
) -> Result<(reqwest::Client, String, Vec<(String, String)>), String> {
    let client = auth.client.lock().await;
    let root = client
        .session()
        .service_url("calendar")
        .ok_or("Apple hasn't offered a calendar service for this account.")?;
    Ok((client.http().clone(), root, client.service_params()))
}

/// Create an event, or save changes to one.
///
/// Apple takes the whole event on every write, so an edit sends the event as it should end
/// up rather than a patch. What comes back from Apple is what gets stored.
/// An event's alerts as Apple holds them: minutes before its start, `0` at the time.
#[tauri::command]
async fn event_alerts(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<i64>> {
    let event = {
        let library = data.0.lock().await;
        library.event(&guid).map_err(|e| e.to_string())?
    };
    let Some(event) = event else {
        return Ok(Vec::new());
    };
    let (http, root, params) = calendar_context(&auth).await?;
    quince_icloud::calendar::CalendarClient::new(&http, &root, params)
        .alerts(&event)
        .await
        .map_err(|e| e.to_string())
}

/// How an event repeats, as Apple holds it.
#[tauri::command]
async fn event_repeat(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Option<quince_icloud::calendar::EventRepeat>> {
    let event = {
        let library = data.0.lock().await;
        library.event(&guid).map_err(|e| e.to_string())?
    };
    let Some(event) = event else { return Ok(None) };
    let (http, root, params) = calendar_context(&auth).await?;
    quince_icloud::calendar::CalendarClient::new(&http, &root, params)
        .repeat_of(&event)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn event_save(
    draft: EventDraft,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let _one = CALENDAR_PASS.lock().await;
    let title = draft.title.trim();
    if title.is_empty() {
        return Err("An event needs a title.".into());
    }
    if draft.ends_at < draft.starts_at {
        return Err("An event can't end before it starts.".into());
    }
    let account = account_now(&auth).await;
    let (http, root, params) = calendar_context(&auth).await?;

    // Apple's guids are uppercase UUIDs; a new event needs one before it can be addressed.
    let was_saved = draft.guid.is_some();
    let guid = draft
        .guid
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().to_uppercase());
    let existing = {
        let library = data.0.lock().await;
        library.event(&guid).map_err(|e| e.to_string())?
    };
    // An event opened as one already saved, and no longer here: deleted on another device
    // since. Saved, it came back as new on every device, without its alerts or guests.
    if was_saved && existing.is_none() && !quince_icloud::calendar::is_occurrence(&guid) {
        return Err("This event was deleted on another device, so it wasn’t saved.".into());
    }
    // The calendar it is in now, when that is not the one it is saved to: a move.
    let moved_from = existing
        .as_ref()
        .map(|e| e.calendar_guid.clone())
        .filter(|old| *old != draft.calendar_guid);

    let event = Event {
        guid: guid.clone(),
        calendar_guid: draft.calendar_guid,
        title: title.to_string(),
        location: draft.location.filter(|s| !s.trim().is_empty()),
        starts_at: draft.starts_at,
        ends_at: draft.ends_at,
        all_day: draft.all_day,
        timezone: draft.timezone,
        url: draft.url.filter(|s| !s.trim().is_empty()),
        notes: draft.notes.filter(|s| !s.trim().is_empty()),
        // Apple rejects a write whose etag is not the version it holds, which is what stops
        // this from landing on top of a change made on a phone.
        etag: existing.and_then(|e| e.etag),
    };

    let ctag = quince_icloud::sync::calendar_ctag(&*data.0.lock().await, &event.calendar_guid);
    // Moving to another calendar: where it is now, and that calendar's tag.
    let from = match moved_from {
        Some(old) => {
            let tag = quince_icloud::sync::calendar_ctag(&*data.0.lock().await, &old);
            Some((old, tag))
        }
        None => None,
    };
    let kept = quince_icloud::sync::save_event_with_alerts(
        &http,
        &root,
        params,
        ctag.as_deref(),
        &event,
        draft.alerts.as_deref(),
        draft.repeat_changed.then_some(draft.repeat.as_ref()),
        draft.scope,
        from.as_ref()
            .map(|(old, tag)| (old.as_str(), tag.as_deref())),
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    still_owned(&library, account.as_deref())?;
    quince_icloud::sync::store_events(&mut library, kept).map_err(|e| e.to_string())?;
    Ok(guid)
}

#[tauri::command]
async fn event_delete(
    guid: String,
    scope: Option<quince_icloud::calendar::Scope>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let account = account_now(&auth).await;
    let _one = CALENDAR_PASS.lock().await;
    let (http, root, params) = calendar_context(&auth).await?;
    let event = {
        let library = data.0.lock().await;
        library
            .event(&guid)
            .map_err(|e| e.to_string())?
            .ok_or("That event is no longer in the local library.")?
    };
    let ctag = quince_icloud::sync::calendar_ctag(&*data.0.lock().await, &event.calendar_guid);
    quince_icloud::calendar::CalendarClient::new(&http, &root, params)
        .delete_event_scoped(&event, ctag.as_deref(), scope)
        .await
        .map_err(|e| e.to_string())?;
    // What went at Apple goes here: the whole series for all of it, this day and those after
    // for this-and-future — not just the one day, which left the rest of a deleted series on
    // screen until the next sync.
    let mut library = data.0.lock().await;
    still_owned(&library, account.as_deref())?;
    let series = quince_icloud::calendar::series_of(&event.guid).to_string();
    let gone: Vec<String> = match scope {
        Some(quince_icloud::calendar::Scope::All)
        | Some(quince_icloud::calendar::Scope::Future) => library
            .events_starting_between(i64::MIN / 2, i64::MAX / 2)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(g, _)| g)
            .filter(|g| quince_icloud::calendar::series_of(g) == series)
            .filter(|g| {
                scope == Some(quince_icloud::calendar::Scope::All)
                    || library
                        .event(g)
                        .ok()
                        .flatten()
                        .is_some_and(|e| e.starts_at >= event.starts_at)
            })
            .collect(),
        _ => vec![event.guid.clone()],
    };
    library.forget_events(&gone).map_err(|e| e.to_string())
}

/// Events overlapping `[from, to]`, as unix seconds. Read straight from the library.
#[tauri::command]
async fn calendar_events(
    from: i64,
    to: i64,
    data: State<'_, Data>,
    occasions: State<'_, Occasions>,
) -> UiResult<Vec<Event>> {
    let (mut events, people) = {
        let library = data.0.lock().await;
        (
            library
                .events_between(from, to)
                .map_err(|e| e.to_string())?,
            library.contacts(None).map_err(|e| e.to_string())?,
        )
    };
    // Birthdays and holidays, as a phone draws them: neither is an iCloud calendar.
    events.extend(quince_icloud::occasions::birthdays(&people, from, to));
    events.extend(quince_icloud::occasions::holidays(
        &occasions.feed.lock().await.1,
        from,
        to,
    ));
    events.sort_by(|a, b| {
        a.starts_at
            .cmp(&b.starts_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(events)
}

/// Events holding every word of `query`, soonest first — and birthdays and holidays in the
/// year either side of now, which are found the same way.
#[tauri::command]
async fn calendar_search(
    query: String,
    data: State<'_, Data>,
    occasions: State<'_, Occasions>,
) -> UiResult<Vec<Event>> {
    let words = quince_core::fold::words(&query);
    let (mut found, people) = {
        let library = data.0.lock().await;
        (
            library.search_events(&query).map_err(|e| e.to_string())?,
            library.contacts(None).map_err(|e| e.to_string())?,
        )
    };
    if !words.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        let (from, to) = (now - 366 * 86_400, now + 366 * 86_400);
        let feed = occasions.feed.lock().await;
        let extra = quince_icloud::occasions::birthdays(&people, from, to)
            .into_iter()
            .chain(quince_icloud::occasions::holidays(&feed.1, from, to))
            .filter(|e| quince_core::fold::holds_all(&quince_core::fold::fold(&e.title), &words));
        found.extend(extra);
        found.sort_by_key(|e| e.starts_at);
    }
    Ok(found)
}

/// The holiday feed for this machine's region, and its name, kept beside the library.
struct Occasions {
    feed: Mutex<(Option<String>, Vec<quince_icloud::occasions::Holiday>)>,
}

/// How long a copy of the holiday feed is used before asking Apple for a fresh one.
const HOLIDAYS_KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Keep the holiday feed: read the copy on disk at once, and fetch a fresh one when that copy
/// is a week old or missing — then once a day after, in case it ages while Quince runs.
fn spawn_holidays(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let path = state_dir(&app).join("holidays.ics");
        if let Ok(text) = std::fs::read_to_string(&path) {
            *app.state::<Occasions>().feed.lock().await =
                quince_icloud::occasions::parse_feed(&text);
        }
        let http = reqwest::Client::new();
        loop {
            let stale = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map_or(true, |t| {
                    t.elapsed().map_or(true, |age| age > HOLIDAYS_KEPT)
                });
            if stale {
                if let Some((region, lang)) = quince_icloud::occasions::this_machines_feed() {
                    if let Ok(text) =
                        quince_icloud::occasions::fetch_feed(&http, &region, &lang).await
                    {
                        std::fs::write(&path, &text).ok();
                        *app.state::<Occasions>().feed.lock().await =
                            quince_icloud::occasions::parse_feed(&text);
                        app.emit(SYNCED, refresh::status(&app.state::<Data>().0).await)
                            .ok();
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

/// After a calendar was made, changed or removed: Apple's own list of calendars, kept here,
/// so the sidebar shows what iCloud now holds rather than what was asked for.
async fn calendars_from_apple(
    client: &quince_icloud::calendar::CalendarClient<'_>,
    data: &Data,
) -> UiResult<()> {
    let today = jiff::Zoned::now().date().to_string();
    let snap = client
        .startup(&today, &today, "UTC")
        .await
        .map_err(|e| e.to_string())?;
    if snap.calendars.is_empty() {
        // An empty answer is never taken as "every calendar is gone".
        return Ok(());
    }
    data.0
        .lock()
        .await
        .put_calendars(&snap.calendars)
        .map_err(|e| e.to_string())
}

/// New Calendar, as on icloud.com: Apple's next unused colour, a name no other calendar has.
#[tauri::command]
async fn calendar_create(
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let _one = CALENDAR_PASS.lock().await;
    let (http, root, params) = calendar_context(&auth).await?;
    let client = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
    let existing = data.0.lock().await.calendars().map_err(|e| e.to_string())?;
    let guid = client
        .create_calendar(&title, &existing)
        .await
        .map_err(|e| e.to_string())?;
    calendars_from_apple(&client, &data).await?;
    Ok(guid)
}

/// Rename or recolour a calendar.
#[tauri::command]
async fn calendar_update(
    guid: String,
    title: Option<String>,
    colour: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = CALENDAR_PASS.lock().await;
    let (http, root, params) = calendar_context(&auth).await?;
    let client = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
    let existing = data.0.lock().await.calendars().map_err(|e| e.to_string())?;
    client
        .update_calendar(&guid, title.as_deref(), colour.as_deref(), &existing)
        .await
        .map_err(|e| e.to_string())?;
    calendars_from_apple(&client, &data).await
}

/// Delete a calendar and its events, once the person has confirmed it in the window.
#[tauri::command]
async fn calendar_delete(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = CALENDAR_PASS.lock().await;
    let (http, root, params) = calendar_context(&auth).await?;
    let client = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
    let calendar = data
        .0
        .lock()
        .await
        .calendars()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|c| c.guid == guid)
        .ok_or("That calendar is no longer here.")?;
    client
        .delete_calendar(&calendar)
        .await
        .map_err(|e| e.to_string())?;
    calendars_from_apple(&client, &data).await
}

#[tauri::command]
async fn calendar_list(
    data: State<'_, Data>,
    occasions: State<'_, Occasions>,
) -> UiResult<Vec<Calendar>> {
    let library = data.0.lock().await;
    let mut calendars = library.calendars().map_err(|e| e.to_string())?;
    let any_birthdays = library
        .contacts(None)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|c| c.birthday.is_some());
    let feed = occasions.feed.lock().await;
    let holidays =
        (!feed.1.is_empty()).then(|| feed.0.clone().unwrap_or_else(|| "Holidays".into()));
    calendars.extend(quince_icloud::occasions::calendars(
        holidays.as_deref(),
        any_birthdays,
    ));
    Ok(calendars)
}

/// What every entry in the Reminders sidebar is holding.
#[tauri::command]
async fn reminder_counts(
    until: i64,
    data: State<'_, Data>,
) -> UiResult<quince_core::reminders::ReminderCounts> {
    let library = data.0.lock().await;
    library.reminder_counts(until).map_err(|e| e.to_string())
}

/// Reminders still to do holding every word of `query`, across every enabled list.
#[tauri::command]
async fn reminders_search(query: String, data: State<'_, Data>) -> UiResult<Vec<Reminder>> {
    let library = data.0.lock().await;
    library.search_reminders(&query).map_err(|e| e.to_string())
}

/// The headings inside one reminders list, in the order they were made.
#[tauri::command]
async fn reminder_sections(
    list_guid: String,
    data: State<'_, Data>,
) -> UiResult<Vec<quince_core::reminders::ReminderSection>> {
    let library = data.0.lock().await;
    library
        .reminder_sections(&list_guid)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn reminder_lists(data: State<'_, Data>) -> UiResult<Vec<ReminderList>> {
    let library = data.0.lock().await;
    library.reminder_lists().map_err(|e| e.to_string())
}

/// The folders lists are filed under.
#[tauri::command]
async fn reminder_groups(
    data: State<'_, Data>,
) -> UiResult<Vec<quince_core::reminders::ReminderGroup>> {
    let library = data.0.lock().await;
    library.reminder_groups().map_err(|e| e.to_string())
}

#[tauri::command]
async fn reminders_in(
    list_guid: String,
    include_done: bool,
    data: State<'_, Data>,
) -> UiResult<Vec<Reminder>> {
    let library = data.0.lock().await;
    library
        .reminders_in(&list_guid, include_done)
        .map_err(|e| e.to_string())
}

/// Tick a reminder off, or put it back.
///
/// Writes to Apple first and only then refreshes locally, so the interface never shows a
/// state Apple has not accepted.
#[tauri::command]
async fn reminder_set_done(
    guid: String,
    list_guid: String,
    done: bool,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    set_done(&auth, &data, &guid, &list_guid, done).await
}

/// Tick a reminder off or put it back — from the window, or from an alert's button.
///
/// In the library at once and queued for Apple (see outbox.rs): the window never waits on the
/// network. A repeating one is the exception — ticking it, Apple rolls it on to its next date,
/// which only Apple's answer can show — so that one is sent and answered here.
async fn set_done(
    auth: &State<'_, Auth>,
    data: &State<'_, Data>,
    guid: &str,
    list_guid: &str,
    done: bool,
) -> UiResult<()> {
    let reminder = {
        let library = data.0.lock().await;
        library
            .reminder(guid)
            .map_err(|e| e.to_string())?
            .filter(|r| r.list_guid == list_guid)
            .ok_or("That reminder is no longer in the local library.")?
    };
    if !(done && reminder.repeats()) {
        let now = jiff::Timestamp::now().as_second();
        let payload =
            serde_json::to_string(&outbox::Done { done, at: now }).map_err(|e| e.to_string())?;
        {
            let mut library = data.0.lock().await;
            library
                .set_reminder_completed(guid, done.then_some(now))
                .map_err(|e| e.to_string())?;
            library
                .outbox_push("reminders", guid, "done", &payload)
                .map_err(|e| e.to_string())?;
        }
        outbox::kick();
        alerts::changed();
        return Ok(());
    }
    let (http, root, params) = cloudkit_context(auth).await?;

    quince_icloud::reminders::set_completed_cloudkit(&http, &root, params.clone(), &reminder, done)
        .await
        .map_err(|e| e.to_string())?;

    refresh_reminders(&http, &root, params, data, false).await
}

/// Set or clear how often a reminder comes back.
///
/// Kept apart from the ordinary edit because a repeat is not a field: it is its own record
/// that has to be written, linked and retired in the right order.
#[tauri::command]
async fn reminder_set_repeat(
    guid: String,
    list_guid: String,
    repeat: Option<quince_icloud::reminders::Repeat>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminder(&guid)
            .map_err(|e| e.to_string())?
            .filter(|r| r.list_guid == list_guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::set_recurrence_cloudkit(
        &http,
        &root,
        params.clone(),
        &reminder,
        repeat,
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

/// Dated reminders inside a window, for the calendar grid.
#[tauri::command]
async fn reminders_due_between(
    from: i64,
    to: i64,
    data: State<'_, Data>,
) -> UiResult<Vec<Reminder>> {
    let library = data.0.lock().await;
    library
        .reminders_due_between(from, to)
        .map_err(|e| e.to_string())
}

/// Add a reminder to a list.
///
/// Writes to Apple first, then refreshes locally, so nothing appears on screen that Apple
/// has not accepted.
#[tauri::command]
async fn reminder_create(
    list_guid: String,
    title: String,
    // What the view it was typed into implies, as on the phone: added in Today it is due
    // today, in Flagged it is flagged. Without it the new reminder belonged to neither, and
    // vanished from the view it was typed into the moment it was made.
    due_at: Option<i64>,
    all_day: Option<bool>,
    flagged: Option<bool>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let guid =
        quince_icloud::reminders::create_cloudkit(&http, &root, params.clone(), &list_guid, &title)
            .await
            .map_err(|e| e.to_string())?;
    if due_at.is_some() || flagged.is_some() {
        let made = quince_core::reminders::Reminder {
            guid,
            list_guid: list_guid.clone(),
            title: title.clone(),
            ..Default::default()
        };
        let edit = quince_icloud::reminders::ReminderEdit {
            due_at: due_at.map(Some),
            all_day,
            flagged,
            ..Default::default()
        };
        // The reminder exists at Apple now, whatever happens next: a failure here must not
        // read as "nothing was made", or the title put back and entered again made a second.
        if let Err(e) =
            quince_icloud::reminders::edit_cloudkit(&http, &root, params.clone(), &made, &edit)
                .await
        {
            quiet!("a new reminder's date or flag was not set: {e}");
        }
    }

    refresh_reminders(&http, &root, params, &data, false).await
}

#[tauri::command]
async fn reminder_delete(
    guid: String,
    list_guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminder(&guid)
            .map_err(|e| e.to_string())?
            .filter(|r| r.list_guid == list_guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    // Its subtasks go with it, as on the phone: left behind they came back at the top level
    // here and stayed in iCloud.
    let children = data.0.lock().await.subtasks_of(&guid).unwrap_or_default();
    quince_icloud::reminders::delete_cloudkit(&http, &root, params.clone(), &reminder)
        .await
        .map_err(|e| e.to_string())?;
    for child in &children {
        if let Err(e) =
            quince_icloud::reminders::delete_cloudkit(&http, &root, params.clone(), child).await
        {
            quiet!("a subtask did not go with its reminder: {e}");
        }
    }

    refresh_reminders(&http, &root, params, &data, false).await
}

/// Apply an edit — title, notes, due date, flag, completion — in one write.
#[tauri::command]
async fn reminder_edit(
    guid: String,
    list_guid: String,
    edit: quince_icloud::reminders::ReminderEdit,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminder(&guid)
            .map_err(|e| e.to_string())?
            .filter(|r| r.list_guid == list_guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::edit_cloudkit(&http, &root, params.clone(), &reminder, &edit)
        .await
        .map_err(|e| e.to_string())?;

    // Moved to another list, a reminder takes its subtasks with it, as on the phone: left
    // behind, they sat in the old list nested under a parent that was no longer in it.
    if let Some(to) = edit.list_guid.as_deref().filter(|to| *to != list_guid) {
        let children: Vec<quince_core::reminders::Reminder> = {
            let library = data.0.lock().await;
            library.subtasks_of(&guid).unwrap_or_default()
        };
        let follow = quince_icloud::reminders::ReminderEdit {
            list_guid: Some(to.to_string()),
            ..Default::default()
        };
        for child in &children {
            if let Err(e) = quince_icloud::reminders::edit_cloudkit(
                &http,
                &root,
                params.clone(),
                child,
                &follow,
            )
            .await
            {
                quiet!("a subtask did not follow its reminder: {e}");
            }
        }
    }

    // A due date on its own notifies nobody. Every reminder on a real account that has a
    // time carries an Alarm pointing at an AlarmTrigger, and one written here used to have
    // the date and no alarm — so a phone showed the right time and then said nothing when it
    // came. The alert is part of setting a date, not a separate thing to remember.
    //
    // The edit is already on Apple by now. If the alert fails, the library is still re-read
    // before saying so: returning early left the old date on screen though Apple had the new
    // one, and the next edit was made against a reminder that was not there any more.
    // An all-day reminder rings no alarm, as on the phone.
    // Made all-day, a reminder keeps no timed alarm: the old one kept ringing at the time it
    // used to have.
    let alarm_change = if edit.all_day == Some(true) {
        Some(None)
    } else {
        edit.due_at
    };
    let alarm = match alarm_change {
        Some(due) => {
            let zone = jiff::tz::TimeZone::system();
            quince_icloud::reminders::set_alarm_cloudkit(
                &http,
                &root,
                params.clone(),
                &guid,
                due,
                zone.iana_name().unwrap_or("UTC"),
            )
            .await
            .map_err(|e| format!("The date was saved, but its alert was not: {e}"))
        }
        None => Ok(()),
    };

    refresh_reminders(&http, &root, params, &data, false).await?;
    alarm
}

/// One of Apple's smart lists. `until` is the end of today in the viewer's own timezone,
/// which the interface knows and the backend does not.
#[tauri::command]
async fn reminders_smart(
    kind: SmartList,
    until: i64,
    data: State<'_, Data>,
) -> UiResult<Vec<Reminder>> {
    let library = data.0.lock().await;
    library
        .reminders_smart(kind, until)
        .map_err(|e| e.to_string())
}

/// Create, rename or delete a list, then re-read so the sidebar matches Apple.
#[tauri::command]
async fn reminder_list_create(
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::create_list_cloudkit(&http, &root, params.clone(), &title)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

#[tauri::command]
async fn reminder_list_rename(
    list_guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::rename_list_cloudkit(
        &http,
        &root,
        params.clone(),
        &list_guid,
        &title,
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

/// Move a reminder within its list, before another or to the end, as a drag does.
#[tauri::command]
async fn reminder_reorder(
    list_guid: String,
    guid: String,
    before: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::reorder_cloudkit(
        &http,
        &root,
        params.clone(),
        &list_guid,
        &guid,
        before.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

/// A list's colour, one of Apple's twelve, as Show List Info sets it.
#[tauri::command]
async fn reminder_list_colour(
    list_guid: String,
    colour: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::set_list_colour_cloudkit(
        &http,
        &root,
        params.clone(),
        &list_guid,
        &colour,
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

#[tauri::command]
async fn reminder_list_delete(
    list_guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::delete_list_cloudkit(&http, &root, params.clone(), &list_guid)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_reminders(&http, &root, params, &data).await)
}

/// A refresh after a write that reached Apple. Failing, it is not the write failing: the
/// change is made, and saying otherwise had people retry it — a second folder, a second
/// upload. It is noted, and the next refresh brings the screen up to date.
fn settle(refreshed: Result<(), String>) -> Result<(), String> {
    if let Err(e) = refreshed {
        quiet!("refresh after a write: {e}");
    }
    Ok(())
}

/// Re-read notes after a write, so what is on screen is what Apple stored.
async fn resync_notes(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
    refresh_notes(http, root, params, data).await
}

/// One note, by record name. `None` once Apple no longer has it.
async fn note_by_guid(data: &State<'_, Data>, guid: &str) -> Result<Note, String> {
    let library = data.0.lock().await;
    // Wherever it is filed, Recently Deleted included: Delete Now and Recover act on notes
    // there, and looking only in "All" answered each of them "no longer in the library".
    library
        .note(guid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "That note is no longer in the local library.".into())
}

/// Save a note's body.
///
/// Writes to Apple first and re-reads after, so the editor never shows a version Apple has
/// not accepted. A note whose document cannot be rewritten safely is refused with the
/// reason rather than flattened.
/// One note's text, whole.
///
/// A list of notes carries no bodies — five thousand ordinary ones come to nearly ten
/// megabytes, and sending that into the window on every refresh to draw text nobody reads is
/// the unbounded payload the plan rules out. The note being read asks for its own.
#[tauri::command]
async fn note_body(guid: String, data: State<'_, Data>) -> UiResult<Option<String>> {
    let library = data.0.lock().await;
    library.note_body(&guid).map_err(|e| e.to_string())
}

/// A note written out as Markdown at `path` (chosen in the save dialog): its text as it is on
/// screen, typing not yet sent included, set as it is in Notes — headings, lists,
/// checklists, code, quotes, bold, italic, links.
#[tauri::command]
async fn note_export_markdown(
    body: String,
    runs: Vec<quince_icloud::ckdocument::Run>,
    path: String,
) -> UiResult<()> {
    let text = quince_icloud::markdown::note_to_markdown(&body, &runs);
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("Could not save it there: {e}"))
}

/// Set one paragraph of a note: `style` from the Aa menu, or `checked` for a checklist tick.
#[tauri::command]
async fn note_paragraph(
    guid: String,
    start: u64,
    style: Option<u64>,
    checked: Option<bool>,
    // The line's text as it was shown, so a tick cannot land on a line that moved there.
    expect: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    use quince_icloud::ckdocument::ParagraphChange;
    let change = match (style, checked) {
        (Some(s), _) => ParagraphChange::Style(s),
        (None, Some(c)) => ParagraphChange::Checked(c),
        (None, None) => return Ok(()),
    };
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    quince_icloud::notes::set_paragraph_cloudkit(
        &http,
        &root,
        params.clone(),
        &note,
        start,
        change,
        expect.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

/// How a note's text is set — Apple's attribute runs, in order — as stored at the last sync.
#[tauri::command]
async fn note_runs(guid: String, data: State<'_, Data>) -> UiResult<serde_json::Value> {
    let library = data.0.lock().await;
    let stored = library.note_runs(&guid).map_err(|e| e.to_string())?;
    Ok(stored
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Array(Vec::new())))
}

/// A table's cells, row by row, as the last pass read them — `None` when none are known.
/// Whether Quince writes to Notes tables. OFF: a cell and a row and a column written here
/// showed on his iPhone (2026-10-21), but Notes there then kept two conflict copies of the note,
/// one with the written cells' text doubled and tripled — it could not merge these edits
/// cleanly. No table is written until a write is shown to merge on the phone with no copy.
const TABLE_CELL_WRITES: bool = false;
const TABLE_LINE_WRITES: bool = false;
const TABLE_WRITES_OFF: &str =
    "Tables are read-only in Quince for now: edits made here didn’t merge cleanly on iPhone yet.";

/// Add an empty row to a Notes table at `at` (0 is the top), as Notes does, and keep the
/// table as Apple now holds it.
#[tauri::command]
async fn note_table_add_row(
    id: String,
    at: usize,
    column: Option<bool>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<Vec<String>>> {
    if !TABLE_LINE_WRITES {
        return Err(TABLE_WRITES_OFF.into());
    }
    let (http, root, params) = cloudkit_context(&auth).await?;
    let cells = quince_icloud::notes::add_table_line_cloudkit(
        &http,
        &root,
        params,
        &id,
        column.unwrap_or(false),
        at,
    )
    .await
    .map_err(|e| e.to_string())?;
    if let Ok(json) = serde_json::to_string(&cells) {
        data.0
            .lock()
            .await
            .set_state(&quince_icloud::table::state_key(&id), &json)
            .map_err(|e| e.to_string())?;
    }
    Ok(cells)
}

/// Delete a row (or, with `column`, a column) of a Notes table, and keep the table as Apple
/// now holds it.
#[tauri::command]
async fn note_table_remove(
    id: String,
    at: usize,
    column: bool,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<Vec<String>>> {
    if !TABLE_LINE_WRITES {
        return Err(TABLE_WRITES_OFF.into());
    }
    let (http, root, params) = cloudkit_context(&auth).await?;
    let cells =
        quince_icloud::notes::remove_table_line_cloudkit(&http, &root, params, &id, column, at)
            .await
            .map_err(|e| e.to_string())?;
    if let Ok(json) = serde_json::to_string(&cells) {
        data.0
            .lock()
            .await
            .set_state(&quince_icloud::table::state_key(&id), &json)
            .map_err(|e| e.to_string())?;
    }
    Ok(cells)
}

/// Change one cell of a Notes table, and keep the table as Apple now holds it.
#[tauri::command]
async fn note_table_set_cell(
    id: String,
    row: usize,
    column: usize,
    text: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<Vec<String>>> {
    if !TABLE_CELL_WRITES {
        return Err(TABLE_WRITES_OFF.into());
    }
    let (http, root, params) = cloudkit_context(&auth).await?;
    let cells = quince_icloud::notes::set_table_cell_cloudkit(
        &http, &root, params, &id, row, column, &text,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    if let Ok(json) = serde_json::to_string(&cells) {
        library
            .set_state(&quince_icloud::table::state_key(&id), &json)
            .map_err(|e| e.to_string())?;
    }
    Ok(cells)
}

#[tauri::command]
async fn note_table(
    id: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Option<Vec<Vec<String>>>> {
    if let Some(cells) = quince_icloud::table::stored(&*data.0.lock().await, &id) {
        return Ok(Some(cells));
    }
    // Not stored yet — a table made before this app read tables, which a pass of changes
    // does not bring — so asked of Apple now, and kept.
    let (http, root, params) = cloudkit_context(&auth).await?;
    let client = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", params);
    let record = client
        .lookup_record("Notes", &id)
        .await
        .map_err(|e| e.to_string())?;
    let cells = record
        .as_ref()
        .and_then(|r| r.pointer("/fields/MergeableDataEncrypted/value"))
        .and_then(serde_json::Value::as_str)
        .and_then(quince_icloud::table::cells_from_base64);
    if let Some(cells) = &cells {
        let mut library = data.0.lock().await;
        if let Ok(json) = serde_json::to_string(cells) {
            library
                .set_state(&quince_icloud::table::state_key(&id), &json)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(cells)
}

#[tauri::command]
async fn note_save(
    guid: String,
    body: String,
    // The text the editor began from. With it a save can tell an edit made on another device
    // since, and keep both rather than write over it.
    base: Option<String>,
    // The note's formatting, when it is edited formatted: runs over `body`.
    runs: Option<Vec<quince_icloud::ckdocument::Run>>,
    // The formatting the editor began from, over `base`: a line not restyled since keeps
    // what Apple holds now, so a tick made on the phone is not undone.
    base_runs: Option<Vec<quince_icloud::ckdocument::Run>>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    if !note.editable {
        return Err(
            "This note holds formatting Quince can't rewrite safely, so it stays read-only.".into(),
        );
    }
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        params.clone(),
        &note,
        base.as_deref(),
        &body,
        runs.as_deref(),
        base_runs.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    // Apple has it; a refresh failing now must not read as the save failing.
    if let Err(e) = resync_notes(&http, &root, params, &data).await {
        quiet!("notes refresh after a save: {e}");
    }

    // Apple accepted the body, but its change feed does not always carry our own write back
    // on the very next pass. Without this the editor re-reads the old text a moment after
    // saving, which reads as the edit having been lost. Only the text is set: the note's
    // formatting and the rest come from Apple's own copy.
    let (title, snippet) = quince_icloud::notes::title_and_snippet(&saved);
    let mut library = data.0.lock().await;
    library
        .set_note_text(
            &guid,
            &saved,
            &title,
            Some(snippet.as_str()).filter(|s| !s.is_empty()),
        )
        .map_err(|e| e.to_string())?;
    if let Some(runs) = runs {
        let laid = if saved == body {
            runs
        } else {
            quince_icloud::ckdocument::carry_runs(&body, &runs, &saved)
        };
        if let Ok(json) = serde_json::to_string(&laid) {
            library
                .set_note_runs(&guid, &json)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(saved)
}

/// Write a new note and return the record name Apple gave it.
#[tauri::command]
async fn note_create(
    folder_guid: Option<String>,
    body: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    // The shape of a document Apple wrote, noticed during a sync. A note built without it
    // reads back here perfectly and shows on a phone as an empty note.
    let template = {
        let library = data.0.lock().await;
        library
            .state(quince_icloud::notes::TEMPLATE_KEY)
            .map_err(|e| e.to_string())?
    };
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        params.clone(),
        folder_guid.as_deref(),
        &body,
        template.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    // Apple has made it. A refresh failing now must not read as the create failing — the
    // editor would try again, and the person would have the note twice.
    if let Err(e) = resync_notes(&http, &root, params, &data).await {
        quiet!("notes refresh after a create: {e}");
    }

    // Apple's change feed does not always carry a new record back on the very next pass.
    // Without this the note just made was missing locally, and the first edit to it failed
    // with "That note is no longer in the local library" — typing into a fresh note, then
    // pasting, lost the paste's save. Saving reads the record from Apple, so a local stand-in
    // with no tag is enough until the feed catches up.
    let mut library = data.0.lock().await;
    let known = library.note(&guid).map_err(|e| e.to_string())?.is_some();
    if !known {
        let (title, snippet) = quince_icloud::notes::title_and_snippet(&body);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        library
            .put_notes(&[Note {
                guid: guid.clone(),
                folder_guid,
                title,
                snippet: Some(snippet).filter(|s| !s.is_empty()),
                body: Some(body),
                locked: false,
                pinned: false,
                created_at: Some(now),
                modified_at: Some(now),
                editable: true,
                etag: None,
                runs: None,
            }])
            .map_err(|e| e.to_string())?;
    }
    Ok(guid)
}

#[tauri::command]
async fn note_delete(guid: String, auth: State<'_, Auth>, data: State<'_, Data>) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    quince_icloud::notes::delete_cloudkit(&http, &root, params.clone(), &note)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

/// Move a note into a folder, or out of every folder when `folder_guid` is absent.
#[tauri::command]
async fn note_move(
    guid: String,
    folder_guid: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    quince_icloud::notes::move_cloudkit(
        &http,
        &root,
        params.clone(),
        &note,
        folder_guid.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

/// Pin a note to the top of its list, or unpin it.
#[tauri::command]
async fn note_pin(
    guid: String,
    pinned: bool,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    quince_icloud::notes::pin_cloudkit(&http, &root, params.clone(), &note, pinned)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

/// A new Notes folder, inside another when `parent` is given.
#[tauri::command]
async fn note_folder_create(
    title: String,
    parent: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let guid = quince_icloud::notes::create_folder_cloudkit(
        &http,
        &root,
        params.clone(),
        &title,
        parent.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)?;
    Ok(guid)
}

#[tauri::command]
async fn note_folder_rename(
    guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::notes::rename_folder_cloudkit(&http, &root, params.clone(), &guid, &title)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

/// Delete a folder: its notes, and those of every folder inside it, to Recently Deleted.
#[tauri::command]
async fn note_folder_delete(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    // Read what is needed, then let go of the library before any network call.
    let (folders, notes) = {
        let library = data.0.lock().await;
        (
            library.note_folders().map_err(|e| e.to_string())?,
            library.notes_in(None).map_err(|e| e.to_string())?,
        )
    };
    quince_icloud::notes::delete_folder_cloudkit(
        &http,
        &root,
        params.clone(),
        &folders,
        &notes,
        &guid,
    )
    .await
    .map_err(|e| e.to_string())?;
    settle(resync_notes(&http, &root, params, &data).await)
}

#[tauri::command]
async fn note_folders(data: State<'_, Data>) -> UiResult<Vec<NoteFolder>> {
    let library = data.0.lock().await;
    library.note_folders().map_err(|e| e.to_string())
}

/// Notes holding every word of `query`, from every folder but Recently Deleted.
#[tauri::command]
async fn notes_search(query: String, data: State<'_, Data>) -> UiResult<Vec<Note>> {
    let library = data.0.lock().await;
    library.search_notes(&query).map_err(|e| e.to_string())
}

/// How many notes each folder holds, and all of them but Recently Deleted.
#[tauri::command]
async fn note_counts(data: State<'_, Data>) -> UiResult<(i64, Vec<(String, i64)>)> {
    let library = data.0.lock().await;
    library.note_counts().map_err(|e| e.to_string())
}

/// Notes in a folder, or every note when no folder is given.
#[tauri::command]
async fn notes_in(folder_guid: Option<String>, data: State<'_, Data>) -> UiResult<Vec<Note>> {
    let library = data.0.lock().await;
    library
        .notes_in(folder_guid.as_deref())
        .map_err(|e| e.to_string())
}

/// Everything a contacts write needs from the session, with the auth lock released first.
async fn contacts_context(
    auth: &State<'_, Auth>,
) -> Result<(reqwest::Client, String, Vec<(String, String)>), String> {
    let client = auth.client.lock().await;
    let root = client
        .session()
        .service_url("contacts")
        .ok_or("Apple hasn't offered a contacts service for this account.")?;
    Ok((client.http().clone(), root, client.service_params()))
}

/// Re-read the address book after a write, so what is on screen is what Apple stored.
async fn resync_contacts(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
    let people = quince_icloud::contacts::fetch(http, root, params)
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    quince_icloud::contacts::apply(&mut library, people)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The Contacts lists, as the last pass stored them.
#[tauri::command]
async fn contact_lists(
    data: State<'_, Data>,
) -> UiResult<Vec<quince_icloud::contacts::ContactList>> {
    let library = data.0.lock().await;
    Ok(quince_icloud::contacts::stored_lists(&library))
}

/// One Contacts list write at a time. Each reads the list as Apple holds it and sends it
/// back whole; two at once would both read the same etag and one would be refused.
static CONTACT_LISTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Change a Contacts list: make one (no `guid`), rename it, add people to it or take them
/// out, or delete it. Answers with the list's id.
///
/// Apple only takes a list whole, so membership is sent as what to add and remove and laid
/// over the list as Apple holds it at the moment of writing — someone added on the phone a
/// second ago stays in. A write that loses to one made elsewhere is read again and retried.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn contact_list_write(
    guid: Option<String>,
    name: Option<String>,
    add: Option<Vec<String>>,
    remove: Option<Vec<String>>,
    delete: Option<bool>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let _one = CONTACTS_PASS.lock().await;
    let _one = CONTACT_LISTS.lock().await;
    let (http, root, params) = contacts_context(&auth).await?;
    let add = add.unwrap_or_default();
    let remove = remove.unwrap_or_default();
    let mut attempt = 0;
    let done = loop {
        attempt += 1;
        let (book, tokens) =
            quince_icloud::contacts::fetch_book_with_tokens(&http, &root, params.clone())
                .await
                .map_err(|e| e.to_string())?;
        let written = match guid.as_deref().filter(|g| !g.is_empty()) {
            None => {
                quince_icloud::contacts::create_list(
                    &http,
                    &root,
                    params.clone(),
                    &tokens,
                    name.as_deref().unwrap_or("Untitled List"),
                    &add,
                )
                .await
            }
            Some(guid) => {
                let Some(mut list) = book
                    .lists
                    .into_iter()
                    .find(|l| l.guid.eq_ignore_ascii_case(guid))
                else {
                    // Already gone — deleted here a moment ago, or on another device.
                    break guid.to_string();
                };
                if delete.unwrap_or(false) {
                    quince_icloud::contacts::delete_list(
                        &http,
                        &root,
                        params.clone(),
                        &tokens,
                        &list.guid,
                    )
                    .await
                } else {
                    if let Some(name) = name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                        list.name = name.to_string();
                    }
                    list.members
                        .retain(|m| !remove.iter().any(|r| r.eq_ignore_ascii_case(m)));
                    for person in &add {
                        if !list.members.iter().any(|m| m.eq_ignore_ascii_case(person)) {
                            list.members.push(person.clone());
                        }
                    }
                    quince_icloud::contacts::update_list(
                        &http,
                        &root,
                        params.clone(),
                        &tokens,
                        &list,
                    )
                    .await
                }
                .map(|()| list.guid)
            }
        };
        match written {
            Ok(guid) => break guid,
            Err(quince_icloud::Error::Conflict) if attempt < 3 => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    settle(resync_contacts(&http, &root, params, &data).await)?;
    Ok(done)
}

/// Create a contact, or save changes to one. Returns the id Apple knows it by.
///
/// An edit starts from Apple's own card, so everything this app does not model — a photo,
/// related names, social profiles — is written back untouched rather than dropped.
#[tauri::command]
async fn contact_save(
    contact: Contact,
    // The card as the form opened it: only what differs from it is written.
    base: Option<Contact>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let _one = CONTACTS_PASS.lock().await;
    let (http, root, params) = contacts_context(&auth).await?;
    let (_, tokens) = quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone())
        .await
        .map_err(|e| e.to_string())?;

    let guid = if contact.guid.is_empty() {
        quince_icloud::contacts::create(&http, &root, params.clone(), &tokens, &contact)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let card = quince_icloud::contacts::card(&http, &root, params.clone(), &contact.guid)
            .await
            .map_err(|e| e.to_string())?;
        quince_icloud::contacts::update_from(
            &http,
            &root,
            params.clone(),
            &tokens,
            &contact,
            card.as_ref(),
            base.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
        contact.guid.clone()
    };

    settle(resync_contacts(&http, &root, params, &data).await)?;
    Ok(guid)
}

#[tauri::command]
async fn contact_delete(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = CONTACTS_PASS.lock().await;
    let (http, root, params) = contacts_context(&auth).await?;
    let (people, tokens) = quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone())
        .await
        .map_err(|e| e.to_string())?;
    let contact = people
        .into_iter()
        .find(|c| c.guid == guid)
        .ok_or("Apple no longer has that contact.")?;

    quince_icloud::contacts::delete(&http, &root, params.clone(), &tokens, &contact)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_contacts(&http, &root, params, &data).await)
}

/// A path in `dir` for `name` that holds nothing yet: a number goes on the end rather than
/// writing over a file already there — the rule a browser follows.
fn free_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
    let mut path = dir.join(name);
    let mut suffix = 1;
    while path.exists() {
        path = dir.join(if ext.is_empty() {
            format!("{stem} {suffix}")
        } else {
            format!("{stem} {suffix}.{ext}")
        });
        suffix += 1;
    }
    path
}

/// These people as a vCard file in Downloads, written by Apple. Returns where it went.
#[tauri::command]
async fn contacts_export(
    guids: Vec<String>,
    file_name: String,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
) -> UiResult<String> {
    if guids.is_empty() {
        return Err("Choose someone to export.".into());
    }
    let (http, root, params) = contacts_context(&auth).await?;
    let (_, tokens) = quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone())
        .await
        .map_err(|e| e.to_string())?;
    let vcard =
        quince_icloud::contacts::export_vcards(&http, &root, params, &tokens, &guids, &file_name)
            .await
            .map_err(|e| e.to_string())?;
    let downloads = app
        .path()
        .download_dir()
        .map_err(|_| "Couldn't find your Downloads folder.".to_string())?;
    // A name fit for a file: no folders smuggled in, never empty.
    let clean: String = file_name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0'))
        .collect();
    let clean = if clean.trim().trim_end_matches(".vcf").is_empty() {
        "iCloud vCards.vcf".into()
    } else {
        clean
    };
    let path = free_path(&downloads, &clean);
    std::fs::write(&path, vcard).map_err(|e| format!("Couldn't save the vCard: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Add everyone in a vCard file to Contacts. Answers how many came in, how many could not be
/// read, and how many were left out because they are in Contacts already.
#[tauri::command]
async fn contacts_import(
    paths: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<(usize, usize, usize)> {
    let _one = CONTACTS_PASS.lock().await;
    // Every file chosen, vCard or CSV (Google Contacts, Outlook), read into cards first, so a
    // file that cannot be read stops the import before anything is sent.
    let mut cards = Vec::new();
    let mut unreadable = 0;
    for path in &paths {
        let text = std::fs::read_to_string(path).map_err(|e| {
            format!(
                "Couldn't read {}: {e}",
                std::path::Path::new(path)
                    .file_name()
                    .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned())
            )
        })?;
        if path.to_ascii_lowercase().ends_with(".csv") {
            let (from_csv, skipped) = quince_icloud::contacts::csv_to_vcards(&text);
            cards.extend(from_csv);
            unreadable += skipped;
        } else {
            cards.extend(quince_icloud::contacts::split_vcards(&text));
        }
    }
    if cards.is_empty() {
        return Err("Those files hold no contacts.".into());
    }
    let (http, root, params) = contacts_context(&auth).await?;
    let (book, tokens) = quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone())
        .await
        .map_err(|e| e.to_string())?;
    // Someone already in Contacts is left out, as Apple's Contacts leaves them: an export of
    // this same book imported back once doubled every person in it.
    let before = cards.len();
    cards.retain(|card| !quince_icloud::contacts::already_in(card, &book));
    let known = before - cards.len();
    if cards.is_empty() {
        return Ok((0, unreadable, known));
    }
    // In batches Apple takes in one import: well under its 10 MB, a few hundred cards each.
    let mut imported = 0;
    for batch in quince_icloud::contacts::batch_vcards(&cards, 8 * 1024 * 1024, 400) {
        let (ids, failed) =
            quince_icloud::contacts::import_vcards(&http, &root, params.clone(), &tokens, &batch)
                .await
                .map_err(|e| e.to_string())?;
        imported += ids.len();
        unreadable += failed;
    }
    settle(resync_contacts(&http, &root, params, &data).await)?;
    Ok((imported, unreadable, known))
}

/// The card that is this person's own, as the last pass stored it.
#[tauri::command]
async fn contact_me(data: State<'_, Data>) -> UiResult<Option<String>> {
    let library = data.0.lock().await;
    Ok(library
        .state(quince_icloud::contacts::ME_KEY)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty()))
}

/// Make this card "My Card", on every device.
#[tauri::command]
async fn contact_set_me(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = CONTACTS_PASS.lock().await;
    let (http, root, params) = contacts_context(&auth).await?;
    let (_, tokens) = quince_icloud::contacts::fetch_with_tokens(&http, &root, params.clone())
        .await
        .map_err(|e| e.to_string())?;
    quince_icloud::contacts::set_me_card(&http, &root, params.clone(), &tokens, &guid)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_contacts(&http, &root, params, &data).await)
}

/// The address book, optionally filtered by name or by any stored number or address.
#[tauri::command]
async fn contacts_list(search: Option<String>, data: State<'_, Data>) -> UiResult<Vec<Contact>> {
    let library = data.0.lock().await;
    library
        .contacts(search.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn drive_sync_folder(
    folder_id: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<usize> {
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("drivews")
            .ok_or("Apple hasn't offered iCloud Drive for this account.")?;
        (client.http().clone(), root, client.service_params())
    };
    let _one = DRIVE_PASS.lock().await;
    let items = match quince_icloud::drive::fetch_folder(&http, &root, params, &folder_id).await {
        Ok(items) => items,
        // Deleted or moved on another device: forgotten here too, so the screen can step back
        // out of it rather than show a folder that is not there.
        Err(quince_icloud::Error::NotFound) => {
            let mut library = data.0.lock().await;
            library
                .forget_drive_folder(&folder_id)
                .map_err(|e| e.to_string())?;
            return Err("This folder was deleted or moved on another device.".into());
        }
        Err(e) => return Err(e.to_string()),
    };
    let mut library = data.0.lock().await;
    quince_icloud::drive::store_folder(&mut library, &folder_id, &items).map_err(|e| e.to_string())
}

#[tauri::command]
async fn drive_children(folder_id: String, data: State<'_, Data>) -> UiResult<Vec<DriveItem>> {
    let library = data.0.lock().await;
    library
        .drive_children(&folder_id)
        .map_err(|e| e.to_string())
}

/// One item, for building the breadcrumb trail back to the root.
#[tauri::command]
async fn drive_item(drivewsid: String, data: State<'_, Data>) -> UiResult<Option<DriveItem>> {
    let library = data.0.lock().await;
    library.drive_item(&drivewsid).map_err(|e| e.to_string())
}

/// Everything a Drive write needs from the session, with the auth lock released first.
async fn drive_context(
    auth: &State<'_, Auth>,
) -> Result<(reqwest::Client, String, Vec<(String, String)>), String> {
    let client = auth.client.lock().await;
    let root = client
        .session()
        .service_url("drivews")
        .ok_or("Apple hasn't offered iCloud Drive for this account.")?;
    Ok((client.http().clone(), root, client.service_params()))
}

/// Re-read one folder after a write, so what is on screen is what Apple stored.
async fn resync_drive_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
    folder_id: &str,
) -> Result<(), String> {
    let _one = DRIVE_PASS.lock().await;
    let items = quince_icloud::drive::fetch_folder(http, root, params, folder_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    quince_icloud::drive::store_folder(&mut library, folder_id, &items)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Save a file's bytes to the user's Downloads folder.
///
/// Apple's own name is kept, with a number appended rather than silently overwriting
/// something already there — the same rule a browser follows. Returns the path saved to.
#[tauri::command]
async fn drive_download(
    drivewsid: String,
    item: Option<quince_core::drive::DriveItem>,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    // A file seen only in Recents was never in a folder this library has read, so the item
    // comes with the request; the library's own copy wins when there is one.
    let stored = {
        let library = data.0.lock().await;
        library.drive_item(&drivewsid).map_err(|e| e.to_string())?
    };
    let item = stored
        .or(item.filter(|i| i.drivewsid == drivewsid))
        .ok_or("That item is no longer in the local library.")?;
    if item.is_container() {
        return Err("That's a folder, not a file.".into());
    }

    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("docws")
            .ok_or("Apple hasn't offered downloads for this account.")?;
        (client.http().clone(), root, client.service_params())
    };

    let downloads = app
        .path()
        .download_dir()
        .map_err(|_| "Couldn't find your Downloads folder.".to_string())?;
    let name = item.display_name();
    let (stem, ext) = name
        .rsplit_once('.')
        .map_or((name.as_str(), ""), |(s, e)| (s, e));
    let mut destination = downloads.join(&name);
    let mut suffix = 1;
    while destination.exists() {
        destination = downloads.join(if ext.is_empty() {
            format!("{stem} {suffix}")
        } else {
            format!("{stem} {suffix}.{ext}")
        });
        suffix += 1;
    }

    quince_icloud::drive::download_to(&http, &root, params, &item, &destination)
        .await
        .map_err(|e| e.to_string())?;
    Ok(destination.to_string_lossy().into_owned())
}

/// Where an upload's progress is announced, one event per file as it moves.
const UPLOAD: &str = "quince://upload";

/// One file's upload, as the window shows it.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UploadEvent {
    /// Unique within the batch the window started.
    id: String,
    batch: String,
    name: String,
    /// The folders it sits in below the one it was dropped on, for a folder upload.
    within: String,
    sent: u64,
    total: u64,
    /// `preparing` (a folder being made on Apple), `waiting`, `sending`, `verifying`, `done`,
    /// `failed` or `cancelled`; and once per batch `planned`, when every file in it is known.
    state: &'static str,
    /// A folder rather than a file: its row stands for making the folder, not for bytes.
    #[serde(default)]
    folder: bool,
    error: Option<String>,
}

/// Uploads that can still be stopped, by id: waiting, or sending their bytes.
fn uploads_running(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, futures_util::future::AbortHandle>>
{
    static RUNNING: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, futures_util::future::AbortHandle>>,
    > = std::sync::OnceLock::new();
    RUNNING.get_or_init(Default::default)
}

/// Batches stopped as a whole: no more of their folders are made or files queued.
fn cancelled_batches() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static CANCELLED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    CANCELLED.get_or_init(Default::default)
}

fn cancelled(batch: &str) -> bool {
    cancelled_batches().lock().is_ok_and(|b| b.contains(batch))
}

/// Stop uploads: these files, or a whole batch. A file already being filed in iCloud is past
/// stopping and finishes; anything waiting or still sending stops, with nothing of it kept.
#[tauri::command]
fn drive_upload_cancel(ids: Vec<String>, batch: Option<String>) {
    if let Some(batch) = &batch {
        cancelled_batches()
            .lock()
            .map(|mut b| b.insert(batch.clone()))
            .ok();
    }
    if let Ok(running) = uploads_running().lock() {
        for (id, handle) in running.iter() {
            let in_batch = batch
                .as_deref()
                .is_some_and(|b| id.starts_with(&format!("{b}:")));
            if in_batch || ids.contains(id) {
                handle.abort();
            }
        }
    }
}

/// One file to send, and the Drive folder it goes into.
struct UploadJob {
    event: UploadEvent,
    path: std::path::PathBuf,
    folder: String,
}

/// The Drive folder called `name` inside `parent`: the one already there, or a new one.
///
/// Dropping a folder that is already in iCloud adds to it, as Finder offers to, rather than
/// making "Photos 2" beside it. Files inside that already exist are kept too: Apple renames
/// the newcomer rather than overwrite.
async fn folder_for(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    data: &State<'_, Data>,
    parent: &str,
    name: &str,
) -> Result<(String, bool), String> {
    settle(resync_drive_folder(http, root, params.to_vec(), data, parent).await)?;
    let existing = {
        let library = data.0.lock().await;
        library
            .drive_children(parent)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|i| i.is_container() && i.name.eq_ignore_ascii_case(name))
    };
    if let Some(folder) = existing {
        return Ok((folder.drivewsid, false));
    }
    quince_icloud::drive::create_folder(http, root, params.to_vec(), parent, name)
        .await
        .map(|f| (f.drivewsid, true))
        .map_err(|e| e.to_string())
}

/// Every file under the paths dropped or chosen, with folders made to match on the way.
///
/// Links are not followed: one that points back up the tree would never finish, and one that
/// points outside it would upload something nobody chose.
#[allow(clippy::too_many_arguments)]
async fn plan_upload(
    app: &tauri::AppHandle,
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    data: &State<'_, Data>,
    batch: &str,
    folder_id: &str,
    paths: &[String],
    failed: &mut Vec<String>,
) -> Vec<UploadJob> {
    let mut jobs = Vec::new();
    let mut folders_made = 0;
    // (path on disk, Drive folder it goes into, folders below the drop target)
    // (path on disk, Drive folder it goes into, folders below the drop target, and for a
    // folder already made on Apple its id and whether this upload made it)
    #[allow(clippy::type_complexity)]
    let mut pending: Vec<(std::path::PathBuf, String, String, Option<(String, bool)>)> = paths
        .iter()
        .map(|p| {
            (
                std::path::PathBuf::from(p),
                folder_id.to_string(),
                String::new(),
                None,
            )
        })
        .collect();
    while let Some((path, into, within, resolved)) = pending.pop() {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        if cancelled(batch) {
            break;
        }
        if meta.is_dir() {
            // Said at once, with its own row: making a folder tree on Apple takes a moment
            // per folder, and a folder upload used to show nothing at all until every folder
            // in it was made.
            let mut shown = UploadEvent {
                id: format!("{batch}:dir:{}", folders_made),
                batch: batch.to_string(),
                name: name.clone(),
                within: within.clone(),
                sent: 0,
                total: 0,
                state: "preparing",
                folder: true,
                error: None,
            };
            folders_made += 1;
            app.emit(UPLOAD, &shown).ok();
            // Made already, with its siblings in one request, when its parent was made by this
            // upload; otherwise found or made by name — dropped into a folder that may hold one.
            let found = match resolved {
                Some(done) => Ok(done),
                None => folder_for(http, root, params, data, &into, &name).await,
            };
            let (folder, fresh) = match found {
                Ok(folder) => {
                    shown.state = "done";
                    app.emit(UPLOAD, &shown).ok();
                    folder
                }
                Err(e) => {
                    shown.state = "failed";
                    shown.error = Some(e.to_string());
                    app.emit(UPLOAD, &shown).ok();
                    failed.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let below = if within.is_empty() {
                name.clone()
            } else {
                format!("{within}/{name}")
            };
            let mut entries = match tokio::fs::read_dir(&path).await {
                Ok(entries) => entries,
                Err(e) => {
                    failed.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let mut children = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                children.push(entry.path());
            }
            // Popped from the end, so reversed to go in name order.
            children.sort();
            // A folder this upload just made holds nothing yet, so its folders cannot clash
            // with anything there: all of them are made in one request, rather than a read of
            // the parent and a request each — hundreds of round trips for a git repository.
            let mut made: std::collections::HashMap<String, Result<String, String>> =
                std::collections::HashMap::new();
            if fresh {
                let dirs: Vec<String> = {
                    let mut d = Vec::new();
                    for child in &children {
                        if tokio::fs::symlink_metadata(child)
                            .await
                            .is_ok_and(|m| m.is_dir())
                        {
                            d.push(
                                child
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                            );
                        }
                    }
                    d
                };
                if !dirs.is_empty() {
                    match quince_icloud::drive::create_folders(
                        http,
                        root,
                        params.to_vec(),
                        &folder,
                        &dirs,
                    )
                    .await
                    {
                        Ok(results) => made.extend(results),
                        Err(e) => {
                            for d in dirs {
                                made.insert(d, Err(e.to_string()));
                            }
                        }
                    }
                }
            }
            for child in children.into_iter().rev() {
                let child_name = child
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match made.remove(&child_name) {
                    Some(Ok(id)) => {
                        pending.push((child, folder.clone(), below.clone(), Some((id, true))))
                    }
                    Some(Err(e)) => failed.push(format!("{child_name}: {e}")),
                    None => pending.push((child, folder.clone(), below.clone(), None)),
                }
            }
        } else if meta.is_file() {
            let event = UploadEvent {
                id: format!("{batch}:{}", jobs.len()),
                batch: batch.to_string(),
                name,
                within,
                sent: 0,
                total: meta.len(),
                state: "waiting",
                folder: false,
                error: None,
            };
            // Shown the moment it is found, not once the whole tree has been walked.
            app.emit(UPLOAD, &event).ok();
            jobs.push(UploadJob {
                event,
                path,
                folder: into,
            });
        }
    }
    jobs
}

/// Put files from this machine into a Drive folder.
///
/// Paths, never bytes. The interface hands over what the person chose — from the file dialog
/// or from a drag onto the window — and the file is read here and streamed straight to Apple,
/// so putting a film in Drive costs no more memory than putting a note in it.
///
/// Each file is reported on separately: one that Apple refuses must not take the rest of a
/// dragged selection down with it.
///
/// Folders are uploaded whole, and each file's progress is announced as it moves.
///
/// Three at a time: enough to keep the connection busy through each file's round trips
/// without asking Apple for dozens of upload slots at once. Every file is proven intact
/// before it counts as done — see `quince_icloud::drive::upload`. Returns what failed.
#[tauri::command]
async fn drive_upload(
    app: tauri::AppHandle,
    batch: String,
    folder_id: String,
    paths: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<String>> {
    use futures_util::StreamExt;

    let (http, root, params) = drive_context(&auth).await?;
    let docws = {
        let client = auth.client.lock().await;
        client
            .session()
            .service_url("docws")
            .ok_or("Apple hasn't offered uploads for this account.")?
    };

    let mut failed = Vec::new();
    let jobs = plan_upload(
        &app,
        &http,
        &root,
        &params,
        &data,
        &batch,
        &folder_id,
        &paths,
        &mut failed,
    )
    .await;
    // Everything in it has been found: the window can count to a total that will not move.
    app.emit(
        UPLOAD,
        &UploadEvent {
            id: format!("{batch}:planned"),
            batch: batch.clone(),
            name: String::new(),
            within: String::new(),
            sent: 0,
            total: jobs.iter().map(|j| j.event.total).sum(),
            state: "planned",
            folder: true,
            error: None,
        },
    )
    .ok();

    let results: Vec<Option<String>> = futures_util::stream::iter(jobs)
        .map(|job| {
            let (app, http, docws, root, params) = (
                app.clone(),
                http.clone(),
                docws.clone(),
                root.clone(),
                params.clone(),
            );
            let (abort, registration) = futures_util::future::AbortHandle::new_pair();
            uploads_running()
                .lock()
                .map(|mut m| m.insert(job.event.id.clone(), abort))
                .ok();
            let id = job.event.id.clone();
            let cancelled_event = {
                let mut e = job.event.clone();
                e.state = "cancelled";
                e
            };
            let app_for_cancel = app.clone();
            let run = async move {
                let mut event = job.event.clone();
                event.state = "sending";
                app.emit(UPLOAD, &event).ok();

                // Told about every quarter-megabyte, passed on at every whole percent: a
                // smooth bar without a flood of messages for a large file.
                let last = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let progress: quince_icloud::drive::Progress = {
                    let (app, event, last) = (app.clone(), event.clone(), last.clone());
                    std::sync::Arc::new(move |sent: u64, total: u64| {
                        let step = (total / 100).max(1);
                        let before = last.load(std::sync::atomic::Ordering::Relaxed);
                        if sent / step != before / step || sent == total {
                            last.store(sent, std::sync::atomic::Ordering::Relaxed);
                            let mut e = event.clone();
                            e.sent = sent;
                            e.state = if sent >= total {
                                // Every byte is up and it is being filed: too late to stop
                                // cleanly, so it can no longer be cancelled.
                                uploads_running().lock().map(|mut m| m.remove(&e.id)).ok();
                                "verifying"
                            } else {
                                "sending"
                            };
                            app.emit(UPLOAD, &e).ok();
                        }
                    })
                };
                let outcome = quince_icloud::drive::upload(
                    &http,
                    &docws,
                    &root,
                    params,
                    &job.folder,
                    &job.path,
                    progress,
                )
                .await;
                event.sent = event.total;
                match outcome {
                    Ok(_) => {
                        event.state = "done";
                        app.emit(UPLOAD, &event).ok();
                        None
                    }
                    Err(e) => {
                        event.state = "failed";
                        event.error = Some(e.to_string());
                        app.emit(UPLOAD, &event).ok();
                        Some(format!("{}: {e}", event.name))
                    }
                }
            };
            async move {
                let outcome = futures_util::future::Abortable::new(run, registration).await;
                uploads_running().lock().map(|mut m| m.remove(&id)).ok();
                match outcome {
                    Ok(result) => result,
                    // Stopped before its bytes were filed: nothing of it is in iCloud.
                    Err(_) => {
                        app_for_cancel.emit(UPLOAD, &cancelled_event).ok();
                        None
                    }
                }
            }
        })
        .buffer_unordered(3)
        .collect()
        .await;
    failed.extend(results.into_iter().flatten());
    cancelled_batches()
        .lock()
        .map(|mut b| b.remove(&batch))
        .ok();

    settle(resync_drive_folder(&http, &root, params, &data, &folder_id).await)?;
    Ok(failed)
}

/// Create a folder, and reflect it locally without waiting for the next full sync.
#[tauri::command]
async fn drive_create_folder(
    parent_id: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::create_folder(&http, &root, params.clone(), &parent_id, &name)
        .await
        .map_err(|e| e.to_string())?;
    settle(resync_drive_folder(&http, &root, params, &data, &parent_id).await)
}

#[tauri::command]
async fn drive_rename(
    drivewsid: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let item = {
        let library = data.0.lock().await;
        library
            .drive_item(&drivewsid)
            .map_err(|e| e.to_string())?
            .ok_or("That item is no longer in the local library.")?
    };
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::rename(&http, &root, params.clone(), &item, &name)
        .await
        .map_err(|e| e.to_string())?;
    let parent = item
        .parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT);
    settle(resync_drive_folder(&http, &root, params, &data, parent).await)
}

#[tauri::command]
async fn drive_delete(
    drivewsid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let item = {
        let library = data.0.lock().await;
        library
            .drive_item(&drivewsid)
            .map_err(|e| e.to_string())?
            .ok_or("That item is no longer in the local library.")?
    };
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::delete(&http, &root, params.clone(), &item)
        .await
        .map_err(|e| e.to_string())?;
    let parent = item
        .parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT);
    settle(resync_drive_folder(&http, &root, params, &data, parent).await)
}

/// Duplicate a file into its own folder, named as Finder names a copy.
#[tauri::command]
async fn drive_duplicate(
    drivewsid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let item = {
        let library = data.0.lock().await;
        library
            .drive_item(&drivewsid)
            .map_err(|e| e.to_string())?
            .ok_or("That item is no longer in the local library.")?
    };
    let (http, drivews, docws, params) = {
        let client = auth.client.lock().await;
        let session = client.session();
        (
            client.http().clone(),
            session
                .service_url("drivews")
                .ok_or("Apple hasn't offered iCloud Drive for this account.")?,
            session
                .service_url("docws")
                .ok_or("Apple hasn't offered iCloud Drive for this account.")?,
            client.service_params(),
        )
    };
    quince_icloud::drive::duplicate(&http, &docws, &drivews, params.clone(), &item)
        .await
        .map_err(|e| e.to_string())?;
    let parent = item
        .parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT);
    settle(resync_drive_folder(&http, &drivews, params, &data, parent).await)
}

/// Apple's thumbnails for one folder's files, as `data:` URLs keyed by drivewsid.
///
/// Kept on disk beside the library, named by item and version, so a folder opened again
/// shows its pictures at once and only a changed file is fetched anew.
#[tauri::command]
async fn drive_thumbnails(
    folder_id: String,
    app: tauri::AppHandle,
) -> UiResult<std::collections::HashMap<String, String>> {
    use base64::Engine;
    Ok(folder_thumbnails(&app, &folder_id)
        .await?
        .into_iter()
        .map(|(id, bytes)| {
            let url = data_url(
                &bytes,
                &base64::engine::general_purpose::STANDARD.encode(&bytes),
            );
            (id, url)
        })
        .collect())
}

/// Apple's pictures of a folder's files, by item id: for the window's grid and for the iCloud
/// Drive folder's thumbnails (mount.rs), from one cache. Kept on disk beside the library,
/// named by item and version, so a folder opened again has its pictures at once and only a
/// changed file is fetched anew.
pub(crate) async fn folder_thumbnails(
    app: &tauri::AppHandle,
    folder_id: &str,
) -> Result<std::collections::HashMap<String, Vec<u8>>, String> {
    let files = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library
            .drive_children(folder_id)
            .map_err(|e| e.to_string())?
    };
    let cache = state_dir(app).join("thumbnails");
    std::fs::create_dir_all(&cache).ok();
    let key = |f: &quince_core::drive::DriveItem| {
        let safe = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };
        cache.join(format!(
            "{}-{}",
            safe(&f.drivewsid),
            safe(f.etag.as_deref().unwrap_or(""))
        ))
    };
    let mut out = std::collections::HashMap::new();
    let mut missing = Vec::new();
    for f in files.iter().filter(|f| f.kind == "FILE") {
        match std::fs::read(key(f)) {
            Ok(bytes) if bytes.is_empty() => {} // known to have none
            Ok(bytes) => {
                out.insert(f.drivewsid.clone(), bytes);
            }
            Err(_) => missing.push(f.clone()),
        }
    }
    if !missing.is_empty() {
        let (http, docws, params) = {
            let auth = app.state::<Auth>();
            let client = auth.client.lock().await;
            (
                client.http().clone(),
                client
                    .session()
                    .service_url("docws")
                    .ok_or("Apple hasn't offered iCloud Drive for this account.")?,
                client.service_params(),
            )
        };
        let got = quince_icloud::drive::thumbnails(&http, &docws, params, &missing)
            .await
            .map_err(|e| e.to_string())?;
        for f in &missing {
            let bytes = got
                .iter()
                .find(|(id, _)| id == &f.drivewsid)
                .map(|(_, b)| b.clone());
            // An empty file records "Apple has no picture for this version", so it is not asked again.
            std::fs::write(key(f), bytes.as_deref().unwrap_or(&[])).ok();
            if let Some(bytes) = bytes {
                out.insert(f.drivewsid.clone(), bytes);
            }
        }
    }
    Ok(out)
}

/// An image as a `data:` URL, its type read from its first bytes.
fn data_url(bytes: &[u8], b64: &str) -> String {
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(b"RIFF") {
        "image/webp"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/jpeg"
    };
    format!("data:{mime};base64,{b64}")
}

/// How much iCloud storage is used and how much there is, for "X GB available".
#[tauri::command]
async fn storage_info(auth: State<'_, Auth>) -> UiResult<quince_icloud::drive::Storage> {
    let (http, setup, params) = {
        let client = auth.client.lock().await;
        (
            client.http().clone(),
            client.setup_url().to_string(),
            client.service_params(),
        )
    };
    quince_icloud::drive::storage(&http, &setup, params)
        .await
        .map_err(|e| e.to_string())
}

/// Drive's Recently Deleted, read live: Apple decides how long things stay there, so a copy
/// kept here would only ever be a wrong count.
#[tauri::command]
async fn drive_trash(auth: State<'_, Auth>) -> UiResult<Vec<quince_icloud::drive::TrashItem>> {
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::trash_items(&http, &root, params)
        .await
        .map_err(|e| e.to_string())
}

/// The files opened most recently, newest first — Drive's Recents.
#[tauri::command]
async fn drive_recents(auth: State<'_, Auth>) -> UiResult<Vec<quince_icloud::drive::RecentItem>> {
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("docws")
            .ok_or("Apple hasn't offered Recents for this account.")?;
        (client.http().clone(), root, client.service_params())
    };
    quince_icloud::drive::recents(&http, &root, params)
        .await
        .map_err(|e| e.to_string())
}

/// Recover an item from Recently Deleted to where it was.
#[tauri::command]
async fn drive_put_back(
    item: quince_core::drive::DriveItem,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::put_back(&http, &root, params, &item)
        .await
        .map_err(|e| e.to_string())
}

/// Delete an item in Recently Deleted for good. The interface asks first.
#[tauri::command]
async fn drive_delete_forever(
    item: quince_core::drive::DriveItem,
    auth: State<'_, Auth>,
) -> UiResult<()> {
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::delete_forever(&http, &root, params, &item)
        .await
        .map_err(|e| e.to_string())
}

/// Recover or delete for good every one of these, in one request, as Recover All and
/// Delete All do on icloud.com. Answers what could not be done, each with why — the rest are
/// done and gone from Recently Deleted.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashRefusal {
    drivewsid: String,
    reason: String,
}

#[tauri::command]
async fn drive_trash_out(
    items: Vec<quince_core::drive::DriveItem>,
    recover: bool,
    auth: State<'_, Auth>,
) -> UiResult<Vec<TrashRefusal>> {
    let (http, root, params) = drive_context(&auth).await?;
    let failed = if recover {
        quince_icloud::drive::put_back_many(&http, &root, params, &items).await
    } else {
        quince_icloud::drive::delete_forever_many(&http, &root, params, &items).await
    }
    .map_err(|e| e.to_string())?;
    Ok(failed
        .into_iter()
        .map(|(drivewsid, reason)| TrashRefusal { drivewsid, reason })
        .collect())
}

/// Move an item into a different folder, refreshing both the old and new location.
#[tauri::command]
async fn drive_move(
    drivewsid: String,
    destination_id: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let item = {
        let library = data.0.lock().await;
        library
            .drive_item(&drivewsid)
            .map_err(|e| e.to_string())?
            .ok_or("That item is no longer in the local library.")?
    };
    let (http, root, params) = drive_context(&auth).await?;
    quince_icloud::drive::move_to(&http, &root, params.clone(), &item, &destination_id)
        .await
        .map_err(|e| e.to_string())?;
    let old_parent = item
        .parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT);
    settle(resync_drive_folder(&http, &root, params.clone(), &data, old_parent).await)?;
    if old_parent != destination_id {
        settle(resync_drive_folder(&http, &root, params, &data, &destination_id).await)?;
    }
    Ok(())
}

/// The event a finished refresh emits, so every open screen re-reads at once.
const SYNCED: &str = "quince://synced";

/// How often the library refreshes itself.
///
/// Short enough that a reminder ticked on the phone is here before it is missed, long
/// enough to stay far away from anything Apple would call hammering. Nobody ever has to
/// press refresh; the control exists for impatience, not for correctness.
const AUTO_SYNC: Duration = Duration::from_secs(5 * 60);

/// Refresh every service at once.
///
/// Never fails as a whole: the result says what each service did, so one endpoint being
/// down leaves the rest current instead of taking the refresh with it.
#[tauri::command]
async fn sync_all(
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<refresh::Refresh> {
    let client = Arc::clone(&auth.client);
    keep_signed_in(&app, &client).await;
    let result = refresh::all(&client, &data.0).await;
    alerts::changed();
    warm_contact_photos(app.clone());
    // Screens read the library, not the network, so they only need to be told to look again.
    app.emit(SYNCED, &result).ok();
    Ok(result)
}

/// What the library already knows about its own freshness, with nothing fetched.
#[tauri::command]
async fn sync_status(data: State<'_, Data>) -> UiResult<refresh::Refresh> {
    Ok(refresh::status(&data.0).await)
}

/// Keep the library current on its own.
///
/// A pass that fails changes nothing about the schedule — the next one simply tries again.
fn spawn_auto_sync(app: tauri::AppHandle, client: Arc<Mutex<Client>>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(AUTO_SYNC).await;
            let signed_in = { client.lock().await.session().webservices.is_some() };
            if !signed_in {
                continue;
            }
            keep_signed_in(&app, &client).await;
            let data = app.state::<Data>();
            let result = refresh::all(&client, &data.0).await;
            alerts::changed();
            warm_contact_photos(app.clone());
            app.emit(SYNCED, &result).ok();
        }
    });
}

/// Where a contact's picture is cached on disk, named by the signature Apple gives it so a
/// changed picture lands at a new path rather than behind a stale one.
/// The edge of a served avatar.
///
/// Drawn at 26px in a list and 68px on a card, so 256 covers both on a display of any density
/// while staying a few kilobytes rather than the few hundred Apple stores.
const AVATAR_PX: u32 = 256;

/// What a picture actually is, read from its own first bytes.
///
/// Apple serves contact pictures as PNG today and said nothing about keeping to it, and a
/// cached copy on disk carries no header to consult — so the bytes are asked directly rather
/// than a type being assumed. An unrecognised one is still handed over: the webview sniffs
/// too, and the interface already falls back to initials if it cannot draw it.
fn image_type(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, 0xff, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "image/heic",
        _ => "application/octet-stream",
    }
}

fn photo_path(dir: &std::path::Path, guid: &str, signature: &str) -> PathBuf {
    // The signature is base64 and can hold characters a filename cannot.
    let safe: String = signature
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let guid_safe: String = guid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // No extension: the format is Apple's to choose, and is read back from the bytes.
    // "faces": the folder before held pictures cut with the crop read upside down, and is
    // never read again.
    dir.join("faces").join(format!("{guid_safe}-{safe}"))
}

/// Serve a contact's picture to the interface.
///
/// Through a scheme of our own rather than over IPC: the webview loads it the way it loads
/// any image, into its own cache, so a list of a hundred people does not push a hundred
/// images through a message channel on every scroll. Fetched from Apple once and kept on
/// disk after that.
/// A picture ready to serve, and whether it is safe to let the webview keep it.
struct Photo {
    bytes: Vec<u8>,
    durable: bool,
}

async fn contact_photo(
    app: &tauri::AppHandle,
    guid: &str,
) -> Result<Photo, Box<dyn std::error::Error + Send + Sync>> {
    let photo = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library
            .contact_photo_source(guid)?
            .ok_or("that contact has no picture")?
    };

    // A picture shared by the person it belongs to arrives with no signature, and Apple's
    // own preference on those is that they update themselves. With nothing to tell a cached
    // copy it has gone stale, none is kept: a stale face is worse than a second fetch.
    let dir = state_dir(app);
    let cached = photo
        .signature
        .as_deref()
        .map(|signature| photo_path(&dir, guid, signature));
    let durable = cached.is_some();
    if let Some(path) = &cached {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return Ok(Photo { bytes, durable });
        }
    }

    let (http, params) = {
        let auth = app.state::<Auth>();
        let client = auth.client.lock().await;
        (client.http().clone(), client.service_params())
    };
    let response = http.get(&photo.url).query(&params).send().await?;
    if !response.status().is_success() {
        return Err(format!("Apple returned {}", response.status()).into());
    }
    let bytes = response.bytes().await?.to_vec();

    // Cut to the square its owner chose and sized for a face, off the interface's thread.
    // Apple stores these whole — one of them is 1696x2528 — and a list of them has to scroll.
    let crop = photo.crop.as_deref().and_then(quince_media::Crop::parse);
    let drawn = tokio::task::spawn_blocking(move || quince_media::avatar(&bytes, crop, AVATAR_PX))
        .await?
        .map_err(|e| format!("that picture could not be read: {e}"))?;

    if let Some(path) = &cached {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(path, &drawn).await.ok();
    }
    Ok(Photo {
        bytes: drawn,
        durable,
    })
}

/// Fetch, in the background, every contact photo not yet on disk, so a face is already here
/// when its row is drawn rather than fetched from Apple as it scrolls into view. Three at a
/// time; one that fails is simply tried again after the next sync.
pub fn warm_contact_photos(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let wanted: Vec<String> = {
            let data = app.state::<Data>();
            let library = data.0.lock().await;
            let dir = state_dir(&app);
            library
                .contacts(None)
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.photo_url.is_some())
                .filter(|c| match &c.photo_signature {
                    Some(sig) => !photo_path(&dir, &c.guid, sig).exists(),
                    None => false,
                })
                .map(|c| c.guid)
                .collect()
        };
        use futures_util::StreamExt;
        futures_util::stream::iter(wanted)
            .for_each_concurrent(3, |guid| {
                let app = app.clone();
                async move {
                    let _ = contact_photo(&app, &guid).await;
                }
            })
            .await;
    });
}

/// Where this installation's writer id is kept.
const WRITER_KEY: &str = "documents.writer";

/// Give note and reminder edits this installation's own writer id.
///
/// Apple's devices merge a note by who wrote which characters, so every edit made here is
/// signed. The same id every run keeps a note's list of writers from growing by one each time
/// the app starts; a different id on every computer keeps two of them from numbering
/// characters the same way. If it cannot be stored, a fresh one for this run is still correct.
fn adopt_writer(library: &mut Library) {
    let stored = library
        .state(WRITER_KEY)
        .ok()
        .flatten()
        .and_then(|s| uuid::Uuid::parse_str(&s).ok());
    let id = stored.unwrap_or_else(|| {
        let fresh = uuid::Uuid::new_v4();
        let _ = library.set_state(WRITER_KEY, &fresh.to_string());
        fresh
    });
    quince_icloud::topotext::set_writer(*id.as_bytes());
}

/// How WebKitGTK hands its frames to the window. Three ways, measured on his Iris Xe at his
/// 1920×1080 (grid scroll / zoomed pan):
///
/// - GPU buffers (DMA-BUF) straight to the compositor: 59 / 60 fps, but on Hyprland the frames
///   arrive out of order — in his recordings the grid and the open viewer alternate frame by
///   frame, a flicker on every open, close and resize;
/// - no GPU hand-off at all (`WEBKIT_DISABLE_DMABUF_RENDERER`): steady, but 38 / 34 fps;
/// - drawn on the GPU, handed over in shared memory (`WEBKIT_DMABUF_RENDERER_FORCE_SHM`):
///   58 / 59 fps, and the frames reach the window in order.
///
/// The last, unless the person has chosen: any WebKit setting of their own is left alone, and
/// `QUINCE_DMABUF=1` keeps GPU buffers, `QUINCE_DMABUF=0` turns the GPU hand-off off.
fn steady_drawing() {
    let set = |k: &str| std::env::var_os(k).is_some();
    if set("WEBKIT_DISABLE_DMABUF_RENDERER") || set("WEBKIT_DMABUF_RENDERER_FORCE_SHM") {
        return;
    }
    // Before any thread starts: nothing else can be reading the environment yet.
    match std::env::var("QUINCE_DMABUF").as_deref() {
        Ok("1") => {}
        Ok("0") => std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
        _ => std::env::set_var("WEBKIT_DMABUF_RENDERER_FORCE_SHM", "1"),
    }
}

/// WebKitGTK's player on its older pipeline (playbin) buffers a video read over HTTP to disk
/// and then stalls: "Pausing stream for buffering" after the whole file has arrived, the
/// picture frozen on its first frame. Its newer pipeline (playbin3, WebKit's own switch) plays
/// and seeks the same file smoothly — proven on this machine's WebKitGTK 2.54. Only when the
/// person has not chosen themselves.
fn smooth_video() {
    if std::env::var_os("WEBKIT_GST_USE_PLAYBIN3").is_none() {
        // Before any thread starts: nothing else can be reading the environment yet.
        std::env::set_var("WEBKIT_GST_USE_PLAYBIN3", "1");
    }
}

/// The window's own colour, set to what the page paints at its edges (see src/lib/tint.ts):
/// what shows for the frames a resize runs ahead of the page.
#[tauri::command]
fn window_tint(window: tauri::WebviewWindow, rgb: [u8; 3]) -> UiResult<()> {
    window
        .set_background_color(Some(tauri::window::Color(rgb[0], rgb[1], rgb[2], 255)))
        .map_err(|e| e.to_string())
}

/// The window full screen, or back, as the photo viewer's button asks (icloud.com's); answers
/// whether it now is.
#[tauri::command]
fn window_fullscreen(window: tauri::WebviewWindow, on: bool) -> UiResult<bool> {
    window.set_fullscreen(on).map_err(|e| e.to_string())?;
    window.is_fullscreen().map_err(|e| e.to_string())
}

/// Whether this session runs under a tiling Wayland compositor, which manages windows itself.
fn tiling_compositor() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("HYPRLAND_INSTANCE_SIGNATURE") || set("SWAYSOCK") || set("NIRI_SOCKET") {
        return true;
    }
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| {
        d.split(':').any(|name| {
            [
                "hyprland", "sway", "niri", "river", "i3", "qtile", "dwl", "wayfire",
            ]
            .contains(&name.to_ascii_lowercase().as_str())
        })
    })
}

/// The window, shown once its first frame is drawn. It is made hidden, so a launch never shows
/// an empty or half-drawn window that then flickers into the app: the page says when it has
/// painted, and the window appears whole. Shown anyway after a few seconds, so a page that
/// never says so still leaves a window to use.
#[tauri::command]
fn window_ready(app: tauri::AppHandle, needs_you: Option<bool>) {
    // Started at login, with nothing for the person to do: everything runs, the window waits
    // in the tray.
    // Never without the tray to come back from.
    if app.tray_by_id("quince").is_some() && login::stay_hidden(needs_you.unwrap_or(true)) {
        return;
    }
    show_window(&app);
}

/// Quince ended, the one way every exit takes — Quit in the tray, Ctrl-C, a logout's signal,
/// the last window closed: the window gone at once, no more work started ahead, the folders
/// let go of (never waited on past three seconds), then the process ended on the spot.
///
/// Ended through `exit()`, the libraries' own teardown ran while a worker was still decoding a
/// HEIC — libheif unloaded under it, and Quince crashed on the way out (his, 2026-10-20: SIGSEGV
/// in libheif, main thread in exit). Nothing needs that teardown: every change is in the
/// library the moment it is made (SQLite commits as it goes), and what waits to be sent is
/// written down as it waits.
fn shutdown(app: &tauri::AppHandle, code: i32) -> ! {
    static ENDING: AtomicBool = AtomicBool::new(false);
    if !ENDING.swap(true, Ordering::SeqCst) {
        if let Some(w) = app.get_webview_window("main") {
            w.hide().ok();
        }
        cpu::stop();
        mount::release_within(Duration::from_secs(3));
    }
    // SAFETY: ends the process at once; nothing after it runs.
    unsafe { libc::_exit(code) }
}

fn show_when_ready(app: &tauri::AppHandle) {
    // Started at login: the window is the first window_ready's to show, or not.
    if login::background() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Some(w) = app.get_webview_window("main") {
            if !w.is_visible().unwrap_or(true) {
                show_window(&app);
            }
        }
    });
}

/// Bring the window back, wherever it went.
fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.show().ok();
        window.unminimize().ok();
        window.set_focus().ok();
    }
}

/// A place in the tray, so Quince stays running — and ringing reminders — with its window
/// closed, and can be opened again or quit from there.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let open = MenuItem::with_id(app, "open", "Open Quince", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Quince", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("quince")
        .tooltip("Quince")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => shutdown(app, 0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    login::note_launch();
    steady_drawing();
    smooth_video();
    tauri::Builder::default()
        // One Quince at a time: opening it again brings back the one already running, which
        // may be sitting in the tray with its window closed.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // The window shows Quince and nothing else. A link to anywhere outside it — a phone
        // number, an address, a web page in a note or an event — goes to the app this
        // computer uses for it. Followed in place, a `tel:` link replaced the whole window
        // with "URL can't be shown", and there was no way back.
        .plugin(
            tauri::plugin::Builder::<tauri::Wry, ()>::new("links")
                .on_navigation(|webview, url| {
                    let inside =
                        matches!(url.scheme(), "tauri" | "quince" | "about" | "data" | "blob")
                            || url
                                .host_str()
                                .is_some_and(|h| h == "tauri.localhost" || h == "localhost");
                    if !inside {
                        use tauri_plugin_opener::OpenerExt;
                        if let Err(e) = webview
                            .app_handle()
                            .opener()
                            .open_url(url.as_str(), None::<&str>)
                        {
                            quiet!("a link could not be opened: {e}");
                        }
                    }
                    inside
                })
                .build(),
        )
        // Closing the window hides it rather than quitting, so reminders still ring. Quit is
        // in the tray. Where no tray could be made there is nowhere to come back from, so
        // there closing quits as it always did.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.app_handle().tray_by_id("quince").is_some() {
                    api.prevent_close();
                    window.hide().ok();
                }
            }
        })
        // Images are served, not messaged. `quince://photo/<contact id>` is loaded by the
        // webview itself, which is what keeps a scrolling list cheap.
        .register_asynchronous_uri_scheme_protocol("quince", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            // WebKitGTK reads `quince://photo/<id>` with "photo" as the host, while Tauri's
            // own helper produces `quince://localhost/photo/<id>`. Accept either, rather than
            // depending on which one the caller happened to build.
            let uri = request.uri();
            let wanted = photo_guid(uri.host(), uri.path());
            let attachment = note_part(uri.path());
            // A drawing's ink for a dark page, rather than a light one.
            let ink_dark = uri.path().ends_with("drawing-dark");
            let picture = photos::requested(uri.path());
            let fit = photos::requested_fit(uri.path());
            let queued = photos::requested_upload(uri.path());
            tauri::async_runtime::spawn(async move {
                // A file on its way into Photos, small, for the upload panel.
                if let Some(id) = queued {
                    let response = match photos::upload_picture(&app, &id).await {
                        Ok(bytes) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", image_type(&bytes))
                            .body(bytes),
                        Err(_) => tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new()),
                    };
                    responder.respond(response.unwrap_or_default());
                    return;
                }
                // A photo's picture made the size it is shown at.
                if let Some((id, long)) = fit {
                    let response = match photos::fitted(&app, &id, long).await {
                        Ok(bytes) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", image_type(&bytes))
                            .header("Cache-Control", "private, max-age=86400")
                            .body(bytes),
                        Err(_) => tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new()),
                    };
                    responder.respond(response.unwrap_or_default());
                    return;
                }
                // A photo's picture: from the disk cache, or fetched from Apple once and kept.
                if let Some((id, size)) = picture {
                    let response = match photos::picture(&app, &id, size).await {
                        Ok(bytes) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", photos::content_type(&bytes, size))
                            .header("Cache-Control", "private, max-age=86400")
                            .body(bytes),
                        Err(_) => tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new()),
                    };
                    responder.respond(response.unwrap_or_default());
                    return;
                }
                // A note's picture, or the rendering of a document or film in it.
                if let Some((id, part)) = attachment {
                    let response = match note_media_shown(&app, &id, part, ink_dark).await {
                        Ok(bytes) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", image_type(&bytes))
                            .header("Cache-Control", "private, max-age=86400")
                            .body(bytes),
                        Err(_) => tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new()),
                    };
                    responder.respond(response.unwrap_or_default());
                    return;
                }
                let Some(guid) = wanted else {
                    responder.respond(
                        tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new())
                            .unwrap_or_default(),
                    );
                    return;
                };
                match contact_photo(&app, &guid).await {
                    Ok(Photo { bytes, durable }) => responder.respond(
                        tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", image_type(&bytes))
                            // A picture with a signature cannot change without the signature
                            // changing too, so the webview may hold it. One without can change
                            // under us, so it is only held long enough to scroll a list.
                            .header(
                                "Cache-Control",
                                if durable {
                                    "private, max-age=604800"
                                } else {
                                    "private, max-age=300"
                                },
                            )
                            .body(bytes)
                            .unwrap_or_default(),
                    ),
                    Err(_) => responder.respond(
                        tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new())
                            .unwrap_or_default(),
                    ),
                }
            });
        })
        .setup(|app| {
            video::serve(app.handle().clone());
            outbox::start(app.handle().clone());
            let dir = state_dir(app.handle());
            keep_private(&dir);
            let client = Arc::new(Mutex::new(Client::new(&dir)?));
            app.manage(Auth {
                client: Arc::clone(&client),
                signing_in: AtomicBool::new(false),
            });
            // The local library. Opened once at startup so every command shares one
            // connection and the WAL behaves.
            let mut library = Library::open(&dir)?;
            adopt_writer(&mut library);
            app.manage(Data(Mutex::new(library)));
            // A photo cache past what this computer can spare now — the disk filled since —
            // is brought back under its budget first thing.
            let trim = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || photos::trim_cache(&trim));
            // Logging out or shutting down ends the app with a signal, not through its own
            // quit, and a crash ends it with neither: either way the iCloud Drive folder is
            // let go of first, never left behind answering "Transport endpoint is not
            // connected".
            #[cfg(unix)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
                        signal(SignalKind::terminate()),
                        signal(SignalKind::interrupt()),
                        signal(SignalKind::hangup()),
                    ) else {
                        return;
                    };
                    // The first: ended as Quit ends it. A second while that is under way (Ctrl-C
                    // pressed again): ended at once.
                    let mut first = true;
                    loop {
                        tokio::select! {
                            _ = term.recv() => {}
                            _ = int.recv() => {}
                            _ = hup.recv() => {}
                        }
                        if !first {
                            // SAFETY: ends the process at once; nothing after it runs.
                            unsafe { libc::_exit(130) };
                        }
                        first = false;
                        let app = handle.clone();
                        std::thread::spawn(move || shutdown(&app, 0));
                    }
                });
                let before = std::panic::take_hook();
                std::panic::set_hook(Box::new(move |info| {
                    mount::release();
                    before(info);
                }));
            }
            app.manage(Occasions {
                feed: Mutex::new((None, Vec::new())),
            });
            spawn_holidays(app.handle().clone());
            spawn_heartbeat(Arc::clone(&client));
            push::spawn(app.handle().clone(), Arc::clone(&client));
            spawn_auto_sync(app.handle().clone(), client);
            alerts::spawn(app.handle().clone());
            alerts::spawn_events(app.handle().clone());
            // A tiling compositor places, sizes and closes windows itself; a title bar with
            // minimise and maximise there only takes room and does nothing it does not. On a
            // desktop that floats windows — GNOME, KDE — it is how a window is moved, so it stays.
            if tiling_compositor() {
                if let Some(window) = app.get_webview_window("main") {
                    window.set_decorations(false).ok();
                }
            }
            if let Err(e) = build_tray(app) {
                quiet!("no tray on this desktop, so closing the window quits: {e}");
            }
            show_when_ready(app.handle());
            // Photos uploads a quit left half done carry on.
            photos::work(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            auth_status,
            auth_resume,
            auth_sign_in,
            auth_submit_code,
            auth_two_factor_options,
            auth_resend_code,
            auth_send_sms,
            auth_submit_sms_code,
            auth_sign_out,
            account_unsent,
            search_everything,
            login::login_item_status,
            mount::drive_kept,
            mount::drive_keep,
            login::login_item_set,
            account_services,
            account_info,
            settings_get,
            settings_set,
            sync_all,
            sync_status,
            calendar_sync,
            calendar_events,
            event_save,
            event_alerts,
            event_repeat,
            event_delete,
            calendar_create,
            calendar_update,
            calendar_delete,
            calendar_list,
            calendar_search,
            reminder_lists,
            reminder_groups,
            reminder_sections,
            reminders_search,
            reminder_counts,
            reminders_in,
            reminders_due_between,
            reminder_create,
            reminder_delete,
            reminder_edit,
            reminders_smart,
            reminder_list_create,
            reminder_list_rename,
            reminder_list_delete,
            reminder_list_colour,
            reminder_reorder,
            note_folders,
            note_folder_create,
            note_folder_rename,
            note_folder_delete,
            note_body,
            note_export_markdown,
            note_runs,
            note_table,
            note_table_set_cell,
            window_ready,
            photos::photos_list,
            photos::photos_counts,
            photos::photos_albums,
            photos::photos_in_album,
            photos::photo_set_flag,
            photos::photo_play,
            mount::drive_folder_status,
            mount::drive_folder_set,
            mount::drive_folder_open,
            mount::photos_folder_status,
            mount::photos_folder_set,
            mount::photos_folder_open,
            photos::photo_video_turn,
            photos::photos_set_flag,
            photos::photos_burst,
            photos::photos_shared_albums,
            photos::photos_shared_create,
            photos::photos_shared_delete,
            photos::photos_prepare,
            video::video_base,
            window_tint,
            window_fullscreen,
            photos::photos_upload,
            photos::photos_upload_cancel,
            photos::photos_album_create,
            photos::photos_album_rename,
            photos::photos_album_delete,
            photos::photos_album_add,
            photos::photos_album_remove,
            photos::photo_set_text,
            photos::photo_set_rating,
            photos::photo_set_keywords,
            photos::photo_open_map,
            photos::photo_set_date,
            photos::photos_search,
            photos::photo_download,
            note_table_add_row,
            note_table_remove,
            note_attachment_info,
            note_attachment_open,
            diagnostics_save,
            note_paragraph,
            note_save,
            note_create,
            note_delete,
            note_move,
            note_pin,
            notes_in,
            notes_search,
            note_counts,
            contacts_list,
            contact_save,
            contact_lists,
            contacts_export,
            contacts_import,
            contact_me,
            contact_set_me,
            contact_list_write,
            contact_delete,
            drive_sync_folder,
            drive_children,
            drive_download,
            drive_upload,
            drive_create_folder,
            drive_rename,
            drive_delete,
            drive_trash,
            drive_recents,
            storage_info,
            drive_duplicate,
            drive_thumbnails,
            drive_put_back,
            drive_delete_forever,
            drive_trash_out,
            drive_upload_cancel,
            drive_move,
            drive_item,
            reminder_set_done,
            reminder_set_repeat,
        ])
        .build(tauri::generate_context!())
        .expect("Quince failed to start")
        .run(|_app, event| {
            // The iCloud Drive folder goes with the app: left mounted, it would answer
            // nothing ("Transport endpoint is not connected").
            if let tauri::RunEvent::Exit = event {
                shutdown(_app, 0);
            }
        });
}

#[cfg(test)]
mod tests {

    /// Two Apple IDs on one computer: each one's library and caches set aside whole while the
    /// other is signed in, and back exactly as they were — never mixed, never deleted — with
    /// this computer's settings staying put through both switches.
    #[test]
    fn accounts_are_set_aside_and_brought_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let mut lib = Library::open(dir).unwrap();
        lib.set_state(ACCOUNT_KEY, "a").unwrap();
        lib.set_state("contacts.me", "A's card").unwrap();
        lib.set_state("settings.alerts.events", "0").unwrap();
        std::fs::create_dir_all(dir.join("photos")).unwrap();
        std::fs::write(dir.join("photos").join("one"), b"A's photo").unwrap();

        // B signs in: A goes aside, B starts clean.
        switch_library(&mut lib, dir, "a", &dir.join(SET_ASIDE).join("b")).unwrap();
        assert_eq!(
            lib.state("contacts.me").unwrap(),
            None,
            "nothing of A's in B's library"
        );
        assert!(
            !dir.join("photos").join("one").exists(),
            "nor in its caches"
        );
        assert_eq!(
            lib.state("settings.alerts.events").unwrap().as_deref(),
            Some("0")
        );
        assert!(dir.join(SET_ASIDE).join("a").join("library.db").exists());
        lib.set_state(ACCOUNT_KEY, "b").unwrap();
        lib.set_state("contacts.me", "B's card").unwrap();
        lib.set_state("settings.alerts.events", "1").unwrap();

        // A again: B aside, A back as it was.
        switch_library(&mut lib, dir, "b", &dir.join(SET_ASIDE).join("a")).unwrap();
        assert_eq!(lib.state(ACCOUNT_KEY).unwrap().as_deref(), Some("a"));
        assert_eq!(
            lib.state("contacts.me").unwrap().as_deref(),
            Some("A's card")
        );
        assert_eq!(
            std::fs::read(dir.join("photos").join("one")).unwrap(),
            b"A's photo"
        );
        assert_eq!(
            lib.state("settings.alerts.events").unwrap().as_deref(),
            Some("1"),
            "the computer's latest setting, not the one A left"
        );
        assert!(
            !dir.join(SET_ASIDE).join("a").exists(),
            "its shelf put away"
        );
        assert!(dir.join(SET_ASIDE).join("b").join("library.db").exists());
    }

    /// A move that cannot finish undoes itself, and never lands over what is already there.
    #[test]
    fn a_move_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir_all(from.join("photos")).unwrap();
        std::fs::create_dir_all(to.join("faces")).unwrap();
        std::fs::write(from.join("library.db"), b"mine").unwrap();
        assert!(move_account(&from, &to).is_err(), "something already there");
        assert!(
            from.join("library.db").exists() && from.join("photos").exists(),
            "nothing moved"
        );
    }

    use super::*;

    /// Apple serves contact pictures as PNG, which is not what the first version of this
    /// assumed. The type is read from the bytes so a change of format is not a broken image.
    #[test]
    fn an_image_is_recognised_by_its_own_first_bytes() {
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), "image/png");
        assert_eq!(image_type(b"\xff\xd8\xff\xe0JFIF"), "image/jpeg");
        assert_eq!(image_type(b"GIF89a"), "image/gif");
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(image_type(b"\0\0\0\x18ftypheic"), "image/heic");
        assert_eq!(image_type(b"not a picture"), "application/octet-stream");
        assert_eq!(image_type(b""), "application/octet-stream", "and no panic");
    }

    /// A signature is base64 and holds `/` and `+`; a filename cannot. Two different pictures
    /// must still land on two different files.
    #[test]
    fn a_cached_picture_is_named_safely_and_uniquely() {
        let dir = std::path::Path::new("/tmp/quince");
        let one = photo_path(dir, "A-B/C", "FAhjvQxoOa/G338LIq+u7A==");
        let two = photo_path(dir, "A-B/C", "ZZhjvQxoOa/G338LIq+u7A==");
        let name = one.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "a filename Apple's base64 cannot break: {name}"
        );
        assert_ne!(one, two, "a new picture must not read the old one's cache");
    }

    /// The address bar of the interface is not a path: a contact id arrives encoded.
    #[test]
    fn a_percent_encoded_id_comes_back_as_itself() {
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        // What convertFileSrc("photo/<id>", "quince") builds, and what WebKitGTK reads.
        let id = "9C2B1E7A-0F4D-4E25-8A61-3C1D2E4F5A6B";
        assert_eq!(
            photo_guid(Some("localhost"), &format!("/photo%2F{id}")).as_deref(),
            Some(id)
        );
        assert_eq!(
            photo_guid(Some("localhost"), &format!("/photo/{id}")).as_deref(),
            Some(id)
        );
        assert_eq!(
            photo_guid(Some("photo"), &format!("/{id}")).as_deref(),
            Some(id)
        );
        assert_eq!(photo_guid(Some("localhost"), "/other%2Fx"), None);
        assert_eq!(photo_guid(Some("localhost"), "/photo%2F"), None);
        // A note's attachments, as convertFileSrc("note/<id>/preview", "quince") builds them.
        assert!(
            matches!(note_part("/note%2FABC-1%2Fpreview"), Some((ref id, quince_icloud::note_media::Part::Preview)) if id == "ABC-1")
        );
        assert!(matches!(
            note_part("/note%2FABC-1%2Ffile"),
            Some((_, quince_icloud::note_media::Part::File))
        ));
        assert!(note_part("/note%2FABC-1%2Fother").is_none());
        assert_eq!(percent_decode("plain-uuid"), "plain-uuid");
        assert_eq!(percent_decode("%zz"), "%zz", "malformed stays as it was");
    }
}
