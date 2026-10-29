//! iCloud Drive's changes, heard as they happen and applied folder by folder.
//!
//! Drive has no push (measured 2026-10-29), so a file added on the phone used to show only
//! when its folder was listed again: the open folder every ten seconds, the rest when opened,
//! and the Drive folder on this computer not until then either. Now one tiny read-only check —
//! whether Drive's CloudKit database moved — runs as often as the other services' checks do;
//! when it moves, Drive's own change feed says which items changed and where they are, and
//! only the folders they left or went into are listed again (quince_icloud::drive_changes).
//! Stored, every view reads them at once: the window is told which folders changed, and the
//! Drive folder on this computer sees the library move.
//!
//! The feed's position is kept in the library, so changes made while the app was closed are
//! read when it starts. The first run reads the feed through to now once — identifiers only,
//! a few hundred bytes a record — before it can tell what is new.

use crate::push::{poll_every, presence, sleep_or_wake, WAKE};
use crate::Data;
use quince_auth::Client;
use quince_icloud::drive_changes::{self, Changed, CONTAINER};
use quince_icloud::Error;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// Told to the window with the folders listed again, so an open one redraws.
pub const CHANGED: &str = "quince://drive-changed";

/// Where the feed was last read to, with the account it belongs to: `<dsid>\t<token>`.
const POSITION: &str = "drive.changes";

/// The most pages one catch-up reads before it stores what it has and lets the next check go
/// on. A first read of a large Drive is a few dozen pages.
const PAGES_AT_ONCE: usize = 500;

pub async fn watch(app: tauri::AppHandle, client: Arc<Mutex<Client>>) {
    let mut database: Option<String> = None;
    // Read the feed on the first check whatever the database says: changes made while the
    // app was closed are behind a database token seeded only now.
    let mut catch_up = true;
    let mut backoff: Option<Duration> = None;
    let mut woken = WAKE.subscribe();
    loop {
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| {
                    Some((
                        s.service_url("ckdatabasews")?,
                        s.service_url("drivews")?,
                        s.dsid.clone()?,
                    ))
                })
                .flatten()
                .map(|(ck, drivews, dsid)| {
                    (
                        guard.http().clone(),
                        guard.service_params(),
                        ck,
                        drivews,
                        dsid,
                    )
                })
        };
        let Some((http, params, ck, drivews, dsid)) = session else {
            (database, catch_up) = (None, true);
            sleep_or_wake(Duration::from_secs(30), &mut woken).await;
            continue;
        };
        let moved = match quince_icloud::push::database_changed(
            &http,
            &ck,
            &params,
            CONTAINER,
            database.as_deref(),
        )
        .await
        {
            Ok((moved, next)) => {
                database = next;
                Ok(moved)
            }
            Err(Error::StaleCursor) => {
                database = None;
                Ok(true)
            }
            Err(e) => Err(e),
        };
        let result = match moved {
            Ok(moved) if moved || catch_up => {
                let read = read_feed(&app, &http, &params, &ck, &drivews, &dsid).await;
                if read.is_ok() {
                    catch_up = false;
                }
                read
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        backoff = match result {
            Ok(()) => None,
            Err(e) => {
                let next = backoff.map_or(Duration::from_secs(5), |b| {
                    (b * 2).min(Duration::from_secs(5 * 60))
                });
                if backoff.is_none() {
                    quiet!("drive watch: {e}; trying again in {}s", next.as_secs());
                }
                Some(next)
            }
        };
        let wait = backoff.unwrap_or_else(|| poll_every(presence(&app)));
        sleep_or_wake(wait, &mut woken).await;
    }
}

/// Read the feed from where it was left, list again the folders it touched, and keep the new
/// position — only once they are stored, so a failure part way reads the same changes again.
async fn read_feed(
    app: &tauri::AppHandle,
    http: &reqwest::Client,
    params: &[(String, String)],
    ck: &str,
    drivews: &str,
    dsid: &str,
) -> quince_icloud::Result<()> {
    let data = app.state::<Data>();
    let kept = data
        .0
        .lock()
        .await
        .state(POSITION)
        .ok()
        .flatten()
        .and_then(|v| {
            let (who, token) = v.split_once('\t')?;
            (who == dsid).then(|| token.to_string())
        });
    let first = kept.is_none();
    let mut token = kept;
    let mut changed: Vec<Changed> = Vec::new();
    for _ in 0..PAGES_AT_ONCE {
        let page = match drive_changes::page(http, ck, params, token.as_deref()).await {
            Err(Error::StaleCursor) if token.is_some() => {
                // Apple no longer knows this position: forgotten, so the next check reads
                // through to now as a first run does. What changed in between is found as
                // before this watch, when each folder is next listed.
                quiet!("drive watch: position expired; reading Drive's changes afresh");
                let _ = data.0.lock().await.set_state(POSITION, "");
                return Err(Error::StaleCursor);
            }
            other => other?,
        };
        // A first read is everything there is, not news: only its end position is kept.
        if !first {
            changed.extend(page.changed);
        }
        token = page.token.or(token);
        if !page.more {
            break;
        }
    }
    let folders = {
        let library = data.0.lock().await;
        drive_changes::folders_to_list(
            &changed,
            |id| {
                library
                    .drive_item(id)
                    .ok()
                    .flatten()
                    .and_then(|i| i.parent_id)
            },
            |id| id == quince_core::drive::ROOT || library.drive_item(id).ok().flatten().is_some(),
        )
    };
    let mut listed = Vec::new();
    for folder in folders {
        let pass = crate::drive_pass(&folder);
        let _one = pass.lock().await;
        match quince_icloud::drive::fetch_folder(http, drivews, params.to_vec(), &folder).await {
            Ok(items) => {
                let mut library = data.0.lock().await;
                quince_icloud::drive::store_folder(&mut library, &folder, &items)?;
            }
            // Gone since: forgotten here too, as when the window finds it gone.
            Err(Error::NotFound) => {
                let mut library = data.0.lock().await;
                library.forget_drive_folder(&folder)?;
            }
            Err(e) => return Err(e),
        }
        listed.push(folder);
    }
    if let Some(t) = token {
        let _ = data
            .0
            .lock()
            .await
            .set_state(POSITION, &format!("{dsid}\t{t}"));
    }
    if !listed.is_empty() {
        quiet!("drive watch: {} folder(s) changed", listed.len());
        app.emit(CHANGED, &listed).ok();
    }
    Ok(())
}
