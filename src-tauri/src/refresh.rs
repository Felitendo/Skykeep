//! One refresh for the whole account.
//!
//! Four services used to each own a button, a spinner and a "last updated" line, so the
//! same work was asked for four times and any one of them could be quietly out of date.
//! There is one pass now: it refreshes everything, records what each service did, and the
//! window states a single honest freshness — the age of the stalest part.
//!
//! Two rules hold the whole thing together.
//!
//! **The network half never holds the library.** Every service fetches first and stores
//! second, so the screens reading SQLite are never blocked behind a slow endpoint.
//!
//! **One service failing changes nothing for the others.** A refresh returns a report per
//! service; the failure is recorded against that service alone and its last good data stays
//! on screen.

use quince_auth::Client;
use quince_core::Library;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// How a service's refresh ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Refreshed cleanly.
    Ok,
    /// Apple does not offer this service for the account, so there is nothing to refresh.
    Unavailable,
    /// It was tried and failed. What is on screen is the last good data.
    Failed,
}

/// What one service's refresh did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReport {
    pub service: &'static str,
    pub label: &'static str,
    pub outcome: Outcome,
    /// A sentence for the person: what came back, or what went wrong.
    pub detail: Option<String>,
    pub last_synced_at: Option<i64>,
}

/// Everything one refresh pass did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refresh {
    pub services: Vec<ServiceReport>,
    /// The stalest successful refresh across the services that are actually available —
    /// the only freshness the window can honestly claim. `None` while something has never
    /// synced.
    pub as_of: Option<i64>,
    pub took_ms: u64,
}

/// How often a service reconciles against everything Apple has, rather than the changes
/// since last time.
///
/// A delta is what almost every refresh should be — seconds rather than a minute. But a
/// delta can only apply what Apple chose to send, and a tombstone is sent once, so a record
/// whose removal was missed would stay forever. Once a day is often enough to be a floor
/// and rare enough that nobody notices the cost.
const FULL_SYNC_EVERY: i64 = 24 * 60 * 60;

/// The services one pass covers, in the order a person would notice them going stale.
const COVERED: [(&str, &str); 6] = [
    ("calendar", "Calendar"),
    ("reminders", "Reminders"),
    ("notes", "Notes"),
    ("contacts", "Contacts"),
    ("drive", "iCloud Drive"),
    ("photos", "Photos"),
];

/// The calendar window a background refresh keeps warm: the month behind, so last week is
/// still there, and a quarter ahead, which is as far as anyone scrolls without meaning to.
fn default_window() -> (String, String, String) {
    let zone = jiff::tz::TimeZone::system();
    let today = jiff::Timestamp::now().to_zoned(zone.clone()).date();
    let start = today
        .first_of_month()
        .checked_sub(jiff::Span::new().months(1))
        .unwrap_or(today);
    let end = today
        .checked_add(jiff::Span::new().months(3))
        .unwrap_or(today)
        .last_of_month();
    (
        start.to_string(),
        end.to_string(),
        zone.iana_name().unwrap_or("UTC").to_string(),
    )
}

/// What each service needs from the session, taken in one go so the auth lock is released
/// before any network work starts.
struct Context {
    http: reqwest::Client,
    params: Vec<(String, String)>,
    calendar: Option<String>,
    contacts: Option<String>,
    cloudkit: Option<String>,
    drive: Option<String>,
    /// The account it is all for (`account_tag`): nothing it brings is kept in a library that
    /// is not that account's.
    account: Option<String>,
}

async fn context(client: &Arc<Mutex<Client>>) -> Context {
    let guard = client.lock().await;
    let session = guard.session();
    Context {
        http: guard.http().clone(),
        params: guard.service_params(),
        calendar: session.service_url("calendar"),
        contacts: session.service_url("contacts"),
        cloudkit: session.service_url("ckdatabasews"),
        drive: session.service_url("drivews"),
        account: session.dsid.as_deref().map(crate::account_tag),
    }
}

/// Told each time one service's part of a pass has been stored, before the rest are done.
static LANDED: std::sync::OnceLock<Box<dyn Fn(&'static str) + Send + Sync>> =
    std::sync::OnceLock::new();

/// What to do when a service lands: set once, by the app, to tell the open screens.
pub fn on_landed(f: impl Fn(&'static str) + Send + Sync + 'static) {
    let _ = LANDED.set(Box::new(f));
}

fn landed(service: &'static str) {
    if let Some(f) = LANDED.get() {
        f(service);
    }
}

/// Refresh every service the account has, and report on each.
///
/// Never returns an error: a refresh that partly failed is a normal outcome with a partial
/// result, not an exception, and the interface has to be able to show exactly that.
pub async fn all(client: &Arc<Mutex<Client>>, data: &Mutex<Library>) -> Refresh {
    run(client, data, &COVERED.map(|(service, _)| service)).await
}

/// Refresh only the services `wanted` names — what a push from Apple says changed — and
/// report on those alone.
pub async fn some(client: &Arc<Mutex<Client>>, data: &Mutex<Library>, wanted: &[&str]) -> Refresh {
    run(client, data, wanted).await
}

async fn run(client: &Arc<Mutex<Client>>, data: &Mutex<Library>, services: &[&str]) -> Refresh {
    let wanted = |service: &str| services.contains(&service);
    let started = std::time::Instant::now();
    let ctx = context(client).await;
    let (from, to, zone) = default_window();

    // The services fetched side by side, not one after another: each has its own pass lock,
    // and waiting on Photos' whole library before Reminders was asked anything left every
    // screen empty through a first sync. Each one says it has landed as it does, so its
    // screen fills then rather than when the slowest is done.

    let calendar = async {
        if !wanted("calendar") {
            return None;
        }
        let report = match ctx.calendar.as_deref() {
            None => unavailable("calendar", "Calendar"),
            Some(root) => {
                // Fetch and store as one, never across a save: see `CALENDAR_PASS`.
                let _one = crate::CALENDAR_PASS.lock().await;
                let fetched = quince_icloud::sync::fetch_calendar(
                    &ctx.http,
                    root,
                    ctx.params.clone(),
                    &from,
                    &to,
                    &zone,
                )
                .await;
                store(
                    "calendar",
                    "Calendar",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    |library, snapshot| {
                        let summary = quince_icloud::sync::apply_calendar(library, snapshot)?;
                        Ok(format!(
                            "{} event{} in {} calendar{}",
                            summary.events,
                            plural(summary.events),
                            summary.calendars,
                            plural(summary.calendars)
                        ))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    let reminders = async {
        if !wanted("reminders") {
            return None;
        }
        let report = match ctx.cloudkit.as_deref() {
            None => unavailable("reminders", "Reminders"),
            Some(root) => {
                let _one = crate::REMINDERS_PASS.lock().await;
                let since = token(data, "reminders").await;
                let full = since.is_none();
                let fetched = quince_icloud::reminders::fetch_cloudkit(
                    &ctx.http,
                    root,
                    ctx.params.clone(),
                    since.as_deref(),
                )
                .await;
                store(
                    "reminders",
                    "Reminders",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    move |library, f| {
                        let (lists, count, _) =
                            quince_icloud::reminders::apply_cloudkit(library, f)?;
                        // Changes still on their way to Apple stay as made here: without this
                        // a reminder ticked offline showed open again after every pass, and
                        // its alert could ring for something already done.
                        crate::outbox::replay_reminders(library)?;
                        if full {
                            library.mark_full_sync("reminders")?;
                        }
                        Ok(format!(
                            "{count} reminder{} in {lists} list{}",
                            plural(count),
                            plural(lists)
                        ))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    let notes = async {
        if !wanted("notes") {
            return None;
        }
        let report = match ctx.cloudkit.as_deref() {
            None => unavailable("notes", "Notes"),
            Some(root) => {
                // One notes pass at a time, from reading the position through storing what came
                // back — see `NOTES_PASS`.
                let _one = crate::NOTES_PASS.lock().await;
                let since = token(data, "notes").await;
                let full = since.is_none();
                let fetched = quince_icloud::notes::fetch_cloudkit(
                    &ctx.http,
                    root,
                    ctx.params.clone(),
                    since.as_deref(),
                )
                .await;
                store(
                    "notes",
                    "Notes",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    move |library, f| {
                        let (folders, notes, _) = quince_icloud::notes::apply_cloudkit(library, f)?;
                        // Edits still on their way to Apple stay as typed here.
                        crate::outbox::replay(library, crate::outbox::NOTES)?;
                        if full {
                            library.mark_full_sync("notes")?;
                        }
                        Ok(format!(
                            "{notes} note{} in {folders} folder{}",
                            plural(notes),
                            plural(folders)
                        ))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    let contacts = async {
        if !wanted("contacts") {
            return None;
        }
        let report = match ctx.contacts.as_deref() {
            None => unavailable("contacts", "Contacts"),
            Some(root) => {
                // Fetch and store as one, never across a save: see `CONTACTS_PASS`. The book
                // is read only when Apple says it changed (contacts::pass).
                let _one = crate::CONTACTS_PASS.lock().await;
                let since = token(data, "contacts")
                    .await
                    .and_then(|t| quince_icloud::contacts::Tokens::from_stored(&t));
                let full = since.is_none();
                let fetched =
                    quince_icloud::contacts::pass(&ctx.http, root, ctx.params.clone(), since).await;
                store(
                    "contacts",
                    "Contacts",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    move |library, pass| {
                        use quince_icloud::contacts::Pass;
                        let count = match pass {
                            Pass::Read(book, tokens) => {
                                let n = quince_icloud::contacts::apply(library, book)?;
                                library.mark_synced("contacts", Some(&tokens.stored()))?;
                                n
                            }
                            Pass::Unchanged(tokens) => {
                                library.mark_synced("contacts", Some(&tokens.stored()))?;
                                let n: i64 = library
                                    .conn()
                                    .query_row("SELECT count(*) FROM contacts", [], |r| r.get(0))
                                    .map_err(quince_core::Error::from)?;
                                usize::try_from(n).unwrap_or(0)
                            }
                        };
                        // Cards saved here and still on their way to Apple stay as saved.
                        crate::outbox::replay(library, crate::outbox::CONTACTS)?;
                        if full {
                            library.mark_full_sync("contacts")?;
                        }
                        Ok(format!("{count} contact{}", plural(count)))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    // Drive is read a folder at a time as it is opened; the pass keeps its top level current,
    // so what changed there on another device is here before anyone looks, and its freshness
    // is stated with the rest. Fetched and stored as one — see `DRIVE_PASS`.
    let drive = async {
        if !wanted("drive") {
            return None;
        }
        let report = match ctx.drive.as_deref() {
            None => unavailable("drive", "iCloud Drive"),
            Some(root) => {
                let _one = crate::DRIVE_PASS.lock().await;
                let fetched = quince_icloud::drive::fetch_folder(
                    &ctx.http,
                    root,
                    ctx.params.clone(),
                    quince_core::drive::ROOT,
                )
                .await;
                store(
                    "drive",
                    "iCloud Drive",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    |library, items| {
                        let n = quince_icloud::drive::store_folder(
                            library,
                            quince_core::drive::ROOT,
                            &items,
                        )?;
                        Ok(format!("{n} item{} at the top of iCloud Drive", plural(n)))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    // Photos: the whole library through the change feed once, then only what changed — under
    // half a second when nothing did. Fetched and stored as one: see `PHOTOS_PASS`.
    let photos = async {
        if !wanted("photos") {
            return None;
        }
        let report = match ctx.cloudkit.as_deref() {
            None => unavailable("photos", "Photos"),
            Some(root) => {
                let _one = crate::PHOTOS_PASS.lock().await;
                let since = token(data, quince_icloud::photos::SERVICE).await;
                let full = since.is_none();
                let fetched = quince_icloud::photos::fetch(
                    &ctx.http,
                    root,
                    ctx.params.clone(),
                    since.as_deref(),
                )
                .await;
                store(
                    "photos",
                    "Photos",
                    data,
                    ctx.account.as_deref(),
                    fetched,
                    move |library, f| {
                        let (_, links) = quince_icloud::photos::apply(library, f)?;
                        // Marked, or every pass after walked the whole library again: the
                        // daily full read is due when none was ever recorded.
                        if full {
                            library.mark_full_sync(quince_icloud::photos::SERVICE)?;
                        }
                        crate::photos::remember(links);
                        // Counted, not read: every row of the library loaded to say how many
                        // there were held the library's lock through each pass.
                        let n = library
                            .photo_counts()?
                            .into_iter()
                            .find(|(v, _)| *v == quince_core::photos::View::Library)
                            .map_or(0, |(_, n)| usize::try_from(n).unwrap_or(0));
                        Ok(format!("{n} photo{}", plural(n)))
                    },
                )
                .await
            }
        };
        landed(report.service);
        Some(report)
    };

    let (calendar, reminders, notes, contacts, drive, photos) =
        tokio::join!(calendar, reminders, notes, contacts, drive, photos);
    // Apple answered: whatever waits to be sent goes now, not on the outbox's own timer.
    if [&reminders, &notes, &contacts]
        .iter()
        .any(|r| r.as_ref().is_some_and(|r| r.outcome == Outcome::Ok))
    {
        crate::outbox::kick();
    }
    let reports: Vec<ServiceReport> = [calendar, reminders, notes, contacts, drive, photos]
        .into_iter()
        .flatten()
        .collect();

    let available: Vec<&str> = reports
        .iter()
        .filter(|r| r.outcome != Outcome::Unavailable)
        .map(|r| r.service)
        .collect();
    let as_of = {
        let library = data.lock().await;
        library.oldest_sync(&available).ok().flatten()
    };

    Refresh {
        services: reports,
        as_of,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// What the library already knows, without asking Apple anything.
///
/// The window renders from this the instant it opens, so freshness is on screen before the
/// first refresh has finished rather than after it.
pub async fn status(data: &Mutex<Library>) -> Refresh {
    let library = data.lock().await;
    let health = library.sync_health().unwrap_or_default();
    let services = COVERED
        .iter()
        .map(|(service, label)| {
            let found = health.iter().find(|h| h.service == *service);
            let error = found.and_then(|h| h.last_error.clone());
            ServiceReport {
                service,
                label,
                outcome: if error.is_some() {
                    Outcome::Failed
                } else {
                    Outcome::Ok
                },
                detail: error,
                last_synced_at: found.and_then(|h| h.last_synced_at),
            }
        })
        .collect();
    let covered: Vec<&str> = COVERED.iter().map(|(service, _)| *service).collect();
    Refresh {
        services,
        as_of: library.oldest_sync(&covered).ok().flatten(),
        took_ms: 0,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn unavailable(service: &'static str, label: &'static str) -> ServiceReport {
    ServiceReport {
        service,
        label,
        outcome: Outcome::Unavailable,
        detail: None,
        last_synced_at: None,
    }
}

/// Where the last sync got to — or `None` when this service is due a full pass, which is
/// what makes the next fetch walk everything.
async fn token(data: &Mutex<Library>, service: &str) -> Option<String> {
    let library = data.lock().await;
    if library
        .needs_full_sync(service, FULL_SYNC_EVERY)
        .unwrap_or(false)
    {
        return None;
    }
    library.sync_token(service).ok().flatten()
}

/// Store what a fetch returned, and turn either half's failure into a report.
///
/// The library is taken only here — after the network work is done — and released as soon
/// as the rows are in.
async fn store<T, F>(
    service: &'static str,
    label: &'static str,
    data: &Mutex<Library>,
    account: Option<&str>,
    fetched: quince_icloud::Result<T>,
    apply: F,
) -> ServiceReport
where
    F: FnOnce(&mut Library, T) -> quince_icloud::Result<String>,
{
    let mut library = data.lock().await;
    // Fetched for an account that is no longer the library's — signed out, or another signed
    // in, while it was on the way: dropped, never laid over someone else's data.
    let owner = library.state(crate::ACCOUNT_KEY).ok().flatten();
    if account.is_none() || owner.as_deref() != account {
        return ServiceReport {
            service,
            label,
            outcome: Outcome::Failed,
            detail: Some("Signed out while this was being read.".into()),
            last_synced_at: None,
        };
    }
    // SQLite is synchronous: a full pass's apply (a whole photo library, every reminder and its
    // alarms) is CPU and disk work that would otherwise sit on one of the runtime's few
    // workers, stalling every task queued behind it there — the pictures the window is asking
    // for among them. Run in place, its worker hands those tasks on first.
    let started = std::time::Instant::now();
    // And as one change: the records, the changes still waiting laid over them and the new
    // position land together, or — failing, or the app dying part way — none of them do.
    let result = tokio::task::block_in_place(|| {
        fetched.and_then(|value| library.atomically(|library| apply(library, value)))
    });
    let took = started.elapsed();
    if took > std::time::Duration::from_millis(50) {
        quiet!("{service}: stored in {} ms", took.as_millis());
    }
    match result {
        Ok(detail) => ServiceReport {
            service,
            label,
            outcome: Outcome::Ok,
            detail: Some(detail),
            last_synced_at: library.last_synced(service).ok().flatten(),
        },
        Err(e) => {
            let message = e.to_string();
            library.mark_sync_failed(service, &message).ok();
            ServiceReport {
                service,
                label,
                outcome: Outcome::Failed,
                detail: Some(message),
                last_synced_at: library.last_synced(service).ok().flatten(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_reaches_back_a_month_and_forward_a_quarter() {
        let (from, to, _zone) = default_window();
        let start: jiff::civil::Date = from.parse().unwrap();
        let end: jiff::civil::Date = to.parse().unwrap();
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date();

        assert!(
            start < today,
            "the window must include what already happened"
        );
        assert!(end > today, "and what is coming");
        assert_eq!(start.day(), 1, "a month starts on the first");
        let days = (end - start).get_days();
        assert!(
            (110..=160).contains(&days),
            "about five months of window, got {days}"
        );
    }

    /// The window is the same shape wherever the machine is, so a refresh never silently
    /// covers less for someone in a different timezone.
    #[test]
    fn the_window_is_a_whole_number_of_months() {
        let (from, to, _) = default_window();
        let start: jiff::civil::Date = from.parse().unwrap();
        let end: jiff::civil::Date = to.parse().unwrap();
        assert_eq!(start.day(), 1);
        assert_eq!(
            end.day(),
            end.last_of_month().day(),
            "the window ends on a month boundary"
        );
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(plural(1), "");
        assert_eq!(plural(0), "s");
        assert_eq!(plural(2), "s");
    }
}
