// Quince — desktop shell.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod refresh;

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
    let mut library = data.0.lock().await;
    quince_icloud::reminders::sync_cloudkit(http, root, params, &mut library, None)
        .await
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
async fn auth_resume(auth: State<'_, Auth>) -> UiResult<bool> {
    let mut client = auth.client.lock().await;
    client.resume().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_sign_in(
    apple_id: String,
    password: String,
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
        Ok(SignInOutcome::Complete) => Ok("complete"),
        Ok(SignInOutcome::NeedsTwoFactor) => Ok("needs_two_factor"),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn auth_submit_code(code: String, auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client
        .submit_two_factor(&code)
        .await
        .map_err(|e| e.to_string())
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
async fn auth_submit_sms_code(phone_id: u64, code: String, auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client
        .submit_sms_code(phone_id, &code)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn auth_sign_out(auth: State<'_, Auth>) -> UiResult<()> {
    let mut client = auth.client.lock().await;
    client.sign_out().map_err(|e| e.to_string())
}

/// Services Apple advertises for this account, for the sidebar.
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
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("calendar")
            .ok_or("Apple hasn't offered a calendar service for this account.")?;
        (client.http().clone(), root, client.service_params())
    };

    let mut library = data.0.lock().await;
    quince_icloud::sync::calendar(&http, &root, params, &mut library, &from, &to, &timezone)
        .await
        .map_err(|e| e.to_string())
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
#[tauri::command]
async fn event_save(
    draft: EventDraft,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let title = draft.title.trim();
    if title.is_empty() {
        return Err("An event needs a title.".into());
    }
    if draft.ends_at < draft.starts_at {
        return Err("An event can't end before it starts.".into());
    }
    let (http, root, params) = calendar_context(&auth).await?;

    // Apple's guids are uppercase UUIDs; a new event needs one before it can be addressed.
    let guid = draft
        .guid
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().to_uppercase());
    let existing = {
        let library = data.0.lock().await;
        library.event(&guid).map_err(|e| e.to_string())?
    };

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

    let mut library = data.0.lock().await;
    quince_icloud::sync::put_event(&http, &root, params, &mut library, &event)
        .await
        .map_err(|e| e.to_string())?;
    Ok(guid)
}

#[tauri::command]
async fn event_delete(guid: String, auth: State<'_, Auth>, data: State<'_, Data>) -> UiResult<()> {
    let (http, root, params) = calendar_context(&auth).await?;
    let event = {
        let library = data.0.lock().await;
        library
            .event(&guid)
            .map_err(|e| e.to_string())?
            .ok_or("That event is no longer in the local library.")?
    };
    let mut library = data.0.lock().await;
    quince_icloud::sync::delete_event(&http, &root, params, &mut library, &event)
        .await
        .map_err(|e| e.to_string())
}

/// Events overlapping `[from, to]`, as unix seconds. Read straight from the library.
#[tauri::command]
async fn calendar_events(from: i64, to: i64, data: State<'_, Data>) -> UiResult<Vec<Event>> {
    let library = data.0.lock().await;
    library.events_between(from, to).map_err(|e| e.to_string())
}

#[tauri::command]
async fn calendar_list(data: State<'_, Data>) -> UiResult<Vec<Calendar>> {
    let library = data.0.lock().await;
    library.calendars().map_err(|e| e.to_string())
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

#[tauri::command]
async fn reminder_lists(data: State<'_, Data>) -> UiResult<Vec<ReminderList>> {
    let library = data.0.lock().await;
    library.reminder_lists().map_err(|e| e.to_string())
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
    let (http, root, params) = cloudkit_context(&auth).await?;

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::set_completed_cloudkit(&http, &root, params.clone(), &reminder, done)
        .await
        .map_err(|e| e.to_string())?;

    let mut library = data.0.lock().await;
    let since = library.sync_token("reminders").map_err(|e| e.to_string())?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params, &mut library, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
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
    resync_reminders(&http, &root, params, &data).await
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
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    quince_icloud::reminders::create_cloudkit(&http, &root, params.clone(), &list_guid, &title)
        .await
        .map_err(|e| e.to_string())?;

    let mut library = data.0.lock().await;
    let since = library.sync_token("reminders").map_err(|e| e.to_string())?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params, &mut library, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::delete_cloudkit(&http, &root, params.clone(), &reminder)
        .await
        .map_err(|e| e.to_string())?;

    let mut library = data.0.lock().await;
    let since = library.sync_token("reminders").map_err(|e| e.to_string())?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params, &mut library, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::edit_cloudkit(&http, &root, params.clone(), &reminder, &edit)
        .await
        .map_err(|e| e.to_string())?;

    let mut library = data.0.lock().await;
    let since = library.sync_token("reminders").map_err(|e| e.to_string())?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, params, &mut library, since.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
    resync_reminders(&http, &root, params, &data).await
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
    resync_reminders(&http, &root, params, &data).await
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
    resync_reminders(&http, &root, params, &data).await
}

/// Re-read notes after a write, so what is on screen is what Apple stored.
async fn resync_notes(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
    let mut library = data.0.lock().await;
    let since = library.sync_token("notes").map_err(|e| e.to_string())?;
    quince_icloud::notes::sync_cloudkit(http, root, params, &mut library, since.as_deref())
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// One note, by record name. `None` once Apple no longer has it.
async fn note_by_guid(data: &State<'_, Data>, guid: &str) -> Result<Note, String> {
    let library = data.0.lock().await;
    library
        .notes_in(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|n| n.guid == guid)
        .ok_or_else(|| "That note is no longer in the local library.".into())
}

/// Save a note's body.
///
/// Writes to Apple first and re-reads after, so the editor never shows a version Apple has
/// not accepted. A note whose document cannot be rewritten safely is refused with the
/// reason rather than flattened.
#[tauri::command]
async fn note_save(
    guid: String,
    body: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    if !note.editable {
        return Err(
            "This note holds formatting Quince can't rewrite safely, so it stays read-only.".into(),
        );
    }
    quince_icloud::notes::edit_cloudkit(&http, &root, params.clone(), &note, &body)
        .await
        .map_err(|e| e.to_string())?;
    resync_notes(&http, &root, params, &data).await?;

    // Apple accepted the body, but its change feed does not always carry our own write back
    // on the very next pass. Without this the editor re-reads the old text a moment after
    // saving, which reads as the edit having been lost — and typing on from there would
    // save the stale version over the good one.
    let (title, snippet) = quince_icloud::notes::title_and_snippet(&body);
    let mut library = data.0.lock().await;
    if let Some(mut stored) = library
        .notes_in(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|n| n.guid == guid)
    {
        stored.body = Some(body);
        stored.title = title;
        stored.snippet = Some(snippet).filter(|s| !s.is_empty());
        library.put_notes(&[stored]).map_err(|e| e.to_string())?;
    }
    Ok(())
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
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        params.clone(),
        folder_guid.as_deref(),
        &body,
    )
    .await
    .map_err(|e| e.to_string())?;
    resync_notes(&http, &root, params, &data).await?;
    Ok(guid)
}

#[tauri::command]
async fn note_delete(guid: String, auth: State<'_, Auth>, data: State<'_, Data>) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(&auth).await?;
    let note = note_by_guid(&data, &guid).await?;
    quince_icloud::notes::delete_cloudkit(&http, &root, params.clone(), &note)
        .await
        .map_err(|e| e.to_string())?;
    resync_notes(&http, &root, params, &data).await
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
    resync_notes(&http, &root, params, &data).await
}

#[tauri::command]
async fn note_folders(data: State<'_, Data>) -> UiResult<Vec<NoteFolder>> {
    let library = data.0.lock().await;
    library.note_folders().map_err(|e| e.to_string())
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

/// Create a contact, or save changes to one. Returns the id Apple knows it by.
///
/// An edit starts from Apple's own card, so everything this app does not model — a photo,
/// related names, social profiles — is written back untouched rather than dropped.
#[tauri::command]
async fn contact_save(
    contact: Contact,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
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
        quince_icloud::contacts::update(
            &http,
            &root,
            params.clone(),
            &tokens,
            &contact,
            card.as_ref(),
        )
        .await
        .map_err(|e| e.to_string())?;
        contact.guid.clone()
    };

    resync_contacts(&http, &root, params, &data).await?;
    Ok(guid)
}

#[tauri::command]
async fn contact_delete(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
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
    resync_contacts(&http, &root, params, &data).await
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
    let mut library = data.0.lock().await;
    quince_icloud::drive::sync_folder(&http, &root, params, &mut library, &folder_id)
        .await
        .map_err(|e| e.to_string())
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
    let mut library = data.0.lock().await;
    quince_icloud::drive::sync_folder(http, root, params, &mut library, folder_id)
        .await
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
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let item = {
        let library = data.0.lock().await;
        library
            .drive_item(&drivewsid)
            .map_err(|e| e.to_string())?
            .ok_or("That item is no longer in the local library.")?
    };
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
    resync_drive_folder(&http, &root, params, &data, &parent_id).await
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
    resync_drive_folder(&http, &root, params, &data, parent).await
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
    resync_drive_folder(&http, &root, params, &data, parent).await
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
    resync_drive_folder(&http, &root, params.clone(), &data, old_parent).await?;
    if old_parent != destination_id {
        resync_drive_folder(&http, &root, params, &data, &destination_id).await?;
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
    let result = refresh::all(&client, &data.0).await;
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
            let data = app.state::<Data>();
            let result = refresh::all(&client, &data.0).await;
            app.emit(SYNCED, &result).ok();
        }
    });
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = state_dir(app.handle());
            let client = Arc::new(Mutex::new(Client::new(&dir)?));
            app.manage(Auth {
                client: Arc::clone(&client),
                signing_in: AtomicBool::new(false),
            });
            // The local library. Opened once at startup so every command shares one
            // connection and the WAL behaves.
            app.manage(Data(Mutex::new(Library::open(&dir)?)));
            spawn_heartbeat(Arc::clone(&client));
            spawn_auto_sync(app.handle().clone(), client);
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
            account_services,
            sync_all,
            sync_status,
            calendar_sync,
            calendar_events,
            event_save,
            event_delete,
            calendar_list,
            reminder_lists,
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
            note_folders,
            note_save,
            note_create,
            note_delete,
            note_move,
            notes_in,
            contacts_list,
            contact_save,
            contact_delete,
            drive_sync_folder,
            drive_children,
            drive_download,
            drive_create_folder,
            drive_rename,
            drive_delete,
            drive_move,
            drive_item,
            reminder_set_done,
            reminder_set_repeat,
        ])
        .run(tauri::generate_context!())
        .expect("Quince failed to start");
}
