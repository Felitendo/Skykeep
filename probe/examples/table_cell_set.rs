//! Live: set one cell of an orphaned table (refused for any table a note places).
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (id, row, col, text) = (
        a[1].clone(),
        a[2].parse::<usize>()?,
        a[3].parse::<usize>()?,
        a[4].clone(),
    );
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
    if walk
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
        })
    {
        return Err(anyhow!("that table is in a note; not touching it"));
    }
    let cells =
        quince_icloud::notes::set_table_cell_cloudkit(&http, &root, p, &id, row, col, &text)
            .await?;
    println!("{:?}", cells.into_iter().take(3).collect::<Vec<_>>());
    Ok(())
}
