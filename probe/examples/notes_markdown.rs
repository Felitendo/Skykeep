//! Read-only: every note in a copy of the app's library written out as Markdown — checks the
//! export on real notes (no panic, every character of text kept) and prints counts only, never
//! a note's text. `notes_markdown <dir with a copy of library.db>`.
use anyhow::{anyhow, Result};
fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("a directory"))?;
    let library = quince_core::Library::open(std::path::Path::new(&dir))?;
    let (mut notes, mut styled, mut lost) = (0, 0, 0);
    let mut marks = [0usize; 6];
    let mut all = library.notes_in(None)?;
    for folder in library.note_folders()? {
        all.extend(library.notes_in(Some(&folder.guid))?);
    }
    all.sort_by(|a, b| a.guid.cmp(&b.guid));
    all.dedup_by(|a, b| a.guid == b.guid);
    for note in all {
        let Some(body) = library.note_body(&note.guid)? else {
            continue;
        };
        let runs: Vec<quince_icloud::ckdocument::Run> = library
            .note_runs(&note.guid)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        notes += 1;
        styled += usize::from(!runs.is_empty());
        let md = quince_icloud::markdown::note_to_markdown(&body, &runs);
        // Every letter and digit of the note is in what was written.
        let letters = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).count();
        if letters(&md) < letters(&body) {
            lost += 1;
        }
        for line in md.lines() {
            let t = line.trim_start();
            for (i, p) in ["# ", "## ", "- [", "- ", "```", "> "].iter().enumerate() {
                if t.starts_with(p) {
                    marks[i] += 1;
                    break;
                }
            }
        }
    }
    println!("{notes} notes ({styled} with runs); {lost} lost text");
    println!(
        "titles {} headings {} checklist {} bullets {} code fences {} quotes {}",
        marks[0], marks[1], marks[2], marks[3], marks[4], marks[5]
    );
    Ok(())
}
