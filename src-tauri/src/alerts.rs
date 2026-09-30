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
use tauri::{AppHandle, Emitter, Manager};
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
const SNOOZE: i64 = 60 * 60;

/// Emitted to the window to open a reminder an alert was clicked for.
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
pub fn spawn(app: AppHandle) {
    watch_for_resume();
    tauri::async_runtime::spawn(async move {
        loop {
            let at = now();
            let (ring, next) = {
                let data = app.state::<Data>();
                let mut library = data.0.lock().await;
                take_due(&mut library, at)
            };
            for due in ring {
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
        let shown = notify_rust::Notification::new()
            .appname("Quince")
            .summary(&due.title)
            .body(&body)
            .icon("quince")
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
                        eprintln!("could not complete {}: {e}", due.reminder_guid);
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
    });
}

#[cfg(test)]
mod tests {
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
