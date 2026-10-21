//! Read-only: a subscription fetched for real through `listed_only`, the way Calendar's New
//! Subscription does — Apple's own public holiday feed by its webcal:// address, so nothing but
//! Apple is contacted — then read and its 2026 events laid out. Also shows an unlisted host
//! refused. Prints counts and the first few titles only.
use anyhow::Result;
#[tokio::main]
async fn main() -> Result<()> {
    use quince_icloud::calendar_feeds as feeds;
    let url = feeds::normalise_url("webcal://calendars.icloud.com/holidays/in_en.ics")?;
    let host = feeds::host_of(url.as_str()).unwrap();
    let http = quince_auth::listed_only(std::slice::from_ref(&host)).build()?;
    let text = feeds::fetch(&http, url.as_str()).await?;
    let feed = feeds::parse(&text, &jiff::tz::TimeZone::system())?;
    let from = jiff::civil::date(2026, 1, 1)
        .to_zoned(jiff::tz::TimeZone::UTC)?
        .timestamp()
        .as_second();
    let to = jiff::civil::date(2027, 1, 1)
        .to_zoned(jiff::tz::TimeZone::UTC)?
        .timestamp()
        .as_second();
    let events = feeds::events(&feed, "quince:sub:probe", from, to);
    println!(
        "host={host} bytes={} name={:?} items={} events2026={}",
        text.len(),
        feed.name,
        feed.len(),
        events.len()
    );
    for e in events.iter().take(3) {
        println!("  {} all_day={} {}", e.title, e.all_day, e.starts_at);
    }
    let refused = http.get("https://www.apple.com/").send().await;
    println!("unlisted host refused: {}", refused.is_err());
    Ok(())
}
