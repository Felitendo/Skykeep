//! Live: does Apple push an iCloud Drive change to the web's push channel? A channel is
//! opened on each candidate container in turn; a throwaway folder ("Quince mount test …") is
//! made in Drive's top level and deleted, and each push (or none) is printed with its delay.
//! The throwaway is cleared from Recently Deleted at the end.
use anyhow::{anyhow, Result};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let device = s
        .service_url("ckdeviceservice")
        .ok_or_else(|| anyhow!("no ckdevice"))?;
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    for container in [
        "com.apple.clouddocs",
        "com.apple.CloudDocs",
        "com.apple.icloud.drive",
    ] {
        let ch = match quince_icloud::push::open(&http, &device, &p, "PROBE-DRIVE", &[container])
            .await
        {
            Ok(ch) => ch,
            Err(e) => {
                println!("{container}: no channel ({e})");
                continue;
            }
        };
        let name = format!("Quince mount test {}", std::process::id());
        let t = Instant::now();
        let made = quince_icloud::drive::create_folder(
            &http,
            &drivews,
            p.clone(),
            "FOLDER::com.apple.CloudDocs::root",
            &name,
        )
        .await?;
        let got = quince_icloud::push::wait(&http, &ch, Duration::from_secs(25)).await;
        println!(
            "{container}: folder made; push {:?} after {:?}",
            got.as_ref().map(|v| v.is_some()),
            t.elapsed()
        );
        let t = Instant::now();
        quince_icloud::drive::delete(&http, &drivews, p.clone(), &made).await?;
        let got = quince_icloud::push::wait(&http, &ch, Duration::from_secs(25)).await;
        println!(
            "{container}: folder deleted; push {:?} after {:?}",
            got.as_ref().map(|v| v.is_some()),
            t.elapsed()
        );
    }
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
