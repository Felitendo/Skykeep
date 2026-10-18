//! A table made on a phone and never typed in reads as the 2×2 of empty cells Notes shows.
#[test]
fn an_empty_table_reads_as_two_by_two() {
    let doc = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/table-empty-2x2.bin"
    ))
    .unwrap();
    let cells = quince_icloud::table::cells(&doc).expect("a table");
    assert_eq!(
        cells,
        vec![
            vec![String::new(), String::new()],
            vec![String::new(), String::new()]
        ]
    );
}
