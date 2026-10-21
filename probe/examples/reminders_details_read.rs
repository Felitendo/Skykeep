//! Live, READ-ONLY: sync Reminders into a throwaway in-memory library and print what the
//! detail readers found — counts and kinds only, never titles or tag names.
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
    let mut lib = quince_core::Library::in_memory()?;
    let (lists, reminders, _) = quince_icloud::reminders::sync_cloudkit(
        client.http(),
        &root,
        client.service_params(),
        &mut lib,
        None,
    )
    .await?;
    println!("lists {lists}, reminders {reminders}");
    println!("tags on account: {}", lib.reminder_tags()?.len());
    let all: Vec<_> = lib
        .reminder_lists()?
        .into_iter()
        .flat_map(|l| lib.reminders_in(&l.guid, true).unwrap_or_default())
        .collect();
    let (mut alerts, mut urls, mut sectioned) = (0, 0, 0);
    let mut units = std::collections::BTreeMap::<(i64, i64), usize>::new();
    for r in &all {
        let d = lib.reminder_detail(&r.guid)?;
        alerts += usize::from(!d.early_alerts.is_empty());
        urls += usize::from(d.url.is_some());
        sectioned += usize::from(r.section_guid.is_some());
        for a in d.early_alerts {
            *units.entry((a.count, a.unit)).or_default() += 1;
        }
    }
    println!("reminders with an early alert: {alerts} {units:?}; with a link: {urls}; in a section: {sectioned}");
    println!(
        "early-alert account id known: {}",
        lib.early_alert_account()?.is_some()
    );
    let sections: usize = lib
        .reminder_lists()?
        .iter()
        .map(|l| lib.reminder_sections(&l.guid).map(|s| s.len()).unwrap_or(0))
        .sum();
    println!("live sections: {sections}");
    for s in lib.reminder_smart_lists()? {
        let filter = s
            .filter
            .as_deref()
            .map(quince_core::reminders::SmartFilter::decode);
        println!(
            "smart list kind={} pinned={} has_filter={} unknown={:?}",
            s.kind,
            s.pinned,
            filter.is_some(),
            filter.map(|f| f.unknown)
        );
    }
    Ok(())
}
