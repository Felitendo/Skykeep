//! Offline: print a table's cells from an inflated attachment document on disk.
fn main() {
    let path = std::env::args().nth(1).expect("pass a file");
    let doc = std::fs::read(path).unwrap();
    for row in quince_icloud::table::cells(&doc).expect("a table") {
        println!("{row:?}");
    }
}
