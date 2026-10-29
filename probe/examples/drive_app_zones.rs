//! Read-only: how iCloud Drive's app-library zones (Pages, Keynote, Scriptable…) name their
//! records and parents in the change feed, to match them with Drive's own ids
//! (`FOLDER::<zone>::documents`, `FILE::<zone>::<id>`). Ids only.
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let base = format!(
        "{}/database/1/com.apple.clouddocs/production/private",
        ck.trim_end_matches('/')
    );
    let db: Value = http
        .post(format!("{base}/changes/database"))
        .query(&p)
        .json(&json!({}))
        .send()
        .await?
        .json()
        .await?;
    for z in db["zones"].as_array().into_iter().flatten() {
        let name = z["zoneID"]["zoneName"].as_str().unwrap_or("");
        if name == "com.apple.CloudDocs" || name.ends_with("container-metadata") {
            continue;
        }
        let mut token: Option<String> = None;
        let mut shown = 0;
        for _ in 0..20 {
            let mut zone =
                json!({"zoneID": z["zoneID"], "resultsLimit": 200, "desiredKeys": ["parent"]});
            if let Some(t) = &token {
                zone["syncToken"] = json!(t);
            }
            let body: Value = http
                .post(format!("{base}/changes/zone"))
                .query(&p)
                .json(&json!({"zones": [zone]}))
                .send()
                .await?
                .json()
                .await?;
            let zz = &body["zones"][0];
            token = zz["syncToken"].as_str().map(str::to_string);
            for r in zz["records"].as_array().into_iter().flatten() {
                if shown < 4 && r["deleted"] != true {
                    println!(
                        "{name}: {} parent={}",
                        r["recordName"], r["fields"]["parent"]["value"]["recordName"]
                    );
                    shown += 1;
                }
            }
            if zz["moreComing"] != true {
                break;
            }
        }
        let root = format!("FOLDER::{name}::documents");
        match quince_icloud::drive::fetch_folder(&http, &drivews, p.clone(), &root).await {
            Ok(items) => {
                for i in items.iter().take(3) {
                    println!("{name}: drivews {}", i.drivewsid);
                }
            }
            Err(e) => println!("{name}: drivews {root}: {e}"),
        }
    }
    Ok(())
}
