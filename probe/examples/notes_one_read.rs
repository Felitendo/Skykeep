//! Read-only: one note's text and runs as Apple holds them, by title.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let want = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("pass a title"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let body = r["fields"]["TextDataEncrypted"]["value"]
            .as_str()
            .unwrap_or("");
        let text = quince_icloud::ckdocument::text_from_base64(body).unwrap_or_default();
        if !text.starts_with(&want) {
            continue;
        }
        println!(
            "{} tag={} modified={}",
            r["recordName"], r["recordChangeTag"], r["fields"]["ModificationDate"]["value"]
        );
        println!("text {:?}", text);
        let runs = quince_icloud::ckdocument::runs_from_base64(body);
        println!(
            "runs {:?}",
            runs.iter()
                .map(|x| (x.length, x.style, x.checked))
                .collect::<Vec<_>>()
        );
        println!(
            "sum {} vs utf16 {}",
            runs.iter().map(|r| r.length).sum::<u64>(),
            text.encode_utf16().count()
        );
        for line in quince_icloud::ckdocument::debug_merge_graph(
            &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.trim())
                .unwrap_or_default(),
        ) {
            println!("  {line}");
        }
        println!(
            "why {}",
            quince_icloud::ckdocument::debug_rewritable(
                &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.trim())
                    .unwrap_or_default()
            )
        );
        println!(
            "rewritable {}",
            quince_icloud::ckdocument::is_rewritable(
                &base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body.trim())
                    .unwrap_or_default()
            )
        );
    }
    Ok(())
}
