//! Live, on a throwaway table only: a row and a column deleted, then a row added with the
//! corrected writer, each read back from Apple.
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
    for (column, at) in [(false, 2usize), (true, 2)] {
        match quince_icloud::notes::remove_table_line_cloudkit(
            &http,
            &root,
            p.clone(),
            &id,
            column,
            at,
        )
        .await
        {
            Ok(cells) => println!(
                "{} {at} deleted, Apple holds: {cells:?}",
                if column { "column" } else { "row" }
            ),
            Err(e) => println!(
                "{} {at} delete refused: {e}",
                if column { "column" } else { "row" }
            ),
        }
    }
    match quince_icloud::notes::add_table_row_cloudkit(&http, &root, p.clone(), &id, 1).await {
        Ok(cells) => println!("row added at 1, Apple holds: {cells:?}"),
        Err(e) => println!("row add refused: {e}"),
    }
    Ok(())
}
