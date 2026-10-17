//! Live, throwaway event only: what eventdetail says of one occurrence of a series — its
//! master dates in particular — then the series deleted.
use anyhow::{anyhow, Result};
use serde_json::Value;
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("calendar")
        .ok_or_else(|| anyhow!("no calendar"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, p.clone());
    let snap = cal
        .startup("2026-10-01", "2026-10-31", "Asia/Kolkata")
        .await?;
    let home = snap
        .calendars
        .iter()
        .find(|c| c.writable)
        .ok_or_else(|| anyhow!("no writable calendar"))?;
    let event = quince_core::calendar::Event {
        guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
        calendar_guid: home.guid.clone(),
        title: "Quince detail test".into(),
        starts_at: 1_793_000_000,
        ends_at: 1_793_003_600,
        timezone: Some("Asia/Kolkata".into()),
        ..Default::default()
    };
    let rule = quince_icloud::calendar::EventRepeat {
        freq: "weekly".into(),
        interval: 1,
        count: Some(3),
    };
    cal.put_event_full(&event, None, None, Some(Some(&rule)), None)
        .await?;
    let occ = format!("{}__20261102T073300Z", event.guid);
    let mut q = p.clone();
    for (k, v) in [
        ("clientVersion", "5.1"),
        ("lang", "en-us"),
        ("usertz", "Asia/Kolkata"),
        ("startDate", "2026-11-02"),
        ("endDate", "2026-11-02"),
    ] {
        q.push((k.into(), v.into()));
    }
    let d: Value = http
        .get(format!(
            "{}/ca/eventdetail/{}/{}",
            root.trim_end_matches('/'),
            home.guid,
            occ
        ))
        .query(&q)
        .send()
        .await?
        .json()
        .await?;
    let e = &d["Event"][0];
    for k in [
        "guid",
        "startDate",
        "endDate",
        "masterStartDate",
        "masterEndDate",
        "recurrenceMaster",
        "recurrence",
        "etag",
    ] {
        println!("{k} = {}", e[k]);
    }
    use quince_icloud::calendar::Scope;
    // All events, from the second occurrence: a new title, an hour later.
    let mut second = event.clone();
    second.guid = occ.clone();
    second.starts_at += 7 * 86_400 + 3600;
    second.ends_at += 7 * 86_400 + 3600;
    second.title = "Quince detail test, all renamed".into();
    cal.put_event_full(&second, None, None, None, Some(Scope::All))
        .await?;
    let mut q2 = p.clone();
    for (k, v) in [
        ("clientVersion", "5.1"),
        ("requestID", "4"),
        ("lang", "en-us"),
        ("usertz", "Asia/Kolkata"),
        ("startDate", "2026-10-26"),
        ("endDate", "2026-11-25"),
    ] {
        q2.push((k.into(), v.into()));
    }
    let all: Value = http
        .get(format!("{}/ca/events", root.trim_end_matches('/')))
        .query(&q2)
        .send()
        .await?
        .json()
        .await?;
    for e in all["Event"].as_array().into_iter().flatten().filter(|e| {
        e["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("Quince detail test"))
    }) {
        println!("  {} {} {}", e["guid"], e["title"], e["startDate"]);
    }
    // Delete all, from an occurrence.
    let mut first = second.clone();
    first.guid = format!("{}__20261026T083300Z", event.guid);
    first.starts_at = event.starts_at + 3600;
    first.ends_at = event.ends_at + 3600;
    match cal
        .delete_event_scoped(&second, None, Some(Scope::All))
        .await
    {
        Ok(()) => println!("delete all from an occurrence: accepted"),
        Err(e) => println!("delete all from an occurrence: refused — {e}"),
    }
    let left: Value = http
        .get(format!("{}/ca/events", root.trim_end_matches('/')))
        .query(&q2)
        .send()
        .await?
        .json()
        .await?;
    let n = left["Event"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e["title"]
                .as_str()
                .is_some_and(|t| t.starts_with("Quince detail test"))
        })
        .count();
    println!("left after delete all: {n}");
    if n > 0 {
        cal.delete_event(&event, None).await?;
        println!("cleaned up by series id");
    }
    Ok(())
}
