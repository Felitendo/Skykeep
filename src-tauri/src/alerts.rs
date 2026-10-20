//! Reminder alerts on the desktop, rung when the phone would ring them.
//!
//! One task sleeps until the next alarm and wakes for it — no polling. It also wakes when the
//! reminders change (a pass, or a write here), when the machine resumes from sleep (logind's
//! `PrepareForSleep`), and at the latest every ten minutes, because a sleep measured on the
//! monotonic clock does not count the time a machine spent suspended and a clock can be set.
//! Every alert is recorded as rung before it is shown, so none is ever shown twice — not by
//! two wakes racing, not after a restart.

use crate::{Auth, Data};
use quince_core::alarms::DueAlarm;
use std::time::Duration;
#[cfg(all(unix, not(target_os = "macos")))]
use tauri::Emitter;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

/// Something that can move the next alarm has happened.
static CHANGED: Notify = Notify::const_new();

/// The longest the task sleeps without looking again.
const LONGEST_SLEEP: Duration = Duration::from_secs(10 * 60);

/// How late an alert may still be shown. An alarm missed by less — the machine was asleep,
/// or Quince was not running — rings once when it can; one missed by more is recorded and
/// left, so opening Quince after a week away is not a wall of old alerts.
const STILL_WORTH_SAYING: i64 = 6 * 60 * 60;

/// Snooze, as the phone offers it.
#[cfg(all(unix, not(target_os = "macos")))]
const SNOOZE: i64 = 60 * 60;

/// The app's icon, carried in the binary so an alert shows it on any desktop. A notification
/// names its icon either from the icon theme — where nothing is installed for a copy run
/// straight from a download or a build, so alerts came up blank — or by file path, which
/// always resolves. So it is written once beside the library and named by path.
const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");
static ICON_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn icon_path(app: &AppHandle) -> Option<&'static str> {
    ICON_PATH
        .get_or_init(|| {
            let path = crate::state_dir(app).join("quince-alert.png");
            let current = std::fs::read(&path).ok();
            if current.as_deref() != Some(ICON) {
                std::fs::create_dir_all(path.parent()?).ok()?;
                std::fs::write(&path, ICON).ok()?;
            }
            Some(path.to_string_lossy().into_owned())
        })
        .as_deref()
}

/// Emitted to the window to open a reminder an alert was clicked for.
#[cfg(all(unix, not(target_os = "macos")))]
pub const OPEN_REMINDER: &str = "quince://open-reminder";

/// Tell the task to look again: the reminders or their alarms may have changed.
pub fn changed() {
    CHANGED.notify_one();
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What is due now, each recorded as rung, and when to look next.
///
/// Recording happens here, under the library's lock, before anything is shown: whatever
/// happens to the notification afterwards, this alert has had its one chance.
pub fn take_due(library: &mut quince_core::Library, now: i64) -> (Vec<DueAlarm>, Option<i64>) {
    let mut ring = Vec::new();
    for due in library.due_alarms(now).unwrap_or_default() {
        if library
            .mark_alarm_fired(&due.key, due.fires_at, now)
            .is_err()
        {
            // Not recorded means it could ring again; better to stay quiet than to repeat.
            continue;
        }
        if now - due.fires_at <= STILL_WORTH_SAYING {
            ring.push(due);
        }
    }
    (ring, library.next_alarm_after(now).unwrap_or(None))
}

/// Start the task. It runs for the life of the process, window open or not.
/// Settings' switches for each kind of alert. An alert due while its switch is off is let
/// go, not saved for later: turned back on, it does not ring for what has passed.
pub const REMINDER_ALERTS: &str = "settings.alerts.reminders";
pub const EVENT_ALERTS: &str = "settings.alerts.events";

/// Whether a kind of alert may ring. On unless Settings turned it off.
pub fn allowed(library: &quince_core::Library, key: &str) -> bool {
    library.state(key).ok().flatten().as_deref() != Some("off")
}

pub fn spawn(app: AppHandle) {
    watch_for_resume();
    tauri::async_runtime::spawn(async move {
        loop {
            let at = now();
            let (ring, next, on) = {
                let data = app.state::<Data>();
                let mut library = data.0.lock().await;
                let (ring, next) = take_due(&mut library, at);
                (ring, next, allowed(&library, REMINDER_ALERTS))
            };
            for due in ring.into_iter().filter(|_| on) {
                show(app.clone(), due);
            }
            let wait = next.map_or(LONGEST_SLEEP, |t| {
                Duration::from_secs(u64::try_from(t - at).unwrap_or(1).max(1)).min(LONGEST_SLEEP)
            });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = CHANGED.notified() => {}
            }
        }
    });
}

/// Wake when the machine comes back from sleep, where logind says so. Where it cannot be
/// asked — no system bus, no logind — the ten-minute ceiling still holds.
fn watch_for_resume() {
    tauri::async_runtime::spawn(async {
        let Ok(bus) = zbus::Connection::system().await else {
            return;
        };
        let Ok(logind) = zbus::Proxy::new(
            &bus,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        else {
            return;
        };
        let Ok(mut signals) = logind.receive_signal("PrepareForSleep").await else {
            return;
        };
        use futures_util::StreamExt;
        while let Some(signal) = signals.next().await {
            // `true` on the way down, `false` on the way back up.
            if let Ok(false) = signal.body().deserialize::<bool>() {
                changed();
            }
        }
    });
}

/// The alert, with the phone's two choices on it, waiting for an answer on its own thread.
fn show(app: AppHandle, due: DueAlarm) {
    std::thread::spawn(move || {
        let body = due
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(ToString::to_string)
            .or_else(|| due.list_title.clone())
            .unwrap_or_default();
        // The phone's choices on the alert are the freedesktop notification spec's actions. Windows
        // and macOS take no buttons through this crate, so the alert says what is due and
        // the reminder is completed or put off in the app.
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            notify_rust::Notification::new()
                .appname("Quince")
                .summary(&due.title)
                .body(&body)
                .icon(icon_path(&app).unwrap_or("quince"))
                .show()
                .ok();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let shown = notify_rust::Notification::new()
                .appname("Quince")
                .summary(&due.title)
                .body(&body)
                .icon(icon_path(&app).unwrap_or("quince"))
                .hint(notify_rust::Hint::Category("reminder".into()))
                .hint(notify_rust::Hint::DesktopEntry("quince".into()))
                .hint(notify_rust::Hint::Resident(true))
                .action("default", "Open")
                .action("complete", "Mark as Completed")
                .action("snooze", "Remind Me in 1 Hour")
                .timeout(notify_rust::Timeout::Never)
                .show();
            let Ok(handle) = shown else {
                return;
            };
            handle.wait_for_action(|action| match action {
                "complete" => {
                    let app = app.clone();
                    let due = due.clone();
                    tauri::async_runtime::block_on(async move {
                        let auth = app.state::<Auth>();
                        let data = app.state::<Data>();
                        if let Err(e) =
                            crate::set_done(&auth, &data, &due.reminder_guid, &due.list_guid, true)
                                .await
                        {
                            quiet!("an alert's reminder could not be completed: {e}");
                        }
                        // Every open screen looks again, as after any pass.
                        let report = crate::refresh::status(&data.0).await;
                        app.emit(crate::SYNCED, &report).ok();
                    });
                }
                "snooze" => {
                    let app = app.clone();
                    let guid = due.reminder_guid.clone();
                    tauri::async_runtime::block_on(async move {
                        let data = app.state::<Data>();
                        let mut library = data.0.lock().await;
                        library.snooze_reminder(&guid, now() + SNOOZE).ok();
                    });
                    changed();
                }
                "default" => {
                    crate::show_window(&app);
                    app.emit(OPEN_REMINDER, &due.reminder_guid).ok();
                }
                _ => {}
            });
        }
    });
}

/// Calendar alerts on the desktop, rung as the phone rings them.
///
/// Apple keeps an event's alerts beside it, read one event at a time, so this looks only at
/// what is coming: the events starting in the next two days, their alerts asked for once and
/// asked again only when the event changes. Each alert is recorded as rung before it shows,
/// under the same rule as reminders' — never twice, and one missed by more than a few hours
/// is let go rather than shown late.
pub fn spawn_events(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Alerts already read, by event: its version, and minutes before its start.
        let mut known: std::collections::HashMap<String, (Option<String>, Vec<i64>)> =
            std::collections::HashMap::new();
        loop {
            let at = now();
            let coming: Vec<quince_core::calendar::Event> = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library
                    // As far ahead as the earliest alert Calendar offers — a week before —
                    // and a day more: looking two days ahead, a week-before alert never rang.
                    .events_starting_between(at - STILL_WORTH_SAYING, at + 8 * 86_400)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(guid, _)| !guid.starts_with("quince:"))
                    .filter_map(|(guid, _)| library.event(&guid).ok().flatten())
                    .collect()
            };
            // Ask for the alerts of any event not yet read, or changed since.
            let stale: Vec<_> = coming
                .iter()
                .filter(|e| known.get(&e.guid).is_none_or(|(tag, _)| *tag != e.etag))
                .cloned()
                .collect();
            if !stale.is_empty() {
                let session = {
                    let auth = app.state::<Auth>();
                    let client = auth.client.lock().await;
                    client
                        .session()
                        .service_url("calendar")
                        .map(|root| (client.http().clone(), root, client.service_params()))
                };
                if let Some((http, root, params)) = session {
                    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
                    for e in &stale {
                        if let Ok(alerts) = cal.alerts(e).await {
                            known.insert(e.guid.clone(), (e.etag.clone(), alerts));
                        }
                    }
                }
            }
            let mut next: Option<i64> = None;
            for e in &coming {
                let Some((_, alerts)) = known.get(&e.guid) else {
                    continue;
                };
                for minutes in alerts {
                    let fires_at = e.starts_at - minutes * 60;
                    if fires_at > at {
                        next = Some(next.map_or(fires_at, |n| n.min(fires_at)));
                        continue;
                    }
                    if at - fires_at > STILL_WORTH_SAYING {
                        continue;
                    }
                    let key = format!("event:{}:{minutes}", e.guid);
                    let ring = {
                        let data = app.state::<Data>();
                        let mut library = data.0.lock().await;
                        !library.alarm_already_fired(&key, fires_at).unwrap_or(true)
                            && library.mark_alarm_fired(&key, fires_at, at).is_ok()
                            && allowed(&library, EVENT_ALERTS)
                    };
                    if ring {
                        show_event(app.clone(), e.clone(), *minutes);
                    }
                }
            }
            // Until the next alert, but look again every two minutes for new and moved events.
            let wait = next.map_or(120, |t| (t - at).clamp(1, 120));
            tokio::time::sleep(Duration::from_secs(u64::try_from(wait).unwrap_or(120))).await;
        }
    });
}

/// "Dentist" — "In 15 minutes · 4:30 PM", as Calendar words an alert.
fn show_event(app: AppHandle, event: quince_core::calendar::Event, minutes: i64) {
    std::thread::spawn(move || {
        let when = jiff::Timestamp::from_second(event.starts_at)
            .map(|t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%-I:%M %p")
                    .to_string()
            })
            .unwrap_or_default();
        let lead = match minutes {
            0 => "Now".to_string(),
            m if m % 1440 == 0 => {
                format!("In {} day{}", m / 1440, if m == 1440 { "" } else { "s" })
            }
            m if m % 60 == 0 => format!("In {} hour{}", m / 60, if m == 60 { "" } else { "s" }),
            m if m > 0 => format!("In {m} minutes"),
            m => format!("Started {} minutes ago", -m),
        };
        let mut body = if event.all_day {
            lead
        } else {
            format!("{lead} · {when}")
        };
        if let Some(place) = event.location.as_deref().filter(|p| !p.trim().is_empty()) {
            body.push_str(&format!("\n{place}"));
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            notify_rust::Notification::new()
                .appname("Quince")
                .summary(&event.title)
                .body(&body)
                .icon(icon_path(&app).unwrap_or("quince"))
                .show()
                .ok();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let shown = notify_rust::Notification::new()
                .appname("Quince")
                .summary(&event.title)
                .body(&body)
                .icon(icon_path(&app).unwrap_or("quince"))
                .hint(notify_rust::Hint::Category("calendar".into()))
                .hint(notify_rust::Hint::DesktopEntry("quince".into()))
                .action("default", "Open")
                .show();
            if let Ok(handle) = shown {
                handle.wait_for_action(|action| {
                    if action == "default" {
                        crate::show_window(&app);
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_alert_rings_unless_settings_turned_it_off() {
        let mut library = quince_core::Library::in_memory().unwrap();
        assert!(allowed(&library, EVENT_ALERTS), "on until turned off");
        library.set_state(EVENT_ALERTS, "off").unwrap();
        assert!(!allowed(&library, EVENT_ALERTS));
        assert!(
            allowed(&library, REMINDER_ALERTS),
            "one switch does not turn off the other"
        );
        library.set_state(EVENT_ALERTS, "on").unwrap();
        assert!(allowed(&library, EVENT_ALERTS));
    }

    use super::*;
    use quince_core::alarms::{Alarm, AlarmTrigger};
    use quince_core::reminders::{Reminder, ReminderList};

    fn library_with(fires: &[(&str, i64)]) -> quince_core::Library {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = quince_core::Library::open(&dir.keep()).unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "L".into(),
            title: "Home".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        let mut reminders = Vec::new();
        let mut alarms = Vec::new();
        let mut triggers = Vec::new();
        for (id, at) in fires {
            reminders.push(Reminder {
                guid: format!("R{id}"),
                list_guid: "L".into(),
                title: format!("Do {id}"),
                ..Default::default()
            });
            alarms.push(Alarm {
                id: format!("A{id}"),
                reminder_guid: format!("R{id}"),
                trigger_id: format!("T{id}"),
                acknowledged_at: None,
            });
            triggers.push(AlarmTrigger {
                id: format!("T{id}"),
                fires_at: *at,
            });
        }
        lib.put_reminders(&reminders).unwrap();
        lib.put_alarms(&alarms, &triggers, &[], true).unwrap();
        lib
    }

    #[test]
    fn a_due_alert_is_taken_once_and_the_next_is_named() {
        let mut lib = library_with(&[("now", 1_000_000), ("later", 1_003_600)]);
        let (ring, next) = take_due(&mut lib, 1_000_000);
        assert_eq!(
            ring.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["Do now"]
        );
        assert_eq!(next, Some(1_003_600));
        // A second wake at the same moment — resume and a pass together — rings nothing.
        let (again, _) = take_due(&mut lib, 1_000_000);
        assert!(again.is_empty());
    }

    #[test]
    fn missed_while_away_rings_once_but_long_gone_stays_quiet() {
        let now = 10_000_000;
        let mut lib = library_with(&[
            ("an hour ago", now - 3_600),
            ("last week", now - 7 * 86_400),
        ]);
        let (ring, _) = take_due(&mut lib, now);
        assert_eq!(
            ring.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["Do an hour ago"]
        );
        // And the old one is recorded, not left to surface later.
        assert!(lib.due_alarms(now).unwrap().is_empty());
    }
}
