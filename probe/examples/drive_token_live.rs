//! Live: does CloudKit's cheap "has anything changed" check (`changes/database`, read-only, a
//! few hundred bytes) move when iCloud Drive changes? Apple's push socket says nothing for
//! Drive (drive_courier_live). A token is seeded for each candidate container; a throwaway
//! folder ("Quince mount test …") is made in Drive's top level and deleted, and each token is
//! asked again after each step. The throwaway is cleared from Recently Deleted at the end.
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
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let ck = s
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let containers = ["com.apple.clouddocs", "com.apple.CloudDocs"];
    let mut tokens: Vec<Option<String>> = Vec::new();
    for c in containers {
        match quince_icloud::push::database_changed(&http, &ck, &p, c, None).await {
            Ok((_, t)) => {
                println!("{c}: seeded {}", t.is_some());
                tokens.push(t);
            }
            Err(e) => {
                println!("{c}: {e}");
                tokens.push(None);
            }
        }
    }
    async fn ask(
        http: &reqwest::Client,
        ck: &str,
        p: &[(String, String)],
        containers: &[&str],
        tokens: &mut [Option<String>],
        step: &str,
    ) {
        let t = Instant::now();
        let mut seen = vec![false; containers.len()];
        while t.elapsed() < Duration::from_secs(25) && seen.iter().any(|s| !s) {
            for (i, c) in containers.iter().enumerate() {
                if seen[i] || tokens[i].is_none() {
                    seen[i] = true;
                    continue;
                }
                if let Ok((moved, next)) =
                    quince_icloud::push::database_changed(http, ck, p, c, tokens[i].as_deref())
                        .await
                {
                    tokens[i] = next;
                    if moved {
                        seen[i] = true;
                        println!("{c}: moved {:.1}s after {step}", t.elapsed().as_secs_f32());
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        for (i, c) in containers.iter().enumerate() {
            if !seen[i] {
                println!("{c}: no move in 25s after {step}");
            }
        }
    }
    let name = format!("Quince mount test {}", std::process::id());
    let made = quince_icloud::drive::create_folder(
        &http,
        &drivews,
        p.clone(),
        "FOLDER::com.apple.CloudDocs::root",
        &name,
    )
    .await?;
    ask(&http, &ck, &p, &containers, &mut tokens, "folder made").await;
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &made).await?;
    ask(&http, &ck, &p, &containers, &mut tokens, "folder deleted").await;
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
