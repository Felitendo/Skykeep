//! Throwaway: put real events on the account, read them back, and take them away.
//!
//! The calendar audit compared nothing because this account holds no events. This gives it
//! something to compare, and exercises the write path — which echoes the whole event back to
//! Apple, so anything it fails to carry is destroyed rather than merely unsaved.
use anyhow::{anyhow, Result};
use quince_core::calendar::Event;

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
        .ok_or_else(|| anyhow!("no calendar"))?;
    let http = client.http().clone();
    let params = client.service_params();
    let zone = jiff::tz::TimeZone::system();
    let tz = zone.iana_name().unwrap_or("UTC").to_string();

    let today = jiff::Zoned::now().date();
    let from = today.to_string();
    let to = today.saturating_add(jiff::Span::new().days(30)).to_string();

    let snapshot =
        quince_icloud::sync::fetch_calendar(&http, &root, params.clone(), &from, &to, &tz).await?;
    let calendar = snapshot
        .calendars
        .iter()
        .find(|c| c.writable)
        .or_else(|| snapshot.calendars.first())
        .ok_or_else(|| anyhow!("no calendar to write to"))?;
    println!("writing into {:?}", calendar.title);

    let start = jiff::Timestamp::now().as_second() + 86_400;
    let cases: Vec<(&str, Event)> = vec![
        (
            "timed with everything",
            Event {
                // Apple's ids are uppercase UUIDs, and a new event needs one before it can be
                // addressed at all — the path is /ca/events/<calendar>/<event>.
                guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
                calendar_guid: calendar.guid.clone(),
                title: "Quince round trip — safe to delete".into(),
                location: Some("Somewhere".into()),
                starts_at: start - start % 60,
                ends_at: start - start % 60 + 3600,
                all_day: false,
                timezone: Some(tz.clone()),
                url: Some("https://example.invalid/quince".into()),
                notes: Some("a note on the event".into()),
                etag: None,
            },
        ),
        (
            "all day, nothing else",
            Event {
                guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
                calendar_guid: calendar.guid.clone(),
                title: "Quince all day — safe to delete".into(),
                location: None,
                starts_at: start - start % 86_400,
                ends_at: start - start % 86_400 + 86_400,
                all_day: true,
                timezone: Some(tz.clone()),
                url: None,
                notes: None,
                etag: None,
            },
        ),
    ];

    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, params.clone());
    let mut failures = 0;
    let mut made = Vec::new();

    for (label, wanted) in &cases {
        cal.put_event(wanted, calendar.ctag.as_deref()).await?;
        let guid = wanted.guid.clone();
        made.push(guid.clone());

        // Read the window back rather than trusting what the write replied with.
        let fresh =
            quince_icloud::sync::fetch_calendar(&http, &root, params.clone(), &from, &to, &tz)
                .await?;
        let Some(got) = fresh.events.iter().find(|e| e.guid == guid) else {
            println!("  FAIL {label}: not in the calendar afterwards");
            failures += 1;
            continue;
        };

        let mut says = Vec::new();
        if got.title != wanted.title {
            says.push(format!("title {:?}", got.title));
        }
        if got.location != wanted.location {
            says.push(format!("location {:?}", got.location));
        }
        if got.notes != wanted.notes {
            says.push(format!("notes {:?}", got.notes));
        }
        if got.url != wanted.url {
            says.push(format!("url {:?}", got.url));
        }
        if got.all_day != wanted.all_day {
            says.push(format!("allDay {}", got.all_day));
        }
        if got.starts_at != wanted.starts_at {
            says.push(format!("starts {} vs {}", got.starts_at, wanted.starts_at));
        }
        if got.ends_at != wanted.ends_at {
            says.push(format!("ends {} vs {}", got.ends_at, wanted.ends_at));
        }
        if says.is_empty() {
            println!("  PASS {label}");
        } else {
            println!("  FAIL {label}: {}", says.join("; "));
            failures += 1;
        }
    }

    // And take them away, confirming from the account rather than the reply.
    let before_removal =
        quince_icloud::sync::fetch_calendar(&http, &root, params.clone(), &from, &to, &tz).await?;
    for guid in &made {
        if let Some(event) = before_removal.events.iter().find(|e| &e.guid == guid) {
            cal.delete_event(event, calendar.ctag.as_deref()).await?;
        }
    }
    let finally =
        quince_icloud::sync::fetch_calendar(&http, &root, params.clone(), &from, &to, &tz).await?;
    let left = finally
        .events
        .iter()
        .filter(|e| made.contains(&e.guid))
        .count();
    println!("\n{left} of {} left behind after deleting", made.len());
    if left > 0 || failures > 0 {
        return Err(anyhow!("the calendar did not hold"));
    }
    println!("every field survived the round trip, and nothing was left behind");
    Ok(())
}
