//! Apple's "mergeable data": the CRDT object graph behind Notes tables and Reminders' list
//! order.
//!
//! Measured on the `ListIDsMergeableOrdering` field of a real Reminders `Account` record. The
//! top-level message holds:
//!
//! - `3`: the objects, each one message. Object 0 is the root.
//! - `4`: key names (`self`, `UUIDIndex`), `5`: type names (`com.apple.CRDT.NSString`,
//!   `com.apple.CRDT.NSUUID`, …), `6`: 16-byte UUIDs. Objects refer to all three by index.
//!
//! An object that is a plain value is a map at field 13: `{1: type index, 3: {1: key index,
//! 2: value}}`, where the value carries a string at 4 or an index at 2 — for an `NSUUID`, the
//! index of its UUID.
//!
//! The root is an ordered set at field 16: `{1: ordering, 2: elements}`. The ordering is
//! `{1: array, 2: contents}`, where the array is a text CRDT whose every character is an
//! attachment — `{2: {1: position, 2: uuid}}` in reading order, one per character still
//! there — and the contents are a dictionary `{1: {1: {6: key object}, 2: {6: value
//! object}, 3: timestamp}}` from each attachment's UUID to what sits at that position.
//!
//! So reading the order is: attachments in order, each UUID looked up in the contents, each
//! answer resolved to its string.

use std::collections::HashMap;

type Field<'a> = (u64, u64, &'a [u8], u64);

fn varint(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7F) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

fn fields(bytes: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let (key, next) = varint(bytes, at)?;
        at = next;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                let (v, next) = varint(bytes, at)?;
                at = next;
                out.push((field, wire, &bytes[0..0], v));
            }
            2 => {
                let (n, next) = varint(bytes, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                out.push((field, wire, bytes.get(next..end)?, 0));
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= bytes.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= bytes.len())?,
            _ => return None,
        }
    }
    Some(out)
}

fn message<'a>(fields: &[Field<'a>], tag: u64) -> Option<&'a [u8]> {
    fields.iter().find(|f| f.0 == tag && f.1 == 2).map(|f| f.2)
}

fn messages<'a>(fields: &[Field<'a>], tag: u64) -> Vec<&'a [u8]> {
    fields
        .iter()
        .filter(|f| f.0 == tag && f.1 == 2)
        .map(|f| f.2)
        .collect()
}

fn number(fields: &[Field<'_>], tag: u64) -> Option<u64> {
    fields.iter().find(|f| f.0 == tag && f.1 == 0).map(|f| f.3)
}

/// A plain value an object can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Uuid(Vec<u8>),
}

struct Graph<'a> {
    objects: Vec<&'a [u8]>,
    types: Vec<String>,
    uuids: Vec<&'a [u8]>,
}

impl<'a> Graph<'a> {
    fn read(bytes: &'a [u8]) -> Option<Self> {
        let top = fields(bytes)?;
        Some(Self {
            objects: messages(&top, 3),
            types: messages(&top, 5)
                .into_iter()
                .map(|t| String::from_utf8_lossy(t).into_owned())
                .collect(),
            uuids: messages(&top, 6),
        })
    }

    /// The value held by the object at `index`, when it is a string or a UUID.
    fn value(&self, index: u64) -> Option<Value> {
        let object = fields(self.objects.get(usize::try_from(index).ok()?)?)?;
        let map = fields(message(&object, 13)?)?;
        let kind = self.types.get(usize::try_from(number(&map, 1)?).ok()?)?;
        let entry = fields(message(&map, 3)?)?;
        let held = fields(message(&entry, 2)?)?;
        if kind.ends_with("NSString") {
            let text = message(&held, 4)?;
            return Some(Value::Text(String::from_utf8(text.to_vec()).ok()?));
        }
        if kind.ends_with("NSUUID") {
            let at = usize::try_from(number(&held, 2)?).ok()?;
            return Some(Value::Uuid(self.uuids.get(at)?.to_vec()));
        }
        None
    }

    fn object_ref(reference: &[u8]) -> Option<u64> {
        number(&fields(reference)?, 6)
    }
}

/// The strings an ordered set holds, in order. `None` when the bytes are not an ordered set
/// shaped the way Apple writes one; a caller then keeps whatever order it had.
#[must_use]
pub fn ordered_strings(bytes: &[u8]) -> Option<Vec<String>> {
    let graph = Graph::read(bytes)?;
    let root = fields(graph.objects.first()?)?;
    let set = fields(message(&root, 16)?)?;
    let ordering = fields(message(&set, 1)?)?;
    let array = fields(message(&ordering, 1)?)?;

    // What each position's UUID stands for. Written in the order the edits happened, so a
    // later entry for the same UUID is the newer one.
    let mut contents: HashMap<Vec<u8>, String> = HashMap::new();
    if let Some(dictionary) = message(&ordering, 2) {
        for element in messages(&fields(dictionary)?, 1) {
            let element = fields(element)?;
            let key = graph.value(Graph::object_ref(message(&element, 1)?)?);
            let value = graph.value(Graph::object_ref(message(&element, 2)?)?);
            if let (Some(Value::Uuid(key)), Some(Value::Text(value))) = (key, value) {
                contents.insert(key, value);
            }
        }
    }

    let mut positions: Vec<(u64, Vec<u8>)> = Vec::new();
    for attachment in messages(&array, 2) {
        let attachment = fields(attachment)?;
        let uuid = message(&attachment, 2)?;
        positions.push((number(&attachment, 1).unwrap_or(0), uuid.to_vec()));
    }
    positions.sort_by_key(|(position, _)| *position);

    let mut out = Vec::with_capacity(positions.len());
    let mut seen = std::collections::HashSet::new();
    for (_, uuid) in positions {
        // A position whose contents are unknown is skipped, not guessed at; one that names
        // an entry already placed keeps its first place.
        if let Some(id) = contents.get(&uuid) {
            if seen.insert(id.clone()) {
                out.push(id.clone());
            }
        }
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod build {
    //! An encoder for the same shape, so tests can make one without real account bytes.

    pub(crate) fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return;
            }
        }
    }
    pub(crate) fn num(tag: u64, value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        varint(tag << 3, &mut out);
        varint(value, &mut out);
        out
    }
    pub(crate) fn msg(tag: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        varint((tag << 3) | 2, &mut out);
        varint(payload.len() as u64, &mut out);
        out.extend_from_slice(payload);
        out
    }

    /// An ordered set holding `ids` in this order, with positions listed in `shuffle` order
    /// on the wire and a stale contents entry ahead of each real one.
    pub(crate) fn ordered_set(ids: &[&str], wire_order: &[usize]) -> Vec<u8> {
        const STRING: u64 = 0;
        const UUID: u64 = 1;
        let uuid = |i: usize| vec![u8::try_from(i).unwrap(); 16];
        let mut objects: Vec<Vec<u8>> = vec![Vec::new()]; // the root, filled in last
        let mut uuids: Vec<Vec<u8>> = Vec::new();
        let value_object = |kind: u64, value: Vec<u8>| {
            let mut entry = num(1, 0);
            entry.extend(msg(2, &value));
            let mut map = num(1, kind);
            map.extend(msg(3, &entry));
            msg(13, &map)
        };
        let mut dictionary = Vec::new();
        let mut attachments = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            uuids.push(uuid(i));
            objects.push(value_object(UUID, num(2, i as u64)));
            let key = objects.len() - 1;
            // An old answer for the same position, which the later one replaces.
            objects.push(value_object(STRING, msg(4, b"STALE")));
            let stale = objects.len() - 1;
            objects.push(value_object(STRING, msg(4, id.as_bytes())));
            let value = objects.len() - 1;
            for v in [stale, value] {
                let mut element = msg(1, &num(6, key as u64));
                element.extend(msg(2, &num(6, v as u64)));
                element.extend(msg(3, &msg(1, &[num(1, 1), num(2, 0)].concat())));
                dictionary.extend(msg(1, &element));
            }
        }
        for &i in wire_order {
            let mut a = num(1, i as u64);
            a.extend(msg(2, &uuid(i)));
            attachments.extend(msg(2, &a));
        }
        let mut array = msg(1, b"");
        array.extend(attachments);
        let mut ordering = msg(1, &array);
        ordering.extend(msg(2, &dictionary));
        let set = msg(1, &ordering);
        objects[0] = msg(16, &set);

        let mut top = Vec::new();
        for o in &objects {
            top.extend(msg(3, o));
        }
        for k in ["self", "UUIDIndex"] {
            top.extend(msg(4, k.as_bytes()));
        }
        for t in ["com.apple.CRDT.NSString", "com.apple.CRDT.NSUUID"] {
            top.extend(msg(5, t.as_bytes()));
        }
        for u in &uuids {
            top.extend(msg(6, u));
        }
        top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_is_read_by_position_and_the_latest_answer_wins() {
        let bytes = build::ordered_set(&["DAILY", "STUDY", "TODO", "LIFE"], &[2, 0, 3, 1]);
        assert_eq!(
            ordered_strings(&bytes).unwrap(),
            vec!["DAILY", "STUDY", "TODO", "LIFE"]
        );
    }

    #[test]
    fn anything_else_is_refused_rather_than_guessed() {
        assert!(ordered_strings(&[]).is_none());
        assert!(ordered_strings(&[0xFF, 0xFF]).is_none());
        assert!(ordered_strings(&build::msg(3, &build::num(1, 1))).is_none());
    }
}
