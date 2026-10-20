//! Live, for mount tests: a change made "on another device" — a throwaway folder made in Drive's
//! top level straight through Apple (`make <name>`, the name must start "Quince mount test"),
//! or every such throwaway deleted and cleared from Recently Deleted (`purge`).
use anyhow::{anyhow, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("make") => {
            let name = args.next().ok_or_else(|| anyhow!("name"))?;
            if !name.starts_with("Quince mount test") {
                return Err(anyhow!("throwaways only"));
            }
            quince_icloud::drive::create_folder(
                &http,
                &drivews,
                p,
                quince_core::drive::ROOT,
                &name,
            )
            .await?;
            println!("made");
        }
        Some("purge") => {
            let top = quince_icloud::drive::fetch_folder(
                &http,
                &drivews,
                p.clone(),
                quince_core::drive::ROOT,
            )
            .await?;
            for item in top
                .iter()
                .filter(|i| i.name.starts_with("Quince mount test"))
            {
                quince_icloud::drive::delete(&http, &drivews, p.clone(), item).await?;
            }
            let trash = quince_icloud::drive::trash(&http, &drivews, p.clone()).await?;
            let mine: Vec<_> = trash
                .into_iter()
                .filter(|i| i.name.starts_with("Quince mount test"))
                .collect();
            if !mine.is_empty() {
                quince_icloud::drive::delete_forever_many(&http, &drivews, p, &mine).await?;
            }
            println!("purged {}", mine.len());
        }
        Some("find") => {
            // How long Apple's listing of the top level takes, and whether it has the name yet.
            let name = args.next().ok_or_else(|| anyhow!("name"))?;
            for _ in 0..10 {
                let t = std::time::Instant::now();
                let top = quince_icloud::drive::fetch_folder(
                    &http,
                    &drivews,
                    p.clone(),
                    quince_core::drive::ROOT,
                )
                .await?;
                let has = top.iter().any(|i| i.name == name);
                println!(
                    "listed {} items in {} ms; has it: {has}",
                    top.len(),
                    t.elapsed().as_millis()
                );
                if has {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        _ => return Err(anyhow!("make <name> | find <name> | purge")),
    }
    Ok(())
}
