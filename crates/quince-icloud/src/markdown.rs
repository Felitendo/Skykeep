//! A note as Markdown, for export: its text set as Apple's attribute runs say — title and
//! headings, bulleted, dashed, numbered and checklist lines (indented as they are), monospaced
//! blocks, block quotes, bold, italic, strikethrough and links. Underline and colour have no
//! Markdown and are left as plain text; an attachment's character is left out.

use crate::ckdocument::Run;

/// Where an attachment stands in a note's text.
const ATTACHMENT: char = '\u{FFFC}';

/// One paragraph style, as `Run::style` has it.
const TITLE: u64 = 0;
const HEADING: u64 = 1;
const SUBHEADING: u64 = 2;
const MONOSPACED: u64 = 4;
const BULLETED: u64 = 100;
const DASHED: u64 = 101;
const NUMBERED: u64 = 102;
const CHECKLIST: u64 = 103;

/// `body` set as `runs` say (their lengths in UTF-16 units, as Apple counts); its first line
/// the title unless it is set otherwise (a list, say).
#[must_use]
pub fn note_to_markdown(body: &str, runs: &[Run]) -> String {
    let lines = styled_lines(body, runs);
    let mut out = String::new();
    // What the last line was, to know where a blank line goes and how a list counts.
    let mut last: Option<Kind> = None;
    let mut numbers: Vec<u64> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // The first line is the note's title, as Notes lists it and Quince's editor shows it,
        // whether or not Apple stored the Title style on it.
        let style = line.style.or((i == 0).then_some(TITLE));
        let kind = Kind::of(style, line.quote);
        if kind != Kind::Number || last != Some(Kind::Number) {
            numbers.clear();
        }
        // A blank line between blocks; none inside a list, a code block or a quote.
        if let Some(was) = last {
            let together =
                was == kind && matches!(kind, Kind::List | Kind::Number | Kind::Code | Kind::Quote);
            let to_list = matches!(
                (was, kind),
                (Kind::List | Kind::Number, Kind::List | Kind::Number)
            );
            if !together && !to_list {
                out.push('\n');
            }
        }
        if kind == Kind::Code && last != Some(Kind::Code) {
            out.push_str("```\n");
        }
        let indent = "    ".repeat(usize::try_from(line.indent).unwrap_or(0));
        let text = if kind == Kind::Code {
            line.plain()
        } else {
            line.inline()
        };
        match style {
            _ if kind == Kind::Code => out.push_str(&text),
            Some(TITLE) => push_heading(&mut out, "# ", &text),
            Some(HEADING) => push_heading(&mut out, "## ", &text),
            Some(SUBHEADING) => push_heading(&mut out, "### ", &text),
            Some(BULLETED | DASHED) => out.push_str(&format!("{indent}- {text}")),
            Some(CHECKLIST) => {
                let tick = if line.checked { "x" } else { " " };
                out.push_str(&format!("{indent}- [{tick}] {text}"));
            }
            Some(NUMBERED) => {
                let depth = usize::try_from(line.indent).unwrap_or(0);
                numbers.truncate(depth + 1);
                while numbers.len() <= depth {
                    numbers.push(0);
                }
                let n = match line.start {
                    Some(start) if numbers[depth] == 0 => start,
                    _ => numbers[depth] + 1,
                };
                numbers[depth] = n;
                out.push_str(&format!("{indent}{n}. {text}"));
            }
            _ if line.quote => out.push_str(&format!("> {text}")),
            _ => out.push_str(&text),
        }
        out.push('\n');
        if kind == Kind::Code
            && lines
                .get(i + 1)
                .is_none_or(|next| Kind::of(next.style, next.quote) != Kind::Code)
        {
            out.push_str("```\n");
        }
        last = Some(kind);
    }
    out
}

fn push_heading(out: &mut String, mark: &str, text: &str) {
    if !text.trim().is_empty() {
        out.push_str(mark);
    }
    out.push_str(text);
}

/// What a line is, as far as the blank lines between them go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    List,
    Number,
    Code,
    Quote,
}

impl Kind {
    fn of(style: Option<u64>, quote: bool) -> Self {
        match style {
            Some(MONOSPACED) => Kind::Code,
            Some(BULLETED | DASHED | CHECKLIST) => Kind::List,
            Some(NUMBERED) => Kind::Number,
            _ if quote => Kind::Quote,
            _ => Kind::Text,
        }
    }
}

/// A line of the note: its paragraph's style, and its text in pieces as they are set.
#[derive(Debug, Default)]
struct Line {
    style: Option<u64>,
    indent: u64,
    checked: bool,
    start: Option<u64>,
    quote: bool,
    pieces: Vec<(String, Inline)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Inline {
    bold: bool,
    italic: bool,
    strike: bool,
    link: Option<String>,
}

impl Line {
    fn plain(&self) -> String {
        self.pieces.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// The text with its inline marks, each piece marked on its own words (Markdown's marks
    /// do not hold across the spaces around a piece).
    fn inline(&self) -> String {
        let mut out = String::new();
        for (text, how) in &self.pieces {
            let core = text.trim();
            if core.is_empty() {
                out.push_str(&escape(text));
                continue;
            }
            let lead = &text[..text.len() - text.trim_start().len()];
            let tail = &text[text.trim_end().len()..];
            let mut s = escape(core);
            if how.strike {
                s = format!("~~{s}~~");
            }
            if how.italic {
                s = format!("*{s}*");
            }
            if how.bold {
                s = format!("**{s}**");
            }
            if let Some(url) = &how.link {
                s = format!("[{s}]({})", url.replace(' ', "%20").replace(')', "%29"));
            }
            out.push_str(lead);
            out.push_str(&s);
            out.push_str(tail);
        }
        out
    }
}

/// Characters that would otherwise be read as Markdown.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '~') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The note's lines, each with its paragraph style (from the run its first character falls
/// in, as Notes styles a paragraph) and its inline pieces.
fn styled_lines(body: &str, runs: &[Run]) -> Vec<Line> {
    // Each run's end, in UTF-16 units.
    let mut ends = Vec::with_capacity(runs.len());
    let mut at = 0u64;
    for r in runs {
        at += r.length;
        ends.push(at);
    }
    let run_at = |pos: u64| -> Option<&Run> {
        let i = ends.partition_point(|&end| end <= pos);
        runs.get(i)
    };
    let mut lines = Vec::new();
    let mut line = Line::default();
    let mut fresh = true;
    let mut pos = 0u64;
    for c in body.chars() {
        let run = run_at(pos);
        pos += c.len_utf16() as u64;
        if fresh {
            if let Some(r) = run {
                line.style = r.style;
                line.indent = r.indent;
                line.checked = r.checked == Some(true);
                line.start = r.start;
                line.quote = r.quote;
            }
            fresh = false;
        }
        if c == '\n' {
            lines.push(std::mem::take(&mut line));
            fresh = true;
            continue;
        }
        if c == ATTACHMENT {
            continue;
        }
        let how = run.map_or_else(Inline::default, |r| Inline {
            bold: r.bold,
            italic: r.italic,
            strike: r.strike,
            link: r.link.clone(),
        });
        match line.pieces.last_mut() {
            Some((text, was)) if *was == how => text.push(c),
            _ => line.pieces.push((c.to_string(), how)),
        }
    }
    if !fresh || !line.pieces.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run `n` long (in UTF-16 units) of paragraph style `style`.
    fn run(n: usize, style: Option<u64>) -> Run {
        Run {
            length: n as u64,
            style,
            ..Run::default()
        }
    }

    /// Runs for `lines`, each line (with its newline) one run of the given style.
    fn per_line(lines: &[(&str, Option<u64>)]) -> (String, Vec<Run>) {
        let body = lines.iter().map(|(t, _)| *t).collect::<Vec<_>>().join("\n");
        let runs = lines
            .iter()
            .enumerate()
            .map(|(i, (t, s))| {
                run(
                    t.encode_utf16().count() + usize::from(i + 1 < lines.len()),
                    *s,
                )
            })
            .collect();
        (body, runs)
    }

    #[test]
    fn headings_lists_and_text() {
        let (body, runs) = per_line(&[
            ("Groceries", Some(TITLE)),
            ("For the week", Some(HEADING)),
            ("Milk", Some(BULLETED)),
            ("Eggs", Some(BULLETED)),
            ("Then", None),
            ("Wash", Some(NUMBERED)),
            ("Dry", Some(NUMBERED)),
        ]);
        assert_eq!(
            note_to_markdown(&body, &runs),
            "# Groceries\n\n## For the week\n\n- Milk\n- Eggs\n\nThen\n\n1. Wash\n2. Dry\n"
        );
    }

    #[test]
    fn checklists_keep_their_ticks_and_indents() {
        let (body, mut runs) = per_line(&[
            ("Pack", Some(TITLE)),
            ("Passport", Some(CHECKLIST)),
            ("Visa", Some(CHECKLIST)),
        ]);
        runs[1].checked = Some(true);
        runs[2].indent = 1;
        assert_eq!(
            note_to_markdown(&body, &runs),
            "# Pack\n\n- [x] Passport\n    - [ ] Visa\n"
        );
    }

    #[test]
    fn inline_marks_links_and_escapes() {
        // "Read the *docs* now": "the" bold, "docs" a link, the rest plain.
        let body = "Note\nRead the 2*3 docs now";
        let mut runs = vec![run(5, Some(TITLE)), run(5, None)];
        runs.push(Run {
            length: 4,
            bold: true,
            ..Run::default()
        });
        runs.push(run(4, None));
        runs.push(Run {
            length: 4,
            link: Some("https://example.com/a b".into()),
            ..Run::default()
        });
        runs.push(run(4, None));
        assert_eq!(
            note_to_markdown(body, &runs),
            "# Note\n\nRead **the** 2\\*3 [docs](https://example.com/a%20b) now\n"
        );
    }

    #[test]
    fn code_blocks_quotes_and_numbered_starts() {
        let (body, mut runs) = per_line(&[
            ("T", Some(TITLE)),
            ("let a = 1;", Some(MONOSPACED)),
            ("let b = *a;", Some(MONOSPACED)),
            ("Wise words", None),
            ("Third", Some(NUMBERED)),
            ("Fourth", Some(NUMBERED)),
        ]);
        runs[3].quote = true;
        runs[4].start = Some(3);
        assert_eq!(
            note_to_markdown(&body, &runs),
            "# T\n\n```\nlet a = 1;\nlet b = *a;\n```\n\n> Wise words\n\n3. Third\n4. Fourth\n"
        );
    }

    #[test]
    fn no_runs_and_attachments_and_emoji() {
        assert_eq!(
            note_to_markdown("Hello 👋\nworld\u{FFFC}", &[]),
            "# Hello 👋\n\nworld\n"
        );
        // Emoji count two UTF-16 units: the run after one still lands on the right word.
        let body = "T\n👋 hi";
        let runs = vec![
            run(2, Some(TITLE)),
            run(3, None),
            Run {
                length: 2,
                bold: true,
                ..Run::default()
            },
        ];
        assert_eq!(note_to_markdown(body, &runs), "# T\n\n👋 **hi**\n");
    }

    /// Runs as the window sends them (NoteEditor's fromDoc): nulls, and fields left out.
    #[test]
    fn runs_from_the_window_are_read() {
        let runs: Vec<Run> = serde_json::from_str(
            r#"[{"length":5,"style":0,"indent":0,"checked":null,"start":null,"bold":false,"italic":false,"underline":false,"strike":false,"link":null},
                {"length":4,"style":103,"indent":0,"checked":true,"bold":false,"italic":false,"underline":false,"strike":false,"link":null,"quote":false}]"#,
        )
        .unwrap();
        assert_eq!(
            note_to_markdown("Pack\nVisa", &runs),
            "# Pack\n\n- [x] Visa\n"
        );
    }
}
