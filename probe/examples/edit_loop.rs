//! Throwaway: type into a note the way a person does, and check Apple holds exactly that.
//!
//! The failure this is for: a note on the account came back with everything typed into it
//! still there, over and over, because each save left behind a record of text that was no
//! longer in it.
use anyhow::{anyhow, Result};
use serde_json::Value;

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
    let nz =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", params.clone());

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::notes::sync_cloudkit(&http, &root, params.clone(), &mut library, None).await?;
    let template = library.state(quince_icloud::notes::TEMPLATE_KEY)?;

    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        params.clone(),
        Some("DefaultFolder-CloudKit"),
        "edit test",
        template.as_deref(),
    )
    .await?;
    println!("created {guid}");

    // What someone typing actually produces: a growing body, saved after each pause.
    let steps = [
        "edit test\n1",
        "edit test\n1\n2",
        "edit test\n1\n2\n3",
        "edit test\n1\n2\n3\n4",
        "edit test\n1\n2\n3\n4\n56",
        // And a deletion, which is where an append-only mistake shows up loudest.
        "edit test\n1\n2",
        "short",
    ];

    let mut failures = 0;
    for (n, want) in steps.iter().enumerate() {
        let note = library_note(&http, &root, &params, &guid).await?;
        quince_icloud::notes::edit_cloudkit(&http, &root, params.clone(), &note, want).await?;

        // Read it off Apple, not off the reply.
        let r = nz
            .lookup_record("Notes", &guid)
            .await?
            .ok_or_else(|| anyhow!("gone"))?;
        let doc = r
            .pointer("/fields/TextDataEncrypted/value")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("no body"))?;
        let got = quince_icloud::ckdocument::text_from_base64(doc).unwrap_or_default();
        let runs = {
            use base64::{engine::general_purpose::STANDARD as B64, Engine};
            let raw = B64.decode(doc).unwrap_or_default();
            quince_icloud::ckdocument::debug_attribute_lengths(&raw)
        };
        let units: u64 = want.encode_utf16().count() as u64;
        let covered: u64 = runs.iter().sum();
        let ok = got == *want && covered == units;
        if !ok {
            failures += 1;
        }
        println!(
            "  {} step {n}: {:?}   runs cover {covered} of {units}",
            if ok { "PASS" } else { "FAIL" },
            got.chars().take(28).collect::<String>()
        );
    }

    let note = library_note(&http, &root, &params, &guid).await?;
    quince_icloud::notes::delete_cloudkit(&http, &root, params.clone(), &note).await?;
    println!("\ncleaned up");
    if failures > 0 {
        return Err(anyhow!("{failures} step(s) did not hold"));
    }
    println!("every edit landed as exactly what was typed");
    Ok(())
}

/// The note as the app would have it: synced, then read from the library.
async fn library_note(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    guid: &str,
) -> Result<quince_core::notes::Note> {
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::notes::sync_cloudkit(http, root, params.to_vec(), &mut library, None).await?;
    library
        .notes_in(None)?
        .into_iter()
        .find(|n| n.guid == guid)
        .or_else(|| {
            library
                .notes_in(Some("TrashFolder-CloudKit"))
                .ok()?
                .into_iter()
                .find(|n| n.guid == guid)
        })
        .ok_or_else(|| anyhow!("the note is not in the library"))
}
