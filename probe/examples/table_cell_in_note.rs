//! Live, with his say-so, on a table in a note he named: one edit through the app's own write
//! (the table's and the note's modification dates stamped too), read back from Apple — for him
//! to look for on his iPhone. Undone the same way.
//! `table_cell_in_note <note title> <attachment id> cell <row> <column> <text>`
//! `table_cell_in_note <note title> <attachment id> +row|-row|+column|-column <at>`
use anyhow::{anyhow, Result};
use quince_icloud::notes::{
    add_table_line_cloudkit, remove_table_line_cloudkit, set_table_cell_cloudkit,
};
#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let [_, title, id, op, rest @ ..] = a.as_slice() else {
        return Err(anyhow!(
            "table_cell_in_note <note title> <attachment id> <op> …"
        ));
    };
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
    // Only a table that note places, once: never one anywhere else.
    let placed = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Note")
        .filter(|n| {
            n["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .and_then(quince_icloud::ckdocument::text_from_base64)
                .is_some_and(|t| t.lines().next().map(str::trim) == Some(title.as_str()))
        })
        .map(|n| {
            n["fields"]["TextDataEncrypted"]["value"]
                .as_str()
                .map(quince_icloud::ckdocument::runs_from_base64)
                .unwrap_or_default()
                .iter()
                .filter(|r| r.attachment.as_deref() == Some(id.as_str()))
                .count()
        })
        .collect::<Vec<_>>();
    if placed.iter().filter(|&&n| n > 0).count() != 1 || placed.iter().sum::<usize>() != 1 {
        return Err(anyhow!(
            "that table is not placed once in one note titled {title:?} ({placed:?})"
        ));
    }
    let cells = match (op.as_str(), rest) {
        ("cell", [row, col, text]) => {
            set_table_cell_cloudkit(&http, &root, p, id, row.parse()?, col.parse()?, text).await?
        }
        ("+row", [at]) => add_table_line_cloudkit(&http, &root, p, id, false, at.parse()?).await?,
        ("+column", [at]) => {
            add_table_line_cloudkit(&http, &root, p, id, true, at.parse()?).await?
        }
        ("-row", [at]) => {
            remove_table_line_cloudkit(&http, &root, p, id, false, at.parse()?).await?
        }
        ("-column", [at]) => {
            remove_table_line_cloudkit(&http, &root, p, id, true, at.parse()?).await?
        }
        _ => return Err(anyhow!("unknown edit")),
    };
    let shape: Vec<Vec<usize>> = cells
        .iter()
        .map(|r| r.iter().map(String::len).collect())
        .collect();
    println!(
        "read back: {} rows x {} columns; cell lengths {shape:?}",
        cells.len(),
        cells.first().map_or(0, Vec::len)
    );
    Ok(())
}
