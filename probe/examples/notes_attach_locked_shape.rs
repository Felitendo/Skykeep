//! Read-only: the SHAPE of (a) password-protected notes and (b) phone-made picture
//! attachments — field names, CloudKit types, byte lengths, numbers. Never note text.
use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::Value;

fn b64len(v: &Value) -> String {
    v.as_str()
        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .map_or("?".into(), |b| format!("{} bytes", b.len()))
}

fn name_shape(n: &str) -> String {
    let up = n
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
    format!(
        "len {} {}{}",
        n.len(),
        if up { "UPPER/hex" } else { "mixed" },
        if n.matches('-').count() == 4 && n.len() == 36 {
            " (uuid)"
        } else {
            ""
        }
    )
}

/// Every field: name, type, and a non-revealing description of its value.
fn describe(r: &Value, safe_text: &[&str]) {
    println!(
        "   recordType={} name: {}",
        r["recordType"],
        name_shape(r["recordName"].as_str().unwrap_or(""))
    );
    let mut top: Vec<&String> = r
        .as_object()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    top.sort();
    println!("   record keys: {top:?}");
    let Some(f) = r["fields"].as_object() else {
        return;
    };
    let mut keys: Vec<&String> = f.keys().collect();
    keys.sort();
    for k in keys {
        let t = f[k]["type"].as_str().unwrap_or("?");
        let v = &f[k]["value"];
        let d = match t {
            "ENCRYPTED_BYTES" | "BYTES" => {
                if safe_text.contains(&k.as_str()) {
                    let raw = v
                        .as_str()
                        .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
                        .unwrap_or_default();
                    format!("{:?}", String::from_utf8_lossy(&raw))
                } else {
                    b64len(v)
                }
            }
            "REFERENCE" => format!(
                "-> {} action={}",
                name_shape(v["recordName"].as_str().unwrap_or("")),
                v["action"]
            ),
            "REFERENCE_LIST" => format!("list of {}", v.as_array().map_or(0, Vec::len)),
            "ASSETID" => format!(
                "asset keys {:?} size {}",
                v.as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
                v["size"]
            ),
            "ASSETID_LIST" => format!("{} assets", v.as_array().map_or(0, Vec::len)),
            "STRING" => {
                if safe_text.contains(&k.as_str()) {
                    format!("{v}")
                } else {
                    format!("string {} chars", v.as_str().map_or(0, str::len))
                }
            }
            _ => format!("{v}"),
        };
        println!("     {k:<38} {t:<16} {d}");
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let walk = zc.all_records("Notes", None).await?;
    let recs = walk.records;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for r in &recs {
        *counts
            .entry(
                r["recordType"]
                    .as_str()
                    .unwrap_or("<tombstone>")
                    .to_string(),
            )
            .or_default() += 1;
    }
    println!("record types: {counts:?}");

    // (a) locked notes
    let mut field_census = std::collections::BTreeMap::<String, usize>::new();
    for r in &recs {
        if let Some(o) = r["fields"].as_object() {
            for k in o.keys() {
                if k.contains("Crypto") || k.contains("Password") || k.contains("Hint") {
                    *field_census
                        .entry(format!("{}.{k}", r["recordType"].as_str().unwrap_or("?")))
                        .or_default() += 1;
                }
            }
        }
    }
    println!("\ncrypto/password fields across zone: {field_census:?}");
    for r in recs
        .iter()
        .filter(|r| {
            r["recordType"]
                .as_str()
                .is_some_and(|t| t.starts_with("PasswordProtected"))
        })
        .take(2)
    {
        println!("\n== locked-related record");
        describe(r, &[]);
    }
    for r in recs
        .iter()
        .filter(|r| {
            r["fields"].get("CryptoSaltEncrypted").is_some()
                || r["fields"].get("CryptoIterationCount").is_some()
        })
        .take(2)
    {
        println!("\n== record carrying crypto fields");
        describe(r, &[]);
    }

    for r in recs.iter().filter(|r| r["recordType"] == "AccountData") {
        println!("\n== AccountData");
        describe(r, &[]);
    }
    let locked: Vec<String> = recs
        .iter()
        .filter(|r| r["recordType"] == "PasswordProtectedNote_UserSpecific")
        .filter_map(|r| {
            r["fields"]["Note"]["value"]["recordName"]
                .as_str()
                .map(String::from)
        })
        .collect();
    for n in &locked {
        let in_walk = recs
            .iter()
            .any(|r| r["recordName"].as_str() == Some(n.as_str()));
        println!("\n== locked note (in walk: {in_walk})");
        match zc.lookup_record("Notes", n).await? {
            Some(r) => describe(&r, &[]),
            None => println!("   lookup: not found"),
        }
    }
    for r in recs.iter().filter(|r| r["recordType"] == "AccountData") {
        let raw = r["fields"]["CryptoPassphraseVerifier"]["value"]
            .as_str()
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .unwrap_or_default();
        println!(
            "verifier head: {:?}",
            String::from_utf8_lossy(&raw[..raw.len().min(8)])
        );
        match plist::Value::from_reader(std::io::Cursor::new(&raw)) {
            Ok(v) => {
                fn shape(v: &plist::Value, pad: usize) {
                    match v {
                        plist::Value::Dictionary(d) => {
                            for (k, x) in d {
                                println!(
                                    "{}{k}: {}",
                                    " ".repeat(pad),
                                    match x {
                                        plist::Value::Integer(i) => format!("int {i:?}"),
                                        plist::Value::Data(b) => format!("data {}B", b.len()),
                                        plist::Value::String(s) => format!("string {}c", s.len()),
                                        plist::Value::Dictionary(_) | plist::Value::Array(_) =>
                                            "nested".into(),
                                        o => format!("{o:?}").chars().take(30).collect(),
                                    }
                                );
                                shape(x, pad + 2);
                            }
                        }
                        plist::Value::Array(a) => {
                            for x in a {
                                shape(x, pad + 2)
                            }
                        }
                        _ => {}
                    }
                }
                shape(&v, 3);
            }
            Err(e) => println!("   not a plist: {e}"),
        }
        let raw = r["fields"]["MergeableDataEncrypted"]["value"]
            .as_str()
            .unwrap_or("");
        if let Some(b) = quince_icloud::ckdocument::debug_inflate(raw) {
            for l in quince_icloud::ckdocument::debug_tree(&b, 3)
                .iter()
                .filter(|l| !l.contains("text"))
            {
                println!("   md {l}");
            }
        }
    }
    let names = |t: &str| -> Vec<std::collections::BTreeSet<String>> {
        recs.iter()
            .filter(|r| r["recordType"] == "Attachment" && r["fields"]["UTI"]["value"] == t)
            .map(|r| r["fields"].as_object().unwrap().keys().cloned().collect())
            .collect()
    };
    for t in [
        "public.jpeg",
        "public.png",
        "public.json",
        "org.openxmlformats.spreadsheetml.sheet",
    ] {
        for set in names(t) {
            println!("{t}: {set:?}");
        }
    }
    if std::env::var("ONLY_LOCKED").is_ok() {
        return Ok(());
    }
    // (b) phone-made picture attachments
    let uti_of = |r: &Value| -> String {
        r["fields"]["UTIEncrypted"]["value"]
            .as_str()
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .or_else(|| r["fields"]["UTI"]["value"].as_str().map(String::from))
            .unwrap_or_default()
    };
    let mut utis = std::collections::BTreeMap::<String, usize>::new();
    for r in recs.iter().filter(|r| r["recordType"] == "Attachment") {
        *utis.entry(uti_of(r)).or_default() += 1;
    }
    println!("\nattachment UTIs: {utis:?}");
    let pics: Vec<&Value> = recs
        .iter()
        .filter(|r| {
            r["recordType"] == "Attachment"
                && matches!(
                    uti_of(r).as_str(),
                    "public.jpeg" | "public.heic" | "public.png"
                )
        })
        .take(6)
        .collect();
    for a in pics {
        println!(
            "\n== image Attachment parent={} chainParentKey? {} pluginFields={}",
            a["parent"]
                .as_object()
                .map(|o| format!("{:?}", o.keys().collect::<Vec<_>>()))
                .unwrap_or_default(),
            a.get("chainParentKey").is_some(),
            a["pluginFields"]
        );
        if let Some(pn) = a["parent"]["recordName"].as_str() {
            println!(
                "   parent == Note ref? {}",
                Some(pn) == a["fields"]["Note"]["value"]["recordName"].as_str()
            );
        }
        describe(a, &["UTIEncrypted", "UTI"]);
        if let Some(m) = a["fields"]["Media"]["value"]["recordName"].as_str() {
            if let Some(media) = zc.lookup_record("Notes", m).await? {
                println!(
                    "  -- its Media; parent == attachment? {} parent==note? {}",
                    media["parent"]["recordName"].as_str() == a["recordName"].as_str(),
                    media["parent"]["recordName"].as_str()
                        == a["fields"]["Note"]["value"]["recordName"].as_str()
                );
                describe(&media, &[]);
            }
        }
        // The run in its note that holds it.
        if let Some(n) = a["fields"]["Note"]["value"]["recordName"].as_str() {
            if let Some(note) = zc.lookup_record("Notes", n).await? {
                println!("  -- its Note's fields (shape)");
                describe(&note, &[]);
                let id = a["recordName"].as_str().unwrap_or("");
                if let Some(doc) = note["fields"]["TextDataEncrypted"]["value"].as_str() {
                    let raw = base64::engine::general_purpose::STANDARD.decode(doc)?;
                    for run in quince_icloud::ckdocument::debug_attribute_runs(&raw) {
                        if run.windows(id.len()).any(|w| w == id.as_bytes()) {
                            println!("  -- attachment run tree:");
                            for l in quince_icloud::ckdocument::debug_tree(&run, 4) {
                                // ids/utis are strings; print only field shape lines
                                println!("     {}", l.replace(id, "<ATTACHMENT-ID>"));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
