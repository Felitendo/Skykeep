//! Read-only: what Apple answers for the photos in each shared album (sharedstreams webgetassets,
//! webgetassetcount, webasseturls), printed as structure — key names, types and short values — so
//! the shapes can be mapped. Nothing is written.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

fn shape(v: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) => {
                        out.push_str(&format!("{pad}{k}:\n"));
                        shape(x, depth + 1, out);
                    }
                    _ => {
                        let s = x.to_string();
                        out.push_str(&format!(
                            "{pad}{k}: {}\n",
                            s.chars().take(60).collect::<String>()
                        ));
                    }
                }
            }
        }
        Value::Array(a) => {
            out.push_str(&format!("{pad}[{} items] first:\n", a.len()));
            if let Some(first) = a.first() {
                shape(first, depth + 1, out);
            }
        }
        other => out.push_str(&format!(
            "{pad}{}\n",
            other.to_string().chars().take(60).collect::<String>()
        )),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("sharedstreams")
        .ok_or_else(|| anyhow!("no sharedstreams"))?;
    let p = client.service_params();
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let http = client.http().clone();
    let call = |base: String, endpoint: &'static str, body: Value| {
        let (http, p) = (http.clone(), p.clone());
        async move {
            let url = format!("{}/{endpoint}", base.trim_end_matches('/'));
            let r = http
                .post(&url)
                .query(&p)
                .header("content-type", "text/plain;charset=UTF-8")
                .body(body.to_string())
                .send()
                .await?;
            let status = r.status();
            let text = r.text().await?;
            println!("── {endpoint}: {status}");
            Ok::<Value, anyhow::Error>(
                serde_json::from_str(&text)
                    .unwrap_or(Value::String(text.chars().take(300).collect())),
            )
        }
    };
    let base = format!("{}/{dsid}/sharedstreams", root.trim_end_matches('/'));
    let list = call(base.clone(), "webgetalbumslist", json!({})).await?;
    let mut s = String::new();
    shape(&list, 0, &mut s);
    println!("{s}");
    for a in list["albums"].as_array().into_iter().flatten() {
        let guid = a["albumguid"].as_str().unwrap_or_default().to_string();
        let ctag = a["albumctag"].clone();
        let at = a["albumlocation"]
            .as_str()
            .map(|l| format!("{}{dsid}/sharedstreams", l))
            .unwrap_or(base.clone());
        let count = call(at.clone(), "webgetassetcount", json!({ "albumguid": guid })).await?;
        println!("{count}");
        let assets = call(
            at.clone(),
            "webgetassets",
            json!({ "albumguid": guid, "offset": "0", "limit": "3", "albumctag": ctag }),
        )
        .await?;
        let mut s = String::new();
        shape(&assets, 0, &mut s);
        println!("{s}");
        let stream = call(
            at.clone(),
            "webstream",
            json!({ "albumguid": guid, "streamCtag": Value::Null }),
        )
        .await?;
        let mut s = String::new();
        shape(&stream, 0, &mut s);
        println!("{}", s.lines().take(80).collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}
