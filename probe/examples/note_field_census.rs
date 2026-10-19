//! Read-only: across every note, how often each attribute-run field (and each paragraph-style
//! field inside field 2) is used — to know which of Apple's formatting the account really holds
//! before building any of it. Nothing is written.
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
    let (mut run_fields, mut para_fields) = (BTreeMap::new(), BTreeMap::new());
    let mut notes_with: BTreeMap<String, u32> = BTreeMap::new();
    let mut notes = 0;
    use base64::Engine;
    for r in walk.records.iter().filter(|r| r["recordType"] == "Note") {
        let Some(v) = r["fields"]["TextDataEncrypted"]["value"].as_str() else {
            continue;
        };
        let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(v) else {
            continue;
        };
        notes += 1;
        let mut seen = std::collections::BTreeSet::new();
        for run in quince_icloud::ckdocument::debug_attribute_runs(&raw) {
            for (f, body, _) in fields(&run) {
                *run_fields.entry(f).or_insert(0u32) += 1;
                seen.insert(format!("run.{f}"));
                if f == 2 {
                    for (pf, _, value) in fields(body.unwrap_or_default()) {
                        if pf == 2 {
                            seen.insert(format!("align={value}"));
                        }
                        *para_fields.entry(pf).or_insert(0u32) += 1;
                        seen.insert(format!("para.{pf}"));
                    }
                }
            }
        }
        for s in seen {
            *notes_with.entry(s).or_insert(0) += 1;
        }
    }
    println!("{notes} notes");
    println!("attribute-run fields (runs using it): {run_fields:?}");
    println!("paragraph-style fields (runs using it): {para_fields:?}");
    println!("notes using each: {notes_with:?}");
    Ok(())
}
