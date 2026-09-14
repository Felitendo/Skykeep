//! Throwaway: every note, as Apple has it and as this app stored it, field by field.
use anyhow::{anyhow, Result};
use serde_json::Value;

fn f<'a>(r: &'a Value, k: &str) -> Option<&'a Value> {
    r.pointer(&format!("/fields/{k}/value"))
}

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
    let http = client.http().clone();
    let params = client.service_params();

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::notes::sync_cloudkit(&http, &root, params.clone(), &mut library, None).await?;

    let nz = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", params);
    let walk = nz.all_records("Notes", None).await?;

    let mut stored: std::collections::HashMap<String, quince_core::notes::Note> =
        std::collections::HashMap::new();
    for folder in [
        None,
        Some("TrashFolder-CloudKit"),
        Some("DefaultFolder-CloudKit"),
    ] {
        for n in library.notes_in(folder)? {
            stored.entry(n.guid.clone()).or_insert(n);
        }
    }

    let mut problems = 0;
    for r in walk
        .records
        .iter()
        .filter(|r| r.get("recordType").and_then(Value::as_str) == Some("Note"))
        .filter(|r| f(r, "Deleted").and_then(Value::as_i64) != Some(1))
    {
        let guid = r
            .pointer("/recordName")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let apple_title = f(r, "TitleEncrypted")
            .and_then(Value::as_str)
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();
        let apple_body = f(r, "TextDataEncrypted")
            .and_then(Value::as_str)
            .and_then(quince_icloud::ckdocument::text_from_base64);
        let apple_folder = r
            .pointer("/fields/Folders/value")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("recordName"))
            .and_then(Value::as_str)
            .or_else(|| {
                r.pointer("/fields/Folder/value/recordName")
                    .and_then(Value::as_str)
            });

        let Some(mine) = stored.get(&guid) else {
            println!(
                "MISSING  {guid}  {:?} — Apple has it, the library does not",
                apple_title.chars().take(30).collect::<String>()
            );
            problems += 1;
            continue;
        };

        let mut says = Vec::new();
        if mine.folder_guid.as_deref() != apple_folder {
            says.push(format!(
                "folder {:?} vs {:?}",
                mine.folder_guid, apple_folder
            ));
        }
        let body_len = |b: &Option<String>| b.as_deref().map(str::len);
        let mine_body = library.note_body(&guid)?;
        if body_len(&mine_body) != body_len(&apple_body) {
            says.push(format!(
                "body {:?} chars vs {:?}",
                body_len(&mine_body),
                body_len(&apple_body)
            ));
        }
        if mine.title != apple_title && !apple_title.is_empty() {
            says.push(format!("title {:?} vs {:?}", mine.title, apple_title));
        }
        if says.is_empty() {
            println!(
                "ok       {:<34} {guid}",
                mine.title.chars().take(32).collect::<String>()
            );
        } else {
            println!(
                "DIFFERS  {:<34} {}",
                mine.title.chars().take(32).collect::<String>(),
                says.join("; ")
            );
            problems += 1;
        }
    }
    // And the other direction: anything the library holds that the account does not, which
    // is how a deleted note lives on as a ghost and gets written back on the next save.
    let live: std::collections::HashSet<&str> = walk
        .records
        .iter()
        .filter(|r| r.get("recordType").and_then(Value::as_str) == Some("Note"))
        .filter(|r| f(r, "Deleted").and_then(Value::as_i64) != Some(1))
        .filter_map(|r| r.pointer("/recordName").and_then(Value::as_str))
        .collect();
    for guid in stored.keys() {
        if !live.contains(guid.as_str()) {
            println!("GHOST    {guid} — the library has it, the account does not");
            problems += 1;
        }
    }

    println!("\n{problems} note(s) where the library and the account disagree");
    Ok(())
}
