//! Live, throwaway event only: a weekly series set to Never from one of its occurrences,
//! counted before and after, then deleted.
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
        title: "Quince never test".into(),
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
    let count = || async {
        let mut q = p.clone();
        for (k, v) in [
            ("clientVersion", "5.1"),
            ("requestID", "4"),
            ("lang", "en-us"),
            ("usertz", "Asia/Kolkata"),
            ("startDate", "2026-10-26"),
            ("endDate", "2026-11-25"),
        ] {
            q.push((k.into(), v.into()));
        }
        let b: Value = http
            .get(format!("{}/ca/events", root.trim_end_matches('/')))
            .query(&q)
            .send()
            .await?
            .json()
            .await?;
        Ok::<usize, anyhow::Error>(
            b["Event"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| {
                    e["title"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("Quince never test"))
                })
                .count(),
        )
    };
    println!("before: {} occurrences", count().await?);
    let mut first = event.clone();
    first.guid = format!("{}__20261026T073300Z", event.guid);
    cal.put_event_full(
        &first,
        None,
        None,
        Some(None),
        Some(quince_icloud::calendar::Scope::All),
    )
    .await?;
    println!("after Never: {} occurrence(s)", count().await?);
    cal.delete_event(&event, None).await.ok();
    println!("left after delete: {}", count().await?);
    Ok(())
}
