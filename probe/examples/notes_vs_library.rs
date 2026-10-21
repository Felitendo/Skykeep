//! Read-only: every note Apple holds against the app's library — which differ, and how (text,
//! or missing on one side). Prints titles and lengths, never note text beyond the title.
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
    let walk = zc.all_records("Notes", None).await?;
    let db = rusqlite::Connection::open_with_flags(
        std::path::PathBuf::from(std::env::var("HOME")?)
            .join(".local/share/io.github.yashwanthkamireddi.Skykeep/library.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut apple = std::collections::HashSet::new();
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let deleted = r["fields"]["Deleted"]["value"].as_i64() == Some(1)
            || r["fields"]["MarkedForDeletion"]["value"].as_i64() == Some(1);
        if deleted {
            continue;
        }
        let id = r["recordName"].as_str().unwrap_or_default().to_string();
        apple.insert(id.clone());
        let text = r["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();
        let modified = r["fields"]["ModificationDate"]["value"]
            .as_i64()
            .unwrap_or(0)
            / 1000;
        let mine: Option<(String, Option<String>, Option<i64>)> = db
            .query_row(
                "SELECT title, body, modified_at FROM notes WHERE guid = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let title: String = text.lines().next().unwrap_or("").chars().take(30).collect();
        match mine {
            None => println!("MISSING here: {title:?} (Apple modified {modified})"),
            Some((_, body, at)) => {
                let body = body.unwrap_or_default();
                if body.trim_end() != text.trim_end() {
                    println!(
                        "DIFFERS: {title:?} — Apple {} chars @ {modified}, here {} chars @ {}",
                        text.chars().count(),
                        body.chars().count(),
                        at.unwrap_or(0)
                    );
                }
            }
        }
    }
    let mut stmt = db.prepare("SELECT guid, title FROM notes")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (guid, title) = row?;
        if !apple.contains(&guid) {
            println!(
                "HERE ONLY: {:?}",
                title.chars().take(30).collect::<String>()
            );
        }
    }
    println!("compared {} notes", apple.len());
    Ok(())
}
