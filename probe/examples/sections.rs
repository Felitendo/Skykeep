//! Throwaway: sync reminders the way the app does and show what landed in each section.
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
    let http = client.http().clone();
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut lib = quince_core::Library::open(&store)?;
    quince_icloud::reminders::sync_cloudkit(&http, &root, client.service_params(), &mut lib, None)
        .await?;
    for list in lib.reminder_lists()? {
        let sections = lib.reminder_sections(&list.guid)?;
        if sections.is_empty() {
            continue;
        }
        let items = lib.reminders_in(&list.guid, false)?;
        println!("{}:", list.title);
        for s in &sections {
            let under: Vec<_> = items
                .iter()
                .filter(|r| r.section_guid.as_deref() == Some(s.guid.as_str()))
                .map(|r| r.title.clone())
                .collect();
            println!("  [{}] {} reminder(s): {:?}", s.title, under.len(), under);
        }
        println!(
            "  (outside any section) {}",
            items.iter().filter(|r| r.section_guid.is_none()).count()
        );
    }
    Ok(())
}
