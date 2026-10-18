//! Apple's pushes, turned into refreshes of exactly what changed.
//!
//! A change made on a phone used to wait for the next five-minute pass. Now a parked
//! connection hears of it from Apple within about a second (see
//! `quince_icloud::push`), and only the service it was in is refreshed. The timed pass stays
//! as the floor beneath this, for the services Apple does not push here and for any push
//! that went missing.

use crate::{alerts, refresh, Data, SYNCED};
use quince_auth::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, Mutex};

/// The containers one connection listens to, and the service each one is.
const CONTAINERS: [(&str, &str); 2] = [
    ("com.apple.notes", "notes"),
    ("com.apple.reminders", "reminders"),
];

/// How long one wait is left parked before it is parked afresh. Apple answers sooner when
/// something changes; this only bounds a connection that went quiet without closing.
const LONGEST_WAIT: Duration = Duration::from_secs(10 * 60);

/// A token is made for a day (`ttl=86400`); a new one is made a little before.
const TOKEN_LIFE: Duration = Duration::from_secs(23 * 60 * 60);

/// A phone sends a burst of pushes for one edit, one per record it saves. They are taken
/// together, so one edit is one refresh.
const SETTLE: Duration = Duration::from_millis(250);

/// The service a container's changes belong to.
fn service_of(container: &str) -> Option<&'static str> {
    CONTAINERS
        .iter()
        .find(|(c, _)| *c == container)
        .map(|(_, s)| *s)
}

pub fn spawn(app: tauri::AppHandle, client: Arc<Mutex<Client>>) {
    let (tx, rx) = mpsc::unbounded_channel::<&'static str>();
    tauri::async_runtime::spawn(refresher(app, Arc::clone(&client), rx));
    tauri::async_runtime::spawn(listen(client, tx));
}

/// Refresh what the pushes named, a burst at a time.
async fn refresher(
    app: tauri::AppHandle,
    client: Arc<Mutex<Client>>,
    mut rx: mpsc::UnboundedReceiver<&'static str>,
) {
    while let Some(first) = rx.recv().await {
        let mut services = vec![first];
        tokio::time::sleep(SETTLE).await;
        while let Ok(more) = rx.try_recv() {
            if !services.contains(&more) {
                services.push(more);
            }
        }
        let data = app.state::<Data>();
        refresh::some(&client, &data.0, &services).await;
        alerts::changed();
        // Every service's freshness, not only the ones refreshed: the window shows the whole.
        app.emit(SYNCED, refresh::status(&data.0).await).ok();
    }
}

/// Keep a connection parked on Apple's push channel for as long as the app runs.
async fn listen(client: Arc<Mutex<Client>>, tx: mpsc::UnboundedSender<&'static str>) {
    let client_id = uuid::Uuid::new_v4().to_string().to_uppercase();
    let containers: Vec<&str> = CONTAINERS.iter().map(|(c, _)| *c).collect();
    let mut backoff = Duration::from_secs(1);
    let mut first = true;
    loop {
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| s.service_url("ckdeviceservice"))
                .flatten()
                .map(|root| (guard.http().clone(), guard.service_params(), root))
        };
        let Some((http, params, root)) = session else {
            // Not signed in, or no CloudKit for this account: look again shortly.
            tokio::time::sleep(Duration::from_secs(30)).await;
            continue;
        };
        let channel =
            match quince_icloud::push::open(&http, &root, &params, &client_id, &containers).await {
                Ok(channel) => channel,
                Err(e) => {
                    eprintln!("push: could not open the channel: {e}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(5 * 60));
                    continue;
                }
            };
        backoff = Duration::from_secs(1);
        // Whatever changed while no connection was parked was never pushed: one pass catches
        // it up. Not at launch, which runs a full pass of its own.
        if !first {
            for (_, service) in CONTAINERS {
                tx.send(service).ok();
            }
        }
        first = false;
        let opened = Instant::now();
        let mut failures = 0;
        while opened.elapsed() < TOKEN_LIFE {
            match quince_icloud::push::wait(&http, &channel, LONGEST_WAIT).await {
                Ok(Some(container)) => {
                    failures = 0;
                    if let Some(service) = service_of(&container) {
                        tx.send(service).ok();
                    }
                }
                Ok(None) => failures = 0,
                Err(e) => {
                    failures += 1;
                    eprintln!("push: the wait failed ({failures}): {e}");
                    if failures >= 3 {
                        // Three in a row is a channel gone bad, not a blip: make a new one.
                        break;
                    }
                    tokio::time::sleep(backoff * failures).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_container_is_its_service() {
        assert_eq!(service_of("com.apple.notes"), Some("notes"));
        assert_eq!(service_of("com.apple.reminders"), Some("reminders"));
        assert_eq!(service_of("com.apple.photos.cloud"), None);
    }
}
