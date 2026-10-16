// Quince — desktop shell.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
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
    refresh_reminders(http, root, params, data, true).await
}

/// Ask Apple what changed in Reminders and store it — without holding the library while
/// Apple answers.
///
/// Every screen reads that library. A refresh that held it across the network made every
/// screen wait on Apple: when CloudKit was slow or overloaded (503, retry in 10s), Reminders,
/// Notes and the rest drew nothing at all until it gave up. The fetch runs unlocked; the
/// library is held only to store what came back.
async fn refresh_reminders(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
    whole: bool,
) -> Result<(), String> {
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
    }
    alerts::changed();
    Ok(())
}

/// The same for Notes.
async fn refresh_notes(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> Result<(), String> {
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
async fn auth_resume(auth: State<'_, Auth>) -> UiResult<&'static str> {
    let mut client = auth.client.lock().await;
    let resumed = client.resume().await.map_err(|e| e.to_string())?;
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

    // Asked of Apple unlocked; the library is held only to store the answer.
    let fetched = quince_icloud::sync::fetch_calendar(&http, &root, params, &from, &to, &timezone)
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
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

    let ctag = quince_icloud::sync::calendar_ctag(&*data.0.lock().await, &event.calendar_guid);
    let kept = quince_icloud::sync::save_event(&http, &root, params, ctag.as_deref(), &event)
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    quince_icloud::sync::store_events(&mut library, kept).map_err(|e| e.to_string())?;
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
    let ctag = quince_icloud::sync::calendar_ctag(&*data.0.lock().await, &event.calendar_guid);
    quince_icloud::calendar::CalendarClient::new(&http, &root, params)
        .delete_event(&event, ctag.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    let mut library = data.0.lock().await;
    library
        .forget_events(std::slice::from_ref(&event.guid))
        .map_err(|e| e.to_string())
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
async fn set_done(
    auth: &State<'_, Auth>,
    data: &State<'_, Data>,
    guid: &str,
    list_guid: &str,
    done: bool,
) -> UiResult<()> {
    let (http, root, params) = cloudkit_context(auth).await?;

    let reminder = {
        let library = data.0.lock().await;
        library
            .reminders_in(list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

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
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::delete_cloudkit(&http, &root, params.clone(), &reminder)
        .await
        .map_err(|e| e.to_string())?;

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
            .reminders_in(&list_guid, true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.guid == guid)
            .ok_or("That reminder is no longer in the local library.")?
    };

    quince_icloud::reminders::edit_cloudkit(&http, &root, params.clone(), &reminder, &edit)
        .await
        .map_err(|e| e.to_string())?;

    // A due date on its own notifies nobody. Every reminder on a real account that has a
    // time carries an Alarm pointing at an AlarmTrigger, and one written here used to have
    // the date and no alarm — so a phone showed the right time and then said nothing when it
    // came. The alert is part of setting a date, not a separate thing to remember.
    if let Some(due) = edit.due_at {
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
        .map_err(|e| e.to_string())?;
    }

    refresh_reminders(&http, &root, params, &data, false).await
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
    refresh_notes(http, root, params, data).await
}

/// One note, by record name. `None` once Apple no longer has it.
async fn note_by_guid(data: &State<'_, Data>, guid: &str) -> Result<Note, String> {
    let library = data.0.lock().await;
    // Recently Deleted too: "All" leaves it out, and Delete Now and Recover act on what is
    // in it — looking only in "All" answered every one of them "no longer in the library".
    let all = library.notes_in(None).map_err(|e| e.to_string())?;
    let trash = library
        .notes_in(Some(quince_core::notes::TRASH_FOLDER))
        .map_err(|e| e.to_string())?;
    all.into_iter()
        .chain(trash)
        .find(|n| n.guid == guid)
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
    resync_notes(&http, &root, params, &data).await?;

    // Apple's change feed does not always carry a new record back on the very next pass.
    // Without this the note just made was missing locally, and the first edit to it failed
    // with "That note is no longer in the local library" — typing into a fresh note, then
    // pasting, lost the paste's save. Saving reads the record from Apple, so a local stand-in
    // with no tag is enough until the feed catches up.
    let mut library = data.0.lock().await;
    let known = library
        .notes_in(None)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|n| n.guid == guid);
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
    resync_notes(&http, &root, params, &data).await
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
    let items = quince_icloud::drive::fetch_folder(&http, &root, params, &folder_id)
        .await
        .map_err(|e| e.to_string())?;
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
    /// `waiting`, `sending`, `verifying`, `done` or `failed`.
    state: &'static str,
    error: Option<String>,
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
) -> Result<String, String> {
    resync_drive_folder(http, root, params.to_vec(), data, parent).await?;
    let existing = {
        let library = data.0.lock().await;
        library
            .drive_children(parent)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|i| i.is_container() && i.name.eq_ignore_ascii_case(name))
    };
    if let Some(folder) = existing {
        return Ok(folder.drivewsid);
    }
    quince_icloud::drive::create_folder(http, root, params.to_vec(), parent, name)
        .await
        .map(|f| f.drivewsid)
        .map_err(|e| e.to_string())
}

/// Every file under the paths dropped or chosen, with folders made to match on the way.
///
/// Links are not followed: one that points back up the tree would never finish, and one that
/// points outside it would upload something nobody chose.
#[allow(clippy::too_many_arguments)]
async fn plan_upload(
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
    // (path on disk, Drive folder it goes into, folders below the drop target)
    let mut pending: Vec<(std::path::PathBuf, String, String)> = paths
        .iter()
        .map(|p| {
            (
                std::path::PathBuf::from(p),
                folder_id.to_string(),
                String::new(),
            )
        })
        .collect();
    while let Some((path, into, within)) = pending.pop() {
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
        if meta.is_dir() {
            let folder = match folder_for(http, root, params, data, &into, &name).await {
                Ok(folder) => folder,
                Err(e) => {
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
            for child in children.into_iter().rev() {
                pending.push((child, folder.clone(), below.clone()));
            }
        } else if meta.is_file() {
            jobs.push(UploadJob {
                event: UploadEvent {
                    id: format!("{batch}:{}", jobs.len()),
                    batch: batch.to_string(),
                    name,
                    within,
                    sent: 0,
                    total: meta.len(),
                    state: "waiting",
                    error: None,
                },
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
    for job in &jobs {
        app.emit(UPLOAD, &job.event).ok();
    }

    let results: Vec<Option<String>> = futures_util::stream::iter(jobs)
        .map(|job| {
            let (app, http, docws, root, params) = (
                app.clone(),
                http.clone(),
                docws.clone(),
                root.clone(),
                params.clone(),
            );
            async move {
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
            }
        })
        .buffer_unordered(3)
        .collect()
        .await;
    failed.extend(results.into_iter().flatten());

    resync_drive_folder(&http, &root, params, &data, &folder_id).await?;
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
    keep_signed_in(&app, &client).await;
    let result = refresh::all(&client, &data.0).await;
    alerts::changed();
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
    dir.join("photos").join(format!("{guid_safe}-{safe}"))
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
            "quit" => app.exit(0),
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
    tauri::Builder::default()
        // One Quince at a time: opening it again brings back the one already running, which
        // may be sitting in the tray with its window closed.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
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
            let mut path = uri.path().trim_start_matches('/').to_string();
            if uri.host() == Some("photo") {
                path = format!("photo/{path}");
            }
            tauri::async_runtime::spawn(async move {
                let Some(guid) = path.strip_prefix("photo/") else {
                    responder.respond(
                        tauri::http::Response::builder()
                            .status(404)
                            .body(Vec::new())
                            .unwrap_or_default(),
                    );
                    return;
                };
                // Percent-encoding: a contact id is a plain UUID, but the scheme should not
                // assume what a future caller puts here.
                let guid = percent_decode(guid);
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
            let dir = state_dir(app.handle());
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
            app.manage(Occasions {
                feed: Mutex::new((None, Vec::new())),
            });
            spawn_holidays(app.handle().clone());
            spawn_heartbeat(Arc::clone(&client));
            spawn_auto_sync(app.handle().clone(), client);
            alerts::spawn(app.handle().clone());
            // A tiling compositor places, sizes and closes windows itself; a title bar with
            // minimise and maximise there only takes room and does nothing it does not. On a
            // desktop that floats windows — GNOME, KDE — it is how a window is moved, so it stays.
            if tiling_compositor() {
                if let Some(window) = app.get_webview_window("main") {
                    window.set_decorations(false).ok();
                }
            }
            if let Err(e) = build_tray(app) {
                eprintln!("no tray on this desktop, so closing the window quits: {e}");
            }
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
            note_folders,
            note_body,
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
            contact_delete,
            drive_sync_folder,
            drive_children,
            drive_download,
            drive_upload,
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

#[cfg(test)]
mod tests {
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
        assert_eq!(percent_decode("plain-uuid"), "plain-uuid");
        assert_eq!(percent_decode("%zz"), "%zz", "malformed stays as it was");
    }
}
