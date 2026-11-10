//! Duplicate cards, found and merged.
//!
//! A merge is a change like any other made here (outbox.rs): shown at once — one card where
//! there were several, in every list any of them was in — and sent behind, offline or not.
//! It waits [`HOLD`] before it goes, and until it goes it can be undone: taken off the queue,
//! and every card put back as it was. Sent, it is done on Apple's own cards
//! (`quince_icloud::contacts::merge`), each step safe to take twice.

use crate::outbox::{self, CONTACTS};
use crate::{Data, UiResult};
use quince_core::contacts::merge::{self as core_merge, Duplicates};
use quince_core::contacts::{Card, Contact, ContactDetails, PostalAddress};
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::contacts::{self as c, merge as apple};
use quince_icloud::Error;
use quince_media::{photo_for_apple as encode_photo, APPLE_PHOTO_LIMIT as PHOTO_LIMIT};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;

/// How long a merge waits before it is sent: the time its Undo is on screen.
const HOLD: i64 = 10;

/// One card as it was before a merge, whole.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub card: Card,
    pub addresses: Vec<PostalAddress>,
}

/// A merge made here: the card it leaves, and everything needed to send it or undo it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContactMerge {
    /// The kept card as merged — what is shown until Apple has it. Named `contact` as a save
    /// names its card, so a notice about it can name the person.
    pub contact: Contact,
    pub details: ContactDetails,
    pub addresses: Vec<PostalAddress>,
    /// The kept card as it was.
    pub before: Kept,
    /// The cards merged into it, as they were.
    pub absorbed: Vec<Kept>,
    /// The lists as they stood, to put back on an undo.
    pub lists_before: String,
    /// Not sent before this moment (seconds since the epoch).
    pub hold_until: i64,
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn json<T: Serialize>(v: &T) -> quince_core::Result<String> {
    serde_json::to_string(v)
        .map_err(|e| quince_core::Error::Io(std::io::Error::other(e.to_string())))
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The lists with `kept` in every one any of `absorbed` was in, and none of `absorbed` left.
fn lists_after(lists: &str, kept: &str, absorbed: &[String]) -> String {
    let mut all: Vec<c::ContactList> = serde_json::from_str(lists).unwrap_or_default();
    for l in &mut all {
        let had = l
            .members
            .iter()
            .any(|m| absorbed.iter().any(|a| same(a, m)));
        l.members.retain(|m| !absorbed.iter().any(|a| same(a, m)));
        if had && !l.members.iter().any(|m| same(m, kept)) {
            l.members.push(kept.to_string());
        }
    }
    serde_json::to_string(&all).unwrap_or_else(|_| lists.to_string())
}

/// Lay a merge over the library: one card where there were several, in all their lists.
pub fn lay(l: &mut Library, m: &ContactMerge) -> quince_core::Result<()> {
    let kept = m.contact.guid.clone();
    let absorbed: Vec<String> = m
        .absorbed
        .iter()
        .map(|a| a.card.contact.guid.clone())
        .collect();
    for guid in &absorbed {
        l.forget_contact(guid)?;
    }
    l.put_card(&Card {
        contact: m.contact.clone(),
        details: m.details.clone(),
    })?;
    l.set_contact_addresses(&kept, &m.addresses)?;
    let lists = l.state(c::LISTS_KEY)?.unwrap_or_else(|| "[]".into());
    l.set_state(c::LISTS_KEY, &lists_after(&lists, &kept, &absorbed))?;
    if let Some(me) = l.state(c::ME_KEY)? {
        if absorbed.iter().any(|a| same(a, &me)) {
            l.set_state(c::ME_KEY, &kept)?;
        }
    }
    Ok(())
}

/// Merge these cards into one, here at once and at Apple behind. Answers the kept card's id.
pub fn record(library: &mut Library, guids: &[String]) -> Result<String, String> {
    let mut wanted: Vec<String> = Vec::new();
    for g in guids {
        if !wanted.iter().any(|w| same(w, g)) {
            wanted.push(g.clone());
        }
    }
    let book = library.cards(None).map_err(|e| e.to_string())?;
    let cards: Vec<Card> = wanted
        .iter()
        .filter_map(|g| book.iter().find(|c| same(&c.contact.guid, g)).cloned())
        .collect();
    if cards.len() < 2 {
        return Err("Choose two or more contacts to merge.".into());
    }
    let me = library
        .state(c::ME_KEY)
        .map_err(|e| e.to_string())?
        .filter(|m| !m.is_empty());
    let at = core_merge::survivor(&cards, me.as_deref());
    let kept = cards[at].clone();
    let others: Vec<Card> = cards
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != at)
        .map(|(_, c)| c.clone())
        .collect();
    let whole = |card: &Card| -> Result<Kept, String> {
        Ok(Kept {
            addresses: library
                .contact_addresses(&card.contact.guid)
                .map_err(|e| e.to_string())?,
            card: card.clone(),
        })
    };
    let before = whole(&kept)?;
    let absorbed = others.iter().map(whole).collect::<Result<Vec<_>, _>>()?;
    let merged = core_merge::merged(&kept, &others);
    let addresses = core_merge::merged_addresses(
        &std::iter::once(&before)
            .chain(&absorbed)
            .map(|k| k.addresses.clone())
            .collect::<Vec<_>>(),
    );
    let m = ContactMerge {
        contact: merged.contact,
        details: merged.details,
        addresses,
        before,
        absorbed,
        lists_before: library
            .state(c::LISTS_KEY)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "[]".into()),
        hold_until: now() + HOLD,
    };
    let guid = kept.contact.guid.clone();
    library
        .atomically(|l| {
            l.outbox_append(CONTACTS, &guid, "merge", &json(&m)?)?;
            lay(l, &m)
        })
        .map_err(|e| e.to_string())?;
    Ok(guid)
}

/// Take back a merge not yet sent: off the queue, and every card as it was. False when it has
/// already gone to Apple (or there is none).
pub fn undo(library: &mut Library, kept: &str) -> Result<bool, String> {
    let waiting = library
        .outbox_waiting(CONTACTS, kept, "merge")
        .map_err(|e| e.to_string())?;
    let Some(p) = waiting.last() else {
        return Ok(false);
    };
    let m: ContactMerge = serde_json::from_str(&p.payload).map_err(|e| e.to_string())?;
    let id = p.id;
    library
        .atomically(|l| {
            if !l.outbox_withdraw(id)? {
                return Ok(false);
            }
            for k in std::iter::once(&m.before).chain(&m.absorbed) {
                l.put_card(&k.card)?;
                l.set_contact_addresses(&k.card.contact.guid, &k.addresses)?;
            }
            l.set_state(c::LISTS_KEY, &m.lists_before)?;
            // Whatever else waits for these cards is laid over them again.
            outbox::replay(l, CONTACTS)?;
            Ok::<_, quince_core::Error>(true)
        })
        .map_err(|e| e.to_string())
}

/// How long until the merge at the head of the queue may go; `None` when nothing waits on one.
pub async fn held(data: &State<'_, Data>) -> Option<Duration> {
    let head = data
        .0
        .lock()
        .await
        .outbox_pending(CONTACTS)
        .ok()?
        .into_iter()
        .next()?;
    if head.kind != "merge" || head.tries > 0 {
        return None;
    }
    let m: ContactMerge = serde_json::from_str(&head.payload).ok()?;
    let left = m.hold_until - now();
    (left > 0).then(|| Duration::from_secs(u64::try_from(left).unwrap_or(1)))
}

/// Whether an error means Apple was never reached.
fn unreached(e: &Error) -> bool {
    matches!(e, Error::Network(_) | Error::ReauthRequired)
}

/// Send a merge, on Apple's own cards. See the module's notes for why each step is safe to
/// take twice.
pub async fn send(
    library: &crate::library_lock::LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &c::Ledger,
    p: &Pending,
) -> Result<(), Error> {
    let m: ContactMerge = serde_json::from_str(&p.payload)
        .map_err(|e| Error::Refused(format!("A queued merge could not be read: {e}")))?;
    let book = match apple::read_book(http, root, params.clone()).await {
        Ok(book) => {
            ledger.hold(book.tokens.clone());
            book
        }
        Err(e) => {
            // Nothing of it reached Apple: it can still be undone.
            if unreached(&e) {
                let _ = library.lock().await.outbox_untry(p.id);
            }
            return Err(e);
        }
    };
    let Some(kept) = book.card(&p.record).filter(apple::readable) else {
        return Err(Error::Refused(format!(
            "{} was deleted on another device, so the merge wasn't sent.",
            m.before.card.contact.display_name()
        )));
    };
    // A card already gone — deleted on another device, or by this merge sent before — still
    // brings what it held, from the copy kept here.
    let others: Vec<serde_json::Value> = m
        .absorbed
        .iter()
        .map(|a| {
            book.card(&a.card.contact.guid)
                .unwrap_or_else(|| apple::snapshot(&a.card, &a.addresses))
        })
        .collect();
    let (mut card, photo) = apple::union(&kept, &others);
    if let Some(photo) = photo {
        if let Some(entry) = carry_photo(http, &params, &photo).await {
            if let Some(o) = card.as_object_mut() {
                o.insert("photo".into(), entry);
            }
        }
    }
    if card != kept {
        apple::put(http, root, params.clone(), ledger, card).await?;
    }

    // Into every list the others were in.
    let absorbed: Vec<&str> = m
        .absorbed
        .iter()
        .map(|a| a.card.contact.guid.as_str())
        .collect();
    let book = apple::read_book(http, root, params.clone()).await?;
    ledger.hold(book.tokens.clone());
    for mut list in book.lists() {
        let had = list
            .members
            .iter()
            .any(|x| absorbed.iter().any(|a| same(a, x)));
        if had && !list.members.iter().any(|x| same(x, &p.record)) {
            list.members.push(p.record.clone());
            c::update_list(http, root, params.clone(), ledger, &list).await?;
        }
    }

    // And the others gone. My Card moves to the kept card first, if it was one of them.
    let book = apple::read_book(http, root, params.clone()).await?;
    ledger.hold(book.tokens.clone());
    let me = book.me();
    if me
        .as_deref()
        .is_some_and(|me| absorbed.iter().any(|a| same(a, me)))
    {
        c::set_me_card(http, root, params.clone(), ledger, &p.record).await?;
    }
    for guid in absorbed {
        let book = apple::read_book(http, root, params.clone()).await?;
        ledger.hold(book.tokens.clone());
        if let Some(card) = book.card(guid) {
            apple::delete_card(http, root, params.clone(), ledger, &card).await?;
        }
    }
    Ok(())
}

/// Another card's picture, as an image of the kept card's own: fetched, and sent as a JPEG
/// under Apple's size, its chosen square scaled with it — what icloud.com sends for a picture
/// chosen there. `None` when it cannot be had; the merge goes ahead without it rather than
/// not at all.
async fn carry_photo(
    http: &reqwest::Client,
    params: &[(String, String)],
    photo: &serde_json::Value,
) -> Option<serde_json::Value> {
    let url = photo.get("url")?.as_str()?;
    let bytes = c::photo_bytes(http, params, url).await.ok()?;
    let crop = photo.get("crop").and_then(|c| {
        let at = |k: &str| c.get(k).and_then(serde_json::Value::as_i64);
        Some((at("x")?, at("y")?, at("width")?, at("height")?))
    });
    let (data_url, scale) = tokio::task::spawn_blocking(move || encode_photo(&bytes, PHOTO_LIMIT))
        .await
        .ok()??;
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let scaled = crop.map(|(x, y, w, h)| {
        let s = |v: i64| (v as f64 * scale).round() as i64;
        (s(x), s(y), s(w).max(1), s(h).max(1))
    });
    Some(apple::photo_entry(&data_url, scaled))
}

// --- Commands.

/// Cards that look like one person: the same name, or a number or email they share.
#[tauri::command]
pub async fn contacts_duplicates(data: State<'_, Data>) -> UiResult<Vec<Duplicates>> {
    let book = data
        .0
        .read(|library| library.contacts(None))
        .await
        .map_err(|e| e.to_string())?;
    Ok(core_merge::find_duplicates(&book))
}

/// Merge these cards into one. Shown at once, sent behind; answers the kept card's id.
#[tauri::command]
pub async fn contacts_merge(guids: Vec<String>, data: State<'_, Data>) -> UiResult<String> {
    let kept = {
        let mut library = data.0.lock().await;
        record(&mut library, &guids)?
    };
    outbox::kick();
    Ok(kept)
}

/// Undo a merge Apple does not have yet. Answers whether it was undone.
#[tauri::command]
pub async fn contacts_merge_undo(kept: String, data: State<'_, Data>) -> UiResult<bool> {
    let mut library = data.0.lock().await;
    undo(&mut library, &kept)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use quince_core::contacts::ContactMethod;

    pub(crate) fn person(guid: &str, first: &str, phone: &str) -> Card {
        Card {
            contact: Contact {
                guid: guid.into(),
                first_name: Some(first.into()),
                middle_name: None,
                last_name: None,
                company: None,
                is_company: false,
                birthday: None,
                notes: None,
                photo_url: None,
                photo_signature: None,
                photo_crop: None,
                etag: None,
                methods: vec![ContactMethod {
                    kind: "phone".into(),
                    label: None,
                    value: phone.into(),
                }],
            },
            details: ContactDetails::default(),
        }
    }

    fn library() -> Library {
        let mut l = Library::in_memory().unwrap();
        let mut a = person("A", "Zoya", "+91 90000 00001");
        a.details.job_title = Some("Architect".into());
        let mut b = person("B", "Zoya", "090000 00001");
        b.contact.notes = Some("Prefers WhatsApp".into());
        b.contact.methods.push(ContactMethod {
            kind: "email".into(),
            label: Some("HOME".into()),
            value: "zoya@example.com".into(),
        });
        for c in [&a, &b, &person("C", "Ravi", "+91 98765 43210")] {
            l.put_card(c).unwrap();
        }
        l.put_addresses(
            &[(
                "B".into(),
                vec![PostalAddress {
                    label: Some("HOME".into()),
                    city: Some("Pune".into()),
                    ..PostalAddress::default()
                }],
            )],
            false,
        )
        .unwrap();
        l.set_state(
            c::LISTS_KEY,
            r#"[{"guid":"L1","name":"Family","members":["B","C"],"etag":null}]"#,
        )
        .unwrap();
        l.set_state(c::ME_KEY, "").unwrap();
        l
    }

    #[test]
    fn a_merge_shows_one_card_at_once_and_undo_puts_every_card_back() {
        let mut l = library();
        let dupes = core_merge::find_duplicates(&l.contacts(None).unwrap());
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].guids, ["A", "B"]);

        let kept = record(&mut l, &dupes[0].guids).unwrap();
        let book = l.cards(None).unwrap();
        assert_eq!(book.len(), 2, "one card where there were two");
        let merged = book.iter().find(|c| c.contact.guid == kept).unwrap();
        assert_eq!(merged.details.job_title.as_deref(), Some("Architect"));
        assert_eq!(merged.contact.notes.as_deref(), Some("Prefers WhatsApp"));
        assert!(merged.contact.methods.iter().any(|m| m.kind == "email"));
        assert_eq!(
            l.contact_addresses(&kept).unwrap()[0].city.as_deref(),
            Some("Pune")
        );
        let lists = c::stored_lists(&l);
        assert!(
            lists[0].members.contains(&kept),
            "in the list the other card was in"
        );
        assert_eq!(l.outbox_pending(CONTACTS).unwrap().len(), 1);

        // Undone before it is sent: every card as it was, and nothing waits.
        assert!(undo(&mut l, &kept).unwrap());
        let book = l.cards(None).unwrap();
        assert_eq!(book.len(), 3);
        let b = book.iter().find(|c| c.contact.guid == "B").unwrap();
        assert_eq!(b.contact.notes.as_deref(), Some("Prefers WhatsApp"));
        assert_eq!(l.contact_addresses("B").unwrap().len(), 1);
        assert_eq!(c::stored_lists(&l)[0].members, ["B", "C"]);
        assert!(l.outbox_pending(CONTACTS).unwrap().is_empty());
        assert!(!undo(&mut l, &kept).unwrap(), "nothing left to undo");
    }

    #[test]
    fn a_merge_on_its_way_cannot_be_undone_and_a_pass_keeps_it_shown() {
        let mut l = library();
        let kept = record(&mut l, &["A".into(), "B".into()]).unwrap();
        l.outbox_take(CONTACTS).unwrap();
        assert!(!undo(&mut l, &kept).unwrap());
        // A pass reads Apple's book from before the merge; what waits is laid over it.
        let a = person("A", "Zoya", "+91 90000 00001");
        let b = person("B", "Zoya", "090000 00001");
        l.put_contacts(&[a.contact, b.contact]).unwrap();
        outbox::replay(&mut l, CONTACTS).unwrap();
        assert_eq!(l.contacts(None).unwrap().len(), 1);
        // Reached nothing at Apple: counted as untried, it can be undone again.
        let id = l.outbox_pending(CONTACTS).unwrap()[0].id;
        l.outbox_untry(id).unwrap();
        assert!(undo(&mut l, &kept).unwrap());
    }

    #[test]
    fn my_card_stays_mine_when_it_is_merged() {
        let mut l = library();
        l.set_state(c::ME_KEY, "B").unwrap();
        let kept = record(&mut l, &["A".into(), "B".into()]).unwrap();
        assert_eq!(kept, "B", "My Card is the one kept");
        assert_eq!(l.state(c::ME_KEY).unwrap().as_deref(), Some("B"));
    }

    #[test]
    fn a_picture_is_made_small_enough_for_apple_to_take() {
        let big = image::RgbImage::from_fn(1600, 1200, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x * y) % 239) as u8])
        });
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big)
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let (url, scale) = encode_photo(&png, PHOTO_LIMIT).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        assert!(url.len() <= PHOTO_LIMIT);
        assert!(scale <= 1.0 && scale > 0.0);
    }

    /// Apple's contacts service, enough of it to merge against: it keeps the cards and lists
    /// it is sent, and counts every write.
    pub(crate) mod apple_stub {
        use serde_json::{json, Value};
        use std::sync::{Arc, Mutex as StdMutex};
        use wiremock::{Request, Respond, ResponseTemplate};

        #[derive(Default)]
        pub struct Book {
            pub cards: Vec<Value>,
            pub groups: Vec<Value>,
            pub writes: Vec<String>,
            pub version: u64,
            /// Whether a write is answered with the new tokens, as Apple may or may not.
            pub answers_tokens: bool,
        }

        #[derive(Clone)]
        pub struct Apple(pub Arc<StdMutex<Book>>);

        impl Respond for Apple {
            fn respond(&self, req: &Request) -> ResponseTemplate {
                let mut b = self.0.lock().unwrap();
                let path = req.url.path().to_string();
                let method = req
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "method")
                    .map_or_else(|| req.method.to_string(), |(_, v)| v.to_string());
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                if path.ends_with("/co/startup") {
                    let order: Vec<Value> =
                        b.cards.iter().map(|c| c["contactId"].clone()).collect();
                    return ResponseTemplate::new(200).set_body_json(json!({
                        "syncToken": format!("S{}", b.version), "prefToken": "P",
                        "contacts": b.cards, "groups": b.groups, "contactsOrder": order,
                    }));
                }
                // As Apple: a write carrying a sync token an earlier write spent is refused.
                let token = req
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "syncToken")
                    .map(|(_, v)| v.to_string());
                if token.as_deref() != Some(format!("S{}", b.version).as_str()) {
                    b.writes.push(format!("refused, spent token {token:?}"));
                    return ResponseTemplate::new(412);
                }
                b.version += 1;
                let v = b.version;
                let answer = if b.answers_tokens {
                    json!({ "syncToken": format!("S{v}"), "prefToken": "P" })
                } else {
                    json!({})
                };
                if path.contains("/co/contacts/card") {
                    let card = body["contacts"][0].clone();
                    let id = card["contactId"].as_str().unwrap_or("").to_string();
                    b.writes.push(format!("{method} card {id}"));
                    b.cards.retain(|c| c["contactId"] != id.as_str());
                    if method != "DELETE" {
                        let mut card = card;
                        card["etag"] = json!(format!("E{v}"));
                        b.cards.push(card);
                    }
                } else if path.contains("/co/groups/card") {
                    let g = body["groups"][0].clone();
                    b.writes.push(format!(
                        "{method} list {}",
                        g["groupId"].as_str().unwrap_or("")
                    ));
                    b.groups.retain(|x| x["groupId"] != g["groupId"]);
                    b.groups.push(g);
                }
                ResponseTemplate::new(200).set_body_json(answer)
            }
        }
    }

    /// Sent, the kept card at Apple holds everything both did, is in the other's list, and the
    /// other is gone; sent again — a send cut off, retried — nothing more is written.
    #[test]
    fn a_merge_sent_twice_lands_once_and_loses_nothing() {
        merge_sent_twice(false);
    }

    /// The same, where Apple answers each write with its new tokens: the writes in a row go
    /// with those, and none is refused for a token an earlier write spent.
    #[test]
    fn a_merge_carries_the_tokens_each_write_is_answered_with() {
        merge_sent_twice(true);
    }

    fn merge_sent_twice(answers_tokens: bool) {
        use serde_json::json;
        tauri::async_runtime::block_on(async {
            let server = wiremock::MockServer::start().await;
            let apple = apple_stub::Apple(std::sync::Arc::new(std::sync::Mutex::new(
                apple_stub::Book {
                    cards: vec![
                        json!({ "contactId": "A", "etag": "ea", "firstName": "Zoya",
                                "jobTitle": "Architect",
                                "phones": [{ "label": "MOBILE", "field": "+91 90000 00001" }] }),
                        json!({ "contactId": "B", "etag": "eb", "firstName": "Zoya",
                                "notes": "Prefers WhatsApp", "X-APPLE-ONLY": "kept",
                                "phones": [{ "field": "090000 00001" }],
                                "emailAddresses": [{ "label": "HOME", "field": "zoya@example.com" }],
                                "streetAddresses": [{ "label": "HOME", "field": { "city": "Pune" } }] }),
                        json!({ "contactId": "C", "etag": "ec", "firstName": "Ravi" }),
                    ],
                    groups: vec![
                        json!({ "groupId": "L1", "name": "Family", "contactIds": ["A", "C"], "etag": "g1" }),
                        json!({ "groupId": "L2", "name": "Work", "contactIds": ["A"], "etag": "g2" }),
                    ],
                    answers_tokens,
                    ..Default::default()
                },
            )));
            wiremock::Mock::given(wiremock::matchers::any())
                .respond_with(apple.clone())
                .mount(&server)
                .await;

            let lib = crate::library_lock::LibraryLock::new(library(), None);
            let kept = record(&mut *lib.lock().await, &["A".into(), "B".into()]).unwrap();
            assert_eq!(kept, "B", "the card holding the most");
            let p = lib.lock().await.outbox_pending(CONTACTS).unwrap().remove(0);
            let http = reqwest::Client::new();
            send(&lib, &http, &server.uri(), vec![], &c::Ledger::new(), &p)
                .await
                .unwrap();

            {
                let b = apple.0.lock().unwrap();
                assert_eq!(b.cards.len(), 2, "{:?}", b.writes);
                let a = b
                    .cards
                    .iter()
                    .find(|c| c["contactId"] == kept.as_str())
                    .unwrap();
                assert_eq!(a["jobTitle"], "Architect");
                assert_eq!(a["notes"], "Prefers WhatsApp");
                assert_eq!(
                    a["X-APPLE-ONLY"], "kept",
                    "even what this app does not show"
                );
                assert_eq!(
                    a["phones"].as_array().unwrap().len(),
                    1,
                    "one number, not two"
                );
                assert_eq!(a["emailAddresses"][0]["field"], "zoya@example.com");
                assert_eq!(a["streetAddresses"][0]["field"]["city"], "Pune");
                assert!(b.groups[0]["contactIds"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(kept)));
                // The card, two lists, the other card gone — and not one write refused.
                assert_eq!(b.writes.len(), 4, "{:?}", b.writes);
            }
            // Sent again: nothing more to write.
            send(&lib, &http, &server.uri(), vec![], &c::Ledger::new(), &p)
                .await
                .unwrap();
            assert_eq!(apple.0.lock().unwrap().writes.len(), 4);
        });
    }
}
