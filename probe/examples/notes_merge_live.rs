//! Live, throwaway note only: another device's edit lands between our read and our save; the
//! save keeps both. Then the same place edited on both sides is refused, writing nothing.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? { return Err(anyhow!("no session")); }
    let root = client.session().service_url("ckdatabasews").ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let original = "Quince merge test\nmilk\neggs";
    let guid = quince_icloud::notes::create_cloudkit(&http, &root, p.clone(), None, original, None).await?;
    let note = quince_core::notes::Note { guid: guid.clone(), folder_guid: None, title: "Quince merge test".into(), snippet: None, body: None, locked: false, pinned: false, created_at: None, modified_at: None, editable: true, etag: None, runs: None };
    // "The phone": changes the title.
    quince_icloud::notes::edit_cloudkit(&http, &root, p.clone(), &note, "Quince merge test (phone)\nmilk\neggs").await?;
    // Quince, from the original: adds a line.
    let saved = quince_icloud::notes::edit_cloudkit_from(&http, &root, p.clone(), &note, Some(original), "Quince merge test\nmilk\neggs\nbread").await?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc.lookup_record("Notes", &guid).await?.unwrap();
    println!("saved:  {saved:?}");
    println!("apple:  {:?}", quince_icloud::ckdocument::text_from_base64(rec["fields"]["TextDataEncrypted"]["value"].as_str().unwrap_or("")));
    // Same place, both sides: refused.
    let refused = quince_icloud::notes::edit_cloudkit_from(&http, &root, p.clone(), &note, Some(original), "Quince merge test\nmilk\neggs and ham").await;
    println!("same-place edit: {}", match refused { Ok(t) => format!("WROTE {t:?}"), Err(e) => format!("refused — {e}") });
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    println!("moved to Recently Deleted");
    Ok(())
}
