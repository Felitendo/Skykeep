# Apple's own protobuf schemas (from the notes3 web bundle, 2026-10-19)
These are copied verbatim from the strings the icloud.com Notes app passes to `loadProto`
(main.js:124694–124696, 126307–126312 and 152642–152644). They are Apple's real field names
and numbers, not guesses.

| File | What it describes |
|---|---|
| `CRDT.proto` | The CRDT object graph: `Document` (version, objects, keyItem, typeItem, uuidItem, ttTimestamp), `DocObject` kinds (6 dictionary, 10 topotext string, 13 custom, 16 tsOrderedSet…), `Dictionary.Element {key, value, timestamp, index}`, `ObjectID` (field 6 = objectIndex), `Timestamp {replicaIndex, counter}`, `VectorTimestamp`. Tables use this. |
| `versioned_document.proto` | The wrapper `Document {serializationVersion, version*}` and `Version {minimumSupportedVersion, data}`. |
| `topotext-*.proto` | Note text: `String`, `Substring`, `CharID`, `AttributeRun` (1 length, 2 paragraphStyle, 3 font, 5 fontHints, 6 underline, 7 strikethrough, 8 superscript, 9 link, 10 color RGBA, 11 writingDirection, 12 attachmentInfo, 13 timestamp, **14 emphasis**, 15 systemAttachmentInfo), `ParagraphStyle` (1 style, 2 alignment, 3 writingDirection, 4 indent, 5 todo, 6 paragraphHints, 7 startingListItemNumber, 8 blockQuoteLevel, 9 uuid), `Todo`, `Color`, `Media`, `AttachmentInfo`. Where more than one variant is kept, it is the Reminders variant, which differs slightly. |

Quince (crates/quince-icloud/src/ckdocument.rs, table.rs) does not yet read these fields:
`color` (10), `superscript` (8), `alignment`, `blockQuoteLevel`, `startingListItemNumber`.
