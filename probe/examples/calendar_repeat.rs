//! Live, throwaway event only: a weekly event three times, its occurrences read back, its
//! title edited without touching the repeat, the repeat read again — then deleted.
use anyhow::{anyhow, Result};
use serde_json::Value;
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
    let (http, p) = (client.http().clone(), client.service_params());
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, p.clone());
    let snap = cal
        .startup("2026-10-01", "2026-10-31", "Asia/Kolkata")
        .await?;
    let home = snap
        .calendars
        .iter()
        .find(|c| c.writable)
        .ok_or_else(|| anyhow!("no writable calendar"))?;
    let event = quince_core::calendar::Event {
        guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
        calendar_guid: home.guid.clone(),
        title: "Quince repeat test".into(),
        starts_at: 1_793_000_000,
        ends_at: 1_793_003_600,
        timezone: Some("Asia/Kolkata".into()),
        ..Default::default()
    };
    let rule = quince_icloud::calendar::EventRepeat {
        freq: "weekly".into(),
        interval: 1,
        count: Some(3),
        ..quince_icloud::calendar::EventRepeat::default()
    };
    cal.put_event_full(&event, None, None, Some(Some(&rule)), None)
        .await?;
    let raw = |from: &str, to: &str| {
        let mut q = p.clone();
        for (k, v) in [
            ("clientVersion", "5.1"),
            ("requestID", "4"),
            ("lang", "en-us"),
            ("usertz", "Asia/Kolkata"),
            ("startDate", from),
            ("endDate", to),
        ] {
            q.push((k.into(), v.into()));
        }
        (format!("{}/ca/events", root.trim_end_matches('/')), q)
    };
    for (from, to) in [("2026-10-26", "2026-11-25")] {
        let (url, q) = raw(from, to);
        let body: Value = http.get(url).query(&q).send().await?.json().await?;
        for e in body["Event"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["title"] == "Quince repeat test")
        {
            println!(
                "occurrence guid={} start={} recurrence={} master={} exception={}",
                e["guid"],
                e["startDate"],
                e["recurrence"],
                e["recurrenceMaster"],
                e["recurrenceException"]
            );
        }
        println!(
            "Recurrence objects: {}",
            body["Recurrence"]
                .to_string()
                .chars()
                .take(300)
                .collect::<String>()
        );
    }
    // As Quince sends an edit of the second occurrence today: its own id, as if an event.
    let second = format!("{}__20261102T073300Z", event.guid);
    let mut occ = event.clone();
    occ.guid = second.clone();
    occ.starts_at += 7 * 86_400;
    occ.ends_at += 7 * 86_400;
    occ.title = "Quince repeat test, second edited".into();
    match cal.put_event(&occ, None).await {
        Ok(_) => println!("edit of occurrence, no scope: ACCEPTED (bad)"),
        Err(e) => println!("edit of occurrence, no scope: refused — {e}"),
    }
    use quince_icloud::calendar::Scope;
    match cal
        .put_event_full(&occ, None, None, None, Some(Scope::This))
        .await
    {
        Ok(_) => println!("edit of occurrence, this event: accepted"),
        Err(e) => println!("edit of occurrence, this event: refused — {e}"),
    }
    let mut third = event.clone();
    third.guid = format!("{}__20261109T073300Z", event.guid);
    third.starts_at += 14 * 86_400;
    third.ends_at += 14 * 86_400;
    match cal
        .delete_event_scoped(&third, None, Some(Scope::This))
        .await
    {
        Ok(_) => println!("delete of third occurrence, this event: accepted"),
        Err(e) => println!("delete of third occurrence, this event: refused — {e}"),
    }
    let (url, q) = raw("2026-10-26", "2026-11-25");
    let body: Value = http.get(url).query(&q).send().await?.json().await?;
    for e in body["Event"].as_array().into_iter().flatten().filter(|e| {
        e["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("Quince repeat test"))
    }) {
        println!(
            "  now: guid={} title={} start={} master={} exception={}",
            e["guid"], e["title"], e["startDate"], e["recurrenceMaster"], e["recurrenceException"]
        );
    }
    cal.delete_event(&event, None).await?;
    let (url, q) = raw("2026-10-26", "2026-11-25");
    let body: Value = http.get(url).query(&q).send().await?.json().await?;
    let left: Vec<_> = body["Event"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e["title"]
                .as_str()
                .is_some_and(|t| t.starts_with("Quince repeat test"))
        })
        .map(|e| e["guid"].clone())
        .collect();
    println!("deleted series; left: {left:?}");
    Ok(())
}
