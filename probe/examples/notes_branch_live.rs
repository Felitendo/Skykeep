//! Live, throwaway note only: a new note given a copy of a real note's document — one whose
//! merge records branch, as a note edited on two devices at once does — then edited the way
//! Quince saves, read back from Apple, and moved to Recently Deleted.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let source_title = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("pass the title of a branched note to copy"))?;
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
    let walk = zc.all_records("Notes", None).await?;
    let source = walk
        .records
        .iter()
        .find(|r| {
            r["recordType"] == "Note"
                && r["fields"]["TextDataEncrypted"]["value"]
                    .as_str()
                    .and_then(quince_icloud::ckdocument::text_from_base64)
                    .is_some_and(|t| t.starts_with(&source_title))
        })
        .ok_or_else(|| anyhow!("no such note"))?;
    let body = source["fields"]["TextDataEncrypted"]["value"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let original = quince_icloud::ckdocument::text_from_base64(&body).unwrap_or_default();
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        p.clone(),
        None,
        "Quince branch test",
        None,
    )
    .await?;
    let made = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("made note missing"))?;
    zc.update_record_exact(
        "Notes",
        &guid,
        "Note",
        made["recordChangeTag"].as_str().unwrap_or(""),
        serde_json::json!({ "TextDataEncrypted": { "type": "ENCRYPTED_BYTES", "value": body } }),
    )
    .await?;
    println!(
        "copied a document of {} units",
        original.encode_utf16().count()
    );
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Quince branch test".into(),
        snippet: None,
        body: None,
        locked: false,
        pinned: false,
        created_at: None,
        modified_at: None,
        editable: true,
        etag: None,
        runs: None,
    };
    let first = original.lines().next().unwrap_or("").to_string();
    let edited = original.replacen(&first, "Quince branch test", 1) + "Spoon";
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        &http,
        &root,
        p.clone(),
        &note,
        Some(&original),
        &edited,
        None,
        None,
    )
    .await?;
    let back = zc.lookup_record("Notes", &guid).await?.unwrap();
    let raw = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        back["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or("")
            .trim(),
    )
    .unwrap_or_default();
    let text = quince_icloud::ckdocument::text_from_base64(
        back["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    )
    .unwrap_or_default();
    println!("saved as sent: {}", saved == edited);
    println!("Apple holds it: {}", text == edited);
    println!(
        "records after: {}",
        quince_icloud::ckdocument::debug_rewritable(&raw)
    );
    quince_icloud::notes::delete_cloudkit(&http, &root, p.clone(), &note).await?;
    println!("moved to Recently Deleted");
    Ok(())
}
