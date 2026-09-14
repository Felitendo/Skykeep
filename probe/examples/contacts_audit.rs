//! Throwaway: every contact, as Apple has it and as this app stored it.
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;

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
    let http = client.http().clone();
    let params = client.service_params();

    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::contacts::sync(&http, &root, params.clone(), &mut library).await?;
    let mine: HashMap<String, quince_core::contacts::Contact> = library
        .contacts(None)?
        .into_iter()
        .map(|c| (c.guid.clone(), c))
        .collect();

    // Apple's own answer, read separately from the sync.
    let mut q = params;
    q.push(("clientVersion".into(), "2.1".into()));
    q.push(("locale".into(), "en_US".into()));
    q.push(("order".into(), "last,first".into()));
    let body: Value = http
        .get(format!("{root}/co/startup"))
        .query(&q)
        .header("Referer", "https://www.icloud.com/")
        .send()
        .await?
        .json()
        .await?;
    let empty = vec![];
    let theirs = body
        .get("contacts")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let mut problems = 0;
    for c in theirs {
        let Some(guid) = c.get("contactId").and_then(Value::as_str) else {
            continue;
        };
        let Some(ours) = mine.get(guid) else {
            println!("MISSING  {guid}  {:?}", c.get("firstName"));
            problems += 1;
            continue;
        };
        let mut says = Vec::new();
        for (field, got) in [
            ("firstName", &ours.first_name),
            ("middleName", &ours.middle_name),
            ("lastName", &ours.last_name),
            ("companyName", &ours.company),
        ] {
            let want = c
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if got.as_deref() != want {
                says.push(format!("{field} {got:?} vs {want:?}"));
            }
        }
        // Every phone and email Apple lists has to be somewhere in what we kept.
        let mut wanted = 0;
        for kind in ["phones", "emailAddresses"] {
            for entry in c.get(kind).and_then(Value::as_array).unwrap_or(&empty) {
                let Some(value) = entry.get("field").and_then(Value::as_str) else {
                    continue;
                };
                wanted += 1;
                if !ours.methods.iter().any(|m| m.value.trim() == value.trim()) {
                    says.push(format!("missing {kind} {value:?}"));
                }
            }
        }
        if ours.methods.len() < wanted {
            says.push(format!("{} methods kept of {wanted}", ours.methods.len()));
        }
        if !says.is_empty() {
            println!(
                "DIFFERS  {:<24} {}",
                ours.display_name().chars().take(22).collect::<String>(),
                says.join("; ")
            );
            problems += 1;
        }
    }

    let live: std::collections::HashSet<&str> = theirs
        .iter()
        .filter_map(|c| c.get("contactId").and_then(Value::as_str))
        .collect();
    for guid in mine.keys() {
        if !live.contains(guid.as_str()) {
            println!("GHOST    {guid}");
            problems += 1;
        }
    }
    println!(
        "\n{} contacts compared, {problems} disagreement(s)",
        theirs.len()
    );
    Ok(())
}
