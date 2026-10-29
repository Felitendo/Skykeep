//! Live: can iCloud Drive's changes be read item by item? Walks the main Drive zone's change
//! feed to its end (ids only), makes a throwaway folder ("Quince mount test …") in the top
//! level, reads the feed again from the end token, and prints the record names and parent
//! links that came — to be matched with the folder's own Drive id. Then deletes it, reads
//! again, and clears it from Recently Deleted. Never prints names or contents.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

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
    let url = format!(
        "{}/database/1/com.apple.clouddocs/production/private/changes/zone",
        ck.trim_end_matches('/')
    );
    let zone_id = json!({"zoneName": "com.apple.CloudDocs", "zoneType": "REGULAR_CUSTOM_ZONE"});
    // One page from a token: (records, next token, more coming).
    let page = |token: Option<String>| {
        let (http, url, p, zone_id) = (http.clone(), url.clone(), p.clone(), zone_id.clone());
        async move {
            let mut z = json!({"zoneID": zone_id, "resultsLimit": 200});
            if let Some(t) = token {
                z["syncToken"] = json!(t);
            }
            let body: Value = http
                .post(&url)
                .query(&p)
                .json(&json!({"zones": [z]}))
                .send()
                .await?
                .json()
                .await?;
            let z = body["zones"][0].clone();
            anyhow::Ok((
                z["records"].as_array().cloned().unwrap_or_default(),
                z["syncToken"].as_str().map(str::to_string),
                z["moreComing"] == true,
            ))
        }
    };
    let t = Instant::now();
    let (mut token, mut total, mut pages) = (None, 0, 0);
    loop {
        let (recs, next, more) = page(token.clone()).await?;
        total += recs.len();
        pages += 1;
        token = next;
        if !more || pages > 2000 {
            break;
        }
    }
    println!("walked {total} records in {pages} pages, {:?}", t.elapsed());
    let show = |recs: &[Value]| {
        for r in recs {
            println!(
                "  type={} name={} deleted={} parent={}",
                r["recordType"], r["recordName"], r["deleted"], r["fields"]["parent"]["value"]
            );
        }
    };
    let name = format!("Quince mount test {}", std::process::id());
    let made = quince_icloud::drive::create_folder(
        &http,
        &drivews,
        p.clone(),
        "FOLDER::com.apple.CloudDocs::root",
        &name,
    )
    .await?;
    println!("made: {made:?}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (recs, next, _) = page(token.clone()).await?;
    println!("after make: {} records", recs.len());
    show(&recs);
    token = next.or(token);
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &made).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (recs, _, _) = page(token).await?;
    println!("after delete: {} records", recs.len());
    show(&recs);
    let trash = quince_icloud::drive::trash(&http, &drivews, p.clone()).await?;
    let mine: Vec<_> = trash
        .into_iter()
        .filter(|i| i.name.starts_with("Quince mount test"))
        .collect();
    println!("throwaways in Recently Deleted: {}", mine.len());
    if !mine.is_empty() {
        quince_icloud::drive::delete_forever_many(&http, &drivews, p, &mine).await?;
    }
    Ok(())
}
