//! Reminders commands for what sits beside a reminder: tags, its link, an early alert, custom
//! smart lists, and sections. Reads come from the library; writes go to Apple (each refused
//! there whenever `REMINDER_DETAIL_WRITES` is off) and the library is read again after.

use crate::{Auth, Data, UiResult};
use quince_core::reminders::{
    Reminder, ReminderDetail, ReminderMarks, ReminderMore, ReminderSection, ReminderSmartList,
    SmartListContents, TagCount,
};
use quince_icloud::reminders::{details, more};
use tauri::State;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// One reminder's tags, link and early alerts.
#[tauri::command]
pub async fn reminder_detail(guid: String, data: State<'_, Data>) -> UiResult<ReminderDetail> {
    let library = data.0.lock().await;
    library.reminder_detail(&guid).map_err(|e| e.to_string())
}

/// The rest of one reminder's Details: its repeat whole, time zone, a place, the person it
/// comes up for when messaging — from the library — and Urgent, which lives in a small file
/// read from Apple the first time it is asked for (read only; offline it stays unknown).
#[tauri::command]
pub async fn reminder_more(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<ReminderMore> {
    let (mut more, urgent_unread) = {
        let library = data.0.lock().await;
        let reminder = library.reminder(&guid).map_err(|e| e.to_string())?;
        // The record as Apple sent it; the reminder as read for the window leaves it out.
        let raw = library.reminder_raw(&guid).map_err(|e| e.to_string())?;
        let raw = raw.as_deref();
        let parts = library.reminder_parts(&guid).map_err(|e| e.to_string())?;
        let mut more = more::more_from(
            &guid,
            raw,
            reminder.as_ref().and_then(|r| r.timezone.as_deref()),
            &parts,
        );
        if let Some(person) = more.messaging.as_mut() {
            person.name = library
                .contact_name_for(&person.handles)
                .map_err(|e| e.to_string())?;
        }
        more.urgent = more::urgent_known(raw);
        let unread = more.urgent.is_none() && raw.is_some();
        (more, unread)
    };
    if urgent_unread {
        if let Ok((http, root, params)) = crate::cloudkit_context(&auth).await {
            more.urgent = more::urgent_cloudkit(&http, &root, params, &guid)
                .await
                .ok()
                .flatten();
        }
    }
    Ok(more)
}

/// Reminders whose tick now is the last of their repeat (End Repeat On Date): ticked, each is
/// completed rather than moved on, so the window shows it done as it does a one-off.
#[tauri::command]
pub async fn reminders_ending(data: State<'_, Data>) -> UiResult<Vec<String>> {
    let library = data.0.lock().await;
    let at = now();
    let mut out = Vec::new();
    for (guid, end) in library.reminder_repeat_ends().map_err(|e| e.to_string())? {
        if let Some(r) = library.reminder(&guid).map_err(|e| e.to_string())? {
            let days = library
                .reminder_repeat_days(&guid)
                .map_err(|e| e.to_string())?;
            if r.tick_ends_series_on(Some(end), &days, at) {
                out.push(guid);
            }
        }
    }
    Ok(out)
}

/// What each of these reminders' rows shows beside its date, as the phone's list does:
/// Weekdays or Weekends, the Urgent alarm, a place, a person, a link. Only rows with any of
/// them come back. Urgent is read from Apple for rows not read yet — a few at a time, read
/// only — and is shown off until it is known.
#[tauri::command]
pub async fn reminder_marks(
    guids: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<ReminderMarks>> {
    /// Urgent files read in one call at most; the rest are read on the next.
    const MOST_UNREAD: usize = 40;
    let mut out = Vec::new();
    let mut unread: Vec<(usize, String, String)> = Vec::new();
    {
        let library = data.0.lock().await;
        for guid in guids.iter().take(2_000) {
            let raw = library.reminder_raw(guid).map_err(|e| e.to_string())?;
            let parts = library.reminder_parts(guid).map_err(|e| e.to_string())?;
            let found = more::more_from(guid, raw.as_deref(), None, &parts);
            let messaging = match found.messaging {
                Some(person) => library
                    .contact_name_for(&person.handles)
                    .map_err(|e| e.to_string())?
                    .or_else(|| person.handles.first().cloned()),
                None => None,
            };
            let url = library
                .reminder_detail(guid)
                .map_err(|e| e.to_string())?
                .url;
            let urgent = more::urgent_known(raw.as_deref());
            let marks = ReminderMarks {
                guid: guid.clone(),
                days_of_week: found.repeat.map(|r| r.days_of_week).unwrap_or_default(),
                urgent: urgent.unwrap_or(false),
                location: found.location,
                messaging,
                url,
            };
            if urgent.is_none() && unread.len() < MOST_UNREAD {
                if let Some(raw) = raw {
                    unread.push((out.len(), guid.clone(), raw));
                }
            }
            out.push(marks);
        }
    }
    if !unread.is_empty() {
        if let Ok((http, root, params)) = crate::cloudkit_context(&auth).await {
            use futures_util::StreamExt;
            let read: Vec<(usize, Option<bool>)> = futures_util::stream::iter(unread)
                .map(|(at, guid, raw)| {
                    let (http, root, params) = (http.clone(), root.clone(), params.clone());
                    async move {
                        (
                            at,
                            more::urgent_for(&http, &root, params, &guid, &raw).await,
                        )
                    }
                })
                .buffer_unordered(6)
                .collect()
                .await;
            for (at, urgent) in read {
                out[at].urgent = urgent.unwrap_or(false);
            }
        }
    }
    out.retain(|m| {
        m.urgent
            || !m.days_of_week.is_empty()
            || m.location.is_some()
            || m.messaging.is_some()
            || m.url.is_some()
    });
    Ok(out)
}

/// Every tag on the account, with how many reminders still to do carry each.
#[tauri::command]
pub async fn reminder_tags(data: State<'_, Data>) -> UiResult<Vec<TagCount>> {
    let library = data.0.lock().await;
    library.reminder_tags().map_err(|e| e.to_string())
}

/// Reminders still to do carrying a tag.
#[tauri::command]
pub async fn reminders_tagged(name: String, data: State<'_, Data>) -> UiResult<Vec<Reminder>> {
    let library = data.0.lock().await;
    library.reminders_tagged(&name).map_err(|e| e.to_string())
}

/// The account's smart lists, built-in and custom.
#[tauri::command]
pub async fn reminder_smart_lists(data: State<'_, Data>) -> UiResult<Vec<ReminderSmartList>> {
    let library = data.0.lock().await;
    library.reminder_smart_lists().map_err(|e| e.to_string())
}

/// What a custom smart list holds now, and any part of its filter not understood.
#[tauri::command]
pub async fn reminders_in_smart_list(
    guid: String,
    data: State<'_, Data>,
) -> UiResult<SmartListContents> {
    let library = data.0.lock().await;
    library
        .reminders_in_smart_list(&guid, now())
        .map_err(|e| e.to_string())
}

/// A write's result, with the library read again either way: on success it brings Apple's
/// copy, on a refusal it puts back whatever was shown ahead of Apple.
async fn finish<T>(
    result: Result<T, quince_icloud::Error>,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> UiResult<T> {
    let refreshed = crate::resync_reminders(http, root, params, data).await;
    let value = result.map_err(|e| e.to_string())?;
    crate::settle(refreshed)?;
    Ok(value)
}

/// Tag a reminder.
#[tauri::command]
pub async fn reminder_tag_add(
    guid: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let made = details::add_tag_cloudkit(&http, &root, params.clone(), &guid, &name).await;
    finish(made.map(|_| ()), &http, &root, params, &data).await
}

/// Take a tag off a reminder.
#[tauri::command]
pub async fn reminder_tag_remove(
    guid: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::remove_tag_cloudkit(&http, &root, params.clone(), &guid, &name).await;
    finish(done, &http, &root, params, &data).await
}

/// Set a reminder's link, or clear it with `null`.
#[tauri::command]
pub async fn reminder_url_set(
    guid: String,
    url: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let url = url.filter(|u| !u.trim().is_empty());
    let done = details::set_url_cloudkit(&http, &root, params.clone(), &guid, url.as_deref()).await;
    finish(done, &http, &root, params, &data).await
}

/// Remind early: `count` units before it is due (unit 0 minutes, 1 hours, 2 days, 3 weeks,
/// 4 months), or `null` to clear.
#[tauri::command]
pub async fn reminder_early_alert_set(
    guid: String,
    count: Option<i64>,
    unit: Option<i64>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let account = {
        let library = data.0.lock().await;
        library.early_alert_account().map_err(|e| e.to_string())?
    };
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let alert = count.map(|c| (c, unit.unwrap_or(0)));
    let done = details::set_early_alert_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        alert,
        account.as_deref(),
    )
    .await;
    finish(done, &http, &root, params, &data).await
}

/// Make a section in a list. Answers it, already stored.
#[tauri::command]
pub async fn reminder_section_create(
    list_guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<ReminderSection> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let made =
        details::create_section_cloudkit(&http, &root, params.clone(), &list_guid, &title).await;
    if let Ok(section) = &made {
        let mut library = data.0.lock().await;
        library
            .put_reminder_section_local(section)
            .map_err(|e| e.to_string())?;
    }
    finish(made, &http, &root, params, &data).await
}

/// Rename a section.
#[tauri::command]
pub async fn reminder_section_rename(
    guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::rename_section_cloudkit(&http, &root, params.clone(), &guid, &title).await;
    finish(done, &http, &root, params, &data).await
}

/// Move a reminder into a section of its list, or out of every one with `null`. Shown here
/// at once; a refusal puts it back as Apple has it.
#[tauri::command]
pub async fn reminder_section_move(
    list_guid: String,
    guid: String,
    section: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    {
        let mut library = data.0.lock().await;
        library
            .set_reminder_section(&guid, section.as_deref())
            .map_err(|e| e.to_string())?;
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::move_to_section_cloudkit(
        &http,
        &root,
        params.clone(),
        &list_guid,
        &guid,
        section.as_deref(),
    )
    .await;
    finish(done, &http, &root, params, &data).await
}
