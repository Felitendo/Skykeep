//! Offline: a row added to saved copies of his tables, in memory only, never sent. For each
//! table, a row at the top, the middle and the end must read back as the same grid with an
//! empty row there, and its first cell must take text.
fn main() {
    for path in std::env::args().skip(1) {
        let doc = std::fs::read(&path).expect("read");
        let name = std::path::Path::new(&path)
            .file_stem()
            .unwrap()
            .to_string_lossy()[..8]
            .to_string();
        let Some(grid) = quince_icloud::table::cells(&doc) else {
            println!("{name}: not a table this reads");
            continue;
        };
        let width = grid.first().map_or(0, Vec::len);
        for at in [0, width / 2, width] {
            let out = quince_icloud::table::with_column(&doc, at);
            let typed = out
                .as_deref()
                .and_then(|o| quince_icloud::table::with_cell(o, 0, at, "col"));
            let back = typed.as_deref().and_then(quince_icloud::table::cells);
            println!(
                "{name}: column at {at} {}; typing in it: {}",
                if out.is_some() { "ok" } else { "REFUSED" },
                match back {
                    Some(b) if b[0][at] == "col" && b.iter().all(|r| r.len() == width + 1) =>
                        "reads back",
                    Some(_) => "WRONG",
                    None => "REFUSED",
                }
            );
        }
        // The new top row saved beside the table, for its merge records to be read.
        if let (Some(out), Ok(dir)) = (
            quince_icloud::table::with_row(&doc, 0),
            std::env::var("QUINCE_DUMP"),
        ) {
            std::fs::write(format!("{dir}/{name}-row0.bin"), out).ok();
        }
        if let (Some(out), Ok(dir)) = (
            quince_icloud::table::without_row(&doc, 0),
            std::env::var("QUINCE_DUMP"),
        ) {
            std::fs::write(format!("{dir}/{name}-del0.bin"), out).ok();
        }
        for at in [0, grid.len() / 2, grid.len()] {
            match quince_icloud::table::with_row(&doc, at) {
                None => println!(
                    "{name} {}x{}: row at {at} REFUSED",
                    grid.len(),
                    grid.first().map_or(0, Vec::len)
                ),
                Some(out) => {
                    let typed = quince_icloud::table::with_cell(&out, at, 0, "new");
                    let back = typed.as_deref().and_then(quince_icloud::table::cells);
                    println!(
                        "{name} {}x{}: row at {at} ok ({} → {} bytes); typing in it: {}",
                        grid.len(),
                        grid.first().map_or(0, Vec::len),
                        doc.len(),
                        out.len(),
                        match back {
                            Some(b) if b[at][0] == "new" => "reads back",
                            Some(_) => "WRONG",
                            None => "REFUSED",
                        }
                    );
                }
            }
        }
    }
}
