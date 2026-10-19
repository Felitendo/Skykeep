//! Read-only: every field the library's assets and masters carry, the item types present,
//! and one video's and one Live Photo's fields in full, to model Photos on. Nothing is written.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
    let (http, mut p) = (client.http().clone(), client.service_params());
    p.push(("remapEnums".into(), "true".into()));
    let url = format!(
        "{}/database/1/com.apple.photos.cloud/production/private/records/query",
        root.trim_end_matches('/')
    );
    for dir in ["DESCENDING", "ASCENDING"] {
        for rank in [0i64, 1, 50, 133, 134] {
            let body = json!({"query":{"recordType":"CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted","filterBy":[
                {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":dir,"type":"STRING"}},
                {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":rank,"type":"INT64"}}]},
                "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":4});
            let v: Value = http
                .post(&url)
                .query(&p)
                .header("content-type", "text/plain")
                .body(body.to_string())
                .send()
                .await?
                .json()
                .await?;
            let names: Vec<String> = v["records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["recordType"] == "CPLAsset")
                .map(|r| {
                    format!(
                        "{}@{}",
                        &r["recordName"].as_str().unwrap_or("")[..6],
                        r["fields"]["assetDate"]["value"]
                    )
                })
                .collect();
            println!(
                "{dir} startRank={rank}: {names:?} more={}",
                v.get("continuationMarker").is_some()
            );
        }
    }
    if std::env::var("ONLY_RANKS").is_ok() {
        return Ok(());
    }
    let (mut keys_a, mut keys_m) = (BTreeSet::new(), BTreeSet::new());
    let mut types: BTreeMap<String, u32> = BTreeMap::new();
    let mut subtypes: BTreeMap<String, u32> = BTreeMap::new();
    let mut shown = BTreeSet::new();
    let mut rank = 0i64;
    let mut total = 0;
    loop {
        let body = json!({"query":{"recordType":"CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted","filterBy":[
            {"fieldName":"direction","comparator":"EQUALS","fieldValue":{"value":"DESCENDING","type":"STRING"}},
            {"fieldName":"startRank","comparator":"EQUALS","fieldValue":{"value":rank,"type":"INT64"}}]},
            "zoneID":{"zoneName":"PrimarySync"},"resultsLimit":200});
        let v: Value = http
            .post(&url)
            .query(&p)
            .header("content-type", "text/plain")
            .body(body.to_string())
            .send()
            .await?
            .json()
            .await?;
        let recs = v["records"].as_array().cloned().unwrap_or_default();
        let assets = recs
            .iter()
            .filter(|r| r["recordType"] == "CPLAsset")
            .count();
        for r in &recs {
            let f = r["fields"].as_object().cloned().unwrap_or_default();
            if r["recordType"] == "CPLAsset" {
                keys_a.extend(f.keys().cloned());
                *subtypes
                    .entry(format!(
                        "{}",
                        f.get("assetSubtypeV2")
                            .map(|x| x["value"].clone())
                            .unwrap_or(Value::Null)
                    ))
                    .or_default() += 1;
            } else {
                keys_m.extend(f.keys().cloned());
                let t = f
                    .get("itemType")
                    .and_then(|x| x["value"].as_str())
                    .unwrap_or("?")
                    .to_string();
                *types.entry(t.clone()).or_default() += 1;
                if (t.contains("movie")
                    || t.contains("mpeg")
                    || f.contains_key("resOriginalVidComplRes"))
                    && shown.insert("video")
                    || f.contains_key("resOriginalVidComplRes") && shown.insert("live")
                {
                    let mut short = serde_json::Map::new();
                    for (k, x) in &f {
                        short.insert(
                            k.clone(),
                            json!(x.to_string().chars().take(90).collect::<String>()),
                        );
                    }
                    println!("MASTER {t}: {}", serde_json::to_string_pretty(&short)?);
                }
            }
        }
        total += assets;
        if assets == 0 || recs.is_empty() {
            break;
        }
        rank += assets as i64;
        if total > 5000 {
            break;
        }
    }
    println!("assets: {total}\nitem types: {types:?}\nasset subtypes: {subtypes:?}\nasset keys: {keys_a:?}\nmaster keys: {keys_m:?}");
    Ok(())
}
