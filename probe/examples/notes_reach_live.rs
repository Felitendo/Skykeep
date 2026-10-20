//! Live, throwaway note only: how fast a note written at Apple — as by another device — reaches
//! the running app's library. Made, then the library (read-only) watched for it; then sent to
//! Recently Deleted, and watched for that too. Pass the record name to notes_delete_now after.
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

fn row(db: &rusqlite::Connection, guid: &str) -> Option<String> {
    db.query_row(
        "SELECT coalesce(folder_guid, '') FROM notes WHERE guid = ?1",
        [guid],
        |r| r.get(0),
    )
    .ok()
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
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let db = rusqlite::Connection::open_with_flags(
        std::path::PathBuf::from(std::env::var("HOME")?)
            .join(".local/share/dev.quince.client/library.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let title = format!("Quince mount test {}", std::process::id());
    let t = Instant::now();
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        p.clone(),
        None,
        &format!("{title}\nsafe to delete"),
        None,
    )
    .await?;
    println!("written at Apple in {} ms", t.elapsed().as_millis());
    let mut seen = None;
    while t.elapsed() < Duration::from_secs(90) {
        if row(&db, &guid).is_some() {
            seen = Some(t.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("in the app's library after: {seen:?}");
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title,
        snippet: None,
        body: None,
        locked: false,
        pinned: false,
        created_at: None,
        modified_at: None,
        editable: true,
        etag: rec["recordChangeTag"].as_str().map(String::from),
        runs: None,
    };
    let before = row(&db, &guid);
    let t = Instant::now();
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    let mut moved = None;
    while t.elapsed() < Duration::from_secs(90) {
        if row(&db, &guid) != before {
            moved = Some(t.elapsed());
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("its deletion in the app's library after: {moved:?}");
    println!("RECORD {guid}");
    Ok(())
}
