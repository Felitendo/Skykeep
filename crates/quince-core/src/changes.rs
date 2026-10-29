//! Which of each app's items a change to the library touched: the reminders, notes, cards and
//! photos added, changed and removed — so a screen can redraw those rows rather than read its
//! whole list again.
//!
//! Watched with triggers rather than read off the records a pass brought: a pass writes
//! through many paths (a whole address book stored again, changes still on their way laid back
//! over it, a tombstone), and the rows themselves are the one place they all meet. Each row
//! written is counted by a digest of what it holds — out once as it was, in once as it is — so
//! a row stored again just as it was cancels out, and what is left is what really changed.
//! A card whose numbers were all rewritten unchanged is not a changed card.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::Serialize;

/// Where each table's rows belong: the app that shows them, the column naming the item a row
/// is part of, and whether it is the item's own row (rather than one of its parts — a number on
/// a card, a tag on a reminder). No column: the table holds how the app is arranged — its
/// lists, folders, albums — and a change there is read whole.
const PLACES: &[(&str, &str, Option<&str>, bool)] = &[
    ("events", "calendar", Some("guid"), true),
    ("calendars", "calendar", None, false),
    ("reminders", "reminders", Some("guid"), true),
    ("reminder_alarms", "reminders", Some("reminder_guid"), false),
    (
        "reminder_attachments",
        "reminders",
        Some("reminder_guid"),
        false,
    ),
    (
        "reminder_early_alerts",
        "reminders",
        Some("reminder_guid"),
        false,
    ),
    (
        "reminder_hashtags",
        "reminders",
        Some("reminder_guid"),
        false,
    ),
    ("reminder_parts", "reminders", Some("reminder_guid"), false),
    ("reminder_lists", "reminders", None, false),
    ("reminder_sections", "reminders", None, false),
    ("reminder_groups", "reminders", None, false),
    ("reminder_smart_lists", "reminders", None, false),
    ("alarm_triggers", "reminders", None, false),
    ("alarm_fired", "reminders", None, false),
    ("alarm_snoozes", "reminders", None, false),
    ("notes", "notes", Some("guid"), true),
    ("note_folders", "notes", None, false),
    ("contacts", "contacts", Some("guid"), true),
    ("contact_methods", "contacts", Some("contact_guid"), false),
    ("contact_addresses", "contacts", Some("contact_guid"), false),
    ("drive_items", "drive", Some("drivewsid"), true),
    ("photos", "photos", Some("id"), true),
    ("photo_albums", "photos", None, false),
    ("photo_album_items", "photos", None, false),
];

/// Tables no screen reads a pass's change in: where each service got to, the search index
/// kept from the rows themselves, and the queues that say their own changes.
const UNSEEN: &[&str] = &[
    "sync_state",
    "search_docs",
    "outbox",
    "photo_uploads",
    "photo_walk",
    "photo_walk_held",
];

/// When to stop counting and say only "a lot": a first pass of a large photo library, which
/// every screen reads whole anyway.
const MOST_ROWS: usize = 200_000;

/// What a change did to one app's items, by their ids.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Items {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl Items {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.added.len() + self.changed.len() + self.removed.len()
    }
}

/// What one change to the library did, as [`crate::Library::changing_items`] saw it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changed {
    /// Rows beyond the bookkeeping were written (`Library::changing`'s answer).
    pub written: bool,
    /// Each app's items it really changed, once rows stored again as they were cancel out.
    /// `None` when not all of it could be placed: a table no app's items live in, or more
    /// rows than are worth counting. Read whole, then.
    pub items: Option<BTreeMap<&'static str, Items>>,
    /// The apps whose arrangement — lists, folders, albums — it changed: read whole.
    pub arranged: BTreeSet<&'static str>,
}

impl Changed {
    /// What it did to `service`, for that app's screen: `None` to read it whole, `Some` with
    /// the ids otherwise — empty when nothing it shows changed.
    #[must_use]
    pub fn for_service(&self, service: &str) -> Option<Items> {
        let items = self.items.as_ref()?;
        if self.arranged.contains(service) {
            return None;
        }
        // Another app's items changed by this one's pass: nothing the ids could say for sure.
        if items.iter().any(|(s, i)| *s != service && !i.is_empty()) {
            return None;
        }
        Some(items.get(service).cloned().unwrap_or_default())
    }
}

/// Rows seen while watching: each row's digest, counted in (+1) and out (−1), by table and item.
#[derive(Default)]
pub(crate) struct Tally {
    rows: HashMap<(usize, Option<String>, u64), i64>,
    /// Too many rows to be worth placing, or a write the watch could not read.
    lost: bool,
}

/// The watch shared between the library and the functions its triggers call.
pub(crate) type Watch = Arc<Mutex<Option<Tally>>>;

/// The two functions the triggers call: whether anything is being watched, and "this row, in
/// or out". Registered once per writing connection.
pub(crate) fn register(conn: &Connection, watch: &Watch) -> rusqlite::Result<()> {
    let on = Arc::clone(watch);
    conn.create_scalar_function(
        "quince_watching",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| Ok(on.lock().unwrap_or_else(PoisonError::into_inner).is_some()),
    )?;
    let tally = Arc::clone(watch);
    conn.create_scalar_function("quince_seen", -1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let mut guard = tally.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(tally) = guard.as_mut() else {
            return Ok(rusqlite::types::Null);
        };
        if tally.lost {
            return Ok(rusqlite::types::Null);
        }
        let table = usize::try_from(ctx.get::<i64>(0)?).unwrap_or(usize::MAX);
        let key = match ctx.get_raw(1) {
            ValueRef::Null => None,
            ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Integer(n) => Some(n.to_string()),
            ValueRef::Real(n) => Some(n.to_string()),
            ValueRef::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
        };
        let sign = ctx.get::<i64>(2)?;
        // DefaultHasher::new() is keyed the same every time: digests compare within a change.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for i in 3..ctx.len() {
            match ctx.get_raw(i) {
                ValueRef::Null => 0u8.hash(&mut hasher),
                ValueRef::Integer(n) => (1u8, n).hash(&mut hasher),
                ValueRef::Real(n) => (2u8, n.to_bits()).hash(&mut hasher),
                ValueRef::Text(t) => (3u8, t).hash(&mut hasher),
                ValueRef::Blob(b) => (4u8, b).hash(&mut hasher),
            }
        }
        *tally.rows.entry((table, key, hasher.finish())).or_insert(0) += sign;
        if tally.rows.len() > MOST_ROWS {
            tally.lost = true;
            tally.rows = HashMap::new();
        }
        Ok(rusqlite::types::Null)
    })?;
    Ok(())
}

/// Every table a write can land in, with its place: (index into [`PLACES`], or `None`).
fn tables(conn: &Connection) -> rusqlite::Result<Vec<(String, Option<usize>)>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM main.sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
           AND sql NOT LIKE 'CREATE VIRTUAL%'
           AND name NOT LIKE 'search_text%'",
    )?;
    let names = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for name in names {
        let name = name?;
        if UNSEEN.contains(&name.as_str()) {
            continue;
        }
        let place = PLACES.iter().position(|(t, ..)| *t == name);
        out.push((name, place));
    }
    Ok(out)
}

/// The triggers that tell [`register`]'s function of each row written, made where missing.
/// Temporary: they belong to this connection alone, are never stored in the library, and do
/// nothing unless a change is being watched.
pub(crate) fn install(conn: &Connection) -> rusqlite::Result<()> {
    for (n, (table, place)) in tables(conn)?.into_iter().enumerate() {
        // Placed tables are numbered by their place; the rest after them, by the order read.
        let at = place.unwrap_or(PLACES.len() + n);
        let mut columns = Vec::new();
        {
            let mut stmt = conn.prepare(&format!("PRAGMA main.table_info(\"{table}\")"))?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for name in names {
                let name = name?;
                // When a row was last stored is not what it holds.
                if name != "updated_at" {
                    columns.push(name);
                }
            }
        }
        let key = place.and_then(|p| PLACES[p].2);
        let row = |side: &str, sign: i64| {
            let key = key.map_or_else(|| "NULL".to_string(), |k| format!("{side}.\"{k}\""));
            let values: Vec<String> = columns.iter().map(|c| format!("{side}.\"{c}\"")).collect();
            format!(
                "SELECT quince_seen({at}, {key}, {sign}, {});",
                values.join(", ")
            )
        };
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER IF NOT EXISTS \"quince_seen_{table}_in\" AFTER INSERT ON \"{table}\"
               WHEN quince_watching() BEGIN {} END;
             CREATE TEMP TRIGGER IF NOT EXISTS \"quince_seen_{table}_set\" AFTER UPDATE ON \"{table}\"
               WHEN quince_watching() BEGIN {} {} END;
             CREATE TEMP TRIGGER IF NOT EXISTS \"quince_seen_{table}_out\" AFTER DELETE ON \"{table}\"
               WHEN quince_watching() BEGIN {} END;",
            row("NEW", 1),
            row("OLD", -1),
            row("NEW", 1),
            row("OLD", -1),
        ))?;
    }
    Ok(())
}

/// What a watch saw, placed: each app's items in, changed and out, and whose arrangement moved.
pub(crate) fn place(tally: Tally, written: bool) -> Changed {
    if tally.lost {
        return Changed {
            written,
            items: None,
            arranged: BTreeSet::new(),
        };
    }
    // Per item: what its own row did (in, out, or both) and whether any part of it moved.
    #[derive(Default)]
    struct Seen {
        own_in: bool,
        own_out: bool,
        parts: bool,
    }
    let mut seen: BTreeMap<(&'static str, String), Seen> = BTreeMap::new();
    let mut arranged = BTreeSet::new();
    for ((at, key, _), n) in tally.rows {
        if n == 0 {
            continue;
        }
        let Some(&(_, service, column, own)) = PLACES.get(at) else {
            // A table no app's items live in: nothing about this change can be placed.
            return Changed {
                written,
                items: None,
                arranged: BTreeSet::new(),
            };
        };
        match (column, key) {
            (Some(_), Some(key)) => {
                let s = seen.entry((service, key)).or_default();
                if !own {
                    s.parts = true;
                } else if n > 0 {
                    s.own_in = true;
                } else {
                    s.own_out = true;
                }
            }
            // An arrangement's row, or a part belonging to no item.
            _ => {
                arranged.insert(service);
            }
        }
    }
    let mut items: BTreeMap<&'static str, Items> = BTreeMap::new();
    for ((service, key), s) in seen {
        let mine = items.entry(service).or_default();
        match (s.own_in, s.own_out) {
            (true, false) => mine.added.push(key),
            (false, true) => mine.removed.push(key),
            _ => mine.changed.push(key),
        }
    }
    Changed {
        written,
        items: Some(items),
        arranged,
    }
}

#[cfg(test)]
mod tests {
    use crate::contacts::{Contact, ContactMethod};
    use crate::Library;

    fn card(guid: &str, last: &str, phone: &str) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: Some("Alex".into()),
            middle_name: None,
            last_name: Some(last.into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: Some("1".into()),
            methods: vec![ContactMethod {
                kind: "phone".into(),
                label: Some("mobile".into()),
                value: phone.into(),
            }],
        }
    }

    /// A whole address book stored again names only the cards that differ: the one renamed,
    /// the one whose number moved, the one new and the one gone — not every card rewritten.
    #[test]
    fn a_book_stored_again_names_only_the_cards_that_changed() {
        let mut library = Library::in_memory().unwrap();
        library
            .put_contacts(&[
                card("A", "Rivera", "1"),
                card("B", "Chen", "2"),
                card("C", "Diaz", "3"),
                card("D", "Evans", "4"),
            ])
            .unwrap();
        let ((), changed) = library.changing_items(&["sync_state"], |l| {
            l.put_contacts(&[
                card("A", "Rivera", "1"),
                card("B", "Chen-Park", "2"),
                card("C", "Diaz", "33"),
                card("E", "Fischer", "5"),
            ])
            .unwrap();
            l.mark_synced("contacts", Some("t")).unwrap();
        });
        assert!(changed.written);
        let items = changed.for_service("contacts").expect("placed");
        assert_eq!(items.added, ["E"]);
        assert_eq!(items.changed, ["B", "C"]);
        assert_eq!(items.removed, ["D"]);
    }

    /// Stored again exactly as it was, nothing is named — and a screen has nothing to read.
    #[test]
    fn rows_stored_again_as_they_were_cancel_out() {
        let mut library = Library::in_memory().unwrap();
        library.put_contacts(&[card("A", "Rivera", "1")]).unwrap();
        library.set_state("contacts.me", "A").unwrap();
        let ((), changed) = library.changing_items(&["sync_state"], |l| {
            l.put_contacts(&[card("A", "Rivera", "1")]).unwrap();
            l.set_state("contacts.me", "A").unwrap();
        });
        assert!(changed.written, "rows were written");
        assert_eq!(
            changed.for_service("contacts"),
            Some(super::Items::default())
        );
    }

    /// A row in a table no app's items live in can't be placed: the screen reads whole. Nor is
    /// anything watched outside a change, or left watched after one.
    #[test]
    fn what_cannot_be_placed_is_read_whole() {
        let mut library = Library::in_memory().unwrap();
        let ((), changed) = library.changing_items(&["sync_state"], |l| {
            l.set_state("contacts.lists", "[1]").unwrap();
        });
        assert_eq!(changed.for_service("contacts"), None);
        library.put_contacts(&[card("A", "Rivera", "1")]).unwrap();
        let ((), changed) = library.changing_items(&["sync_state"], |_| {});
        assert!(!changed.written);
        assert_eq!(
            changed.for_service("contacts"),
            Some(super::Items::default())
        );
    }
}
