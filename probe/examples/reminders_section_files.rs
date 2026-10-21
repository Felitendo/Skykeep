//! Live, READ-ONLY: download every list's section-membership file (and SectionIDsOrdering if
//! any) from the zone dump made by `reminders_detail_census`, saving each beside the dump.
//! Writes nothing to iCloud. Prints only sizes and checksums.
use anyhow::{anyhow, Result};
use serde_json::Value;
#[tokio::main]
async fn main() -> Result<()> {
    let dump = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("usage: <zone.json>"))?;
    let out = std::path::Path::new(&dump)
        .parent()
        .unwrap()
        .join("memberships");
    std::fs::create_dir_all(&out)?;
    let records: Vec<Value> = serde_json::from_slice(&std::fs::read(&dump)?)?;
    let http = reqwest::Client::new();
    for (i, r) in records.iter().enumerate() {
        if r["recordType"] != "List" {
            continue;
        }
        for f in [
            "MembershipsOfRemindersInSectionsAsData",
            "SectionIDsOrderingAsData",
        ] {
            let Some(url) = r
                .pointer(&format!("/fields/{f}/value/downloadURL"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let bytes = http
                .get(url.replace("${f}", "file"))
                .send()
                .await?
                .bytes()
                .await?;
            let name = format!("{i}-{f}.bin");
            std::fs::write(out.join(&name), &bytes)?;
            let ck = r.pointer("/fields/MembershipsOfRemindersInSectionsChecksum/value");
            println!(
                "{name}: {} bytes, checksum field {:?}, asset {:?}",
                bytes.len(),
                ck,
                r.pointer(&format!("/fields/{f}/value/fileChecksum"))
            );
        }
    }
    Ok(())
}
