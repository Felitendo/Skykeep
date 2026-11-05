//! Read-only: every Photos zone in the private and shared databases, as icloud.com lists them
//! with and without `excludeSCZones` (shared-collection zones: the new Shared Albums), and the
//! records of the first shared-collection zone, shown as names, types and short values.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let p = client.service_params();
    let http = client.http().clone();
    let mut sc: Vec<(String, Value)> = Vec::new();
    for db in ["private", "shared"] {
        for exclude in [true, false] {
            let url = format!(
                "{}/database/1/com.apple.photos.cloud/production/{db}/zones/list",
                root.trim_end_matches('/')
            );
            let mut q = p.clone();
            if exclude {
                q.push(("excludeSCZones".into(), "true".into()));
            }
            let r = http
                .get(&url)
                .query(&q)
                .header("X-Apple-Internal-IncludeCMMZones", "true")
                .send()
                .await?;
            let status = r.status();
            let v: Value = serde_json::from_str(&r.text().await?).unwrap_or(Value::Null);
            println!("── {db} excludeSCZones={exclude}: {status}");
            for z in v["zones"].as_array().into_iter().flatten() {
                let id = &z["zoneID"];
                let name = id["zoneName"].as_str().unwrap_or("?");
                println!(
                    "  {name}  owner={}  type={}",
                    id["ownerRecordName"]
                        .as_str()
                        .map(|s| &s[..s.len().min(10)])
                        .unwrap_or("-"),
                    id["zoneType"]
                );
                if !exclude
                    && !name.starts_with("PrimarySync")
                    && !name.starts_with("CMM")
                    && !sc.iter().any(|(_, i)| i == id)
                {
                    sc.push((db.to_string(), id.clone()));
                }
            }
        }
    }
    for (db, zone) in sc.iter().take(3) {
        let url = format!(
            "{}/database/1/com.apple.photos.cloud/production/{db}/changes/zone",
            root.trim_end_matches('/')
        );
        let body = json!({ "zones": [{ "zoneID": zone, "resultsLimit": 20 }] });
        let r = http
            .post(&url)
            .query(&p)
            .header("content-type", "text/plain")
            .body(body.to_string())
            .send()
            .await?;
        println!("── records in {} ({db}): {}", zone["zoneName"], r.status());
        let v: Value = serde_json::from_str(&r.text().await?).unwrap_or(Value::Null);
        for z in v["zones"].as_array().into_iter().flatten() {
            for rec in z["records"].as_array().into_iter().flatten() {
                let fields: Vec<String> = rec["fields"]
                    .as_object()
                    .map(|m| {
                        m.iter()
                            .map(|(k, f)| format!("{k}:{}", f["type"].as_str().unwrap_or("?")))
                            .collect()
                    })
                    .unwrap_or_default();
                println!("  {} [{}]", rec["recordType"], fields.join(", "));
            }
            println!("  moreComing={}", z["moreComing"]);
        }
    }
    Ok(())
}
