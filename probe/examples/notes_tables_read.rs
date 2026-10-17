//! Read-only: every table in Notes — its attachment record and the CRDT inside — dumped so
//! the shape can be matched exactly before Quince writes one. Nothing is written.
use anyhow::{anyhow, Result};
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
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for r in &walk.records {
        *kinds
            .entry(r["recordType"].as_str().unwrap_or("?").to_string())
            .or_default() += 1;
    }
    println!("records: {kinds:?}");
    for r in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment")
    {
        let f = &r["fields"];
        let uti = f["UTI"]["value"]
            .as_str()
            .or(f["UTIEncrypted"]["value"].as_str())
            .unwrap_or("");
        let keys: Vec<&String> = f
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        for k in [
            "Note",
            "TitleEncrypted",
            "FallbackTitle",
            "FallbackSubtitleIOS",
            "MinimumSupportedNotesVersion",
            "UTIEncrypted",
            "CreationDate",
        ] {
            println!(
                "  {k} = {}",
                f[k].to_string().chars().take(160).collect::<String>()
            );
        }
        if let Some(note) = f["Note"]["value"]["recordName"].as_str() {
            if let Some(n) = walk.records.iter().find(|x| x["recordName"] == note) {
                let body = n["fields"]["TextDataEncrypted"]["value"]
                    .as_str()
                    .unwrap_or("");
                let raw = quince_icloud::ckdocument::debug_inflate(body).unwrap_or_default();
                println!(
                    "  note text: {:?}",
                    quince_icloud::ckdocument::text_from_base64(body)
                );
                for (i, run) in quince_icloud::ckdocument::debug_attribute_runs(&raw)
                    .iter()
                    .enumerate()
                {
                    println!(
                        "  run {i}: {}",
                        quince_icloud::ckdocument::debug_tree(run, 6).join(" | ")
                    );
                }
            }
        }
        println!("attachment {} uti={uti} keys={keys:?}", r["recordName"]);
        if let Some(m) = f["MergeableDataEncrypted"]["value"].as_str() {
            if let Some(raw) = quince_icloud::ckdocument::debug_inflate(m) {
                for line in quince_icloud::ckdocument::debug_tree(&raw, 12).iter() {
                    println!("  {line}");
                }
            }
        }
    }
    Ok(())
}
