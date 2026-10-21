//! Changes go to Apple behind the window, never in front of it. A command changes the library
//! and queues the change (quince_core::outbox), then returns: every view shows it at once, as
//! a native app's own store would. This worker sends what is queued, in the order it was made,
//! one service at a time:
//!
//! - taken: off the queue; one sync pass afterwards brings Apple's own version of the record;
//! - Apple can't be reached (offline, signed out, a server fault): kept, and tried again with a
//!   growing wait, so a change made offline lands when the connection comes back — even after
//!   a restart, since the queue is in the library;
//! - refused (changed elsewhere, gone, turned down): off the queue, Apple's version read back
//!   over ours, and the window told why — `quince://outbox-refused`.
//!
//! A sync pass that lands while changes wait would put Apple's older version over them, so
//! every pass lays the waiting changes over what it read (see `replay_reminders`).

use crate::{cloudkit_context, Auth, Data};
use quince_core::outbox::Pending;
use quince_icloud::Error;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

static WAKE: Notify = Notify::const_new();

/// Something was queued: send it now.
pub fn kick() {
    WAKE.notify_one();
}

/// What a waiting change does to a reminder: ticked (`at` the moment) or not.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Done {
    pub done: bool,
    pub at: i64,
}

/// Runs for as long as the app does.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut wait = Duration::from_secs(30);
        loop {
            tokio::select! {
                () = WAKE.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
            wait = match drain_reminders(&app).await {
                Outcome::Clear => Duration::from_secs(30),
                // Apple out of reach: wait longer each time, up to two minutes.
                Outcome::Later => {
                    (wait * 2).clamp(Duration::from_secs(4), Duration::from_secs(120))
                }
            };
        }
    });
}

enum Outcome {
    Clear,
    Later,
}

/// Whether Apple could not be reached (keep it) rather than said no (drop it).
/// How many times an answer this app does not understand is retried before the change is
/// given up and said to be refused — a fault that will never clear held every change queued
/// after it forever, each shown as made.
const PROTOCOL_TRIES: i64 = 5;

fn transient(e: &Error, tries: i64) -> bool {
    match e {
        Error::Network(_)
        | Error::ReauthRequired
        | Error::StaleCursor
        | Error::Busy
        | Error::Io(_)
        | Error::Storage(_) => true,
        Error::Protocol(_) => tries < PROTOCOL_TRIES,
        _ => false,
    }
}

#[derive(Clone, serde::Serialize)]
struct Refused {
    service: &'static str,
    record: String,
    message: String,
}

async fn drain_reminders(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        // Not signed in, or the session is being renewed: nothing can go yet.
        return if data
            .0
            .lock()
            .await
            .outbox_pending("reminders")
            .map(|p| p.is_empty())
            .unwrap_or(true)
        {
            Outcome::Clear
        } else {
            Outcome::Later
        };
    };
    let report = drain(&data.0, &crate::REMINDERS_PASS, |reminder, d| {
        let (http, root, params) = (http.clone(), root.clone(), params.clone());
        async move {
            // A change made on another device after this one was queued wins; the pass after
            // sending reads Apple's version back.
            if quince_icloud::reminders::tick_overtaken(
                &http,
                &root,
                params.clone(),
                &reminder,
                d.done,
                d.at,
            )
            .await?
            {
                return Ok(());
            }
            quince_icloud::reminders::set_completed_cloudkit(
                &http, &root, params, &reminder, d.done,
            )
            .await
        }
    })
    .await;
    for (record, message) in &report.refused {
        let _ = app.emit(
            "quince://outbox-refused",
            Refused {
                service: "reminders",
                record: record.clone(),
                message: message.clone(),
            },
        );
    }
    // Apple's own version of what was sent — its new change tags, and the truth over anything
    // it refused.
    if report.sent || !report.refused.is_empty() {
        let _ = crate::refresh_reminders(&http, &root, params, &data, false).await;
        let _ = app.emit("quince://reminders-synced", ());
    }
    report.outcome
}

/// What one pass over the queue came to.
struct Report {
    outcome: Outcome,
    sent: bool,
    /// Each refused change: its record and Apple's reason.
    refused: Vec<(String, String)>,
}

/// Send what is waiting for Reminders, oldest first, with `send`. Stops at the first change
/// Apple can't be reached for, leaving it and everything after it queued, in order.
async fn drain<F, Fut>(
    library: &tokio::sync::Mutex<quince_core::Library>,
    pass: &tokio::sync::Mutex<()>,
    send: F,
) -> Report
where
    F: Fn(quince_core::reminders::Reminder, Done) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    let mut report = Report {
        outcome: Outcome::Clear,
        sent: false,
        refused: Vec::new(),
    };
    loop {
        let next: Option<Pending> = match library.lock().await.outbox_pending("reminders") {
            Ok(p) => p.into_iter().next(),
            Err(_) => {
                report.outcome = Outcome::Later;
                return report;
            }
        };
        let Some(p) = next else { break };
        // No sync pass between sending and taking it off the queue: one fetched just before
        // the write and stored just after would put the old state back with nothing waiting
        // to lay over it.
        let _pass = pass.lock().await;
        let reminder = {
            let lib = library.lock().await;
            lib.reminder(&p.record).ok().flatten()
        };
        let Some(reminder) = reminder else {
            // Gone from the library (deleted here or elsewhere): nothing left to send.
            let _ = library.lock().await.outbox_done(p.id);
            continue;
        };
        let result = match (p.kind.as_str(), serde_json::from_str::<Done>(&p.payload)) {
            ("done", Ok(d)) => send(reminder, d).await,
            ("done", Err(e)) => Err(Error::Refused(format!(
                "A queued change could not be read: {e}"
            ))),
            (other, _) => Err(Error::Refused(format!(
                "Pomander does not know how to send “{other}”."
            ))),
        };
        match result {
            Ok(()) => {
                let _ = library.lock().await.outbox_done(p.id);
                report.sent = true;
            }
            Err(e) if transient(&e, p.tries) => {
                let _ = library.lock().await.outbox_retry(p.id, &e.to_string());
                report.outcome = Outcome::Later;
                return report;
            }
            Err(e) => {
                let _ = library.lock().await.outbox_done(p.id);
                report.refused.push((p.record.clone(), e.to_string()));
            }
        }
    }
    report
}

/// Lay the reminder changes still waiting over what a sync pass just stored.
pub fn replay_reminders(library: &mut quince_core::Library) -> quince_core::Result<()> {
    for p in library.outbox_pending("reminders")? {
        if p.kind == "done" {
            if let Ok(d) = serde_json::from_str::<Done>(&p.payload) {
                library.set_reminder_completed(&p.record, d.done.then_some(d.at))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::reminders::{Reminder, ReminderList};
    use std::sync::{Arc, Mutex as StdMutex};

    fn library() -> tokio::sync::Mutex<quince_core::Library> {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let list: ReminderList = serde_json::from_value(serde_json::json!({
            "guid": "L", "title": "Today", "colour": null, "ctag": null, "sortOrder": 0, "enabled": true
        }))
        .unwrap();
        lib.put_reminder_lists(&[list]).unwrap();
        let r = |g: &str| -> Reminder {
            serde_json::from_value(serde_json::json!({
                "guid": g, "listGuid": "L", "title": g, "notes": null, "dueAt": null,
                "completedAt": null, "priority": 0, "etag": "t"
            }))
            .unwrap()
        };
        lib.put_reminders(&[r("a"), r("b"), r("c")]).unwrap();
        tokio::sync::Mutex::new(lib)
    }
    fn queue(lib: &mut quince_core::Library, g: &str, done: bool) {
        lib.set_reminder_completed(g, done.then_some(1)).unwrap();
        let payload = serde_json::to_string(&Done { done, at: 1 }).unwrap();
        lib.outbox_push("reminders", g, "done", &payload).unwrap();
    }

    #[test]
    fn sends_in_order_keeps_what_cannot_reach_apple_and_reports_refusals() {
        tauri::async_runtime::block_on(async {
            let lib = library();
            let pass = tokio::sync::Mutex::new(());
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", true);
                queue(&mut l, "b", true);
                queue(&mut l, "c", true);
            }
            // Apple turns "b" down; everything goes through in the order it was made.
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let s2 = seen.clone();
            let report = drain(&lib, &pass, move |r, d| {
                s2.lock().unwrap().push((r.guid.clone(), d.done));
                let g = r.guid;
                async move {
                    if g == "b" {
                        Err(Error::Refused("changed on another device".into()))
                    } else {
                        Ok(())
                    }
                }
            })
            .await;
            assert_eq!(
                *seen.lock().unwrap(),
                vec![("a".into(), true), ("b".into(), true), ("c".into(), true)]
            );
            assert!(report.sent && matches!(report.outcome, Outcome::Clear));
            assert_eq!(
                report.refused,
                vec![("b".to_string(), "changed on another device".to_string())]
            );
            assert!(lib
                .lock()
                .await
                .outbox_pending("reminders")
                .unwrap()
                .is_empty());

            // Offline: the first stays, with all after it, in order, for next time.
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", false);
                queue(&mut l, "c", false);
            }
            let report = drain(&lib, &pass, |_r, _d| async {
                Err(Error::Protocol("CloudKit refused the change (503)".into()))
            })
            .await;
            assert!(matches!(report.outcome, Outcome::Later) && !report.sent);
            let left = lib.lock().await.outbox_pending("reminders").unwrap();
            assert_eq!(
                left.iter().map(|p| p.record.as_str()).collect::<Vec<_>>(),
                vec!["a", "c"]
            );
            assert_eq!(left[0].tries, 1);

            // A sync pass that read Apple's older copy is laid over with what is still waiting.
            {
                let mut l = lib.lock().await;
                l.set_reminder_completed("a", Some(5)).unwrap();
                replay_reminders(&mut l).unwrap();
                let rs = l.reminders_in("L", true).unwrap();
                assert_eq!(
                    rs.iter().find(|r| r.guid == "a").unwrap().completed_at,
                    None
                );
            }

            // Back online: both go.
            let report = drain(&lib, &pass, |_r, _d| async { Ok(()) }).await;
            assert!(report.sent && matches!(report.outcome, Outcome::Clear));
            assert!(lib
                .lock()
                .await
                .outbox_pending("reminders")
                .unwrap()
                .is_empty());
        });
    }
}
