//! Comparing text the way a person reads it rather than the way it is encoded.
//!
//! SQLite folds case for ASCII alone, and byte order puts "Émile" after "Zoya". Apple's apps
//! search and sort without regard to case or accents, so every search and every name-sorted
//! view goes through here instead.

use std::cmp::Ordering;

/// Accents that sit on a letter without changing which letter it is.
///
/// Named ranges rather than every combining mark: the vowel signs of Devanagari, Tamil and
/// the rest are combining marks too, and dropping them turns one word into another.
fn is_accent(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// `text` with case and accents gone, so "Émile", "EMILE" and "emile" are one word.
#[must_use]
pub fn fold(text: &str) -> String {
    icu_normalizer::DecomposingNormalizerBorrowed::new_nfd()
        .normalize(text)
        .chars()
        .filter(|&c| !is_accent(c))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The words of a query, folded; empty when there is nothing to look for.
#[must_use]
pub fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(fold).collect()
}

/// True when every word appears somewhere in `haystack`, which must already be folded.
#[must_use]
pub fn holds_all(haystack: &str, words: &[String]) -> bool {
    words.iter().all(|w| haystack.contains(w.as_str()))
}

/// Names in the order Finder lists them: case and accents ignored, and runs of digits
/// compared as numbers, so "Scan 2" comes before "Scan 10".
#[must_use]
pub fn natural(a: &str, b: &str) -> Ordering {
    let (a, b) = (fold(a), fold(b));
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let m: String = std::iter::from_fn(|| x.next_if(char::is_ascii_digit)).collect();
                let n: String = std::iter::from_fn(|| y.next_if(char::is_ascii_digit)).collect();
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                // More digits is a bigger number; equal lengths compare digit by digit. No
                // parsing, so a forty-digit run in a file name cannot overflow anything.
                let order = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(p), Some(q)) => {
                if p != q {
                    return p.cmp(&q);
                }
                x.next();
                y.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_and_accents_fold_away() {
        assert_eq!(fold("Émile"), "emile");
        assert_eq!(fold("ZOË"), "zoe");
        assert_eq!(fold("Straße"), "straße");
        // Composed and decomposed spellings of the same name are the same name.
        assert_eq!(fold("Jose\u{301}"), fold("José"));
    }

    #[test]
    fn vowel_signs_are_not_accents() {
        // "कि" and "क" are different syllables; folding one into the other would be wrong.
        assert_ne!(fold("किताब"), fold("कताब"));
        assert_eq!(fold("किताब"), "किताब");
    }

    #[test]
    fn numbers_in_names_sort_as_numbers() {
        let mut names = vec!["Scan 10", "scan 2", "Scan 1", "Éclair", "apple", "Zebra"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(
            names,
            ["apple", "Éclair", "Scan 1", "scan 2", "Scan 10", "Zebra"]
        );
    }

    #[test]
    fn long_digit_runs_do_not_overflow() {
        let big = format!("IMG {}", "9".repeat(40));
        assert_eq!(natural("IMG 1", &big), Ordering::Less);
        assert_eq!(natural("IMG 007", "IMG 7"), Ordering::Equal);
    }
}
