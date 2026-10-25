//! Cards that are one person twice, found and put back together.
//!
//! Found the way Contacts on the iPhone finds them: the same name, or a phone number or an
//! email address two cards share — numbers compared by their digits, so "+91 90000 00001" and
//! "090000 00001" are one. Put together without losing anything: every number, address, date
//! and person from every card, a name or a job filled from whichever card has one, notes from
//! each, and a picture kept.

use super::{labels, Card, Contact, ContactMethod, PostalAddress, DETAIL_KEYS};
use serde::Serialize;
use std::collections::HashMap;

/// A number as the digits that identify it: formatting, a trunk `0` and a country code aside
/// (its last ten), an extension cut off. `None` for fewer than seven digits — too short to say
/// two cards are one person.
#[must_use]
pub fn phone_key(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let main = lower
        .split([',', ';', 'x'])
        .next()
        .unwrap_or("")
        .split("ext")
        .next()
        .unwrap_or("");
    let digits: String = main.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 7).then(|| digits[digits.len().saturating_sub(10)..].to_string())
}

/// What makes two of a card's details the same detail: a number's digits, an address's
/// letters folded, any other value folded and trimmed.
#[must_use]
pub fn method_key(m: &ContactMethod) -> String {
    let folded = crate::fold::fold(m.value.trim());
    match m.kind.as_str() {
        "phone" => format!(
            "phone:{}",
            phone_key(&m.value).unwrap_or_else(|| folded.clone())
        ),
        "email" => format!("email:{}", m.value.trim().to_lowercase()),
        "im" | "profile" => format!(
            "{}:{}:{}",
            m.kind,
            labels::meaning(m.label.as_deref()),
            folded.trim_start_matches('@')
        ),
        "related" | "date" => format!(
            "{}:{}:{}",
            m.kind,
            labels::meaning(m.label.as_deref()),
            folded
        ),
        kind => format!(
            "{kind}:{}",
            folded.split_whitespace().collect::<Vec<_>>().join(" ")
        ),
    }
}

/// A person's name as compared for duplicates: first, middle and last folded, or a company's
/// name when the card is a company. `None` when there is no name to go on.
fn name_key(c: &Contact) -> Option<String> {
    let name = if c.is_company {
        c.company.clone().unwrap_or_default()
    } else {
        [&c.first_name, &c.middle_name, &c.last_name]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let folded = crate::fold::fold(&name)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (folded.chars().count() >= 2).then_some(folded)
}

/// Whether two names could be one person's: either has none, or each word of the shorter is
/// the start of a word of the longer ("A R", "Asha R" and "Asha Rao"; never "Ravi Rao").
fn same_person(a: &Option<String>, b: &Option<String>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return true;
    };
    let (a, b): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let mut used = vec![false; long.len()];
    short.iter().all(|w| {
        let found = long
            .iter()
            .enumerate()
            .find(|(k, l)| !used[*k] && (l.starts_with(w) || w.starts_with(*l)));
        found.map(|(k, _)| used[k] = true).is_some()
    })
}

/// Cards that look like one person, and what they have in common.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Duplicates {
    /// In the address book's order.
    pub guids: Vec<String>,
    /// What they share, of `name`, `phone` and `email`.
    pub reasons: Vec<String>,
}

/// Every group of two or more cards that share a name, a number or an email address, in the
/// address book's order. Shared through a third counts: A has B's number and C's name, so all
/// three are one person.
#[must_use]
pub fn find_duplicates(book: &[Contact]) -> Vec<Duplicates> {
    let mut parent: Vec<usize> = (0..book.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let names: Vec<Option<String>> = book.iter().map(name_key).collect();
    // Each key, the first card that had it and why it counts.
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut shared: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, c) in book.iter().enumerate() {
        let mut keys: Vec<String> = Vec::new();
        if let Some(n) = name_key(c) {
            keys.push(format!("name:{n}"));
        }
        for m in &c.methods {
            match m.kind.as_str() {
                "phone" => {
                    if let Some(p) = phone_key(&m.value) {
                        keys.push(format!("phone:{p}"));
                    }
                }
                "email" => {
                    let e = m.value.trim().to_lowercase();
                    if e.contains('@') {
                        keys.push(format!("email:{e}"));
                    }
                }
                _ => {}
            }
        }
        keys.sort();
        keys.dedup();
        for key in keys {
            shared.entry(key.clone()).or_default().push(i);
            match first.get(&key) {
                // A number or an address shared by two people named differently is a home's
                // landline or an office's switchboard, not one person: Merge All made Mom and
                // Dad one card.
                Some(&j) if !key.starts_with("name:") && !same_person(&names[i], &names[j]) => {}
                Some(&j) => {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    first.insert(key, i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..book.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<(usize, Duplicates)> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .map(|mut g| {
            g.sort_unstable();
            let mut reasons: Vec<String> = shared
                .iter()
                .filter(|(_, who)| who.len() > 1 && who.iter().all(|w| g.contains(w)))
                .filter_map(|(k, _)| k.split(':').next().map(ToString::to_string))
                .collect();
            reasons.sort_by_key(|r| ["name", "phone", "email"].iter().position(|x| x == r));
            reasons.dedup();
            (
                g[0],
                Duplicates {
                    guids: g.iter().map(|&i| book[i].guid.clone()).collect(),
                    reasons,
                },
            )
        })
        .collect();
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, d)| d).collect()
}

/// How much a card holds, for choosing which of several to keep.
fn richness(c: &Card) -> usize {
    let p = &c.contact;
    let names = [
        &p.first_name,
        &p.middle_name,
        &p.last_name,
        &p.company,
        &p.birthday,
        &p.notes,
    ]
    .into_iter()
    .filter(|v| v.as_deref().is_some_and(|s| !s.trim().is_empty()))
    .count();
    let details = DETAIL_KEYS
        .iter()
        .filter(|k| c.details.get(k).is_some_and(Option::is_some))
        .count();
    names + details + p.methods.len()
}

/// Which card the others are merged into: this person's own card when it is one of them —
/// so My Card stays My Card — else one with a picture, else the one holding the most, else
/// the first.
#[must_use]
pub fn survivor(cards: &[Card], me: Option<&str>) -> usize {
    if let Some(i) = me.and_then(|me| cards.iter().position(|c| c.contact.guid == me)) {
        return i;
    }
    let mut best = 0;
    for (i, c) in cards.iter().enumerate().skip(1) {
        let rank = |c: &Card| (c.contact.photo_url.is_some(), richness(c));
        if rank(c) > rank(&cards[best]) {
            best = i;
        }
    }
    best
}

fn filled(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// Notes from each card, each once, a blank line between: none dropped, none repeated —
/// merging the merged card again adds nothing.
#[must_use]
pub fn joined_notes<'a>(notes: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    // Kept paragraph by paragraph, a paragraph dropped only when it is there already whole:
    // matched as text anywhere, "peanuts" was lost beside "Allergic to peanuts".
    let mut out = String::new();
    let mut seen: Vec<String> = Vec::new();
    for n in notes.into_iter().flatten() {
        for p in n.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
            if seen.iter().any(|s| s == p) {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(p);
            seen.push(p.to_string());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The cards as one: `survivor`'s id, version and choices, with everything the others hold
/// that it lacks. A name, a company, a birthday or a detail is the survivor's when it has one,
/// else the first other card's that does; every number, email, link, profile, date and person
/// is kept, each once (labels aside: the survivor's label stands); notes from each are joined;
/// the picture is the survivor's, else the first other card's. Addresses are merged on their
/// own ([`merged_addresses`]); the flattened address lines are left to them.
#[must_use]
pub fn merged(survivor: &Card, others: &[Card]) -> Card {
    let mut out = survivor.clone();
    let c = &mut out.contact;
    for other in others {
        let o = &other.contact;
        for (mine, theirs) in [
            (&mut c.first_name, &o.first_name),
            (&mut c.middle_name, &o.middle_name),
            (&mut c.last_name, &o.last_name),
            (&mut c.company, &o.company),
            (&mut c.birthday, &o.birthday),
        ] {
            if !filled(mine) && filled(theirs) {
                mine.clone_from(theirs);
            }
        }
        for key in DETAIL_KEYS {
            let theirs = other.details.get(key).cloned().flatten();
            if out.details.get(key).is_some_and(|v| !filled(v)) && theirs.is_some() {
                out.details.set(key, theirs.as_deref());
            }
        }
        if c.photo_url.is_none() && o.photo_url.is_some() {
            c.photo_url.clone_from(&o.photo_url);
            c.photo_signature.clone_from(&o.photo_signature);
            c.photo_crop.clone_from(&o.photo_crop);
        }
    }
    c.notes = joined_notes(
        std::iter::once(survivor.contact.notes.as_deref())
            .chain(others.iter().map(|o| o.contact.notes.as_deref())),
    );
    let mut seen: Vec<String> = c.methods.iter().map(method_key).collect();
    for other in others {
        for m in other.contact.methods.iter().filter(|m| m.kind != "address") {
            let key = method_key(m);
            if !seen.contains(&key) {
                seen.push(key);
                c.methods.push(m.clone());
            }
        }
    }
    out
}

/// Every card's addresses, each once — the same address written with different spacing or
/// case is one — the first card's first.
#[must_use]
pub fn merged_addresses(lists: &[Vec<PostalAddress>]) -> Vec<PostalAddress> {
    let key = |a: &PostalAddress| {
        crate::fold::fold(&a.formatted())
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for a in lists.iter().flatten().filter(|a| !a.is_empty()) {
        let k = key(a);
        if !seen.contains(&k) {
            seen.push(k);
            out.push(a.clone());
        }
    }
    out
}

/// A card's details that the merge would add to `survivor`, for saying so before it is done.
#[must_use]
pub fn added_count(survivor: &Card, merged: &Card) -> usize {
    let before = survivor.contact.methods.len();
    merged.contact.methods.len().saturating_sub(before)
        + DETAIL_KEYS
            .iter()
            .filter(|k| {
                survivor.details.get(k).is_some_and(|v| !filled(v))
                    && merged.details.get(k).is_some_and(filled)
            })
            .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::ContactDetails;

    /// A note is dropped only when it is already there whole, not when its words happen to
    /// appear inside another card's note: "peanuts" was lost beside "Allergic to peanuts".
    #[test]
    fn a_note_inside_another_is_kept_and_a_merged_note_adds_nothing_again() {
        assert_eq!(
            joined_notes([Some("Allergic to peanuts and shellfish"), Some("peanuts")]).as_deref(),
            Some("Allergic to peanuts and shellfish\n\npeanuts")
        );
        assert_eq!(
            joined_notes([Some("Mom's friend"), Some("Mom")]).as_deref(),
            Some("Mom's friend\n\nMom")
        );
        // Merged again with either card: nothing more.
        let once = joined_notes([Some("Gate code 4410"), Some("Lives upstairs")]).unwrap();
        assert_eq!(
            joined_notes([Some(once.as_str()), Some("Lives upstairs")]),
            Some(once.clone())
        );
        assert_eq!(
            joined_notes([Some("Gate code 4410"), Some(once.as_str())]),
            Some(once)
        );
    }

    fn person(guid: &str, first: &str, last: &str, methods: &[(&str, &str, &str)]) -> Card {
        Card {
            contact: Contact {
                guid: guid.into(),
                first_name: Some(first.into()).filter(|s: &String| !s.is_empty()),
                middle_name: None,
                photo_url: None,
                photo_signature: None,
                photo_crop: None,
                last_name: Some(last.into()).filter(|s: &String| !s.is_empty()),
                company: None,
                is_company: false,
                birthday: None,
                notes: None,
                etag: None,
                methods: methods
                    .iter()
                    .map(|(k, l, v)| ContactMethod {
                        kind: (*k).into(),
                        label: Some((*l).into()).filter(|s: &String| !s.is_empty()),
                        value: (*v).into(),
                    })
                    .collect(),
            },
            details: ContactDetails::default(),
        }
    }

    #[test]
    fn a_number_is_its_digits_however_it_was_typed() {
        assert_eq!(phone_key("+91 90000-00001"), phone_key("090000 00001"));
        assert_eq!(
            phone_key("(555) 010-0199 ext. 4"),
            phone_key("555 010 0199")
        );
        assert_eq!(phone_key("112"), None, "too short to say who it is");
    }

    #[test]
    fn cards_sharing_a_name_a_number_or_an_email_are_found_together() {
        let book: Vec<Contact> = [
            person(
                "A",
                "Zoya",
                "Ahuja",
                &[("phone", "MOBILE", "+91 90000 00001")],
            ),
            person("B", "Zoya", "", &[("phone", "", "090000 00001")]),
            person("C", "Z.", "Ahuja", &[("email", "HOME", "zoya@example.com")]),
            person(
                "D",
                "Zoe",
                "Ahuja",
                &[("email", "WORK", "ZOYA@example.com ")],
            ),
            person("E", "Ravi", "Nair", &[("phone", "", "+91 98765 43210")]),
            person("F", "Émile", "Zola", &[]),
            person("G", "EMILE", "ZOLA", &[]),
        ]
        .into_iter()
        .map(|c| c.contact)
        .collect();
        let found = find_duplicates(&book);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].guids, ["A", "B"]);
        assert_eq!(found[0].reasons, ["phone"]);
        assert_eq!(found[1].guids, ["C", "D"]);
        assert_eq!(found[1].reasons, ["email"]);
        assert_eq!(found[2].guids, ["F", "G"], "accents and case aside");
        assert_eq!(found[2].reasons, ["name"]);
    }

    #[test]
    fn a_link_through_a_third_card_joins_all_three() {
        let book: Vec<Contact> = [
            person("A", "Asha", "Rao", &[("phone", "", "+91 90000 00009")]),
            person(
                "B",
                "A",
                "R",
                &[("phone", "", "90000 00009"), ("email", "", "a@x.in")],
            ),
            person("C", "Asha R", "", &[("email", "", "A@X.IN")]),
        ]
        .into_iter()
        .map(|c| c.contact)
        .collect();
        let found = find_duplicates(&book);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].guids, ["A", "B", "C"]);
    }

    /// People who share a home's landline or an office's number are not one person: only a
    /// card with no name, or one whose name could be theirs, joins them by it.
    #[test]
    fn people_who_share_a_landline_are_not_duplicates() {
        let book: Vec<Contact> = [
            person("A", "Asha", "Rao", &[("phone", "HOME", "+1 555 010 0100")]),
            person("B", "Ravi", "Rao", &[("phone", "HOME", "555 010 0100")]),
            person("C", "", "", &[("phone", "", "(555) 010-0100")]),
            person("D", "Asha", "R", &[("phone", "", "555-010-0100")]),
        ]
        .into_iter()
        .map(|c| c.contact)
        .collect();
        let found = find_duplicates(&book);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].guids, ["A", "C", "D"]);
    }

    #[test]
    fn a_card_with_nothing_to_go_on_is_never_a_duplicate() {
        let book: Vec<Contact> = [person("A", "", "", &[]), person("B", "", "", &[])]
            .into_iter()
            .map(|c| c.contact)
            .collect();
        assert!(find_duplicates(&book).is_empty());
    }

    #[test]
    fn merging_loses_nothing_and_repeats_nothing() {
        let mut a = person(
            "A",
            "Zoya",
            "Ahuja",
            &[
                ("phone", "MOBILE", "+91 90000 00001"),
                ("related", "MOTHER", "Amma"),
            ],
        );
        a.contact.notes = Some("Met at the conference".into());
        let mut b = person(
            "B",
            "Zoya",
            "",
            &[
                ("phone", "_$!<Mobile>!$_", "090000 00001"),
                ("email", "HOME", "zoya@example.com"),
                ("date", "ANNIVERSARY", "2015-02-14"),
            ],
        );
        b.contact.notes = Some("Prefers WhatsApp".into());
        b.contact.birthday = Some("1604-03-17".into());
        b.contact.photo_url = Some("https://p/B".into());
        b.details.job_title = Some("Architect".into());
        b.details.nickname = Some("Zo".into());
        a.details.nickname = Some("Zee".into());

        let m = merged(&a, std::slice::from_ref(&b));
        assert_eq!(m.contact.guid, "A");
        assert_eq!(m.contact.last_name.as_deref(), Some("Ahuja"));
        assert_eq!(m.contact.birthday.as_deref(), Some("1604-03-17"));
        assert_eq!(m.details.job_title.as_deref(), Some("Architect"));
        assert_eq!(
            m.details.nickname.as_deref(),
            Some("Zee"),
            "the kept card's stands"
        );
        assert_eq!(
            m.contact.photo_url.as_deref(),
            Some("https://p/B"),
            "the picture kept"
        );
        assert_eq!(
            m.contact.notes.as_deref(),
            Some("Met at the conference\n\nPrefers WhatsApp")
        );
        let kinds: Vec<&str> = m.contact.methods.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["phone", "related", "email", "date"],
            "one phone, not two"
        );
        assert_eq!(added_count(&a, &m), 3);

        // Merged again — a merge sent twice — nothing more is added.
        let again = merged(&m, std::slice::from_ref(&b));
        assert_eq!(again, m);
    }

    #[test]
    fn my_card_is_kept_else_the_one_with_a_picture_else_the_fullest() {
        let a = person("A", "Zoya", "", &[]);
        let mut b = person("B", "Zoya", "Ahuja", &[("phone", "", "+91 90000 00001")]);
        let mut c = person("C", "Zoya", "", &[]);
        assert_eq!(survivor(&[a.clone(), b.clone(), c.clone()], None), 1);
        c.contact.photo_url = Some("u".into());
        assert_eq!(survivor(&[a.clone(), b.clone(), c.clone()], None), 2);
        b.contact.guid = "ME".into();
        assert_eq!(survivor(&[a, b, c], Some("ME")), 1);
    }

    #[test]
    fn the_same_address_twice_is_kept_once() {
        let home = |street: &str| PostalAddress {
            label: Some("HOME".into()),
            street: Some(street.into()),
            city: Some("Pune".into()),
            ..PostalAddress::default()
        };
        let all = merged_addresses(&[
            vec![home("2 New Rd")],
            vec![home("2  new rd"), home("9 Hill St")],
        ]);
        assert_eq!(all, [home("2 New Rd"), home("9 Hill St")]);
    }
}
