//! Live, throwaway lists only: a reminder with a subtask moved to another list, the subtask
//! moved after it as the app now does, both read back from Apple — then both lists deleted.
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
    use quince_icloud::reminders as r;
    let a = r::create_list_cloudkit(&http, &root, p.clone(), "Quince move test A").await?;
    let b = r::create_list_cloudkit(&http, &root, p.clone(), "Quince move test B").await?;
    let parent = r::create_cloudkit(&http, &root, p.clone(), &a, "Parent test").await?;
    let child = r::create_cloudkit(&http, &root, p.clone(), &a, "Subtask test").await?;
    let child_r = quince_core::reminders::Reminder {
        guid: child.clone(),
        list_guid: a.clone(),
        title: "Subtask test".into(),
        ..Default::default()
    };
    r::edit_cloudkit(
        &http,
        &root,
        p.clone(),
        &child_r,
        &r::ReminderEdit {
            parent_guid: Some(Some(parent.clone())),
            ..Default::default()
        },
    )
    .await?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let fresh = |rec: Option<serde_json::Value>| {
        rec.and_then(|r| r["recordChangeTag"].as_str().map(str::to_string))
    };
    let parent_r = quince_core::reminders::Reminder {
        guid: parent.clone(),
        list_guid: a.clone(),
        title: "Parent test".into(),
        etag: fresh(zc.lookup_record("Reminders", &parent).await?),
        ..Default::default()
    };
    let to_b = r::ReminderEdit {
        list_guid: Some(b.clone()),
        ..Default::default()
    };
    r::edit_cloudkit(&http, &root, p.clone(), &parent_r, &to_b).await?;
    let child_r = quince_core::reminders::Reminder {
        etag: fresh(zc.lookup_record("Reminders", &child).await?),
        ..child_r
    };
    r::edit_cloudkit(&http, &root, p.clone(), &child_r, &to_b).await?;
    let list_of = |rec: &serde_json::Value| {
        rec["fields"]["List"]["value"]["recordName"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    let pr = zc
        .lookup_record("Reminders", &parent)
        .await?
        .ok_or_else(|| anyhow!("parent gone"))?;
    let cr = zc
        .lookup_record("Reminders", &child)
        .await?
        .ok_or_else(|| anyhow!("child gone"))?;
    println!(
        "parent in B: {}; subtask in B: {}; subtask still under parent: {}",
        list_of(&pr) == b,
        list_of(&cr) == b,
        cr["fields"]["ParentReminder"]["value"]["recordName"].as_str() == Some(parent.as_str())
    );
    r::delete_list_cloudkit(&http, &root, p.clone(), &a).await?;
    r::delete_list_cloudkit(&http, &root, p, &b).await?;
    println!("test lists deleted");
    Ok(())
}
