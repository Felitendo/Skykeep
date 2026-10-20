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
