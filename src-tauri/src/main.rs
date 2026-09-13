// Quince — desktop shell.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use quince_auth::{Client, SignInOutcome, TwoFactorOptions};
use quince_core::calendar::{Calendar, Event};
use quince_core::reminders::{Reminder, ReminderList};
use quince_core::Library;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Manager, State};
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

/// Refresh reminders from Apple. Unlike calendar there is no date window, so one call.
#[tauri::command]
async fn reminders_sync(
    timezone: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<(usize, usize)> {
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("reminders")
            .ok_or("Apple hasn't offered a reminders service for this account.")?;
        (client.http().clone(), root, client.service_params())
    };
    let mut library = data.0.lock().await;
    quince_icloud::reminders::sync(&http, &root, params, &mut library, &timezone)
        .await
        .map_err(|e| e.to_string())
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

/// Tick a reminder off, or put it back. Writes to Apple first, then refreshes locally, so
/// the interface never shows a state Apple has not accepted.
#[tauri::command]
async fn reminder_set_done(
    guid: String,
    list_guid: String,
    done: bool,
    timezone: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = {
        let client = auth.client.lock().await;
        let root = client
            .session()
            .service_url("reminders")
            .ok_or("Apple hasn't offered a reminders service for this account.")?;
        (client.http().clone(), root, client.service_params())
    };

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::set_completed(&http, &root, &params, &reminder, done, &timezone)
        .await
        .map_err(|e| e.to_string())?;

    // Re-read from Apple so what we show is what Apple stored, not what we hoped.
    let mut library = data.0.lock().await;
    quince_icloud::reminders::sync(&http, &root, params, &mut library, &timezone)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn reminders_last_synced(data: State<'_, Data>) -> UiResult<Option<i64>> {
    let library = data.0.lock().await;
    library.last_synced("reminders").map_err(|e| e.to_string())
}

/// When the calendar last synced, as unix seconds.
#[tauri::command]
async fn calendar_last_synced(data: State<'_, Data>) -> UiResult<Option<i64>> {
    let library = data.0.lock().await;
    library.last_synced("calendar").map_err(|e| e.to_string())
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
            spawn_heartbeat(client);
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
            calendar_sync,
            calendar_events,
            calendar_list,
            calendar_last_synced,
            reminders_sync,
            reminder_lists,
            reminders_in,
            reminders_last_synced,
            reminder_set_done,
        ])
        .run(tauri::generate_context!())
        .expect("Quince failed to start");
}
