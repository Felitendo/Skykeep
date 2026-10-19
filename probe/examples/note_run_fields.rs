//! Read-only: every field of every attribute run of the note whose title starts with the
//! given words — numbers and raw values, to see how colour is kept.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let start = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("title start"))?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    let note = walk
        .records
        .iter()
        .find(|r| {
            r["recordType"] == "Note"
                && r["fields"]["TextDataEncrypted"]["value"]
                    .as_str()
                    .and_then(quince_icloud::ckdocument::text_from_base64)
                    .is_some_and(|t| t.starts_with(&start))
        })
        .ok_or_else(|| anyhow!("no such note"))?;
    let v = note["fields"]["TextDataEncrypted"]["value"]
        .as_str()
        .unwrap();
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD.decode(v)?;
    let text = quince_icloud::ckdocument::text_from_base64(v).unwrap_or_default();
    println!(
        "text ({} chars): {:?}",
        text.chars().count(),
        text.chars().take(700).collect::<String>()
    );
    println!(
        "modified: {} by device {:?}",
        note["modified"]["timestamp"], note["modified"]["deviceID"]
    );
    for (i, run) in quince_icloud::ckdocument::debug_attribute_runs(&raw)
        .iter()
        .enumerate()
    {
        let mut at = 0;
        let mut parts = vec![];
        let b = run.as_slice();
        let vint = |b: &[u8], mut at: usize| {
            let mut v = 0u64;
            let mut s = 0;
            loop {
                let x = b[at];
                at += 1;
                v |= u64::from(x & 0x7f) << s;
                s += 7;
                if x < 0x80 {
                    return (v, at);
                }
            }
        };
        while at < b.len() {
            let (k, n) = vint(b, at);
            at = n;
            match k & 7 {
                0 => {
                    let (x, n) = vint(b, at);
                    at = n;
                    parts.push(format!("{}={x}", k >> 3));
                }
                2 => {
                    let (len, n) = vint(b, at);
                    let s = &b[n..n + len as usize];
                    at = n + len as usize;
                    parts.push(format!(
                        "{}:[{}]",
                        k >> 3,
                        s.iter()
                            .map(|c| format!("{c:02x}"))
                            .collect::<Vec<_>>()
                            .join("")
                    ));
                }
                5 => {
                    parts.push(format!("{}=f32 {:?}", k >> 3, &b[at..at + 4]));
                    at += 4;
                }
                1 => {
                    at += 8;
                    parts.push(format!("{}=64", k >> 3));
                }
                _ => break,
            }
        }
        println!("run {i}: {}", parts.join(" "));
    }
    Ok(())
}
