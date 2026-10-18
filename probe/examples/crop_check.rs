//! The app's own avatar cut, on a saved photo and crop: `crop_check <in.jpg> <x,y,w,h> <out>`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1])?;
    let out = quince_media::avatar(&bytes, quince_media::Crop::parse(&a[2]), 256)?;
    std::fs::write(&a[3], out)?;
    Ok(())
}
