//! Live, throwaway event only: made with an alert in one calendar, moved to another as the app
//! moves one, and read back — one event, in the new calendar, its alert kept — then deleted.
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
    let (http, params) = (client.http().clone(), client.service_params());
    let tz = jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string();
    let today = jiff::Zoned::now().date();
    let (from, to) = (
        today.to_string(),
        today.saturating_add(jiff::Span::new().days(3)).to_string(),
    );
    let read = || {
        let (http, root, params, from, to, tz) = (&http, &root, params.clone(), &from, &to, &tz);
        async move { quince_icloud::sync::fetch_calendar(http, root, params, from, to, tz).await }
    };
    let snap = read().await?;
    let writable: Vec<_> = snap.calendars.iter().filter(|c| c.writable).collect();
    let [a, b, ..] = writable.as_slice() else {
        return Err(anyhow!("two writable calendars are needed"));
    };
    let start = jiff::Timestamp::now().as_second() + 86_400;
    let start = start - start % 3600;
    let mut event = Event {
        guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
        calendar_guid: a.guid.clone(),
        title: "Quince move test — safe to delete".into(),
        location: None,
        starts_at: start,
        ends_at: start + 1800,
        all_day: false,
        timezone: Some(tz.clone()),
        url: None,
        notes: None,
        etag: None,
    };
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, params.clone());
    quince_icloud::sync::save_event_with_alerts(
        &http,
        &root,
        params.clone(),
        a.ctag.as_deref(),
        &event,
        Some(&[15]),
        None,
        None,
        None,
    )
    .await?;
    let result = async {
        let made = read().await?;
        let held = made
            .events
            .iter()
            .find(|e| e.guid == event.guid)
            .ok_or_else(|| anyhow!("not made"))?
            .clone();
        println!(
            "made in {:?}, alerts {:?}",
            a.title,
            cal.alerts(&held).await?
        );
        // Moved as the app moves one: its etag, and where it was.
        event.etag = held.etag.clone();
        event.calendar_guid = b.guid.clone();
        let a_tag = made
            .calendars
            .iter()
            .find(|c| c.guid == a.guid)
            .and_then(|c| c.ctag.clone());
        let b_tag = made
            .calendars
            .iter()
            .find(|c| c.guid == b.guid)
            .and_then(|c| c.ctag.clone());
        quince_icloud::sync::save_event_with_alerts(
            &http,
            &root,
            params.clone(),
            b_tag.as_deref(),
            &event,
            None,
            None,
            None,
            Some((a.guid.as_str(), a_tag.as_deref())),
        )
        .await?;
        let after = read().await?;
        let copies: Vec<_> = after
            .events
            .iter()
            .filter(|e| e.guid == event.guid || e.title == event.title)
            .collect();
        println!(
            "after the move: {} cop{}, in {:?}",
            copies.len(),
            if copies.len() == 1 { "y" } else { "ies" },
            copies
                .iter()
                .map(|e| e.calendar_guid == b.guid)
                .collect::<Vec<_>>()
        );
        anyhow::ensure!(copies.len() == 1 && copies[0].calendar_guid == b.guid);
        let alerts = cal.alerts(copies[0]).await?;
        println!("its alerts: {alerts:?}");
        anyhow::ensure!(alerts == vec![15]);
        anyhow::Ok(())
    }
    .await;
    // Away, wherever it is.
    let last = read().await?;
    for e in last.events.iter().filter(|e| e.title == event.title) {
        let tag = last
            .calendars
            .iter()
            .find(|c| c.guid == e.calendar_guid)
            .and_then(|c| c.ctag.clone());
        cal.delete_event(e, tag.as_deref()).await?;
    }
    let left = read()
        .await?
        .events
        .iter()
        .filter(|e| e.title == event.title)
        .count();
    println!("{left} left behind");
    result?;
    anyhow::ensure!(left == 0);
    Ok(())
}
