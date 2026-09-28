//! Throwaway: the real account through the real sync, as the Reminders screen would receive
//! it, written out so the interface can be looked at against real data.
use anyhow::{anyhow, Result};
use quince_core::reminders::SmartList;
use serde_json::json;

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
    let until: i64 = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("until"))?
        .parse()?;
    let lists = lib.reminder_lists()?;
    let mut by_list = serde_json::Map::new();
    let mut sections = serde_json::Map::new();
    for l in &lists {
        by_list.insert(l.guid.clone(), json!(lib.reminders_in(&l.guid, false)?));
        sections.insert(l.guid.clone(), json!(lib.reminder_sections(&l.guid)?));
    }
    let out = json!({
        "lists": lists,
        "groups": lib.reminder_groups()?,
        "counts": lib.reminder_counts(until)?,
        "smart": {
            "today": lib.reminders_smart(SmartList::Today, until)?,
            "scheduled": lib.reminders_smart(SmartList::Scheduled, until)?,
            "all": lib.reminders_smart(SmartList::All, until)?,
            "flagged": lib.reminders_smart(SmartList::Flagged, until)?,
        },
        "byList": by_list,
        "sections": sections,
    });
    println!("{out}");
    Ok(())
}
