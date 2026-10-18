//! Read-only: each table the account holds, a typed cell changed in memory only — never
//! written — to show the rewrite is exact and reads back as meant.
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
    for a in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Attachment")
    {
        let Some(v) = a["fields"]["MergeableDataEncrypted"]["value"].as_str() else {
            continue;
        };
        let Some(cells) = quince_icloud::table::cells_from_base64(v) else {
            continue;
        };
        let typed = cells
            .iter()
            .enumerate()
            .find_map(|(r, row)| row.iter().position(|c| !c.is_empty()).map(|c| (r, c)));
        match typed {
            None => println!(
                "{}x{} table, nothing typed in",
                cells.len(),
                cells.first().map_or(0, Vec::len)
            ),
            Some((r, c)) => {
                let edited = quince_icloud::table::with_cell_in_base64(v, r, c, "Quince edit test");
                let back = edited
                    .as_deref()
                    .and_then(quince_icloud::table::cells_from_base64);
                println!(
                    "{}x{} table, cell ({r},{c}): rewrite {}; reads back {:?}; others unchanged {}",
                    cells.len(),
                    cells.first().map_or(0, Vec::len),
                    if edited.is_some() { "exact" } else { "REFUSED" },
                    back.as_ref().map(|b| &b[r][c]),
                    back.as_ref()
                        .is_some_and(|b| b.iter().enumerate().all(|(i, row)| row
                            .iter()
                            .enumerate()
                            .all(|(j, x)| (i, j) == (r, c) || *x == cells[i][j])))
                );
            }
        }
    }
    Ok(())
}
