//! Live, on an orphaned table only (one no note's text places, so nothing anyone sees): a
//! typed cell changed through the app's own write, read back from Apple, then put back as it
//! was and read back again.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("pass the orphaned attachment id"))?;
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
    // Refuse anything a note actually shows.
    let walk = zc.all_records("Notes", None).await?;
    let placed = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Note")
        .any(|n| {
            n["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .is_some_and(|b| {
                    quince_icloud::ckdocument::runs_from_base64(b)
                        .iter()
                        .any(|r| r.attachment.as_deref() == Some(id.as_str()))
                })
        });
    if placed {
        return Err(anyhow!("that table is in a note; not touching it"));
    }
    let before = zc
        .lookup_record("Notes", &id)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let raw_before = before["fields"]["MergeableDataEncrypted"]["value"]
        .as_str()
        .unwrap()
        .to_string();
    let cells = quince_icloud::table::cells_from_base64(&raw_before)
        .ok_or_else(|| anyhow!("unreadable"))?;
    let was = cells[0][0].clone();
    let after = quince_icloud::notes::set_table_cell_cloudkit(
        &http,
        &root,
        p.clone(),
        &id,
        0,
        0,
        "Quince edit test",
    )
    .await?;
    let reread = zc.lookup_record("Notes", &id).await?.and_then(|r| {
        r["fields"]["MergeableDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::table::cells_from_base64)
    });
    println!(
        "written: Apple answered {:?}; read back {:?}; others unchanged {}",
        after[0][0],
        reread.as_ref().map(|c| c[0][0].clone()),
        reread
            .as_ref()
            .is_some_and(|c| c.iter().enumerate().all(|(i, row)| row
                .iter()
                .enumerate()
                .all(|(j, x)| (i, j) == (0, 0) || *x == cells[i][j])))
    );
    let restored =
        quince_icloud::notes::set_table_cell_cloudkit(&http, &root, p.clone(), &id, 0, 0, &was)
            .await?;
    println!(
        "restored: {:?} (was {was:?}); whole table as before {}",
        restored[0][0],
        restored == cells
    );
    Ok(())
}
