//! Reminders left alive inside deleted lists — invisible in Reminders, still shown by the
//! phone's Calendar. Lists them; with `--delete`, marks deleted only those whose title this
//! probe's own tests use ("Daily test", "All-day test", "A", "B", "C"). Nothing else.
use anyhow::{anyhow, Result};
use serde_json::Value;
#[tokio::main]
async fn main() -> Result<()> {
    let delete = std::env::args().any(|a| a == "--delete");
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, "com.apple.reminders", p.clone());
    let walk = zc.all_records("Reminders", None).await?;
    let dead: std::collections::HashSet<&str> = walk
        .records
        .iter()
        .filter(|r| {
            r["recordType"] == "List" && r["fields"]["Deleted"]["value"].as_i64() == Some(1)
        })
        .filter_map(|r| r["recordName"].as_str())
        .collect();
    let ours = ["Daily test", "All-day test", "A", "B", "C"];
    let mut n = 0;
    for r in walk.records.iter().filter(|r| {
        r["recordType"] == "Reminder" && r["fields"]["Deleted"]["value"].as_i64() != Some(1)
    }) {
        let list = r["fields"]["List"]["value"]["recordName"]
            .as_str()
            .unwrap_or("");
        if !dead.contains(list) {
            continue;
        }
        let title = r["fields"]["TitleDocument"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default();
        let name = r["recordName"].as_str().unwrap_or("");
        let mine = ours.contains(&title.as_str());
        println!(
            "{} {:?} in deleted list {list} — {}",
            name,
            title,
            if mine { "ours" } else { "NOT OURS, left alone" }
        );
        if delete && mine {
            zc.update_record_signed(
                "Reminders",
                name,
                "Reminder",
                None,
                serde_json::json!({
                    "Deleted": { "type": "NUMBER_INT64", "value": 1 },
                }),
            )
            .await?;
            n += 1;
        }
    }
    let _: Option<Value> = None;
    println!("{n} marked deleted");
    Ok(())
}
