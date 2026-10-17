//! Live, read-only: does Calendar's startup/events answer carry each event's Alarm objects?
use anyhow::{anyhow, Result};
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
    let mut q = client.service_params();
    for (k, v) in [
        ("clientVersion", "5.1"),
        ("lang", "en-us"),
        ("usertz", "Asia/Kolkata"),
        ("startDate", "2026-10-01"),
        ("endDate", "2026-10-31"),
        ("requestID", "4"),
    ] {
        q.push((k.into(), v.into()));
    }
    for path in ["ca/startup", "ca/events"] {
        let body: serde_json::Value = client
            .http()
            .get(format!("{}/{path}", root.trim_end_matches('/')))
            .query(&q)
            .send()
            .await?
            .json()
            .await?;
        let keys: Vec<_> = body
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let alarms = body["Alarm"].as_array().map_or(0, Vec::len);
        let with = body["Event"].as_array().map_or(0, |e| {
            e.iter()
                .filter(|e| e["alarms"].as_array().is_some_and(|a| !a.is_empty()))
                .count()
        });
        println!("{path}: keys={keys:?} Alarm objects={alarms} events with alarms={with}");
        if let Some(a) = body["Alarm"].as_array().and_then(|a| a.first()) {
            println!(
                "  e.g. {}",
                a.to_string().chars().take(300).collect::<String>()
            );
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    let body = serde_json::json!({ "ClientState": { "Collection": [], "alarmRange": 60, "fullState": true, "userTime": now } });
    let r = client
        .http()
        .post(format!("{}/ca/startup", root.trim_end_matches('/')))
        .query(&q)
        .json(&body)
        .send()
        .await?;
    let st = r.status();
    let t = r.text().await?;
    println!(
        "POST startup {st}: {}",
        t.chars().take(600).collect::<String>()
    );
    Ok(())
}
