//! Live, on a throwaway table only: rows added at the top and at the end, a column in the
//! middle, read back from Apple, and the new row and column typed in. Refuses any attachment one of his notes still uses
//! outside Recently Deleted (checked by the caller against the local library first).
use anyhow::{anyhow, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let id = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("attachment id"))?;
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
    let before = quince_icloud::table::cells_from_base64(
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone())
            .lookup_record("Notes", &id)
            .await?
            .ok_or_else(|| anyhow!("gone"))?["fields"]["MergeableDataEncrypted"]["value"]
            .as_str()
            .unwrap_or(""),
    )
    .ok_or_else(|| anyhow!("not a table"))?;
    println!("before: {before:?}");
    let top = quince_icloud::notes::add_table_row_cloudkit(&http, &root, p.clone(), &id, 0).await?;
    println!("row at top, Apple holds: {top:?}");
    let end = quince_icloud::notes::add_table_row_cloudkit(&http, &root, p.clone(), &id, top.len())
        .await?;
    println!("row at end, Apple holds: {end:?}");
    let typed = quince_icloud::notes::set_table_cell_cloudkit(
        &http,
        &root,
        p.clone(),
        &id,
        0,
        0,
        "Quince row test",
    )
    .await?;
    println!("typed in the new top row: {typed:?}");
    let col = quince_icloud::notes::add_table_line_cloudkit(&http, &root, p.clone(), &id, true, 1)
        .await?;
    println!("column at 1, Apple holds: {col:?}");
    let typed = quince_icloud::notes::set_table_cell_cloudkit(
        &http,
        &root,
        p.clone(),
        &id,
        0,
        1,
        "Quince column test",
    )
    .await?;
    println!("typed in the new column: {typed:?}");
    Ok(())
}
