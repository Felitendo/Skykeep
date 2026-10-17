//! Live, throwaway list only: three reminders, the last moved to the front, the order read
//! back from Apple — then the list deleted.
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
    let (http, p) = (client.http().clone(), client.service_params());
    use quince_icloud::reminders as r;
    let list = r::create_list_cloudkit(&http, &root, p.clone(), "Quince order test").await?;
    let a = r::create_cloudkit(&http, &root, p.clone(), &list, "A").await?;
    let _b = r::create_cloudkit(&http, &root, p.clone(), &list, "B").await?;
    let c = r::create_cloudkit(&http, &root, p.clone(), &list, "C").await?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let order = |v: serde_json::Value| {
        v["fields"]["ReminderIDs"]["value"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    println!(
        "made:  {}",
        order(zc.lookup_record("Reminders", &list).await?.unwrap())
    );
    r::reorder_cloudkit(&http, &root, p.clone(), &list, &c, Some(&a)).await?;
    let after = zc.lookup_record("Reminders", &list).await?.unwrap();
    println!("moved: {}", order(after.clone()));
    let rtm: serde_json::Value = serde_json::from_str(
        after["fields"]["ResolutionTokenMap"]["value"]
            .as_str()
            .unwrap_or("{}"),
    )?;
    println!(
        "order token: {}",
        rtm["map"]["reminderIDsMergeableOrdering"]
    );
    r::delete_list_cloudkit(&http, &root, p, &list).await?;
    println!("deleted");
    Ok(())
}
