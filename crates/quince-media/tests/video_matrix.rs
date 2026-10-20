//! A film's turn read from its own header: the corners of each test film are coloured (red
//! top left, green top right, blue bottom left), and the matrices are the ones that draw them
//! as the film's maker meant — checked against WebKitGTK's own drawing of the same files.
use std::path::Path;

fn matrix(name: &str) -> Option<[i32; 4]> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    quince_media::video_matrix(&path).unwrap().map(|(_, m)| m)
}

#[test]
fn upright_turned_and_mirrored() {
    assert_eq!(matrix("base.mp4"), Some([1, 0, 0, 1]), "as recorded");
    assert_eq!(
        matrix("turn.mp4"),
        Some([0, 1, -1, 0]),
        "a quarter turn, as a phone held upright records"
    );
    assert_eq!(
        matrix("flipturn.mp4"),
        Some([0, 1, 1, 0]),
        "a quarter turn and a mirror, as the front camera records"
    );
}

#[test]
fn the_matrix_is_found_where_it_is() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/turn.mp4");
    let (at, _) = quince_media::video_matrix(&path).unwrap().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let a = i32::from_be_bytes(bytes[at as usize..at as usize + 4].try_into().unwrap());
    let b = i32::from_be_bytes(bytes[at as usize + 4..at as usize + 8].try_into().unwrap());
    assert_eq!((a, b), (0, 0x10000), "its first two words are a and b");
}

/// The thumbnail cache's own rules: a path's URI escaped as GLib escapes it, and a PNG that
/// carries that URI and the file's time.
#[test]
fn freedesktop_thumbnails() {
    assert_eq!(
        quince_media::file_uri(Path::new("/home/a b/Résumé (v2)+x;y#z.txt")),
        "file:///home/a%20b/R%C3%A9sum%C3%A9%20(v2)+x%3By%23z.txt"
    );
    let mut picture = image::RgbImage::new(400, 200);
    for (x, _, p) in picture.enumerate_pixels_mut() {
        *p = image::Rgb([(x % 255) as u8, 80, 200]);
    }
    let mut jpeg = std::io::Cursor::new(Vec::new());
    picture
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    let png =
        quince_media::freedesktop_thumbnail(jpeg.get_ref(), 128, "file:///x.jpg", 1_700_000_100)
            .unwrap();
    let back = image::load_from_memory(&png).unwrap();
    assert_eq!(
        (back.width(), back.height()),
        (128, 64),
        "no larger than 128 on its longer side"
    );
    assert_eq!(quince_media::thumbnail_mtime(&png), Some(1_700_000_100));
}
