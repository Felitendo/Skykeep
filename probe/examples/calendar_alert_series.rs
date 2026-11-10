//! Live, throwaway event only: a weekly event three times, an alert set from its second
//! occurrence for all events — every occurrence must then carry it — then deleted.
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
        title: "Quince alert series test".into(),
        starts_at: 1_793_000_000,
        ends_at: 1_793_003_600,
        timezone: Some("Asia/Kolkata".into()),
        ..Default::default()
    };
    let rule = quince_icloud::calendar::EventRepeat {
        freq: "weekly".into(),
        interval: 1,
        count: Some(3),
        ..quince_icloud::calendar::EventRepeat::default()
    };
    cal.put_event_full(&event, None, None, Some(Some(&rule)), None)
        .await?;
    let _raw = |from: &str, to: &str| {
        let mut q = p.clone();
        for (k, v) in [
            ("clientVersion", "5.1"),
            ("requestID", "4"),
            ("lang", "en-us"),
            ("usertz", "Asia/Kolkata"),
            ("startDate", from),
            ("endDate", to),
        ] {
            q.push((k.into(), v.into()));
        }
        (format!("{}/ca/events", root.trim_end_matches('/')), q)
    };
    let occ = |n: i64| {
        let mut e = event.clone();
        let d = 7 * 86_400 * n;
        e.starts_at += d;
        e.ends_at += d;
        e.guid = format!(
            "{}__{}",
            event.guid,
            jiff::Timestamp::from_second(e.starts_at)
                .unwrap()
                .strftime("%Y%m%dT%H%M%SZ")
        );
        e
    };
    let second = occ(1);
    cal.put_event_full(
        &second,
        None,
        Some(&[15]),
        None,
        Some(quince_icloud::calendar::Scope::All),
    )
    .await?;
    for (label, e) in [("first", occ(0)), ("second", occ(1)), ("third", occ(2))] {
        println!("{label}: alerts {:?}", cal.alerts(&e).await);
    }
    cal.delete_event(&event, None).await?;
    println!("deleted");
    Ok(())
}
