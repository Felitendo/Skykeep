# Notes tables: the CRDT, and what a phone writes

Measured 2026-10-21 on the iPhone's own tables in the note "Table Test"
(`EACC6CE8-…`), read only, plus a structure-only census of every table attachment on
the account (`probe/examples/notes_table_writers.rs`, `notes_table_replicas.rs`). No
cell text is reproduced here.

Evidence set:

| attachment | what the phone did | doc size |
|---|---|---|
| `913885E6`, `A2E4F35E`, `FDC89AFA`, `C185424B` (deleted) | made a blank 2×2 | 1817 B |
| `017CA145` | made a blank 2×2, typed one character and deleted it (2.4 s later) | 1839 B |
| `043907AA` | made a 2×2, typed three cells; a row was added and the first row removed | 4001 B |

`t2-before.bin` ("Colors" before the bad edit) was not on disk; the live "Colors"
tables and the conflict copies were read from Apple instead (below).

## Document layout

```
Document { 1: 0, 2: Version { 1: 0, 2: 0, 3: Data } }      // field 2 may repeat (see b00ea4f)
Data {
  1: VersionVector { 1: { 1: replica, 2: clock } * }       // replica = index into field 6
  2: <empty>                                                // present once a phone has edited
  3: Object *        // one-field messages: 1 register, 6 dictionary, 10 text, 13 custom, 16 ordered set
  4: key name *      5: type name *      6: uuid (16 B) *
  7: Writers { 1: { 1: uuid, 2: { 1: next char }, 2: { 1: next timestamp } } * }
}
```

Dictionary elements carry a stamp `{1: {1: replica, 2: clock}}` (replica = field-6 index).
Texts (cells, and the ordered sets' `"￼￼"` strings) are topotext:
`{2: string, 3: substring *, 5: run *}`, substring `{1: id{1: writer, 2: clock}, 2: length,
3: stamp{1: writer, 2: clock}, 4: deleted?, 5: next}`, start marker `(0,0)`, end marker
`(0, 0xFFFFFFFF)`.

## What a phone's cell edit changes (blank → touched → typed)

Blank → touched (`017CA145`), a text-only edit, changes exactly:

1. **The cell's text object.** A new substring `id (1, 2), len 1, stamp (1, 8), deleted`
   linked between the start and end markers. The cached string is the new text.
2. **Data field 7** (writers): the phone's entry `[2, 1] → [3, 9]` — next character after
   the one it used (2 → 3), next timestamp after the one it stamped (8 → 9).
3. **Data field 2**: an empty field 2 appears right after field 1.

Nothing else: the **version vector (field 1) stays `{0: 1}`**, the uuid list (field 6)
is unchanged, every other object is byte-identical.

Blank → typed (`043907AA`) shows the same for text, plus structure:

- Version vector `{0: 1} → {0: 3}`. The structure changed (a row element and its cells
  added, the first row's element removed from the ordered set, its array entry
  re-stamped `(0, 2)`). New dictionary elements are stamped `(0, 1)`, so the stamp is
  not simply "the clock at write"; field 1 moves with structure, not with text.
- Writers `[2, 1] → [158, 76]`: next char = last char id + length (`(1,150)+8`), next
  timestamp = last stamp + 1 (75 + 1).
- Each deleted substring carries its own stamp — the timestamp of the deletion (single
  backspaces give one stamp per character, a selection delete one stamp for the range).
  Live substrings carry either `(1, 0)` or a per-insertion stamp.
- **Runs.** A blank cell has none. Every run the phone writes in a cell is
  `{1: length, 2: {9: paragraph uuid (16 B)}, 13: unix seconds}`, one run per typing
  burst. Field 13 is the time of the edit (e.g. 1792589418 against a CreationDate of
  1792589407870 ms). The paragraph uuid is shared by a cell's runs (and was seen shared
  between two cells of one column), and is fresh for a cell typed into for the first time.
- The attachment record's `SummaryEncrypted` changes from `CgoKCg==` (`"\n\n\n\n"`) to a
  summary of the cells' text (length fits: cell texts + one `\n` each). Pomander does not
  write it yet; it should not affect merging.

## Replicas

- **The doc's writer list (field 7) is a map uuid → clocks, kept sorted by uuid
  (byte order), and a writer number in a text is the 1-based place in that sorted list.**
  Evidence: every multi-writer list an Apple device wrote is sorted (`[1dc1…, e308…]`,
  `[23ab…, 28d3…, 766e…, e308…, ef77…]`); only lists Pomander last wrote are not.
- The phone's replica is `e3081e68…` on every table it made: field 6 `[0]`, field 7
  `[0]`, version-vector replica 0. A device is one stable uuid across documents.
- **A new writer registering for text** is just an entry in field 7 at its sorted
  place. Text-only writers are not added to field 6 or field 1: Apple's own rewrite of a
  table Pomander had edited kept Pomander's text writers out of field 6. A writer that
  changes structure is in field 6 and gets a version-vector entry (`1dc1…` in the
  "Colors" tables).
- **`ReplicaIDToNotesVersionDataEncrypted`** is not per document. It is plain (not
  gzipped) CRDT data, byte-identical on the note and all six attachments: a dictionary
  replica-uuid → `NSNumber{integerValue: 36}` for three devices (`e3081e68…` the
  iPhone, `62d4d8fa…`, `6a0af31c…`), version vector `{0: 3}`. `ReplicaIDToUserIDEncrypted`
  (on the note) maps the same uuids to the user record. No edit in the evidence changed
  either; the earlier finding (notes.rs) is that Apple drops client writes of them. A
  table edit must leave them alone.

## Why Pomander's table edits doubled text

The conflict-copy note `B5459772` (deleted) holds three tables created by a second Apple
device (CloudKit deviceID `1BA2…`, not the iPhone `FE8C…`, not the web `"2"`) at
1792552406, between Pomander's writes to "Colors". Copy `015AB30E` has its writer list
**sorted** — `[23ab, 28d3, 766e, e308, ef77]` — while every substring keeps the numbers
of the list Pomander had **appended to** — `[e308, 23ab, ef77, 28d3, 766e]`. Read by
sorted place, the phone's own two characters in each ordered set, `(1, 0..1)`, became
`23ab`'s; Pomander's 11-character cell became `28d3`'s; and so on. The clocks show the
same remap: the copy's clocks are the element-wise max of the uuid-keyed and the
place-keyed values (`[11, 11, 1, 7, 1]` from `[2, 11, 1, 7, 1]` by place and
`{23ab: 11, 28d3: 7, …}` by uuid). The same characters then existed under two ids — one
copy from the phone, one from the misread document — so the merge kept both: doubled
rows and doubled/tripled cell text, and conflict copies.

So the cause is **appending a new writer to field 7 instead of inserting it at its
sorted place and renumbering every text**, made worse by a fresh writer uuid per run
(probe runs never called `set_writer`): each write landed at a different sorted place.
It is not the version vector — the phone itself leaves field 1 alone on a text edit.

The live "Colors" tables `FE5372D5` and `5E44E504` were last written by Pomander and
still have unsorted lists (`[e308, 23ab, ef77, 28d3, 766e, 6109]`, `[1dc1, e308, 4dcf]`);
an Apple device reading them now misattributes their text the same way. The new writer
refuses such a list rather than guess.

Pomander's stable writer (`documents.writer` in the app's library) is `6bc6a71d…`, which
sorts **before** the phone's `e3081e68…`: an append would shift every phone character.

## The write path (`table::with_cell_by`)

Exactly what the phone's text edit does:

1. Read field 7; refuse unless sorted and re-encoding byte-identical.
2. Insert Pomander's replica at its sorted place with clocks `[0, 0]` if new; if any
   writer moved up, renumber every text in the table (cells, kind 10; ordered sets,
   kind 16 → 1 → 1 → 1) — ids and stamps ≥ that place go up by one. Objects of other
   kinds than 1/6/10/13/16 make it refuse.
3. Edit the cell: removed units marked deleted and stamped `(me, next ts)`; typed units
   a new substring `(me, next char)`, stamped `(me, next ts)`; clocks advanced.
4. Runs: the removed span cut out; the typed span a new run patterned on its neighbour
   (paragraph style carried), field 13 = now; in a cell with no runs,
   `{1: n, 2: {9: fresh uuid}, 13: now}`.
5. Field 2 (empty) added after field 1 if absent. Field 1, field 6, every other object,
   `ReplicaIDToNotesVersionDataEncrypted`: untouched.
6. The result must read back as the same table with only that cell changed.

Rows and columns (`with_row`, `with_column`, `without_*`) still append and are now
refused whenever that would leave field 7 unsorted. They were not re-measured here.

## Proposed live test (not run; flags stay off)

One cell, the blank table `913885E6` in "Table Test", by Pomander's stable writer:

1. Turn `TABLE_CELL_WRITES` on in a local build only. Quit Notes on the iPhone and Mac
   first so nothing edits the note concurrently, then set cell (row 0, column 0) to
   `Pomander 1`.
2. Read it back with `notes_table_writers` / `notes_note_tables`: field 7 must be
   `[6bc6a71d… [10, 1], e3081e68… [2, 1]]`, the ordered sets' two characters numbered
   2 (the phone), field 1 still `{0: 1}`.
3. On the iPhone: open "Table Test" and wait for sync. Check: the table shows
   `Pomander 1` once, no other cell changed, **no conflict copy note** appears in the
   folder, and the note list shows one "Table Test". Then type `X` after it on the phone,
   wait, and read the attachment again: the phone's write must keep `6bc6a71d…` in
   field 7 at place 1, the cell must read `Pomander 1X`, still no conflict copy.
4. Check the other Apple device too (CloudKit deviceID `1BA2…`, which made the earlier conflict copies).
