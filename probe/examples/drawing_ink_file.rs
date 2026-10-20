//! Local: a drawing's fallback image made its ink for a light and a dark page
//! (quince_media::drawing_ink), written beside it — to look at the result.
fn main() -> anyhow::Result<()> {
    let input = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("png"))?,
    );
    let bytes = std::fs::read(&input)?;
    for (dark, name) in [(false, "ink-light.png"), (true, "ink-dark.png")] {
        let t = std::time::Instant::now();
        let out = quince_media::drawing_ink(&bytes, dark).map_err(|e| anyhow::anyhow!(e))?;
        std::fs::write(input.with_file_name(name), &out)?;
        println!(
            "{name}: {} bytes in {} ms",
            out.len(),
            t.elapsed().as_millis()
        );
    }
    Ok(())
}
