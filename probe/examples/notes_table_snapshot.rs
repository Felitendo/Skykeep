//! Read-only: one note's record and its attachments' records, every field — plain values as
//! they are, encrypted ones as their length and a short hash — written as JSON, so two
//! snapshots (before and after an edit on the phone) show exactly which fields the phone
//! changes when it edits a table. Picks the note by the start of its text.
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use sha2::Digest;

fn shape(fields: &Value) -> Value {
    let mut out = Map::new();
    for (k, v) in fields.as_object().into_iter().flatten() {
        let val = &v["value"];
        let t = v["type"].as_str().unwrap_or("");
        let shown = if t == "ENCRYPTED_BYTES"
            || t == "BYTES"
            || val.is_string() && val.as_str().is_some_and(|s| s.len() > 80)
        {
            let s = val.as_str().unwrap_or("");
            json!({ "len": s.len(), "sha": format!("{:x}", sha2::Sha256::digest(s.as_bytes()))[..12].to_string() })
        } else {
            val.clone()
        };
        out.insert(k.clone(), shown);
    }
    Value::Object(out)
}

#[tokio::main]
async fn main() -> Result<()> {
    let want = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("start of the note's text"))?;
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p);
    let walk = zc.all_records("Notes", None).await?;
    let note = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Note")
        .find(|r| {
            r["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .and_then(quince_icloud::ckdocument::text_from_base64)
                .is_some_and(|t| t.starts_with(&want))
        })
        .ok_or_else(|| anyhow!("no such note"))?;
    let id = note["recordName"].as_str().unwrap_or_default();
    let attachments: Vec<Value> = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment" && r["fields"]["Note"]["value"]["recordName"] == id)
        .map(|a| json!({ "id": a["recordName"], "tag": a["recordChangeTag"], "fields": shape(&a["fields"]) }))
        .collect();
    let snap = json!({ "note": { "id": id, "tag": note["recordChangeTag"], "fields": shape(&note["fields"]) }, "attachments": attachments });
    println!("{}", serde_json::to_string_pretty(&snap)?);
    Ok(())
}
