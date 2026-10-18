//! Live, read-only: `/co/changeset` every two seconds from the startup tokens, printing what
//! each answer carries, so a change made elsewhere can be timed.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let (_, mut tokens) =
        quince_icloud::contacts::fetch_with_tokens(&http, &root, p.clone()).await?;
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    };
    println!("from {:.3}", now());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        let mut q = p.clone();
        q.push(("clientVersion".into(), "2.1".into()));
        q.push(("locale".into(), "en_US".into()));
        q.push(("order".into(), "last,first".into()));
        q.push(("prefToken".into(), tokens.pref.clone()));
        q.push(("syncToken".into(), tokens.sync.clone()));
        let t = now();
        let r = http
            .post(format!("{root}/co/changeset"))
            .query(&q)
            .send()
            .await?;
        let st = r.status();
        let v: serde_json::Value = r.json().await.unwrap_or_default();
        let n = |k: &str| v.get(k).and_then(|a| a.as_array()).map_or(0, |a| a.len());
        let moved = v["syncToken"].as_str().is_some_and(|s| s != tokens.sync);
        if moved || n("contacts") > 0 || st != 200 {
            println!(
                "{:.3} ({:.2}s) {st}: contacts {} groups {} deletes {} token moved {moved}",
                now(),
                now() - t,
                n("contacts"),
                n("groups"),
                v["deletes"]
            );
        }
        if let Some(s) = v["syncToken"].as_str() {
            tokens.sync = s.to_string();
        }
        if let Some(s) = v["prefToken"].as_str() {
            tokens.pref = s.to_string();
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    println!("end; keys seen: {:?}", ());
    Ok(())
}
