//! Live, throwaway event only: a weekly event four times, its third occurrence renamed for
//! "all future events" — the first two must keep their name — then every piece deleted.
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
        title: "Quince future test".into(),
        starts_at: 1_793_000_000,
        ends_at: 1_793_003_600,
        timezone: Some("Asia/Kolkata".into()),
        ..Default::default()
    };
    let rule = quince_icloud::calendar::EventRepeat {
        freq: "weekly".into(),
        interval: 1,
        count: Some(4),
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
    let list = |label: &'static str| {
        let (url, q) = raw("2026-10-26", "2026-12-01");
        let http = http.clone();
        async move {
            let body: Value = http.get(url).query(&q).send().await?.json().await?;
            let found: Vec<(String, String, String)> = body["Event"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| {
                    e["title"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("Quince future test"))
                })
                .map(|e| {
                    (
                        e["guid"].as_str().unwrap_or("").to_string(),
                        e["title"].as_str().unwrap_or("").to_string(),
                        e["startDate"].to_string(),
                    )
                })
                .collect();
            for f in &found {
                println!("{label}: {f:?}");
            }
            anyhow::Ok(found)
        }
    };
    let before = list("before").await?;
    let third_id = before
        .get(2)
        .map(|f| f.0.clone())
        .ok_or_else(|| anyhow!("no third occurrence"))?;
    let mut third = event.clone();
    third.guid = third_id;
    third.starts_at += 14 * 86_400;
    third.ends_at += 14 * 86_400;
    third.title = "Quince future test, from third".into();
    let outcome = cal
        .put_event_full(
            &third,
            None,
            None,
            None,
            Some(quince_icloud::calendar::Scope::Future),
        )
        .await;
    println!(
        "future edit: {:?}",
        outcome
            .as_ref()
            .map(|_| "accepted")
            .map_err(|e| e.to_string())
    );
    let after = list("after").await?;
    let renamed = after
        .iter()
        .filter(|f| f.1 == "Quince future test, from third")
        .count();
    let kept = after.iter().filter(|f| f.1 == "Quince future test").count();
    println!("RESULT kept={kept} renamed={renamed} (want 2 and 2)");
    // Deleting from the last day on leaves the one before it.
    if let Some(last) = after.iter().max_by(|a, b| a.2.cmp(&b.2)) {
        let mut e = third.clone();
        e.guid = last.0.clone();
        e.starts_at += 7 * 86_400;
        e.ends_at += 7 * 86_400;
        let gone = cal
            .delete_event_scoped(&e, None, Some(quince_icloud::calendar::Scope::Future))
            .await;
        println!(
            "future delete: {:?}",
            gone.map(|_| "accepted").map_err(|e| e.to_string())
        );
    }
    let after = list("after delete").await?;
    println!("RESULT after delete: {} left (want 3)", after.len());
    // Every series that came of it, deleted.
    let mut series: Vec<String> = after
        .iter()
        .map(|f| quince_icloud::calendar::series_of(&f.0).to_string())
        .collect();
    series.sort();
    series.dedup();
    for s in series {
        let mut e = event.clone();
        e.guid = s;
        if let Err(err) = cal.delete_event(&e, None).await {
            println!("delete {}: {err}", e.guid);
        }
    }
    list("left").await?;
    Ok(())
}
