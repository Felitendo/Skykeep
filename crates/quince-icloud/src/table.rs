//! A Notes table, read from the attachment that holds it.
//!
//! A table is an `Attachment` record of type `com.apple.notes.table` whose
//! `MergeableDataEncrypted` is a versioned CRDT document (gzip or zlib, then protobuf):
//!
//! - `Document{2: Version{3: data}}`; `data` is `{3: object*, 4: key*, 5: type*, 6: uuid*}`.
//! - Object 0 is a custom object of type `com.apple.notes.ICTable` whose entries name, by
//!   object index, `crRows`, `crColumns` and `cellColumns`.
//! - `crRows` and `crColumns` are ordered sets (object kind 16): a list of elements — each an
//!   index and a uuid — and a dictionary from each element's own `NSUUID` object to the row's
//!   or column's `NSUUID` object. The elements in index order are the rows (or columns) in
//!   reading order.
//! - `cellColumns` is a dictionary: column object → dictionary: row object → a text object
//!   (kind 10) whose field 2 is the cell's text. A cell never typed in may be missing; it is
//!   empty.
//!
//! Measured on tables made on a phone and on icloud.com: a 2×2 never typed in, and an 11×2
//! with text in its cells, read back exactly as Notes shows them.

use std::collections::HashMap;

/// The attachment type a table is.
pub const UTI: &str = "com.apple.notes.table";

/// Where a table's cells are kept in the library, by attachment id.
#[must_use]
pub fn state_key(id: &str) -> String {
    format!("notes.table.{id}")
}

/// A table's cells as the last pass stored them; `None` when none are known.
#[must_use]
pub fn stored(library: &quince_core::Library, id: &str) -> Option<Vec<Vec<String>>> {
    library
        .state(&state_key(id))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn varint(b: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for shift in 0..10 {
        let byte = *b.get(at)?;
        at += 1;
        v |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some((v, at));
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Wire<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

fn fields(b: &[u8]) -> Option<Vec<(u64, Wire<'_>)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let (key, next) = varint(b, at)?;
        at = next;
        match key & 7 {
            0 => {
                let (v, next) = varint(b, at)?;
                at = next;
                out.push((key >> 3, Wire::Int(v)));
            }
            2 => {
                let (n, next) = varint(b, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                out.push((key >> 3, Wire::Bytes(b.get(next..end)?)));
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= b.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= b.len())?,
            _ => return None,
        }
    }
    Some(out)
}

fn bytes<'a, 'f>(fs: &'f [(u64, Wire<'a>)], tag: u64) -> impl Iterator<Item = &'a [u8]> + 'f {
    fs.iter().filter_map(move |(t, w)| match (t, w) {
        (t, Wire::Bytes(b)) if *t == tag => Some(*b),
        _ => None,
    })
}

fn int(fs: &[(u64, Wire<'_>)], tag: u64) -> Option<u64> {
    fs.iter().find_map(|(t, w)| match (t, w) {
        (t, Wire::Int(v)) if *t == tag => Some(*v),
        _ => None,
    })
}

/// An object id's object index, where it is one (field 6).
fn object_index(id: &[u8]) -> Option<usize> {
    usize::try_from(int(&fields(id)?, 6)?).ok()
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    if data.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(data)
            .read_to_end(&mut out)
            .ok()?;
    } else if flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .is_ok()
    {
    } else {
        return Some(data.to_vec());
    }
    Some(out)
}

struct Doc<'a> {
    objects: Vec<&'a [u8]>,
    keys: Vec<String>,
    uuids: Vec<&'a [u8]>,
}

impl<'a> Doc<'a> {
    /// An object's kind (its one field's number) and payload.
    fn object(&self, i: usize) -> Option<(u64, &'a [u8])> {
        let fs = fields(self.objects.get(i)?)?;
        match fs.first()? {
            (kind, Wire::Bytes(b)) => Some((*kind, *b)),
            _ => None,
        }
    }

    /// A custom object's entries: key name → the object id's bytes.
    fn custom(&self, i: usize) -> Option<HashMap<String, &'a [u8]>> {
        let (kind, body) = self.object(i)?;
        if kind != 13 {
            return None;
        }
        let mut out = HashMap::new();
        for entry in bytes(&fields(body)?, 3) {
            let e = fields(entry)?;
            let key = usize::try_from(int(&e, 1)?).ok()?;
            let value = bytes(&e, 2).next()?;
            out.insert(self.keys.get(key)?.clone(), value);
        }
        Some(out)
    }

    /// A dictionary object (kind 6, or a dictionary message itself): object → object.
    fn dictionary(body: &[u8]) -> Option<HashMap<usize, usize>> {
        let mut out = HashMap::new();
        for element in bytes(&fields(body)?, 1) {
            let e = fields(element)?;
            let (Some(k), Some(v)) = (bytes(&e, 1).next(), bytes(&e, 2).next()) else {
                continue;
            };
            if let (Some(k), Some(v)) = (object_index(k), object_index(v)) {
                out.insert(k, v);
            }
        }
        Some(out)
    }

    /// The `NSUUID` object holding each uuid.
    fn uuid_objects(&self) -> HashMap<&'a [u8], usize> {
        let mut out = HashMap::new();
        for i in 0..self.objects.len() {
            if let Some(entries) = self.custom(i) {
                if let Some(at) = entries
                    .get("UUIDIndex")
                    .and_then(|id| int(&fields(id)?, 2).or_else(|| int(&fields(id)?, 1)))
                {
                    if let Some(u) = usize::try_from(at).ok().and_then(|at| self.uuids.get(at)) {
                        out.insert(*u, i);
                    }
                }
            }
        }
        out
    }

    /// An ordered set's members, in order, as the objects the table keys its cells by.
    fn ordered(&self, i: usize, by_uuid: &HashMap<&[u8], usize>) -> Option<Vec<usize>> {
        let (_, body) = self.object(i)?;
        let top = fields(body)?;
        let array = fields(bytes(&top, 1).next()?)?;
        let list = fields(bytes(&array, 1).next()?)?;
        let mut elements: Vec<(u64, &[u8])> = Vec::new();
        for element in bytes(&list, 2) {
            let e = fields(element)?;
            elements.push((int(&e, 1).unwrap_or(0), bytes(&e, 2).next()?));
        }
        elements.sort_by_key(|(at, _)| *at);
        let mut members: HashMap<usize, usize> = HashMap::new();
        for d in bytes(&array, 2) {
            members.extend(Self::dictionary(d)?);
        }
        Some(
            elements
                .into_iter()
                .filter_map(|(_, u)| {
                    let own = *by_uuid.get(u)?;
                    Some(members.get(&own).copied().unwrap_or(own))
                })
                .collect(),
        )
    }

    fn text(&self, i: usize) -> String {
        self.object(i)
            .filter(|(kind, _)| *kind == 10)
            .and_then(|(_, body)| fields(body))
            .and_then(|fs| {
                bytes(&fs, 2)
                    .next()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            })
            .unwrap_or_default()
    }
}

fn put_varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `message` with the `nth` length-delimited field `tag` replaced by `payload`, every other
/// byte copied through. `None` when there is no such field.
fn splice(message: &[u8], tag: u64, nth: usize, payload: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(message.len() + payload.len());
    let (mut at, mut seen, mut done) = (0usize, 0usize, false);
    while at < message.len() {
        let start = at;
        let (key, next) = varint(message, at)?;
        at = next;
        match key & 7 {
            0 => at = varint(message, at)?.1,
            2 => {
                let (n, next) = varint(message, at)?;
                let end = next.checked_add(usize::try_from(n).ok()?)?;
                message.get(next..end)?;
                if key >> 3 == tag && !done {
                    if seen == nth {
                        put_varint(key, &mut out);
                        put_varint(payload.len() as u64, &mut out);
                        out.extend_from_slice(payload);
                        done = true;
                        at = end;
                        continue;
                    }
                    seen += 1;
                }
                at = end;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= message.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= message.len())?,
            _ => return None,
        }
        out.extend_from_slice(message.get(start..at)?);
    }
    done.then_some(out)
}

/// Where a cell's text lives: the index of its text object among the document's objects.
fn cell_object(document: &[u8], row: usize, column: usize) -> Option<usize> {
    let top = fields(document)?;
    let version = fields(bytes(&top, 2).next()?)?;
    let data = fields(bytes(&version, 3).next()?)?;
    let doc = Doc {
        objects: bytes(&data, 3).collect(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: bytes(&data, 6).collect(),
    };
    let root = doc.custom(0)?;
    let at = |name: &str| root.get(name).and_then(|id| object_index(id));
    let by_uuid = doc.uuid_objects();
    let rows = doc.ordered(at("crRows")?, &by_uuid)?;
    let columns = doc.ordered(at("crColumns")?, &by_uuid)?;
    let (_, cells_body) = doc.object(at("cellColumns")?)?;
    let by_column = Doc::dictionary(cells_body)?;
    let inner = *by_column.get(columns.get(column)?)?;
    let cells = Doc::dictionary(doc.object(inner)?.1)?;
    let cell = *cells.get(rows.get(row)?)?;
    // A text object, as a cell's is.
    (doc.object(cell)?.0 == 10).then_some(cell)
}

/// The table's document with one cell's text changed, edited as a device edits it.
///
/// Only a cell already typed in is written: its text object exists, and is rewritten in
/// place with its merge records moved in step (the same rewrite a note's body takes). A cell
/// never typed in has no object yet, and making one is not attempted. Before anything is
/// returned it is proven that the rewrite is exact: written back with the text it already
/// had, the document must come out byte for byte as it was — anything this does not model
/// would break that, and the cell is then left alone.
#[must_use]
pub fn with_cell(document: &[u8], row: usize, column: usize, text: &str) -> Option<Vec<u8>> {
    let cell = cell_object(document, row, column)?;
    let top = fields(document)?;
    let version_bytes = bytes(&top, 2).next()?;
    let version = fields(version_bytes)?;
    let data_bytes = bytes(&version, 3).next()?;
    let data = fields(data_bytes)?;
    let object = bytes(&data, 3).nth(cell)?;
    let string = bytes(&fields(object)?, 10).next()?;
    let old = bytes(&fields(string)?, 2)
        .next()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default();

    let rebuild = |new_text: &str| -> Option<Vec<u8>> {
        let new_string = crate::ckdocument::rewrite_text_object(string, &old, new_text)?;
        let new_object = splice(object, 10, 0, &new_string)?;
        let new_data = splice(data_bytes, 3, cell, &new_object)?;
        let new_version = splice(version_bytes, 3, 0, &new_data)?;
        splice(document, 2, 0, &new_version)
    };
    // The proof: the splice itself, with the string as it was, changes nothing.
    let same = splice(object, 10, 0, string)
        .and_then(|o| splice(data_bytes, 3, cell, &o))
        .and_then(|d| splice(version_bytes, 3, 0, &d))
        .and_then(|v| splice(document, 2, 0, &v))?;
    if same != document {
        return None;
    }
    let out = rebuild(text)?;
    // And what was written reads back as meant.
    let read = cells(&out)?;
    (read.get(row)?.get(column)? == text).then_some(out)
}

/// Each field of a message with its whole encoding (key, length and payload), in order.
fn raw_fields(b: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let start = at;
        let (key, next) = varint(b, at)?;
        at = next;
        match key & 7 {
            0 => at = varint(b, at)?.1,
            2 => {
                let (n, next) = varint(b, at)?;
                at = next.checked_add(usize::try_from(n).ok()?)?;
                b.get(..at)?;
            }
            1 => at = at.checked_add(8).filter(|&e| e <= b.len())?,
            5 => at = at.checked_add(4).filter(|&e| e <= b.len())?,
            _ => return None,
        }
        out.push((key >> 3, b.get(start..at)?));
    }
    Some(out)
}

fn put_len(tag: u64, payload: &[u8], out: &mut Vec<u8>) {
    put_varint((tag << 3) | 2, out);
    put_varint(payload.len() as u64, out);
    out.extend_from_slice(payload);
}

fn put_int(tag: u64, v: u64, out: &mut Vec<u8>) {
    put_varint(tag << 3, out);
    put_varint(v, out);
}

fn object_id(index: usize) -> Vec<u8> {
    let mut id = Vec::new();
    put_int(6, index as u64, &mut id);
    id
}

/// A dictionary element as Apple writes one: key and value objects, stamped by one replica.
fn dictionary_element(key: usize, value: usize, replica: u64, clock: u64) -> Vec<u8> {
    let mut stamp_one = Vec::new();
    put_int(1, replica, &mut stamp_one);
    put_int(2, clock, &mut stamp_one);
    let mut stamp = Vec::new();
    put_len(1, &stamp_one, &mut stamp);
    let mut e = Vec::new();
    put_len(1, &object_id(key), &mut e);
    put_len(2, &object_id(value), &mut e);
    put_len(3, &stamp, &mut e);
    e
}

/// An `NSUUID` custom object naming uuid number `uuid_index`, in the shape `template` (an
/// existing one) has.
fn nsuuid_object(template: &[u8], uuid_index: usize) -> Option<Vec<u8>> {
    let custom = bytes(&fields(template)?, 13).next()?;
    let cf = fields(custom)?;
    let entry = bytes(&cf, 3).next()?;
    let ef = fields(entry)?;
    let mut value = Vec::new();
    put_int(2, uuid_index as u64, &mut value);
    let mut new_entry = Vec::new();
    put_int(1, int(&ef, 1)?, &mut new_entry);
    put_len(2, &value, &mut new_entry);
    let mut new_custom = Vec::new();
    put_int(1, int(&cf, 1)?, &mut new_custom);
    put_len(3, &new_entry, &mut new_custom);
    let mut object = Vec::new();
    put_len(13, &new_custom, &mut object);
    Some(object)
}

/// The table's document with an empty row added at `at` (0 is the top), as Notes adds one:
/// a new member of the ordered row set, written as this installation's replica, and an
/// empty text in every column so its cells can be typed in at once.
///
/// Nothing is returned unless it is proven: the pieces this writes are first rebuilt from
/// the table's own and must come out byte for byte as Apple wrote them, and the result must
/// read back as the same grid with one empty row at `at`.
#[must_use]
pub fn with_row(document: &[u8], at: usize) -> Option<Vec<u8>> {
    with_member(document, false, at)
}

/// The table's document with an empty column added at `at` (0 is the leading edge), as Notes
/// adds one: a new member of the ordered column set and an empty text in it for every row,
/// proven as [`with_row`] is.
#[must_use]
pub fn with_column(document: &[u8], at: usize) -> Option<Vec<u8>> {
    with_member(document, true, at)
}

fn with_member(document: &[u8], column: bool, at: usize) -> Option<Vec<u8>> {
    let before = cells(document)?;
    let width = before.first().map_or(0, Vec::len);
    let count = if column { width } else { before.len() };
    if at > count {
        return None;
    }
    let top = fields(document)?;
    let version_bytes = bytes(&top, 2).next()?;
    let version = fields(version_bytes)?;
    let data_bytes = bytes(&version, 3).next()?;
    let data = fields(data_bytes)?;
    let objects: Vec<&[u8]> = bytes(&data, 3).collect();
    let uuids: Vec<&[u8]> = bytes(&data, 6).collect();
    let doc = Doc {
        objects: objects.clone(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: uuids.clone(),
    };
    let root = doc.custom(0)?;
    let at_key = |name: &str| root.get(name).and_then(|id| object_index(id));
    let rows_object = at_key(if column { "crColumns" } else { "crRows" })?;
    let cells_object = at_key("cellColumns")?;

    // This installation as a replica of the document, and the clock its writes carry.
    let me = crate::topotext::writer_bytes();
    let mut new_uuids: Vec<Vec<u8>> = Vec::new();
    let replica = match uuids.iter().position(|u| *u == me) {
        Some(i) => i,
        None => {
            new_uuids.push(me.to_vec());
            uuids.len()
        }
    };
    let versions: Vec<(u64, u64)> = bytes(&fields(bytes(&data, 1).next()?)?, 1)
        .filter_map(|e| {
            let f = fields(e)?;
            Some((int(&f, 1).unwrap_or(0), int(&f, 2).unwrap_or(0)))
        })
        .collect();
    let clock = versions
        .iter()
        .find(|(r, _)| *r == replica as u64)
        .map_or(1, |(_, c)| c + 1);

    // The new row's element and identity: two uuids and an NSUUID object for each.
    let template = (0..objects.len()).find_map(|i| {
        let entries = doc.custom(i)?;
        entries.contains_key("UUIDIndex").then_some(objects[i])
    })?;
    // Proof the NSUUID shape is Apple's: an existing one rebuilt comes out the same.
    let template_index = usize::try_from(int(
        &fields(
            bytes(
                &fields(bytes(&fields(bytes(&fields(template)?, 13).next()?)?, 3).next()?)?,
                2,
            )
            .next()?,
        )?,
        2,
    )?)
    .ok()?;
    if nsuuid_object(template, template_index)? != template {
        return None;
    }
    let element_uuid = *uuid::Uuid::new_v4().as_bytes();
    let identity_uuid = *uuid::Uuid::new_v4().as_bytes();
    let element_uuid_index = uuids.len() + new_uuids.len();
    new_uuids.push(element_uuid.to_vec());
    new_uuids.push(identity_uuid.to_vec());
    let element_object = objects.len();
    let identity_object = objects.len() + 1;
    let mut new_objects = vec![
        nsuuid_object(template, element_uuid_index)?,
        nsuuid_object(template, element_uuid_index + 1)?,
    ];

    // The ordered row set: the text gains a character at `at`, the attachments make room for
    // it, and the element joins both dictionaries.
    let (kind, set_body) = doc.object(rows_object)?;
    if kind != 16 {
        return None;
    }
    let set_fields = raw_fields(set_body)?;
    let sf = fields(set_body)?;
    let array_body = bytes(&sf, 1).next()?;
    let af = fields(array_body)?;
    let string_array = bytes(&af, 1).next()?;
    let saf = fields(string_array)?;
    let text = bytes(&saf, 1).next()?;
    let live = count;
    let old_text: String = std::iter::repeat_n('\u{FFFC}', live).collect();
    let mut new_text = old_text.clone();
    let byte_at = old_text
        .char_indices()
        .nth(at)
        .map_or(old_text.len(), |(i, _)| i);
    new_text.insert(byte_at, '\u{FFFC}');
    let new_string = crate::ckdocument::rewrite_text_object(text, &old_text, &new_text)?;
    let mut attachments: Vec<(u64, Vec<u8>)> = Vec::new();
    for a in bytes(&saf, 2) {
        let f = fields(a)?;
        let index = int(&f, 1).unwrap_or(0);
        attachments.push((
            if index >= at as u64 { index + 1 } else { index },
            bytes(&f, 2).next()?.to_vec(),
        ));
    }
    attachments.push((at as u64, element_uuid.to_vec()));
    attachments.sort_by_key(|(i, _)| *i);
    let mut new_string_array = Vec::new();
    for (tag, raw) in raw_fields(string_array)? {
        match tag {
            1 => put_len(1, &new_string, &mut new_string_array),
            2 => {}
            _ => new_string_array.extend_from_slice(raw),
        }
    }
    for (index, uuid) in &attachments {
        let mut a = Vec::new();
        put_int(1, *index, &mut a);
        put_len(2, uuid, &mut a);
        put_len(2, &a, &mut new_string_array);
    }
    let with_element = |dictionary: &[u8], value: usize| -> Option<Vec<u8>> {
        // Proof the element shape is Apple's: the dictionary's last element, rebuilt from its
        // key and value with its own stamp copied, is the same.
        if let Some(last) = bytes(&fields(dictionary)?, 1).last() {
            let f = fields(last)?;
            let mut rebuilt = Vec::new();
            put_len(
                1,
                &object_id(object_index(bytes(&f, 1).next()?)?),
                &mut rebuilt,
            );
            put_len(
                2,
                &object_id(object_index(bytes(&f, 2).next()?)?),
                &mut rebuilt,
            );
            put_len(3, bytes(&f, 3).next()?, &mut rebuilt);
            if rebuilt != last {
                return None;
            }
        }
        let mut out = dictionary.to_vec();
        put_len(
            1,
            &dictionary_element(element_object, value, replica as u64, clock),
            &mut out,
        );
        Some(out)
    };
    let mut new_array = Vec::new();
    for (tag, raw) in raw_fields(array_body)? {
        match tag {
            1 => put_len(1, &new_string_array, &mut new_array),
            2 => put_len(
                2,
                &with_element(bytes(&fields(raw)?, 2).next()?, identity_object)?,
                &mut new_array,
            ),
            _ => new_array.extend_from_slice(raw),
        }
    }
    let mut new_set = Vec::new();
    for (tag, raw) in set_fields {
        match tag {
            1 => put_len(1, &new_array, &mut new_set),
            2 => put_len(
                2,
                &with_element(bytes(&fields(raw)?, 2).next()?, element_object)?,
                &mut new_set,
            ),
            _ => new_set.extend_from_slice(raw),
        }
    }
    let mut new_rows_object = Vec::new();
    put_len(16, &new_set, &mut new_rows_object);

    // An empty text in each new cell, in the shape a phone gives an empty cell.
    let empty_cell = (0..objects.len())
        .find(|&i| doc.object(i).is_some_and(|(k, _)| k == 10) && doc.text(i).is_empty())?;
    let (cells_kind, cells_body) = doc.object(cells_object)?;
    let by_column = Doc::dictionary(cells_body)?;
    let mut changed_objects: HashMap<usize, Vec<u8>> = HashMap::new();
    if column {
        // A dictionary for the new column, a cell in it for every row, and the column in
        // `cellColumns`.
        let row_identities = doc.ordered(at_key("crRows")?, &doc.uuid_objects())?;
        let column_dictionary = objects.len() + new_objects.len();
        new_objects.push(Vec::new()); // filled once its cells have places
        let mut dictionary = Vec::new();
        for row in row_identities {
            let cell = objects.len() + new_objects.len();
            new_objects.push(objects[empty_cell].to_vec());
            put_len(
                1,
                &dictionary_element(row, cell, replica as u64, clock),
                &mut dictionary,
            );
        }
        let mut object = Vec::new();
        put_len(6, &dictionary, &mut object);
        new_objects[column_dictionary - objects.len()] = object;
        let mut grown = with_element(cells_body, column_dictionary)?;
        // `with_element` keys the element by the new member's element object; `cellColumns`
        // is keyed by its identity.
        grown.truncate(cells_body.len());
        put_len(
            1,
            &dictionary_element(identity_object, column_dictionary, replica as u64, clock),
            &mut grown,
        );
        let mut object = Vec::new();
        put_len(cells_kind, &grown, &mut object);
        changed_objects.insert(cells_object, object);
    } else {
        for &column_dictionary in by_column.values() {
            let (k, body) = doc.object(column_dictionary)?;
            if k != 6 {
                return None;
            }
            let cell = objects.len() + new_objects.len();
            new_objects.push(objects[empty_cell].to_vec());
            let mut dictionary = body.to_vec();
            put_len(
                1,
                &dictionary_element(identity_object, cell, replica as u64, clock),
                &mut dictionary,
            );
            let mut object = Vec::new();
            put_len(6, &dictionary, &mut object);
            changed_objects.insert(column_dictionary, object);
        }
    }

    // The document: changed objects in place, new ones and new uuids after the last of their
    // kind, and this replica's clock raised.
    // Every other replica's entry copied as it is (some carry a subclock); this one's set.
    let mut new_version_vector = Vec::new();
    let mut seen_me = false;
    for (_, raw) in raw_fields(bytes(&data, 1).next()?)? {
        let f = fields(bytes(&fields(raw)?, 1).next()?)?;
        if int(&f, 1).unwrap_or(0) == replica as u64 {
            let mut e = Vec::new();
            put_int(1, replica as u64, &mut e);
            put_int(2, clock, &mut e);
            put_len(1, &e, &mut new_version_vector);
            seen_me = true;
        } else {
            new_version_vector.extend_from_slice(raw);
        }
    }
    if !seen_me {
        let mut e = Vec::new();
        put_int(1, replica as u64, &mut e);
        put_int(2, clock, &mut e);
        put_len(1, &e, &mut new_version_vector);
    }
    let raw = raw_fields(data_bytes)?;
    let last_object = raw.iter().rposition(|(t, _)| *t == 3)?;
    let last_uuid = raw.iter().rposition(|(t, _)| *t == 6)?;
    let mut new_data = Vec::with_capacity(data_bytes.len() + 512);
    let mut object_n = 0usize;
    for (i, (tag, field)) in raw.iter().enumerate() {
        match *tag {
            1 => put_len(1, &new_version_vector, &mut new_data),
            3 => {
                if object_n == rows_object {
                    put_len(3, &new_rows_object, &mut new_data);
                } else if let Some(o) = changed_objects.get(&object_n) {
                    put_len(3, o, &mut new_data);
                } else {
                    new_data.extend_from_slice(field);
                }
                object_n += 1;
            }
            _ => new_data.extend_from_slice(field),
        }
        if i == last_object {
            for o in &new_objects {
                put_len(3, o, &mut new_data);
            }
        }
        if i == last_uuid {
            for u in &new_uuids {
                put_len(6, u, &mut new_data);
            }
        }
    }
    let new_version = splice(version_bytes, 3, 0, &new_data)?;
    let out = splice(document, 2, 0, &new_version)?;

    // And it reads back as the same table with one empty row or column at `at`.
    let mut expected = before;
    if column {
        for row in &mut expected {
            row.insert(at, String::new());
        }
    } else {
        expected.insert(at, vec![String::new(); width]);
    }
    (cells(&out)? == expected).then_some(out)
}

/// [`with_cell`] over the attachment's `MergeableDataEncrypted` as Apple sends it (base64,
/// compressed), answered in the same compression it came in.
#[must_use]
pub fn with_cell_in_base64(value: &str, row: usize, column: usize, text: &str) -> Option<String> {
    edit_in_base64(value, |plain| with_cell(plain, row, column, text))
}

/// [`with_row`] over the attachment's `MergeableDataEncrypted` as Apple sends it, answered in
/// the same compression it came in.
#[must_use]
pub fn with_row_in_base64(value: &str, at: usize) -> Option<String> {
    edit_in_base64(value, |plain| with_row(plain, at))
}

/// [`with_column`] over the attachment's `MergeableDataEncrypted` as Apple sends it.
#[must_use]
pub fn with_column_in_base64(value: &str, at: usize) -> Option<String> {
    edit_in_base64(value, |plain| with_column(plain, at))
}

/// A table edit made to the inflated document, packed back as it came: gzip, zlib or plain.
fn edit_in_base64(value: &str, edit: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Option<String> {
    use base64::Engine;
    use std::io::Write;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    let gz = raw.starts_with(&[0x1f, 0x8b]);
    let plain = inflate(&raw)?;
    let edited = edit(&plain)?;
    let packed = if gz {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&edited).ok()?;
        e.finish().ok()?
    } else if plain != raw {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&edited).ok()?;
        e.finish().ok()?
    } else {
        edited
    };
    Some(base64::engine::general_purpose::STANDARD.encode(packed))
}

/// The table's cells, row by row, from the attachment's `MergeableDataEncrypted` (base64).
/// `None` for data that is not a table this understands.
#[must_use]
pub fn cells_from_base64(value: &str) -> Option<Vec<Vec<String>>> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    cells(&inflate(&raw)?)
}

/// [`cells_from_base64`] over the inflated document.
#[must_use]
pub fn cells(document: &[u8]) -> Option<Vec<Vec<String>>> {
    let top = fields(document)?;
    let version = fields(bytes(&top, 2).next()?)?;
    let data = fields(bytes(&version, 3).next()?)?;
    let doc = Doc {
        objects: bytes(&data, 3).collect(),
        keys: bytes(&data, 4)
            .map(|k| String::from_utf8_lossy(k).into_owned())
            .collect(),
        uuids: bytes(&data, 6).collect(),
    };
    let root = doc.custom(0)?;
    let at = |name: &str| root.get(name).and_then(|id| object_index(id));
    let by_uuid = doc.uuid_objects();
    let rows = doc.ordered(at("crRows")?, &by_uuid)?;
    let columns = doc.ordered(at("crColumns")?, &by_uuid)?;
    let (_, cells_body) = doc.object(at("cellColumns")?)?;
    let by_column = Doc::dictionary(cells_body)?;
    let column_cells: HashMap<usize, HashMap<usize, usize>> = by_column
        .iter()
        .filter_map(|(col, inner)| Some((*col, Doc::dictionary(doc.object(*inner)?.1)?)))
        .collect();
    Some(
        rows.iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|col| {
                        column_cells
                            .get(col)
                            .and_then(|cells| cells.get(row))
                            .map(|&cell| doc.text(cell))
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .collect(),
    )
}
