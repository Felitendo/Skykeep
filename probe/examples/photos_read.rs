//! Read-only: the library's count, the newest few photos and one thumbnail, as icloud.com
//! asks for them. Nothing is written.
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
    let (http, mut p) = (client.http().clone(), client.service_params());
    p.push(("remapEnums".into(), "true".into()));
    p.push(("getCurrentSyncToken".into(), "true".into()));
    let base = format!(
        "{}/database/1/com.apple.photos.cloud/production/private",
        root.trim_end_matches('/')
    );
    let post = |op: &str, body: Value| {
        let (http, url, p, op) = (
            http.clone(),
            format!("{base}/{op}"),
            p.clone(),
            op.to_string(),
        );
        async move {
            let r = http
                .post(url)
                .query(&p)
                .header("content-type", "text/plain")
                .body(body.to_string())
                .send()
                .await?;
            let status = r.status();
            let text = r.text().await?;
            println!("{op}: {status} ({} bytes)", text.len());
            anyhow::Ok(serde_json::from_str::<Value>(&text).unwrap_or(Value::Null))
        }
    };
    let zones = http
        .get(format!("{base}/zones/list"))
        .query(&p)
        .send()
        .await?
        .json::<Value>()
        .await?;
    for z in zones["zones"].as_array().into_iter().flatten() {
        println!(
            "zone {} {}",
            z["zoneID"]["zoneName"], z["zoneID"]["zoneType"]
        );
    }
    let count = post("records/query", json!({"query":{"recordType":"HyperionIndexCountLookup","filterBy":[{"fieldName":"indexCountID","comparator":"IN","fieldValue":{"value":["CPLAssetByAssetDateWithoutHiddenOrDeleted"],"type":"STRING_LIST"}}]},"zoneID":{"zoneName":"PrimarySync"}})).await?;
    println!(
        "count record: {}",
        count["records"][0]["fields"]
            .to_string()
            .chars()
            .take(300)
            .collect::<String>()
    );
    let page = post("records/query", json!({"query":{"recordType":"CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted","filterBy":[{"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"DESCENDING","type":"STRING"}},{"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":0,"type":"INT64"}}]},"zoneID":{"zoneName":"PrimarySync"},"resultsLimit":6})).await?;
    let records = page["records"].as_array().cloned().unwrap_or_default();
    let mut thumb = None;
    for r in &records {
        let f = &r["fields"];
        let keys: Vec<&String> = f
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        println!("{} {} keys={keys:?}", r["recordType"], r["recordName"]);
        if r["recordType"] == "CPLMaster" {
            println!(
                "   filenameEnc={} itemType={} thumb={}x{}",
                f["filenameEnc"]["value"],
                f["itemType"]["value"],
                f["resJPEGThumbWidth"]["value"],
                f["resJPEGThumbHeight"]["value"]
            );
            if thumb.is_none() {
                thumb = f["resJPEGThumbRes"]["value"]["downloadURL"]
                    .as_str()
                    .map(String::from);
            }
        } else {
            println!(
                "   masterRef={} assetDate={} isFavorite={} addedDate={}",
                f["masterRef"]["value"]["recordName"],
                f["assetDate"]["value"],
                f["isFavorite"]["value"],
                f["addedDate"]["value"]
            );
        }
    }
    if let Some(url) = thumb {
        let url = url.replace("${f}", "thumb.jpg");
        let r = http.get(&url).send().await?;
        let status = r.status();
        let b = r.bytes().await?;
        println!(
            "thumbnail: {status} {} bytes, jpeg={}",
            b.len(),
            b.starts_with(&[0xff, 0xd8])
        );
    }
    Ok(())
}
