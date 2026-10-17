//! Live, read-only: the styled runs of the first notes that have any, as Quince parses them.
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
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &root,
        "com.apple.notes",
        client.service_params(),
    );
    let walk = zc.all_records("Notes", None).await?;
    let mut shown = 0;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let Some(doc) = r["fields"]["TextDataEncrypted"]["value"].as_str() else {
            continue;
        };
        let runs = quince_icloud::ckdocument::runs_from_base64(doc);
        if !runs
            .iter()
            .any(|x| x.style.is_some() || x.bold || x.checked.is_some())
        {
            continue;
        }
        let text = quince_icloud::ckdocument::text_from_base64(doc).unwrap_or_default();
        println!(
            "== {} ({} runs)",
            text.lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(40)
                .collect::<String>(),
            runs.len()
        );
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut at = 0usize;
        for run in runs.iter().take(14) {
            let end = (at + run.length as usize).min(units.len());
            let piece = String::from_utf16_lossy(&units[at..end]).replace('\n', "⏎");
            println!(
                "  style={:?} indent={} checked={:?} b={} i={} u={} s={} | {}",
                run.style,
                run.indent,
                run.checked,
                run.bold,
                run.italic,
                run.underline,
                run.strike,
                piece.chars().take(50).collect::<String>()
            );
            at = end;
        }
        shown += 1;
        if shown == 3 {
            break;
        }
    }
    Ok(())
}
