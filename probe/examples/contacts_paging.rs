//! Read-only: does startup hold every card its own order names, and does the page read
//! (co/contacts/card/?offset&limit) answer with cards — counts only.
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
        .service_url("contacts")
        .ok_or_else(|| anyhow!("no contacts"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let (people, tokens) =
        quince_icloud::contacts::fetch_with_tokens(&http, &root, p.clone()).await?;
    println!("fetched {} cards", people.len());
    let mut q = p.clone();
    q.push(("syncToken".into(), tokens.sync.clone()));
    q.push(("prefToken".into(), tokens.pref.clone()));
    for (k, v) in [
        ("clientVersion", "2.1"),
        ("locale", "en_US"),
        ("order", "last,first"),
        ("offset", "100"),
        ("limit", "50"),
    ] {
        q.push((k.into(), v.into()));
    }
    let r = http
        .get(format!("{root}/co/contacts/card/"))
        .query(&q)
        .send()
        .await?;
    let st = r.status();
    let v: serde_json::Value = r.json().await.unwrap_or_default();
    println!(
        "page offset 100 limit 50: {st}, {} cards, keys {:?}",
        v["contacts"].as_array().map_or(0, |a| a.len()),
        v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
    );
    Ok(())
}
