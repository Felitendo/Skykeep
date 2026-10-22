//! Notes commands for locked notes and attachments.
//!
//! Locked notes open read-only: the password crosses from the window once, opens the note,
//! and is kept in this process's memory for the session only — never in the library, the
//! settings or the keyring — so the same note opens again without asking until the app quits,
//! the account signs out, or the window says to lock again ([`note_lock_forget`]). The opened
//! text is handed to the window and not stored either: a locked note's words never reach disk
//! here.

use crate::{Auth, Data, UiResult};
use quince_icloud::notes::attach::Attached;
use quince_icloud::notes::locked::Unlocked;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// A password held in memory, wiped when let go of.
struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// The password that opened each locked note this session, by note.
static PASSWORDS: Mutex<Option<HashMap<String, Secret>>> = Mutex::new(None);

fn remembered(guid: &str) -> Option<String> {
    PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()?
        .get(guid)
        .and_then(|s| String::from_utf8(s.0.clone()).ok())
}

fn remember(guid: &str, password: &str) {
    PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(guid.to_string(), Secret(password.as_bytes().to_vec()));
}

/// Every remembered password let go of: on sign-out, or when the window locks notes again.
pub fn forget_passwords() {
    *PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Open a locked note: its text and formatting. With no password, the one that opened it
/// earlier this session is used; a wrong one is forgotten, never kept.
/// Whether attaching files to notes is switched on (`NOTE_ATTACH_WRITES`): the window shows
/// its paperclip and takes drops only then, so turning the backend flag on is the whole change.
#[tauri::command]
pub fn note_attach_enabled() -> bool {
    quince_icloud::notes::attach::NOTE_ATTACH_WRITES
}

#[tauri::command]
pub async fn note_unlock(
    guid: String,
    password: Option<String>,
    auth: State<'_, Auth>,
) -> UiResult<Unlocked> {
    let password = password
        .filter(|p| !p.is_empty())
        .or_else(|| remembered(&guid))
        .ok_or("Enter this note’s password to open it.")?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    match quince_icloud::notes::locked::unlock_cloudkit(&http, &root, params, &guid, &password)
        .await
    {
        Ok(opened) => {
            remember(&guid, &password);
            Ok(opened)
        }
        Err(e) => {
            if let Some(map) = PASSWORDS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                map.remove(&guid);
            }
            Err(e.to_string())
        }
    }
}

/// The hint left with a locked note's password, if any.
#[tauri::command]
pub async fn note_lock_hint(guid: String, auth: State<'_, Auth>) -> UiResult<Option<String>> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    quince_icloud::notes::locked::hint_cloudkit(&http, &root, params, &guid)
        .await
        .map_err(|e| e.to_string())
}

/// Lock every opened note again: the passwords this session held are wiped.
#[tauri::command]
pub fn note_lock_forget() {
    forget_passwords();
}

/// Add the file at `path` to a note at `at_offset` (UTF-16 units of `base`, the text the
/// editor shows, or of Apple's text when no base is given). Answers what was added, then
/// brings the library up to date.
/// A small picture of a file about to be added to a note — shown at once while the file goes
/// up to Apple, so adding never looks stuck. Only for a file the person just picked or dropped
/// (the same grant the add itself checks), read on this computer, never sent anywhere. Empty
/// for a file that is not a picture.
#[tauri::command]
pub async fn note_attach_preview(path: String) -> UiResult<tauri::ipc::Response> {
    crate::grants::take_read(&path)?;
    crate::cpu::run(move || {
        let Ok(bytes) = std::fs::read(&path) else {
            return Vec::new();
        };
        let bytes = if quince_media::is_heif(&bytes) {
            match quince_media::heic_to_jpeg(&bytes) {
                Ok(jpeg) => jpeg,
                Err(_) => return Vec::new(),
            }
        } else {
            bytes
        };
        match quince_media::fit(&bytes, 480) {
            Ok(Some(small)) => small,
            Ok(None) => bytes,
            Err(_) => Vec::new(),
        }
    })
    .await
    .map(tauri::ipc::Response::new)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn note_attach_file(
    guid: String,
    path: String,
    at_offset: u64,
    base: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Attached> {
    crate::grants::take_read(&path)?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let attached = quince_icloud::notes::attach::attach_file_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        std::path::Path::new(&path),
        base.as_deref(),
        at_offset,
    )
    .await
    .map_err(|e| e.to_string())?;
    // Apple has it. A refresh failing now must not read as the attach failing.
    if let Err(e) = crate::resync_notes(&http, &root, params, &data).await {
        quiet!("notes refresh after an attachment: {e}");
    }
    Ok(attached)
}
