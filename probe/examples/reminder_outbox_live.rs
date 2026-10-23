//! Live, throwaway list and reminder only: the outbox's way. Ticked, unticked and ticked again
//! here before anything is sent — the queue keeps only the last word — then what is queued is
//! sent in order, with Apple's state read back after each. The list is deleted at the end.
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
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    use quince_icloud::reminders as r;
    let list = r::create_list_cloudkit(&http, &root, p.clone(), "Quince outbox test").await?;
    let guid = r::create_cloudkit(&http, &root, p.clone(), &list, "Outbox test").await?;
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.reminders", p.clone());
    let completed = || {
        let (zc, guid) = (&zc, guid.clone());
        async move {
            let rec = zc
                .lookup_record("Reminders", &guid)
                .await?
                .ok_or_else(|| anyhow!("gone"))?;
            anyhow::Ok(quince_icloud::cloudkit::field::int(&rec, "Completed").unwrap_or(0) == 1)
        }
    };
    let reminder = quince_core::reminders::Reminder {
        guid: guid.clone(),
        list_guid: list.clone(),
        title: "Outbox test".into(),
        ..Default::default()
    };
    let mut lib = quince_core::Library::in_memory()?;
    let result = async {
        // Three quick taps before anything goes: only the last needs to reach Apple.
        for done in [true, false, true] {
            lib.outbox_push(
                "reminders",
                &guid,
                "done",
                &format!("{{\"done\":{done},\"at\":1}}"),
            )?;
        }
        let queued = lib.outbox_pending("reminders")?;
        println!(
            "queued after three taps: {}",
            queued
                .iter()
                .map(|q| q.payload.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        );
        anyhow::ensure!(queued.len() == 1 && queued[0].payload.contains("true"));
        let t = std::time::Instant::now();
        r::set_completed_cloudkit(&http, &root, p.clone(), &reminder, true).await?;
        lib.outbox_done(queued[0].id)?;
        println!(
            "sent in {:?}; Apple says completed = {}",
            t.elapsed(),
            completed().await?
        );
        anyhow::ensure!(completed().await?);
        // Ticked here, the time it was ticked is Apple's own field, as the phone writes it.
        let rec = zc
            .lookup_record("Reminders", &guid)
            .await?
            .ok_or_else(|| anyhow!("gone"))?;
        let when = rec
            .pointer("/fields/CompletionDate/value")
            .and_then(serde_json::Value::as_i64);
        let now = jiff::Timestamp::now().as_second();
        println!(
            "CompletionDate written: {:?} ({}s ago)",
            when,
            when.map_or(-1, |w| now - w / 1000)
        );
        anyhow::ensure!(when.is_some_and(|w| (now - w / 1000).abs() < 120));
        // A queued tick Apple already holds steps aside; an untick queued behind this run's own
        // tick goes — its own send never overtakes a change made after it.
        let base = r::TickBase::of(&reminder);
        let held = r::tick_overtaken(&http, &root, p.clone(), &reminder, true, &base).await?;
        let after = r::tick_overtaken(&http, &root, p.clone(), &reminder, false, &base).await?;
        println!("overtaken: already held {held}, behind our own tick {after}");
        anyhow::ensure!(held && !after);
        // Untick then, sent on the change tag the library held before the tick — stale, as the
        // worker's would be until the next pass.
        lib.outbox_push("reminders", &guid, "done", "{\"done\":false,\"at\":1}")?;
        let q = lib.outbox_pending("reminders")?;
        r::set_completed_cloudkit(&http, &root, p.clone(), &reminder, false).await?;
        lib.outbox_done(q[0].id)?;
        println!(
            "untick on a stale tag: Apple says completed = {}",
            completed().await?
        );
        anyhow::ensure!(!completed().await?);
        anyhow::ensure!(lib.outbox_pending("reminders")?.is_empty());
        anyhow::Ok(())
    }
    .await;
    r::delete_list_cloudkit(&http, &root, p, &list).await?;
    println!("test list deleted");
    result
}
