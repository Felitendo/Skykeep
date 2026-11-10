//! Test data for the Android calendar sync, in a calendar of its own (never touching another):
//!
//!   cargo run -p quince-probe --example calendar_test_events -- create
//!       makes "Skykeep test" with a timed event (New York), an all-day event and a weekly
//!       repeat (four times, Kolkata); prints the calendar's id
//!   cargo run -p quince-probe --example calendar_test_events -- cleanup <calendar id>
//!       deletes that calendar, and with it its events: only the id given, and only if it is
//!       still titled "Skykeep test"
use anyhow::{anyhow, bail, Result};
use quince_core::calendar::Event;
use quince_icloud::calendar::{CalendarClient, EventRepeat};

const TITLE: &str = "Skykeep test";

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar service"))?;
    let cal = CalendarClient::new(client.http(), &root, client.service_params());
    let today = jiff::Zoned::now().date();
    let window = |d: jiff::civil::Date| (d.to_string(), d.to_string());
    let (a, b) = window(today);
    let calendars = cal.startup(&a, &b, "UTC").await?.calendars;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("create") => {
            if calendars.iter().any(|c| c.title == TITLE) {
                bail!("a calendar titled {TITLE:?} is there already: clean that one up first");
            }
            let guid = cal.create_calendar(TITLE, &calendars).await?;
            println!("calendar {guid}");
            let at = |date: jiff::civil::Date, h: i8, m: i8, zone: &str| -> Result<i64> {
                Ok(date
                    .at(h, m, 0, 0)
                    .to_zoned(jiff::tz::TimeZone::get(zone)?)?
                    .timestamp()
                    .as_second())
            };
            let day = |n: i64| today.checked_add(jiff::Span::new().days(n));
            // Each new event named by the caller, as icloud.com names one: a fresh UUID.
            let base = Event {
                calendar_guid: guid.clone(),
                ..Event::default()
            };
            let fresh = || uuid::Uuid::new_v4().to_string().to_uppercase();
            // Timed, named zone: 3–4 pm in New York, two days on.
            let timed = Event {
                guid: fresh(),
                title: "Skykeep test: timed".into(),
                starts_at: at(day(2)?, 15, 0, "America/New_York")?,
                ends_at: at(day(2)?, 16, 0, "America/New_York")?,
                timezone: Some("America/New_York".into()),
                ..base.clone()
            };
            cal.put_event(&timed, None).await?;
            // All day, three days on: midnight to midnight on this computer's clock, as the app
            // writes one.
            let here = jiff::tz::TimeZone::system();
            let midnight = |d: jiff::civil::Date| -> Result<i64> {
                Ok(d.at(0, 0, 0, 0)
                    .to_zoned(here.clone())?
                    .timestamp()
                    .as_second())
            };
            let all_day = Event {
                guid: fresh(),
                title: "Skykeep test: all-day".into(),
                starts_at: midnight(day(3)?)?,
                ends_at: midnight(day(4)?)?,
                all_day: true,
                ..base.clone()
            };
            cal.put_event(&all_day, None).await?;
            // Weekly, four times, 9:00–9:30 in Kolkata, from four days on.
            let weekly = Event {
                guid: fresh(),
                title: "Skykeep test: weekly".into(),
                starts_at: at(day(4)?, 9, 0, "Asia/Kolkata")?,
                ends_at: at(day(4)?, 9, 30, "Asia/Kolkata")?,
                timezone: Some("Asia/Kolkata".into()),
                ..base.clone()
            };
            let repeat = EventRepeat {
                freq: "weekly".into(),
                interval: 1,
                count: Some(4),
            };
            cal.put_event_full(&weekly, None, None, Some(Some(&repeat)), None)
                .await?;
            println!(
                "made: timed {}, all-day {}, weekly from {} (x4)",
                day(2)?,
                day(3)?,
                day(4)?
            );
        }
        Some("edit") => {
            // The timed test event, found in the test calendar by its title there, renamed and
            // moved an hour later.
            let id = args.get(1).ok_or_else(|| anyhow!("which calendar id?"))?;
            let found = calendars
                .iter()
                .find(|c| &c.guid == id && c.title == TITLE)
                .ok_or_else(|| anyhow!("no test calendar {id}"))?;
            let (a, b) = (
                today.to_string(),
                today.checked_add(jiff::Span::new().days(10))?.to_string(),
            );
            let snap = quince_icloud::sync::fetch_calendar(
                client.http(),
                &root,
                client.service_params(),
                &a,
                &b,
                "UTC",
            )
            .await?;
            let mut event = snap
                .events
                .into_iter()
                .find(|e| e.calendar_guid == found.guid && e.title == "Skykeep test: timed")
                .ok_or_else(|| anyhow!("the timed test event is not there"))?;
            event.title = "Skykeep test: timed (edited)".into();
            event.starts_at += 3600;
            event.ends_at += 3600;
            cal.put_event(&event, found.ctag.as_deref()).await?;
            println!("edited {}", event.guid);
        }
        Some("cleanup") => {
            let id = args.get(1).ok_or_else(|| anyhow!("which calendar id?"))?;
            let Some(found) = calendars.iter().find(|c| &c.guid == id) else {
                bail!("no calendar with id {id}: nothing removed");
            };
            if found.title != TITLE {
                bail!(
                    "calendar {id} is titled {:?}, not {TITLE:?}: refused",
                    found.title
                );
            }
            cal.delete_calendar(found).await?;
            println!("removed calendar {id}");
        }
        _ => bail!("create, or cleanup <calendar id>"),
    }
    Ok(())
}
