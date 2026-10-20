//! Read only: which fields Apple puts on a burst's photos and on a Live Photo, and their values
//! across his library — so bursts can be stacked and Live Photos marked by Apple's own words.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let fetched = quince_icloud::photos::fetch(&http, &root, p, None).await?;
    let mut names: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut bursts: BTreeMap<String, Vec<(String, i64, i64)>> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    for r in &fetched.records {
        let kind = r["recordType"].as_str().unwrap_or("?").to_string();
        *types.entry(kind.clone()).or_default() += 1;
        let Some(fields) = r["fields"].as_object() else {
            continue;
        };
        for (k, v) in fields {
            let lk = k.to_lowercase();
            if lk.contains("burst")
                || lk.contains("live")
                || lk.contains("subtype")
                || lk.contains("vidcompl")
                || lk.contains("adjustmenttype")
                || lk.contains("playback")
                || lk.contains("variation")
            {
                let val = v["value"].clone();
                let shown = if val.is_object() {
                    "{object}".to_string()
                } else {
                    val.to_string()
                };
                let shown = if shown.len() > 40 {
                    format!("{}…", &shown[..40])
                } else {
                    shown
                };
                *names
                    .entry(format!("{kind}.{k}"))
                    .or_default()
                    .entry(shown)
                    .or_default() += 1;
            }
        }
        if kind == "CPLAsset" {
            if let Some(id) = fields.get("burstId").and_then(|v| v["value"].as_str()) {
                bursts.entry(id.to_string()).or_default().push((
                    r["recordName"].as_str().unwrap_or("").to_string(),
                    fields
                        .get("burstFlags")
                        .and_then(|v| v["value"].as_i64())
                        .unwrap_or(-1),
                    fields
                        .get("burstFlagsExt")
                        .and_then(|v| v["value"].as_i64())
                        .unwrap_or(-1),
                ));
            }
        }
    }
    println!("record types: {types:?}");
    for (k, vals) in &names {
        let mut v: Vec<_> = vals.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!(
            "{k}: {}",
            v.iter()
                .take(8)
                .map(|(x, n)| format!("{x}×{n}"))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
    println!("bursts: {}", bursts.len());
    for (id, members) in bursts.iter().take(5) {
        let flags: Vec<String> = members.iter().map(|m| format!("{}/{}", m.1, m.2)).collect();
        println!(
            "  {}…: {} photos, flags {}",
            &id[..8.min(id.len())],
            members.len(),
            flags.join(" ")
        );
    }
    Ok(())
}
