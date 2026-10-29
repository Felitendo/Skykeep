//! Live: Skykeep's Drive change reading (quince_icloud::drive_changes) end to end. Reads the
//! feed through to now (timed), makes a throwaway folder ("Quince mount test …") in the top
//! level, and shows which folders the watcher would list again; then deletes it and shows the
//! same. Clears it from Recently Deleted at the end. Ids and timings only.
use anyhow::{anyhow, Result};
use quince_icloud::drive_changes::{folders_to_list, page, Changed};
use std::time::{Duration, Instant};

const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

async fn read(
    http: &reqwest::Client,
    ck: &str,
    p: &[(String, String)],
    token: &mut Option<String>,
) -> Result<Vec<Changed>> {
    let mut all = Vec::new();
    loop {
        let pg = page(
            http,
            ck,
            p,
            quince_icloud::drive_changes::MAIN_ZONE,
            token.as_deref(),
        )
        .await?;
        all.extend(pg.changed);
        *token = pg.token.or(token.take());
        if !pg.more {
            return Ok(all);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let ck = s
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let t = Instant::now();
    let mut token = None;
    let n = read(&http, &ck, &p, &mut token).await?.len();
    println!("first read: {n} items in {:?}", t.elapsed());
    let name = format!("Quince mount test {}", std::process::id());
    let made = quince_icloud::drive::create_folder(&http, &drivews, p.clone(), ROOT, &name).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let t = Instant::now();
    let changed = read(&http, &ck, &p, &mut token).await?;
    let mine = made.drivewsid.clone();
    let folders = folders_to_list(&changed, |_| None, |id| id == ROOT);
    println!(
        "after make ({:?}): {} changed, made one named: {}, list again: {:?}",
        t.elapsed(),
        changed.len(),
        changed
            .iter()
            .any(|c| c.drivewsid == mine && c.parent.as_deref() == Some(ROOT)),
        folders
    );
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &made).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let changed = read(&http, &ck, &p, &mut token).await?;
    // As the library would answer: the folder was in the top level.
    let folders = folders_to_list(
        &changed,
        |id| (id == mine).then(|| ROOT.to_string()),
        |id| id == ROOT,
    );
    println!(
        "after delete: {} changed, list again: {:?}",
        changed.len(),
        folders
    );
    let trash = quince_icloud::drive::trash(&http, &drivews, p.clone()).await?;
    let mine: Vec<_> = trash
        .into_iter()
        .filter(|i| i.name.starts_with("Quince mount test"))
        .collect();
    println!("throwaways in Recently Deleted: {}", mine.len());
    if !mine.is_empty() {
        quince_icloud::drive::delete_forever_many(&http, &drivews, p, &mine).await?;
    }
    Ok(())
}
