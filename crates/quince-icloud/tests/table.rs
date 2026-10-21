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

/// The same table as a newer phone keeps one: an empty version first, the table in the next
/// (measured on a table in his notes, 2026-10-21). Read, and written, in the version that holds
/// it — the empty one left exactly as it was.
#[test]
fn a_table_after_an_empty_version_is_read_and_written_where_it_lives() {
    use quince_icloud::table::{cells, with_cell, with_column, with_row, without_row};
    let one = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/table-empty-2x2.bin"
    ))
    .unwrap();
    // `08 00` (field 1), then the empty version `12 06 08 00 10 00 1a 00`, then the rest.
    assert_eq!(&one[..2], &[0x08, 0x00]);
    let empty = [0x12, 0x06, 0x08, 0x00, 0x10, 0x00, 0x1a, 0x00];
    let two = [&one[..2], &empty[..], &one[2..]].concat();
    let blank = vec![vec![String::new(), String::new()]; 2];
    assert_eq!(cells(&two), Some(blank));

    let typed = with_cell(&two, 0, 1, "Hi").expect("a cell written in the live version");
    assert_eq!(
        &typed[..10],
        &two[..10],
        "the empty version is kept as it was"
    );
    assert_eq!(cells(&typed).unwrap()[0][1], "Hi");

    let grown = with_row(&typed, 2).expect("a row added in the live version");
    assert_eq!(&grown[..10], &two[..10]);
    assert_eq!(cells(&grown).unwrap().len(), 3);
    let wider = with_column(&grown, 0).expect("a column added");
    assert_eq!(cells(&wider).unwrap()[0], vec!["", "", "Hi"]);
    let back = without_row(&wider, 2).expect("a row removed");
    assert_eq!(cells(&back).unwrap().len(), 2);
    assert_eq!(&back[..10], &two[..10]);
}
