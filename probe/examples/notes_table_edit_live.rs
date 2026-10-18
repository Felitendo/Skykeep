//! Live, throwaway note only: a new note given a copy of a real note's document holding a
//! table, then edited above and below the table in one save, read back from Apple — the
//! table must still be attached where it was — and moved to Recently Deleted.
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
        "Quince table test",
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
        title: "Quince table test".into(),
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
    let edited = format!("Quince table test above\n{original}below");
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
    let runs = quince_icloud::ckdocument::debug_attribute_runs(&raw);
    println!("runs after: {}", runs.len());
    let mut at = 0u64;
    for run in &runs {
        if quince_icloud::ckdocument::debug_tree(run, 8)
            .join(" ")
            .contains("com.apple.notes.table")
        {
            let ch = text.encode_utf16().nth(at as usize);
            println!(
                "table attached at {at}: is the placeholder: {}",
                ch == Some(0xFFFC)
            );
        }
        at += quince_icloud::ckdocument::debug_run_length(run);
    }
    quince_icloud::notes::delete_cloudkit(&http, &root, p.clone(), &note).await?;
    println!("moved to Recently Deleted");
    Ok(())
}
