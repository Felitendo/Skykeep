//! Live, READ-ONLY: ask CloudKit (records/query, which changes nothing) about record types and
//! fields this account holds no records of, by how it answers a filter on each field name.
//! Prints only Apple's error text and record counts.
use anyhow::{anyhow, Result};
use serde_json::json;
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &root,
        "com.apple.reminders",
        client.service_params(),
    );
    let probes: &[(&str, &str, serde_json::Value)] = &[
        ("Hashtag", "", json!(null)),
        ("Hashtag", "Name", json!({"value":"x","type":"STRING"})),
        ("Hashtag", "Type", json!({"value":0,"type":"NUMBER_INT64"})),
        (
            "Hashtag",
            "CreationDate",
            json!({"value":0,"type":"TIMESTAMP"}),
        ),
        (
            "Hashtag",
            "Deleted",
            json!({"value":0,"type":"NUMBER_INT64"}),
        ),
        (
            "Hashtag",
            "NoSuchFieldXyz",
            json!({"value":0,"type":"NUMBER_INT64"}),
        ),
        ("Attachment", "", json!(null)),
        ("Attachment", "URL", json!({"value":"x","type":"STRING"})),
        ("Attachment", "Type", json!({"value":"URL","type":"STRING"})),
        (
            "Attachment",
            "UTI",
            json!({"value":"public.url","type":"STRING"}),
        ),
        (
            "Attachment",
            "NoSuchFieldXyz",
            json!({"value":0,"type":"NUMBER_INT64"}),
        ),
        ("NoSuchTypeXyz", "", json!(null)),
        ("ListSection", "", json!(null)),
        ("SmartList", "", json!(null)),
        (
            "Reminder",
            "DueDateDeltaAlertsData",
            json!({"value":"eA==","type":"BYTES"}),
        ),
    ];
    let extra: Vec<String> = std::env::args().skip(1).collect();
    let owned: Vec<(String, String, serde_json::Value)> = if extra.is_empty() {
        probes
            .iter()
            .map(|(a, b, c)| (a.to_string(), b.to_string(), c.clone()))
            .collect()
    } else {
        extra
            .into_iter()
            .map(|t| (t, String::new(), json!(null)))
            .collect()
    };
    for (ty, f, v) in &owned {
        let q = if f.is_empty() {
            json!({"recordType": ty})
        } else {
            json!({"recordType": ty, "filterBy": [{"comparator":"EQUALS","fieldName": f, "fieldValue": v}]})
        };
        match zc.records_query("Reminders", q).await {
            Ok(r) => println!("{ty}.{f}: ok, {} records", r.len()),
            Err(e) => println!("{ty}.{f}: {e}"),
        }
    }
    Ok(())
}
