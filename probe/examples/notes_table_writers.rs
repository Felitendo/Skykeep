//! Read-only: every table attachment on the account, by structure only — its document's
//! version vector (data field 1), its text writers (data field 7: uuid prefix and clocks),
//! its uuid count, and whether a given writer uuid appears. Tables naming that writer are
//! saved (inflated) to the dump dir for diffing. No cell text is printed.
//!
//! `notes_table_writers <writer-uuid> <dumpdir> [all]` — `all` saves every table, not only
//! those naming the writer.
use anyhow::{anyhow, Result};

fn varint(b: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for s in 0..10 {
        let x = *b.get(at)?;
        at += 1;
        v |= u64::from(x & 0x7f) << (s * 7);
        if x & 0x80 == 0 {
            return Some((v, at));
        }
    }
    None
}

/// (field, Some(bytes) | None, int)
/// A message's fields: number, bytes when length-delimited, and value when a varint.
type Fields<'a> = Vec<(u64, Option<&'a [u8]>, u64)>;

fn fields(b: &[u8]) -> Option<Fields<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let (k, n) = varint(b, at)?;
        at = n;
        match k & 7 {
            0 => {
                let (v, n) = varint(b, at)?;
                at = n;
                out.push((k >> 3, None, v));
            }
            2 => {
                let (l, n) = varint(b, at)?;
                let end = n.checked_add(usize::try_from(l).ok()?)?;
                out.push((k >> 3, Some(b.get(n..end)?), 0));
                at = end;
            }
            1 => at += 8,
            5 => at += 4,
            _ => return None,
        }
    }
    Some(out)
}

fn sub<'a>(fs: &[(u64, Option<&'a [u8]>, u64)], t: u64) -> Vec<&'a [u8]> {
    fs.iter().filter(|f| f.0 == t).filter_map(|f| f.1).collect()
}

fn ints(b: &[u8]) -> Vec<(u64, u64)> {
    fields(b)
        .unwrap_or_default()
        .iter()
        .filter(|f| f.1.is_none())
        .map(|f| (f.0, f.2))
        .collect()
}

#[tokio::main]
async fn main() -> Result<()> {
    let writer = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("writer uuid"))?;
    let out = std::env::args().nth(2).ok_or_else(|| anyhow!("dump dir"))?;
    std::fs::create_dir_all(&out)?;
    let me = *uuid::Uuid::parse_str(&writer)?.as_bytes();
    let all = std::env::args().nth(3).as_deref() == Some("all");
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p);
    let walk = zc.all_records("Notes", None).await?;
    for r in walk.records.iter().filter(|r| {
        r["recordType"] == "Attachment" && r["fields"]["UTI"]["value"] == quince_icloud::table::UTI
    }) {
        let name = r["recordName"].as_str().unwrap_or("x");
        let note = r["fields"]["Note"]["value"]["recordName"]
            .as_str()
            .unwrap_or("?");
        let Some(raw) = r["fields"]["MergeableDataEncrypted"]["value"]
            .as_str()
            .and_then(quince_icloud::ckdocument::debug_inflate)
        else {
            continue;
        };
        let Some(top) = fields(&raw) else { continue };
        let mut lines = Vec::new();
        let mut mine = raw.windows(16).any(|w| w == me);
        for (vi, version) in sub(&top, 2).into_iter().enumerate() {
            let Some(vf) = fields(version) else { continue };
            for data in sub(&vf, 3) {
                let Some(df) = fields(data) else { continue };
                let vv: Vec<Vec<(u64, u64)>> = sub(&df, 1)
                    .iter()
                    .flat_map(|v| sub(&fields(v).unwrap_or_default(), 1))
                    .map(ints)
                    .collect();
                let uuids = sub(&df, 6);
                let mut writers = Vec::new();
                for w in sub(&df, 7)
                    .iter()
                    .flat_map(|v| sub(&fields(v).unwrap_or_default(), 1))
                {
                    let wf = fields(w).unwrap_or_default();
                    let id = sub(&wf, 1).first().map_or(String::new(), |u| {
                        u.iter().take(4).map(|b| format!("{b:02x}")).collect()
                    });
                    let clocks: Vec<u64> = sub(&wf, 2)
                        .iter()
                        .flat_map(|c| ints(c))
                        .map(|x| x.1)
                        .collect();
                    writers.push(format!("{id}{clocks:?}"));
                }
                let u0 = uuids.first().map_or(String::new(), |u| {
                    u.iter().take(4).map(|b| format!("{b:02x}")).collect()
                });
                if uuids.contains(&&me[..]) {
                    mine = true;
                }
                lines.push(format!(
                    "  v{vi}: data {}B vv={vv:?} f2={} uuids={} uuid0={u0} writers={writers:?}",
                    data.len(),
                    sub(&df, 2).len(),
                    uuids.len()
                ));
            }
        }
        println!(
            "== {name} note={note} deleted={} mine={mine} top={:?} created={}@{} modified={}@{}",
            r["fields"]["Deleted"]["value"],
            ints(&raw),
            r["created"]["deviceID"],
            r["created"]["timestamp"],
            r["modified"]["deviceID"],
            r["modified"]["timestamp"],
        );
        for l in lines {
            println!("{l}");
        }
        if mine || all {
            std::fs::write(format!("{out}/{name}.bin"), &raw).ok();
        }
    }
    Ok(())
}
