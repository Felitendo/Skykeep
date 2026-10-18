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

/// [`with_cell`] over the attachment's `MergeableDataEncrypted` as Apple sends it (base64,
/// compressed), answered in the same compression it came in.
#[must_use]
pub fn with_cell_in_base64(value: &str, row: usize, column: usize, text: &str) -> Option<String> {
    use base64::Engine;
    use std::io::Write;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .ok()?;
    let gz = raw.starts_with(&[0x1f, 0x8b]);
    let plain = inflate(&raw)?;
    let edited = with_cell(&plain, row, column, text)?;
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
