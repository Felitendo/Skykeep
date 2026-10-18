//! Read-only: Apple's raw answer for a folder, a folder with nothing in it, and one that
//! does not exist — what a fetch must tell apart before it replaces a stored folder.
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
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let http = client.http().clone();
    let mut q = client.service_params();
    q.push(("clientVersion".into(), "2.1".into()));
    let ask = |id: String| {
        let (http, q, root) = (http.clone(), q.clone(), root.clone());
        async move {
            let r = http
                .post(format!(
                    "{}/retrieveItemDetailsInFolders",
                    root.trim_end_matches('/')
                ))
                .query(&q)
                .json(&serde_json::json!([{ "drivewsid": id, "partialData": false }]))
                .send()
                .await?;
            let s = r.status();
            let v: serde_json::Value = r.json().await?;
            anyhow::Ok((s, v))
        }
    };
    let (s, v) = ask("FOLDER::com.apple.CloudDocs::root".into()).await?;
    let items = v[0]["items"].as_array().cloned().unwrap_or_default();
    println!("root {s}: {} items, status {}", items.len(), v[0]["status"]);
    if let Some(empty) = items
        .iter()
        .find(|i| i["type"] == "FOLDER" && i["numberOfItems"] == 0)
    {
        let (s, v) = ask(empty["drivewsid"].as_str().unwrap().into()).await?;
        println!(
            "empty {s}: {}",
            v.to_string().chars().take(400).collect::<String>()
        );
    } else {
        println!("no empty folder at the top level");
    }
    let (s, v) =
        ask("FOLDER::com.apple.CloudDocs::00000000-0000-0000-0000-000000000000".into()).await?;
    println!(
        "missing {s}: {}",
        v.to_string().chars().take(400).collect::<String>()
    );
    Ok(())
}
