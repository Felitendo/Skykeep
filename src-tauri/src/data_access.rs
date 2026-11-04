//! Advanced Data Protection approvals, followed for the window (see `quince_auth::pcs`).
//!
//! An account with Advanced Data Protection and "Access iCloud Data on the Web" on lets this
//! session read Notes, Reminders, Photos and iCloud Drive only once the person approves it on
//! one of their devices, and for about an hour at a time. Here that is asked for, followed and
//! told: the approval is put on their devices when they sign in or open a window — at most once
//! a lapse unless they ask again — and never by a background pass; while it is on its way the
//! client is taken only for each step, so nothing else waits behind it; and once it is given,
//! the services that were waiting on it are read at once.

use crate::Auth;
use quince_auth::pcs::{self, DataAccess};
use quince_auth::Client;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// Told to every window when where the approval stands changes.
pub const EVENT: &str = "quince://data-access";

/// Where the approval stands, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Access {
    pub state: DataAccess,
    /// The services waiting on it, by the app's names; empty when `state` is `open`.
    pub services: Vec<&'static str>,
    /// Why the last step failed, in words, when it did.
    pub problem: Option<String>,
}

impl Access {
    const OPEN: Access = Access {
        state: DataAccess::Open,
        services: Vec::new(),
        problem: None,
    };
}

static NOW: std::sync::Mutex<Access> = std::sync::Mutex::new(Access::OPEN);
/// An approval is being followed: a second follower would only double the requests.
static FOLLOWING: AtomicBool = AtomicBool::new(false);
/// The approval was put on the person's devices without their asking, this lapse: not again
/// until access opens or they ask.
static ASKED_UNASKED: AtomicBool = AtomicBool::new(false);
/// Services a pass found refused for want of their key, for the next step to ask for again.
static REFUSED: std::sync::Mutex<BTreeSet<&'static str>> = std::sync::Mutex::new(BTreeSet::new());

/// Who is asking for a step, which decides whether it may put a prompt on the person's phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The person, from the prompt's button: always.
    Person,
    /// A sign-in, or a window opening or coming back: once a lapse.
    Window,
    /// A background pass: never.
    Background,
}

/// Where it stands now.
pub fn current() -> Access {
    NOW.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// A new sign-in, or a sign-out: nothing from the session before carries over — its approval,
/// its refusals, or that it already asked once.
pub fn forget(app: &tauri::AppHandle) {
    ASKED_UNASKED.store(false, Ordering::SeqCst);
    REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    set(app, Access::OPEN);
}

/// A pass found `service` refused by Advanced Data Protection.
pub fn refused(service: &'static str) {
    REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(service);
}

fn set(app: &tauri::AppHandle, next: Access) {
    let changed = {
        let mut now = NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = *now != next;
        *now = next.clone();
        changed
    };
    if changed {
        app.emit(EVENT, next).ok();
    }
}

/// Whether this step may ask the person's devices.
fn may_ask(ask: Ask) -> bool {
    match ask {
        Ask::Person => true,
        Ask::Window => !ASKED_UNASKED.swap(true, Ordering::SeqCst),
        Ask::Background => false,
    }
}

/// Take one step, tell the window, and follow an approval that is on its way.
pub async fn check(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>, ask: Ask) {
    let (step, missing) = {
        let mut guard = client.lock().await;
        if guard.session().webservices.is_none() {
            return;
        }
        for service in std::mem::take(
            &mut *REFUSED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) {
            guard.data_access_lapsed(service);
        }
        let missing = guard.data_access_missing();
        if missing.is_empty() && guard.session().web_access_allowed != Some(false) {
            (Ok(DataAccess::Open), missing)
        } else {
            let asking = may_ask(ask);
            let step = guard.data_access(asking).await;
            (step, guard.data_access_missing())
        }
    };
    settle(app, client, step, missing, ask);
}

/// After a pass: when Advanced Data Protection kept a service out, the step that asks for its
/// key again — at once, so the window says so rather than the service's failure alone.
pub async fn after_pass(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>, ask: Ask) {
    let any = !REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty();
    if any {
        check(app, client, ask).await;
    }
}

/// What one step came to, told; an approval on its way followed until it lands.
fn settle(
    app: &tauri::AppHandle,
    client: &Arc<Mutex<Client>>,
    step: Result<DataAccess, quince_auth::Error>,
    waiting_on: Vec<&'static str>,
    ask: Ask,
) {
    let was = current();
    match step {
        Ok(DataAccess::Open) => opened(app, client, &was),
        Ok(DataAccess::Waiting) => {
            set(
                app,
                Access {
                    state: DataAccess::Waiting,
                    services: waiting_on.clone(),
                    problem: None,
                },
            );
            follow(app.clone(), Arc::clone(client), waiting_on);
        }
        Ok(state) => set(
            app,
            Access {
                state,
                services: waiting_on,
                problem: None,
            },
        ),
        // Not reached, or Apple busy: nothing is known to have changed. Said only when the
        // person asked; a background pass simply tries again next time.
        Err(e @ (quince_auth::Error::Network(_) | quince_auth::Error::Unavailable(_))) => {
            if ask == Ask::Person {
                set(
                    app,
                    Access {
                        problem: Some(e.to_string()),
                        ..was
                    },
                );
            }
        }
        Err(e) => {
            quiet!("data access: {e:?}");
            set(
                app,
                Access {
                    state: DataAccess::Needed,
                    services: waiting_on,
                    problem: Some(e.to_string()),
                },
            );
        }
    }
}

/// Every key is here (or none was ever needed): said, and what was waiting on it read now.
fn opened(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>, was: &Access) {
    ASKED_UNASKED.store(false, Ordering::SeqCst);
    set(app, Access::OPEN);
    if was.state == DataAccess::Open || was.services.is_empty() {
        return;
    }
    let services = was.services.clone();
    let app = app.clone();
    let client = Arc::clone(client);
    tauri::async_runtime::spawn(async move {
        let data = app.state::<crate::Data>();
        crate::refresh::some(&client, &data.0, &services).await;
        crate::alerts::changed();
        let mut status = crate::refresh::status(&data.0).await;
        status.changed = Some(Vec::new());
        app.emit(crate::SYNCED, status).ok();
    });
}

/// Follow an approval on its way, one step at a time on [`pcs::APPROVAL_SCHEDULE`], taking the
/// client only for each step.
fn follow(app: tauri::AppHandle, client: Arc<Mutex<Client>>, waiting_on: Vec<&'static str>) {
    if FOLLOWING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let stepping = Arc::clone(&client);
        let landed = pcs::follow(pcs::APPROVAL_SCHEDULE, || {
            let client = Arc::clone(&stepping);
            async move {
                let mut guard = client.lock().await;
                if guard.session().webservices.is_none() {
                    // Signed out while it was on its way: nothing left to follow.
                    return Ok(DataAccess::Open);
                }
                match guard.data_access(false).await {
                    // A dropped connection or a busy Apple is waited out like any other step;
                    // the schedule still bounds how long.
                    Err(quince_auth::Error::Network(_) | quince_auth::Error::Unavailable(_)) => {
                        Ok(DataAccess::Waiting)
                    }
                    other => other,
                }
            }
        })
        .await;
        FOLLOWING.store(false, Ordering::SeqCst);
        let still = client.lock().await.data_access_missing();
        let waiting_on = if still.is_empty() { waiting_on } else { still };
        settle(&app, &client, landed, waiting_on, Ask::Background);
    });
}

/// Where the approval stands, for a window opening.
#[tauri::command]
pub async fn data_access_status() -> Result<Access, String> {
    Ok(current())
}

/// The person asked: put the approval on their devices again, and follow it.
#[tauri::command]
pub async fn data_access_ask(
    app: tauri::AppHandle,
    auth: tauri::State<'_, Auth>,
) -> Result<Access, String> {
    check(&app, &auth.client, Ask::Person).await;
    Ok(current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_pass_never_puts_a_prompt_on_the_phone() {
        assert!(!may_ask(Ask::Background));
        assert!(may_ask(Ask::Person));
    }

    #[test]
    fn a_window_asks_once_a_lapse_and_the_person_whenever_they_like() {
        ASKED_UNASKED.store(false, Ordering::SeqCst);
        assert!(may_ask(Ask::Window), "the first window asks");
        assert!(
            !may_ask(Ask::Window),
            "the next ones do not, until access opens"
        );
        assert!(may_ask(Ask::Person), "the person's own ask always goes");
        ASKED_UNASKED.store(false, Ordering::SeqCst);
        assert!(
            may_ask(Ask::Window),
            "after access opened, the next lapse asks again"
        );
    }

    #[test]
    fn the_window_reads_it_in_its_own_words() {
        let told = serde_json::to_value(Access {
            state: DataAccess::Waiting,
            services: vec!["notes", "drive"],
            problem: None,
        })
        .unwrap();
        assert_eq!(
            told,
            serde_json::json!({ "state": "waiting", "services": ["notes", "drive"], "problem": null })
        );
        assert_eq!(
            serde_json::to_value(DataAccess::WebAccessOff).unwrap(),
            "webAccessOff"
        );
        assert_eq!(
            serde_json::to_value(DataAccess::NotSent).unwrap(),
            "notSent"
        );
    }
}
