//! Read-only: every reminder and calendar event whose title mentions "test", with what keeps
//! it alive — deleted or not, its list and whether that list is deleted, due date, repeat.
use anyhow::{anyhow, Result};
use serde_json::Value;
#[tokio::main]
async fn main() -> Result<()> {
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
    let text = |v: &Value| {
        v.as_str()
            .and_then(quince_icloud::ckdocument::text_from_base64)
            .unwrap_or_default()
    };
    let lists: std::collections::HashMap<String, (String, i64)> = walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "List")
        .map(|r| {
            let name = r["fields"]["Name"]["value"]
                .as_str()
                .map(|s| {
                    String::from_utf8(
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
                            .unwrap_or_default(),
                    )
                    .unwrap_or_default()
                })
                .unwrap_or_default();
            (
                r["recordName"].as_str().unwrap_or("").to_string(),
                (name, r["fields"]["Deleted"]["value"].as_i64().unwrap_or(0)),
            )
        })
        .collect();
    for r in walk
        .records
        .iter()
        .filter(|r| r["recordType"] == "Reminder")
    {
        let f = &r["fields"];
        let title = text(&f["TitleDocument"]["value"]);
        if !title.to_lowercase().contains("test") {
            continue;
        }
        let list = f["List"]["value"]["recordName"].as_str().unwrap_or("");
        println!(
            "REMINDER {:?} id={} deleted={} completed={} due={} allday={} recur={} list={:?}",
            title,
            r["recordName"],
            f["Deleted"]["value"],
            f["Completed"]["value"],
            f["DueDate"]["value"],
            f["AllDay"]["value"],
            f["RecurrenceRuleIDs"]["value"]
                .to_string()
                .chars()
                .take(60)
                .collect::<String>(),
            lists.get(list)
        );
    }
    let cal_root = client
        .session()
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar"))?;
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &cal_root, p.clone());
    for (a, b) in [
        ("2026-09-01", "2026-09-30"),
        ("2026-10-01", "2026-10-31"),
        ("2026-11-01", "2026-11-30"),
    ] {
        match cal.startup(a, b, "Asia/Kolkata").await {
            Ok(snap) => {
                for e in snap
                    .events
                    .iter()
                    .filter(|e| e.title.to_lowercase().contains("test"))
                {
                    println!(
                        "EVENT {:?} guid={} cal={} starts={} allday={}",
                        e.title, e.guid, e.calendar_guid, e.starts_at, e.all_day
                    );
                }
            }
            Err(e) => println!("{a}: {e}"),
        }
    }
    Ok(())
}
