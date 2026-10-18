//! Read-only: a note's table by the chain the app follows — the note's runs name the
//! attachment, the attachment record is fetched, its cells are read.
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let note = zc
        .lookup_record("Notes", "50a80840-1931-45d9-9774-7d7ab70d9022")
        .await?
        .ok_or_else(|| anyhow!("no note"))?;
    println!("by {} / created {}", note["modified"], note["created"]);
    println!(
        "text {:?} modified {}",
        quince_icloud::ckdocument::stored_text_from_base64(
            note["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .unwrap_or("")
        ),
        note["fields"]["ModificationDate"]["value"]
    );
    let runs = quince_icloud::ckdocument::runs_from_base64(
        note["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    );
    let raw = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        note["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or("")
            .trim(),
    )
    .unwrap_or_default();
    for r in quince_icloud::ckdocument::debug_attribute_runs(&raw) {
        println!(
            "{}",
            quince_icloud::ckdocument::debug_tree(&r, 6).join(" | ")
        );
    }
    println!(
        "{} runs: {:?}",
        runs.len(),
        runs.iter()
            .map(|r| (r.length, r.attachment.clone()))
            .collect::<Vec<_>>()
    );
    for id in runs.iter().filter_map(|r| r.attachment.clone()) {
        let rec = zc.lookup_record("Notes", &id).await?;
        let cells = rec
            .as_ref()
            .and_then(|r| r["fields"]["MergeableDataEncrypted"]["value"].as_str())
            .and_then(quince_icloud::table::cells_from_base64);
        println!("attachment {id}:");
        for row in cells.unwrap_or_default() {
            println!("  {row:?}");
        }
    }
    Ok(())
}
