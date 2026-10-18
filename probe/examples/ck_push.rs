//! Read-only: CloudKit's push channel as icloud.com's CloudKit JS uses it — a token from
//! `device/1/<container>/production/tokens/create`, the subscriptions the account already
//! has, then one parked long-poll on the `webcourierURL` the token came with.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let container = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "com.apple.notes".into());
    let wait: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(90);
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let ck = ck.trim_end_matches('/');
    let dev = client
        .session()
        .service_url("ckdeviceservice")
        .ok_or_else(|| anyhow!("no ckdevice"))?;
    let http = client.http().clone();
    let mut q = client.service_params();
    q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
    q.push(("ckjsVersion".into(), "2.6.1".into()));
    let client_id = uuid::Uuid::new_v4().to_string();
    let r = http
        .post(format!(
            "{dev}/device/1/{container}/production/tokens/create"
        ))
        .query(&q)
        .json(&serde_json::json!({ "apnsEnvironment": "production", "clientId": client_id }))
        .send()
        .await?;
    let s = r.status();
    let token: serde_json::Value = r.json().await.unwrap_or_default();
    println!(
        "tokens/create {s}: {}",
        token.to_string().chars().take(300).collect::<String>()
    );
    let r = http
        .get(format!(
            "{ck}/database/1/{container}/production/private/subscriptions/list"
        ))
        .query(&q)
        .send()
        .await?;
    let s = r.status();
    let subs: serde_json::Value = r.json().await.unwrap_or_default();
    println!(
        "subscriptions/list {s}: {}",
        subs.to_string().chars().take(800).collect::<String>()
    );
    // The same token for more containers, as CloudKit JS shares one connector between them.
    for other in std::env::args().skip(3) {
        let r = http.post(format!("{dev}/device/1/{other}/production/tokens/register")).query(&q)
            .json(&serde_json::json!({ "apnsEnvironment": "production", "apnsToken": token["apnsToken"], "clientId": client_id })).send().await?;
        println!(
            "tokens/register {other} {}: {}",
            r.status(),
            r.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        );
    }
    let Some(url) = token["webcourierURL"].as_str() else {
        return Ok(());
    };
    println!("parking on the push channel for up to {wait}s…");
    let t = std::time::Instant::now();
    let r = http
        .get(url)
        .timeout(std::time::Duration::from_secs(wait))
        .send()
        .await;
    match r {
        Ok(r) => println!(
            "after {:?}: {} {}",
            t.elapsed(),
            r.status(),
            r.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(600)
                .collect::<String>()
        ),
        Err(e) => println!("after {:?}: {e}", t.elapsed()),
    }
    Ok(())
}
