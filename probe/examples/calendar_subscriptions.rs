//! READ-ONLY probe: which collections the iCloud calendar service lists, by kind, and whether
//! any subscribed calendar's events come back with them. Prints kinds, flags and counts only,
//! never titles or URLs. Writes nothing.
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
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
    let mut q = p.clone();
    for (k, v) in [
        ("clientVersion", "5.1"),
        ("requestID", "7"),
        ("lang", "en-us"),
        ("usertz", "Asia/Kolkata"),
        ("startDate", "2026-09-01"),
        ("endDate", "2026-09-30"),
        ("bday", "true"),
    ] {
        q.push((k.into(), v.into()));
    }
    let v = http
        .get(format!("{root}/ca/startup"))
        .query(&q)
        .send()
        .await?;
    let status = v.status();
    let text = v.text().await?;
    println!("status {status}, {} bytes", text.len());
    let v: serde_json::Value = serde_json::from_str(&text)?;
    let top: Vec<_> = v
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    println!("top-level keys: {top:?}");
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    for e in v["Event"].as_array().into_iter().flatten() {
        *per.entry(e["pGuid"].as_str().unwrap_or("?").to_string())
            .or_default() += 1;
    }
    for (i, c) in v["Collection"].as_array().into_iter().flatten().enumerate() {
        let keys: Vec<_> = c
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let guid = c["guid"].as_str().unwrap_or("");
        println!(
            "#{i} objectType={} supportedType={} readOnly={} isFamily={} enabled={} events={} keys={keys:?}",
            c["objectType"], c["supportedType"], c["readOnly"], c["isFamily"], c["enabled"],
            per.get(guid).copied().unwrap_or(0)
        );
    }
    let bdays = v["Event"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e.get("birthdayIsYearlessBday").is_some()
                || e["pGuid"].as_str().is_some_and(|g| g.contains("irthday"))
        })
        .count();
    println!(
        "events total={} with birthday fields={bdays}",
        v["Event"].as_array().map_or(0, |a| a.len())
    );
    Ok(())
}
