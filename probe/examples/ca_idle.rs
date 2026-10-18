//! Read-only: whether `/ca/idle` (icloud.com Calendar's push-or-poll call) holds the request
//! open until a calendar changes, and what it answers when one does.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let wait: u64 = std::env::args()
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
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, p.clone());
    let snap = cal
        .startup("2026-10-01", "2026-10-31", "Asia/Kolkata")
        .await?;
    let state: Vec<_> = snap
        .calendars
        .iter()
        .filter(|c| c.ctag.is_some())
        .map(|c| serde_json::json!({ "guid": c.guid, "ctag": c.ctag }))
        .collect();
    println!("{} calendars with a ctag", state.len());
    let mut q = p.clone();
    q.push(("clientVersion".into(), "5.1".into()));
    q.push(("lang".into(), "en-us".into()));
    q.push(("usertz".into(), "Asia/Kolkata".into()));
    q.push(("requestID".into(), "9".into()));
    q.push(("startDate".into(), "2026-10-01".into()));
    q.push(("endDate".into(), "2026-10-31".into()));
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    };
    let mut state = state;
    for round in 0..3 {
        let t = now();
        println!(
            "round {round}: idle sent at {t:.3} with {} collections",
            state.len()
        );
        let r = http.post(format!("{root}/ca/idle")).query(&q).timeout(std::time::Duration::from_secs(wait))
            .json(&serde_json::json!({ "ClientState": { "Collection": state, "alarmRange": 60, "fullState": true, "userTime": t as i64 } }))
            .send().await;
        match r {
            Ok(r) => {
                let st = r.status();
                let v: serde_json::Value = r.json().await.unwrap_or_default();
                println!(
                    "  answered after {:.1}s: {st} {}",
                    now() - t,
                    v.to_string().chars().take(400).collect::<String>()
                );
                for c in v
                    .pointer("/ChangeSet/inserts/Collection")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .chain(
                        v.pointer("/ChangeSet/updates/Collection")
                            .and_then(|c| c.as_array())
                            .cloned()
                            .unwrap_or_default(),
                    )
                {
                    state.retain(|s| s["guid"] != c["guid"]);
                    state.push(serde_json::json!({ "guid": c["guid"], "ctag": c["ctag"] }));
                }
            }
            Err(e) => {
                println!("  no answer after {:.1}s: {e}", now() - t);
                break;
            }
        }
    }
    Ok(())
}
