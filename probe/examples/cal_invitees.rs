//! Read-only: how many events this year have invitees, and the shape of one (statuses and
//! flags only, no names or addresses).
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
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let mut q = p.clone();
    for (k, v) in [
        ("clientVersion", "5.1"),
        ("requestID", "4"),
        ("lang", "en-us"),
        ("usertz", "Asia/Kolkata"),
        ("startDate", "2026-09-01"),
        ("endDate", "2026-12-31"),
    ] {
        q.push((k.into(), v.into()));
    }
    let v: serde_json::Value = http
        .get(format!("{root}/ca/startup"))
        .query(&q)
        .send()
        .await?
        .json()
        .await?;
    let events = v["Event"].as_array().cloned().unwrap_or_default();
    let with: Vec<_> = events
        .iter()
        .filter(|e| e["invitees"].as_array().is_some_and(|a| !a.is_empty()))
        .collect();
    println!(
        "{} events, {} with invitees; Invitee objects: {}",
        events.len(),
        with.len(),
        v["Invitee"].as_array().map_or(0, |a| a.len())
    );
    for i in v["Invitee"].as_array().into_iter().flatten().take(6) {
        println!(
            "  status={} isMe={} isOrganizer={} hasName={} hasEmail={}",
            i["inviteeStatus"],
            i["isMe"],
            i["isOrganizer"],
            i["commonName"].is_string(),
            i["email"].is_string()
        );
    }
    println!(
        "inbox items: {}",
        v["InboxItem"].as_array().map_or(0, |a| a.len())
    );
    Ok(())
}
