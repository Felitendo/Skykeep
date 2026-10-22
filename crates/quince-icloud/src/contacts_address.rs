//! A card's postal addresses, edited part by part.
//!
//! An edit names an address by where it sat in the list the form opened, and says what to
//! do to it: set one part, set its label, take it out, or add a new one. The edit is laid
//! over the card's addresses as Apple holds them at the moment of writing — a part changed
//! on the phone since stays changed, as a number added there does in `merge_methods` — and
//! sent with the card's current etag and the book's sync token, so Apple refuses it rather
//! than overwrite a change it has not seen.

use super::{address_wire, postal_from, startup, write, Tokens};
use crate::{Error, Result};
use quince_core::contacts::{country_code_for, PostalAddress, ADDRESS_FIELDS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One change to a card's addresses. `index` is a position in the list as it stands after
/// the edits before it in the same batch, starting from the list the form opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum AddressEdit {
    /// One part, by Apple's name for it (`street`, `city`, `postalCode`, ...). Blank clears.
    #[serde(rename_all = "camelCase")]
    SetField {
        index: usize,
        field: String,
        value: Option<String>,
    },
    /// `HOME`, `WORK` or anything typed. Blank clears.
    #[serde(rename_all = "camelCase")]
    SetLabel {
        index: usize,
        label: Option<String>,
    },
    Add {
        address: PostalAddress,
    },
    Remove {
        index: usize,
    },
}

/// What an edit did to one address of the list the form opened.
// A handful at a time, built and dropped within one write: boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressChange {
    Edited {
        base: PostalAddress,
        mine: PostalAddress,
    },
    Removed(PostalAddress),
    Added(PostalAddress),
}

/// The list after `edits`, and what changed from `base`. Refused when an edit names an
/// address or a part that is not there, before anything is sent.
///
/// # Errors
/// [`Error::Refused`] for an index past the end of the list or a part Apple does not have.
pub fn apply_edits(
    base: &[PostalAddress],
    edits: &[AddressEdit],
) -> Result<(Vec<PostalAddress>, Vec<AddressChange>)> {
    // Each address with where it came from in `base`, so an edit stays an edit however the
    // batch moves things around.
    let mut list: Vec<(Option<usize>, PostalAddress)> = base
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, a)| (Some(i), a))
        .collect();
    let missing = || Error::Refused("That address is no longer on this card.".into());
    for edit in edits {
        match edit {
            AddressEdit::SetField {
                index,
                field,
                value,
            } => {
                let (_, address) = list.get_mut(*index).ok_or_else(missing)?;
                if !address.set_field(field, value.as_deref()) {
                    return Err(Error::Refused(format!("An address has no part {field:?}.")));
                }
                // A country changed by name takes its code along, so Apple lays it out as the
                // new country rather than the old one; one this app has no code for drops a
                // code that would now be wrong.
                if field == "country"
                    && !edits.iter().any(|e| {
                        matches!(e, AddressEdit::SetField { index: i, field: f, .. }
                            if i == index && f == "countryCode")
                    })
                {
                    let code = address.country.as_deref().and_then(country_code_for);
                    address.country_code = code.map(ToString::to_string);
                }
            }
            AddressEdit::SetLabel { index, label } => {
                let (_, address) = list.get_mut(*index).ok_or_else(missing)?;
                address.label = label
                    .as_deref()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(ToString::to_string);
            }
            AddressEdit::Add { address } => {
                // Through `set_field`, so what is stored is trimmed as a read would store it.
                let mut clean = PostalAddress::default();
                for key in ADDRESS_FIELDS {
                    clean.set_field(key, address.field(key).and_then(Option::as_deref));
                }
                clean.label = address.label.clone();
                if clean.country_code.is_none() {
                    clean.country_code = clean
                        .country
                        .as_deref()
                        .and_then(country_code_for)
                        .map(ToString::to_string);
                }
                list.push((None, clean));
            }
            AddressEdit::Remove { index } => {
                if *index >= list.len() {
                    return Err(missing());
                }
                list.remove(*index);
            }
        }
    }

    let mut changes = Vec::new();
    for (i, b) in base.iter().enumerate() {
        match list.iter().find(|(from, _)| *from == Some(i)) {
            None => changes.push(AddressChange::Removed(b.clone())),
            // Emptied of every part is taken out: Apple keeps no blank address.
            Some((_, m)) if m.is_empty() => changes.push(AddressChange::Removed(b.clone())),
            Some((_, m)) if m != b => changes.push(AddressChange::Edited {
                base: b.clone(),
                mine: m.clone(),
            }),
            Some(_) => {}
        }
    }
    for (from, m) in &list {
        if from.is_none() && !m.is_empty() {
            changes.push(AddressChange::Added(m.clone()));
        }
    }
    let after = list
        .into_iter()
        .map(|(_, a)| a)
        .filter(|a| !a.is_empty())
        .collect();
    Ok((after, changes))
}

/// Apple's entry for an address: its label, and every part that holds something.
pub(super) fn entry(address: &PostalAddress) -> Value {
    let mut field = Map::new();
    for key in ADDRESS_FIELDS {
        if let Some(Some(v)) = address.field(key) {
            field.insert(address_wire(key).into(), Value::String(v.clone()));
        }
    }
    let mut e = Map::new();
    if let Some(label) = &address.label {
        e.insert("label".into(), Value::String(label.clone()));
    }
    e.insert("field".into(), Value::Object(field));
    Value::Object(e)
}

/// How many parts two addresses share, and how many `a` has.
fn shared(a: &PostalAddress, b: &PostalAddress) -> (usize, usize) {
    let mut same = 0;
    let mut has = 0;
    for key in ADDRESS_FIELDS {
        if let Some(Some(v)) = a.field(key) {
            has += 1;
            if b.field(key).and_then(Option::as_ref) == Some(v) {
                same += 1;
            }
        }
    }
    (same, has)
}

/// Which of Apple's entries is the address the form opened as `base`: the one equal to it,
/// or else — changed on another device since — the one sharing the most of its parts, as
/// long as that is at least half of them. Entries already claimed are skipped.
fn find(theirs: &[Option<PostalAddress>], claimed: &[bool], base: &PostalAddress) -> Option<usize> {
    let open = |i: &usize| !claimed[*i];
    if let Some(i) = (0..theirs.len())
        .filter(open)
        .find(|&i| theirs[i].as_ref() == Some(base))
    {
        return Some(i);
    }
    (0..theirs.len())
        .filter(open)
        .filter_map(|i| {
            let t = theirs[i].as_ref()?;
            let (same, has) = shared(base, t);
            let label = u8::from(t.label == base.label);
            (same > 0 && same * 2 >= has).then_some((same, label, i))
        })
        // Most parts in common, then the same label, then the earliest.
        .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(b.2.cmp(&a.2)))
        .map(|(_, _, i)| i)
}

/// Apple's current `streetAddresses` with `changes` laid over it. An edited address has only
/// the parts (and label) the form changed written onto Apple's entry; every other part keeps
/// Apple's value now, and every key this app does not model stays as Apple sent it. Entries
/// the form never touched stay exactly as they are. An address edited here but gone at Apple
/// is added back, as an edited number is.
#[must_use]
pub fn merge_addresses(theirs: &[Value], changes: &[AddressChange]) -> Value {
    let parsed: Vec<Option<PostalAddress>> = theirs.iter().map(postal_from).collect();
    let mut claimed = vec![false; theirs.len()];
    let mut out: Vec<Option<Value>> = theirs.iter().cloned().map(Some).collect();
    let mut appended: Vec<Value> = Vec::new();

    for change in changes {
        match change {
            AddressChange::Removed(base) => {
                if let Some(i) = find(&parsed, &claimed, base) {
                    claimed[i] = true;
                    out[i] = None;
                }
            }
            AddressChange::Edited { base, mine } => {
                let Some(i) = find(&parsed, &claimed, base) else {
                    appended.push(entry(mine));
                    continue;
                };
                claimed[i] = true;
                let mut e = match out[i].take() {
                    Some(Value::Object(e)) => e,
                    _ => Map::new(),
                };
                // A field Apple sent as a plain string becomes parts.
                let mut field = match e.remove("field") {
                    Some(Value::Object(f)) => f,
                    Some(Value::String(text)) if !text.trim().is_empty() => {
                        let mut f = Map::new();
                        f.insert("street".into(), Value::String(text));
                        f
                    }
                    _ => Map::new(),
                };
                for key in ADDRESS_FIELDS {
                    let (was, now) = (base.field(key), mine.field(key));
                    if was == now {
                        continue;
                    }
                    // Under Apple's spelling of the part, whichever one this entry used.
                    let wire = if field.contains_key(key) {
                        key
                    } else {
                        address_wire(key)
                    };
                    match now.and_then(Option::as_ref) {
                        Some(v) => {
                            field.insert(wire.into(), Value::String(v.clone()));
                        }
                        None => {
                            field.remove(wire);
                        }
                    }
                }
                if base.label != mine.label {
                    match &mine.label {
                        Some(l) => {
                            e.insert("label".into(), Value::String(l.clone()));
                        }
                        None => {
                            e.remove("label");
                        }
                    }
                }
                e.insert("field".into(), Value::Object(field));
                out[i] = Some(Value::Object(e));
            }
            AddressChange::Added(mine) => {
                let already = parsed.iter().any(|t| t.as_ref() == Some(mine));
                if !already {
                    appended.push(entry(mine));
                }
            }
        }
    }
    Value::Array(out.into_iter().flatten().chain(appended).collect())
}

/// `edits`, made against `base`, laid over `current` the way a write lays them over Apple's
/// card: what changed since `base` elsewhere stays, and only what the edits touch moves. What a
/// queued edit shows while it waits, and what it shows again over each sync pass.
///
/// # Errors
/// [`Error::Refused`] when an edit names an address or part `base` does not have.
pub fn laid_over(
    current: &[PostalAddress],
    base: &[PostalAddress],
    edits: &[AddressEdit],
) -> Result<Vec<PostalAddress>> {
    let (_, changes) = apply_edits(base, edits)?;
    let theirs: Vec<Value> = current.iter().map(entry).collect();
    Ok(match merge_addresses(&theirs, &changes) {
        Value::Array(list) => list.iter().filter_map(postal_from).collect(),
        _ => Vec::new(),
    })
}

/// Write `edits` to the card `guid`, over its addresses as Apple holds them now. `base` is
/// the list the edits were made against. Answers the card's addresses as written.
///
/// A write that loses to one made elsewhere in between is read again and laid over again,
/// up to three times.
///
/// # Errors
/// [`Error::Refused`] for an edit that names nothing, or a card Apple does not have;
/// [`Error::Conflict`] when it keeps losing; otherwise when Apple cannot be reached.
pub async fn edit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    base: &[PostalAddress],
    edits: &[AddressEdit],
) -> Result<Vec<PostalAddress>> {
    let (after, changes) = apply_edits(base, edits)?;
    if changes.is_empty() {
        return Ok(after);
    }
    let mut attempt = 0;
    loop {
        attempt += 1;
        let body = startup(http, root, params.clone()).await?;
        let token = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
        };
        let tokens = Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        };
        let Some(card) = body
            .get("contacts")
            .and_then(Value::as_array)
            .and_then(|cards| {
                cards
                    .iter()
                    .find(|c| c.get("contactId").and_then(Value::as_str) == Some(guid))
            })
            .cloned()
        else {
            return Err(Error::Refused(
                "Apple doesn't have this contact (yet, or any more), so the address wasn't saved."
                    .into(),
            ));
        };
        let card = with_addresses(card, &changes);
        let written = card
            .get("streetAddresses")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(postal_from).collect())
            .unwrap_or_default();
        match write(
            http,
            root,
            params.clone(),
            &tokens,
            reqwest::Method::PUT,
            card,
        )
        .await
        {
            Ok(_) => return Ok(written),
            Err(Error::Conflict) if attempt < 3 => {}
            Err(e) => return Err(e),
        }
    }
}

/// Apple's card with its addresses merged and everything else — its etag included, so the
/// write is against the version just read — as Apple sent it.
fn with_addresses(mut card: Value, changes: &[AddressChange]) -> Value {
    let theirs = card
        .get("streetAddresses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(object) = card.as_object_mut() {
        object.insert("streetAddresses".into(), merge_addresses(&theirs, changes));
    }
    card
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn home() -> PostalAddress {
        PostalAddress {
            label: Some("HOME".into()),
            street: Some("12 Example Street\nFlat 4".into()),
            city: Some("Springfield".into()),
            state: Some("ST".into()),
            postal_code: Some("123456".into()),
            country: Some("India".into()),
            ..PostalAddress::default()
        }
    }

    fn apple_home() -> Value {
        json!({
            "label": "HOME",
            "field": {
                "street": "12 Example Street\nFlat 4",
                "city": "Springfield",
                "state": "ST",
                "postalCode": "123456",
                "country": "India"
            }
        })
    }

    #[test]
    fn apples_address_reads_part_by_part() {
        let body = json!({ "contacts": [
            { "contactId": "C1", "streetAddresses": [apple_home(), { "field": {} }] },
            { "contactId": "C2" },
            { "streetAddresses": [apple_home()] }
        ]});
        let book = super::super::parse_addresses(&body);
        assert_eq!(
            book,
            [("C1".to_string(), vec![home()]), ("C2".to_string(), vec![])],
            "a blank address and a card with no id are left out"
        );
        let typed =
            postal_from(&json!({ "field": { "countryCode": "de", "subLocality": "Mitte" } }))
                .unwrap();
        assert_eq!(typed.country_code.as_deref(), Some("de"));
        assert_eq!(typed.sub_locality.as_deref(), Some("Mitte"));
    }

    /// The form changed the postcode while the phone changed the town: both stand, and the
    /// key this app does not model is sent back as Apple had it.
    #[test]
    fn a_part_changed_on_the_phone_survives_an_edit_of_another() {
        let (_, changes) = apply_edits(
            &[home()],
            &[AddressEdit::SetField {
                index: 0,
                field: "postalCode".into(),
                value: Some("654321".into()),
            }],
        )
        .unwrap();
        let mut theirs = apple_home();
        theirs["field"]["city"] = json!("Shelbyville");
        theirs["field"]["x-unmodelled"] = json!("kept");
        let other = json!({ "label": "WORK", "field": { "city": "Capital City" } });
        let merged = merge_addresses(&[theirs, other.clone()], &changes);
        let list = merged.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["field"]["postalCode"], "654321", "the edit here");
        assert_eq!(
            list[0]["field"]["city"], "Shelbyville",
            "the phone's change"
        );
        assert_eq!(list[0]["field"]["x-unmodelled"], "kept");
        assert_eq!(list[0]["label"], "HOME");
        assert_eq!(list[1], other, "an address not edited is untouched");
    }

    #[test]
    fn adding_removing_and_relabelling_lay_over_apples_list() {
        let work = PostalAddress {
            label: Some("WORK".into()),
            city: Some("Capital City".into()),
            ..PostalAddress::default()
        };
        let (after, changes) = apply_edits(
            &[home(), work.clone()],
            &[
                AddressEdit::Remove { index: 0 },
                // Index 0 is now the work address.
                AddressEdit::SetLabel {
                    index: 0,
                    label: Some("OFFICE".into()),
                },
                AddressEdit::Add {
                    address: PostalAddress {
                        city: Some(" Berlin ".into()),
                        country: Some("Germany".into()),
                        ..PostalAddress::default()
                    },
                },
            ],
        )
        .unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[1].city.as_deref(), Some("Berlin"), "trimmed");
        assert_eq!(after[1].country_code.as_deref(), Some("de"), "coded");

        // The phone added a third address meanwhile.
        let phones = json!({ "label": "OTHER", "field": { "city": "Ogdenville" } });
        let merged = merge_addresses(
            &[
                apple_home(),
                json!({ "label": "WORK", "field": { "city": "Capital City" } }),
                phones.clone(),
            ],
            &changes,
        );
        let list = merged.as_array().unwrap();
        let labels: Vec<&str> = list
            .iter()
            .map(|e| e["label"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(labels, ["OFFICE", "OTHER", ""]);
        assert_eq!(list[1], phones);
        assert_eq!(list[2]["field"]["city"], "Berlin");
        assert_eq!(list[2]["field"]["countryCode"], "de");
    }

    #[test]
    fn a_cleared_part_is_taken_off_and_a_new_country_brings_its_code() {
        let mut coded = home();
        coded.country_code = Some("in".into());
        let (after, changes) = apply_edits(
            &[coded],
            &[
                AddressEdit::SetField {
                    index: 0,
                    field: "state".into(),
                    value: Some("  ".into()),
                },
                AddressEdit::SetField {
                    index: 0,
                    field: "country".into(),
                    value: Some("France".into()),
                },
            ],
        )
        .unwrap();
        assert_eq!(after[0].country_code.as_deref(), Some("fr"));
        let mut theirs = apple_home();
        theirs["field"]["countryCode"] = json!("in");
        let merged = merge_addresses(&[theirs], &changes);
        let field = &merged[0]["field"];
        assert!(field.get("state").is_none(), "cleared, not sent blank");
        assert_eq!(field["country"], "France");
        assert_eq!(field["countryCode"], "fr");
    }

    #[test]
    fn an_edit_naming_nothing_is_refused_before_anything_is_sent() {
        let bad_index = apply_edits(&[home()], &[AddressEdit::Remove { index: 1 }]);
        assert!(matches!(bad_index, Err(Error::Refused(_))));
        let bad_field = apply_edits(
            &[home()],
            &[AddressEdit::SetField {
                index: 0,
                field: "planet".into(),
                value: None,
            }],
        );
        assert!(matches!(bad_field, Err(Error::Refused(_))));
        let (_, none) = apply_edits(&[home()], &[]).unwrap();
        assert!(none.is_empty());
    }

    /// Edited here, deleted on the phone: the edit comes back as an address, as an edited
    /// number does. Removed here and already gone there: nothing to do.
    #[test]
    fn an_address_gone_at_apple_comes_back_only_if_edited_here() {
        let (_, edited) = apply_edits(
            &[home()],
            &[AddressEdit::SetField {
                index: 0,
                field: "city".into(),
                value: Some("Pune".into()),
            }],
        )
        .unwrap();
        let merged = merge_addresses(&[], &edited);
        assert_eq!(merged[0]["field"]["city"], "Pune");
        let (_, removed) = apply_edits(&[home()], &[AddressEdit::Remove { index: 0 }]).unwrap();
        assert_eq!(merge_addresses(&[], &removed), json!([]));
    }

    #[test]
    fn the_card_keeps_its_etag_and_everything_else() {
        let (_, changes) = apply_edits(
            &[home()],
            &[AddressEdit::SetField {
                index: 0,
                field: "city".into(),
                value: Some("Pune".into()),
            }],
        )
        .unwrap();
        let card = with_addresses(
            json!({ "contactId": "C1", "etag": "e9", "phones": [{ "field": "+1" }],
                    "streetAddresses": [apple_home()] }),
            &changes,
        );
        assert_eq!(card["etag"], "e9");
        assert_eq!(card["phones"][0]["field"], "+1");
        assert_eq!(card["streetAddresses"][0]["field"]["city"], "Pune");
    }

    #[test]
    fn an_edit_reads_back_as_json_the_window_sends() {
        let edits: Vec<AddressEdit> = serde_json::from_value(json!([
            { "op": "setField", "index": 0, "field": "postalCode", "value": "1" },
            { "op": "setLabel", "index": 0, "label": null },
            { "op": "add", "address": { "city": "Pune" } },
            { "op": "remove", "index": 1 }
        ]))
        .unwrap();
        assert_eq!(edits.len(), 4);
    }

    /// A queued edit shows over whatever a pass last read: a part changed on the phone since
    /// stays, the part edited here wins, and laying it twice changes nothing more.
    #[test]
    fn a_queued_edit_lays_over_what_a_pass_read() {
        let base = vec![home()];
        let edits = vec![
            AddressEdit::SetField {
                index: 0,
                field: "city".into(),
                value: Some("Pune".into()),
            },
            AddressEdit::Add {
                address: PostalAddress {
                    label: Some("WORK".into()),
                    city: Some("Mumbai".into()),
                    ..PostalAddress::default()
                },
            },
        ];
        let mut phone = home();
        phone.postal_code = Some("654321".into());
        let once = laid_over(&[phone], &base, &edits).unwrap();
        assert_eq!(once.len(), 2);
        assert_eq!(once[0].city.as_deref(), Some("Pune"));
        assert_eq!(once[0].postal_code.as_deref(), Some("654321"));
        assert_eq!(once[1].city.as_deref(), Some("Mumbai"));
        assert_eq!(laid_over(&once, &base, &edits).unwrap(), once);
        // A brand-new card: nothing at Apple yet, the additions are the list.
        let fresh = laid_over(&[], &[], &edits[1..]).unwrap();
        assert_eq!(fresh.len(), 1);
    }
}
