//! Read-only: one note's tables — each attachment's cells as Quince reads them, its record's
//! fields (encrypted ones as length and hash), and the raw document saved for diffing.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let note = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("note record name"))?;
    let out = std::env::args().nth(2).ok_or_else(|| anyhow!("dump dir"))?;
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
    for r in walk.records.iter().filter(|r| {
        r["recordType"] == "Attachment"
            && r["fields"]["Note"]["value"]["recordName"] == note.as_str()
    }) {
        let name = r["recordName"].as_str().unwrap_or("x");
        println!("== attachment {name} tag={}", r["recordChangeTag"]);
        for (k, v) in r["fields"].as_object().into_iter().flatten() {
            let val = v["value"].to_string();
            println!(
                "   {k}: {}",
                if val.len() > 90 {
                    format!("<{} chars>", val.len())
                } else {
                    val
                }
            );
        }
        if let Some(m) = r["fields"]["MergeableDataEncrypted"]["value"].as_str() {
            if let Some(raw) = quince_icloud::ckdocument::debug_inflate(m) {
                std::fs::write(format!("{out}/{name}.bin"), &raw).ok();
            }
            match quince_icloud::table::cells_from_base64(m) {
                Some(cells) => {
                    for row in cells {
                        println!("   | {}", row.join(" | "));
                    }
                }
                None => println!("   (cells not readable)"),
            }
        }
    }
    Ok(())
}
