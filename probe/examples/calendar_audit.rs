//! Throwaway: every event in a real window, as Apple has it and as this app stored it.
use anyhow::{anyhow, Result};
use std::collections::HashMap;

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

    let today = jiff::Zoned::now().date();
    let from = today
        .saturating_sub(jiff::Span::new().months(2))
        .to_string();
    let to = today
        .saturating_add(jiff::Span::new().months(4))
        .to_string();
    let zone = jiff::tz::TimeZone::system();
    let tz = zone.iana_name().unwrap_or("UTC");

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    let snapshot =
        quince_icloud::sync::fetch_calendar(&http, &root, params.clone(), &from, &to, tz).await?;
    let summary = quince_icloud::sync::apply_calendar(&mut library, snapshot)?;
    println!(
        "{} calendars, {} events in {from}..{to}",
        summary.calendars, summary.events
    );

    let mine: HashMap<String, quince_core::calendar::Event> = library
        .events_between(0, i64::MAX)?
        .into_iter()
        .map(|e| (e.guid.clone(), e))
        .collect();

    // Ask again, into a fresh library, and compare the two answers.
    let second = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&second)?;
    let mut fresh = quince_core::Library::open(&second)?;
    let again = quince_icloud::sync::fetch_calendar(&http, &root, params, &from, &to, tz).await?;
    quince_icloud::sync::apply_calendar(&mut fresh, again)?;

    let mut problems = 0;
    let mut checked = 0;
    for theirs in fresh.events_between(0, i64::MAX)? {
        let Some(ours) = mine.get(&theirs.guid) else {
            println!("MISSING  {:?}", theirs.title);
            problems += 1;
            continue;
        };
        checked += 1;
        let mut says = Vec::new();
        if ours.title != theirs.title {
            says.push(format!("title {:?} vs {:?}", ours.title, theirs.title));
        }
        if ours.starts_at != theirs.starts_at {
            says.push(format!("starts {} vs {}", ours.starts_at, theirs.starts_at));
        }
        if ours.ends_at != theirs.ends_at {
            says.push(format!("ends {} vs {}", ours.ends_at, theirs.ends_at));
        }
        if ours.all_day != theirs.all_day {
            says.push(format!("allDay {} vs {}", ours.all_day, theirs.all_day));
        }
        if ours.calendar_guid != theirs.calendar_guid {
            says.push(format!(
                "calendar {:?} vs {:?}",
                ours.calendar_guid, theirs.calendar_guid
            ));
        }
        if ours.timezone != theirs.timezone {
            says.push(format!("zone {:?} vs {:?}", ours.timezone, theirs.timezone));
        }
        if !says.is_empty() {
            println!(
                "DIFFERS  {:<28} {}",
                theirs.title.chars().take(26).collect::<String>(),
                says.join("; ")
            );
            problems += 1;
        }
    }
    println!("\n{checked} events compared, {problems} disagreement(s)");
    Ok(())
}
