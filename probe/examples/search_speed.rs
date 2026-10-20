//! Local: how long one search across everything takes on a copy of a real library.
fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("library folder"))?,
    );
    let lib = quince_core::Library::open(&dir)?;
    for q in ["the", "a", "photo", "zzzz", "plan"] {
        let t = std::time::Instant::now();
        let found = lib.search_everything(q, 5)?;
        println!(
            "{q:>6}: {:>5.1} ms  counts {:?}",
            t.elapsed().as_secs_f64() * 1000.0,
            found.counts
        );
    }
    Ok(())
}
