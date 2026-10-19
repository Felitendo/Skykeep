//! Live: a throwaway calendar made, renamed, recoloured and deleted, each step checked against
//! Apple's own record. Only the calendar this makes is ever written to; one of his calendars is
//! only read, to see the shape `ca/collectiondetail` answers with.
use anyhow::{anyhow, Result};
use quince_icloud::calendar::CalendarClient;

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
        .ok_or_else(|| anyhow!("no calendar service"))?;
    let http = client.http().clone();
    let cal = CalendarClient::new(&http, &root, client.service_params());
    let snap = cal.startup("2026-10-01", "2026-10-31", "UTC").await?;
    let mine = snap.calendars.clone();
    println!(
        "{} calendars: {:?}",
        mine.len(),
        mine.iter()
            .map(|c| (&c.title, &c.colour))
            .collect::<Vec<_>>()
    );

    // Read-only: the shape of one of his.
    if let Some(c) = mine.iter().find(|c| c.writable) {
        let detail = cal.collection_detail(&c.guid).await?;
        let keys: Vec<String> = detail
            .as_ref()
            .and_then(|d| d.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        println!("collectiondetail keys of {:?}: {keys:?}", c.title);
        println!(
            "  etag present: {}",
            detail.as_ref().is_some_and(|d| d.get("etag").is_some())
        );
    }

    // The throwaway, made, changed and removed.
    let name = "Quince calendar test";
    if let Some(left) = mine.iter().find(|c| c.title.starts_with(name)) {
        println!("a test calendar was left from before; removing it first");
        cal.delete_calendar(left).await?;
    }
    let guid = cal.create_calendar(name, &mine).await?;
    let made = cal
        .collection_detail(&guid)
        .await?
        .ok_or_else(|| anyhow!("not made"))?;
    println!(
        "made {guid}: title {:?} color {:?}",
        made["title"], made["color"]
    );
    let dup = cal
        .create_calendar(
            name,
            &cal.startup("2026-10-01", "2026-10-31", "UTC")
                .await?
                .calendars,
        )
        .await;
    println!("same name again refused: {}", dup.is_err());

    let now = cal
        .startup("2026-10-01", "2026-10-31", "UTC")
        .await?
        .calendars;
    cal.update_calendar(&guid, Some("Quince calendar test renamed"), None, &now)
        .await?;
    println!(
        "renamed: {:?}",
        cal.collection_detail(&guid)
            .await?
            .map(|c| c["title"].clone())
    );
    cal.update_calendar(&guid, None, Some("#a2845e"), &now)
        .await?;
    println!(
        "recoloured: {:?}",
        cal.collection_detail(&guid)
            .await?
            .map(|c| (c["color"].clone(), c["symbolicColor"].clone()))
    );

    let made_cal = cal
        .startup("2026-10-01", "2026-10-31", "UTC")
        .await?
        .calendars
        .into_iter()
        .find(|c| c.guid == guid)
        .ok_or_else(|| anyhow!("not listed"))?;
    cal.delete_calendar(&made_cal).await?;
    let still = cal
        .startup("2026-10-01", "2026-10-31", "UTC")
        .await?
        .calendars
        .iter()
        .any(|c| c.guid == guid);
    println!("deleted; still listed: {still}");
    Ok(())
}
