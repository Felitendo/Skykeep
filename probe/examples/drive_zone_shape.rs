//! Read-only: what iCloud Drive's CloudKit container holds — its zones, and for a few records
//! of each, the record type and field NAMES only (never values). Tells whether Drive's changes
//! can be read item by item from `changes/zone`.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let ck = s
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let base = format!(
        "{}/database/1/com.apple.clouddocs/production/private",
        ck.trim_end_matches('/')
    );
    let post = |path: &str, body: Value| {
        http.post(format!("{base}/{path}"))
            .query(&p)
            .json(&body)
            .send()
    };
    let r = post("changes/database", json!({})).await?;
    println!("changes/database: {}", r.status());
    let db: Value = r.json().await?;
    let zones: Vec<Value> = db["zones"].as_array().cloned().unwrap_or_default();
    println!("zones: {}", zones.len());
    // Follow the main Drive zone until records show, and print their ids and parent links.
    let main = zones
        .iter()
        .find(|z| z["zoneID"]["zoneName"] == "com.apple.CloudDocs")
        .cloned()
        .unwrap_or(Value::Null);
    let mut token: Option<String> = None;
    let mut shown = 0;
    for page in 0..40 {
        let mut zone = json!({"zoneID": main["zoneID"], "resultsLimit": 50});
        if let Some(t) = &token {
            zone["syncToken"] = json!(t);
        }
        let body: Value = post("changes/zone", json!({"zones": [zone]}))
            .await?
            .json()
            .await?;
        let z = &body["zones"][0];
        token = z["syncToken"].as_str().map(str::to_string);
        for rec in z["records"].as_array().into_iter().flatten() {
            if shown < 6 {
                println!(
                    "page {page} type={} name={} parent={} deleted={}",
                    rec["recordType"], rec["recordName"], rec["fields"]["parent"], rec["deleted"]
                );
                shown += 1;
            }
        }
        if z["moreComing"] != true || shown >= 6 {
            break;
        }
    }
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let root = quince_icloud::drive::fetch_folder(
        &http,
        &drivews,
        p.clone(),
        "FOLDER::com.apple.CloudDocs::root",
    )
    .await?;
    for i in root.iter().take(4) {
        println!("drivews id: {}", i.drivewsid);
    }
    return Ok(());
    #[allow(unreachable_code)]
    for z in zones.iter().take(6) {
        let zid = &z["zoneID"];
        println!("zone {}", zid["zoneName"]);
        let r = post(
            "changes/zone",
            json!({"zones": [{"zoneID": zid, "resultsLimit": 4}]}),
        )
        .await?;
        let status = r.status();
        let body: Value = r.json().await.unwrap_or(Value::Null);
        let zone = &body["zones"][0];
        println!(
            "  changes/zone {status}: moreComing={} records={} err={}",
            zone["moreComing"],
            zone["records"].as_array().map_or(0, Vec::len),
            zone["serverErrorCode"]
        );
        for rec in zone["records"].as_array().into_iter().flatten() {
            let mut fields: Vec<_> = rec["fields"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            fields.sort();
            println!(
                "  type={} deleted={} fields={fields:?}",
                rec["recordType"], rec["deleted"]
            );
        }
    }
    Ok(())
}
