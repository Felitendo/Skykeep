//! Read-only: the values of the formatting fields Quince does not read yet (run field 13,
//! paragraph fields 2, 3, 8, 9), with the live notes (not deleted) that hold each — to know
//! which change how a note looks. Prints titles (first 30 chars) and values only.
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

fn varint(b: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    let mut s = 0;
    loop {
        let x = *b.get(at)?;
        at += 1;
        v |= u64::from(x & 0x7f) << s;
        s += 7;
        if x < 0x80 {
            return Some((v, at));
        }
    }
}

/// Each field's number, its bytes when length-delimited, and its value when a varint.
fn fields(b: &[u8]) -> Vec<(u64, Option<&[u8]>, u64)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let Some((key, n)) = varint(b, at) else { break };
        at = n;
        match key & 7 {
            0 => {
                let Some((v, n)) = varint(b, at) else { break };
                at = n;
                out.push((key >> 3, None, v));
            }
            2 => {
                let Some((len, n)) = varint(b, at) else { break };
                let end = n + len as usize;
                if end > b.len() {
                    break;
                }
                out.push((key >> 3, Some(&b[n..end]), 0));
                at = end;
            }
            5 => {
                at += 4;
                out.push((key >> 3, None, 0));
            }
            1 => {
                at += 8;
                out.push((key >> 3, None, 0));
            }
            _ => break,
        }
    }
    out
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    let mut samples: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    use base64::Engine;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let gone = r["fields"]["Deleted"]["value"].as_i64() == Some(1)
            || r["fields"]["MarkedForDeletion"]["value"].as_i64() == Some(1);
        if gone {
            continue;
        }
        let Some(v) = r["fields"]["TextDataEncrypted"]["value"].as_str() else {
            continue;
        };
        let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(v) else {
            continue;
        };
        let title: String = quince_icloud::ckdocument::text_from_base64(v)
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(30)
            .collect();
        let mut note = |key: String, value: String| {
            *samples
                .entry(key)
                .or_default()
                .entry(format!("{value}  in {title:?}"))
                .or_insert(0) += 1;
        };
        for run in quince_icloud::ckdocument::debug_attribute_runs(&raw) {
            for (f, body, value) in fields(&run) {
                if f == 13 {
                    let shape = body.map_or(format!("varint {value}"), |b| {
                        format!(
                            "{} bytes, inner fields {:?}",
                            b.len(),
                            fields(b).iter().map(|x| x.0).collect::<Vec<_>>()
                        )
                    });
                    note("run.13".into(), shape);
                }
                if f == 2 {
                    for (pf, pbody, pvalue) in fields(body.unwrap_or_default()) {
                        if matches!(pf, 2 | 3 | 8 | 9) {
                            let shape = pbody.map_or(format!("varint {pvalue}"), |b| {
                                format!(
                                    "{} bytes, inner fields {:?}",
                                    b.len(),
                                    fields(b).iter().map(|x| x.0).collect::<Vec<_>>()
                                )
                            });
                            note(format!("para.{pf}"), shape);
                        }
                    }
                }
            }
        }
    }
    for (k, vals) in samples {
        println!("{k}:");
        for (v, n) in vals.iter().take(12) {
            println!("   {n:>4} × {v}");
        }
    }
    Ok(())
}
