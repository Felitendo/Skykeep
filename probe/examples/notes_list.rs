//! Read-only: every note Apple holds — its first line, folder, whether deleted, how many
//! attachment references it has — for checking the library against the account.
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p);
    let walk = zc.all_records("Notes", None).await?;
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for r in &walk.records {
        *kinds
            .entry(r["recordType"].as_str().unwrap_or("?").to_string())
            .or_default() += 1;
    }
    println!("records: {kinds:?}");
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let f = &r["fields"];
        let text = f["TextDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();
        let atts = f["TextDataEncrypted"]["value"]
            .as_str()
            .map(quince_icloud::ckdocument::runs_from_base64)
            .unwrap_or_default()
            .iter()
            .filter(|x| x.attachment.is_some())
            .count();
        let first: String = text.lines().next().unwrap_or("").chars().take(30).collect();
        let ids: Vec<String> = f["TextDataEncrypted"]["value"]
            .as_str()
            .map(quince_icloud::ckdocument::runs_from_base64)
            .unwrap_or_default()
            .iter()
            .filter_map(|x| x.attachment.clone())
            .map(|a| {
                if std::env::var("FULL").is_ok() {
                    a
                } else {
                    a.chars().take(4).collect()
                }
            })
            .collect();
        println!(
            "{} | {:30} | deleted {} | folder {} | atts {} | created {} | {:?} | modified {}",
            &r["recordName"].as_str().unwrap_or("")[..8],
            first,
            f["Deleted"]["value"],
            f["Folder"]["value"]["recordName"]
                .as_str()
                .unwrap_or("-")
                .chars()
                .take(8)
                .collect::<String>(),
            atts,
            f["CreationDate"]["value"],
            ids,
            f["ModificationDate"]["value"]
        );
    }
    Ok(())
}
