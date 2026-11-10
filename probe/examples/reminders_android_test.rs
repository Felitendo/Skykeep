//! Test data for the Android Reminders, in a list of its own (never another):
//!
//!   cargo run -p quince-probe --example reminders_android_test -- create
//!       makes "Quince Android test" with "Test: due soon" (due in three minutes, for the alert)
//!       and "Test: plain"; prints the list's id
//!   cargo run -p quince-probe --example reminders_android_test -- peek
//!       what iCloud holds in that list now: each reminder, done or open, and its date
//!
//! Cleaned up with reminders_cleanup_tests, which deletes only lists named "Quince … test".
use anyhow::{anyhow, bail, Result};
use quince_icloud::reminders as r;

const LIST: &str = "Quince Android test";

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
    // Apple's Reminders read whole into a library of the probe's own, thrown away after.
    let scratch = dir.join("android-test-scratch");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;
    let mut library = quince_core::Library::open(&scratch)?;
    let fetched = r::fetch_cloudkit(&http, &root, p.clone(), None).await?;
    r::apply_cloudkit(&mut library, fetched)?;
    let list = library
        .reminder_lists()?
        .into_iter()
        .find(|l| l.title == LIST);
    match std::env::args().nth(1).as_deref() {
        Some("create") => {
            if list.is_some() {
                bail!("{LIST:?} is there already: clean it up first");
            }
            let list = r::create_list_cloudkit(&http, &root, p.clone(), LIST).await?;
            let soon = r::create_cloudkit(&http, &root, p.clone(), &list, "Test: due soon").await?;
            r::create_cloudkit(&http, &root, p.clone(), &list, "Test: plain").await?;
            let due = jiff::Timestamp::now().as_second() + 180;
            let made = quince_core::reminders::Reminder {
                guid: soon,
                list_guid: list.clone(),
                title: "Test: due soon".into(),
                ..Default::default()
            };
            let edit = r::ReminderEdit {
                due_at: Some(Some(due)),
                all_day: Some(false),
                ..Default::default()
            };
            r::edit_cloudkit(&http, &root, p, &made, &edit).await?;
            println!(
                "list {list}; \"Test: due soon\" due at {}",
                jiff::Timestamp::from_second(due)?
            );
        }
        Some("peek") => {
            let Some(list) = list else {
                bail!("no {LIST:?}")
            };
            for x in library.reminders_in(&list.guid, true)? {
                let due = x
                    .due_at
                    .map(|d| jiff::Timestamp::from_second(d).map(|t| t.to_string()));
                println!(
                    "{} | {} | due {:?}",
                    if x.completed_at.is_some() {
                        "done"
                    } else {
                        "open"
                    },
                    x.title,
                    due.transpose()?
                );
            }
        }
        _ => bail!("create or peek"),
    }
    drop(library);
    std::fs::remove_dir_all(&scratch)?;
    Ok(())
}
