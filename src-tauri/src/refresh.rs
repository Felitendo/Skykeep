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

/// The services one pass covers, in the order a person would notice them going stale.
const COVERED: [(&str, &str); 4] = [
    ("calendar", "Calendar"),
    ("reminders", "Reminders"),
    ("notes", "Notes"),
    ("contacts", "Contacts"),
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
    }
}

/// Refresh every service the account has, and report on each.
///
/// Never returns an error: a refresh that partly failed is a normal outcome with a partial
/// result, not an exception, and the interface has to be able to show exactly that.
pub async fn all(client: &Arc<Mutex<Client>>, data: &Mutex<Library>) -> Refresh {
    let started = std::time::Instant::now();
    let ctx = context(client).await;
    let (from, to, zone) = default_window();

    let mut reports = Vec::with_capacity(COVERED.len());

    reports.push(match ctx.calendar.as_deref() {
        None => unavailable("calendar", "Calendar"),
        Some(root) => {
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
    });

    reports.push(match ctx.cloudkit.as_deref() {
        None => unavailable("reminders", "Reminders"),
        Some(root) => {
            let since = token(data, "reminders").await;
            let fetched = quince_icloud::reminders::fetch_cloudkit(
                &ctx.http,
                root,
                ctx.params.clone(),
                since.as_deref(),
            )
            .await;
            store("reminders", "Reminders", data, fetched, |library, f| {
                let (lists, count, _) = quince_icloud::reminders::apply_cloudkit(library, f)?;
                Ok(format!(
                    "{count} reminder{} in {lists} list{}",
                    plural(count),
                    plural(lists)
                ))
            })
            .await
        }
    });

    reports.push(match ctx.cloudkit.as_deref() {
        None => unavailable("notes", "Notes"),
        Some(root) => {
            let since = token(data, "notes").await;
            let fetched = quince_icloud::notes::fetch_cloudkit(
                &ctx.http,
                root,
                ctx.params.clone(),
                since.as_deref(),
            )
            .await;
            store("notes", "Notes", data, fetched, |library, f| {
                let (folders, notes, _) = quince_icloud::notes::apply_cloudkit(library, f)?;
                Ok(format!(
                    "{notes} note{} in {folders} folder{}",
                    plural(notes),
                    plural(folders)
                ))
            })
            .await
        }
    });

    reports.push(match ctx.contacts.as_deref() {
        None => unavailable("contacts", "Contacts"),
        Some(root) => {
            let fetched = quince_icloud::contacts::fetch(&ctx.http, root, ctx.params.clone()).await;
            store("contacts", "Contacts", data, fetched, |library, people| {
                let count = quince_icloud::contacts::apply(library, people)?;
                Ok(format!("{count} contact{}", plural(count)))
            })
            .await
        }
    });

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

async fn token(data: &Mutex<Library>, service: &str) -> Option<String> {
    let library = data.lock().await;
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
    fetched: quince_icloud::Result<T>,
    apply: F,
) -> ServiceReport
where
    F: FnOnce(&mut Library, T) -> quince_icloud::Result<String>,
{
    let mut library = data.lock().await;
    let result = fetched.and_then(|value| apply(&mut library, value));
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
