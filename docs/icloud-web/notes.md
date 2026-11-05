# iCloud web Notes (notes3) — reverse-engineered facts

Source: `notes3/main.js` (build 2638Build17), beautified copy. Line numbers refer to
the beautified file. Read-only grep; nothing executed. Tags: **VERIFIED-IN-CODE**
(seen literally) / **INFERRED** (deduced, not seen directly).

## Endpoints / transport

- Base service `ckdatabasews` → `https://ckdatabasews.<servicesDomain>` — L108543 `url: "https://ckdatabasews." + c.servicesDomain`. VERIFIED-IN-CODE
- Startup query URL `database/1/com.apple.notes/production/private/records/query` — L112867. VERIFIED-IN-CODE
- Writes go through bundled CloudKit JS: `saveRecords` → `newRecordsBatch(t).createOrUpdate(e).commit()` (L147964) → `setApiEntityName("records").setApiActionName("modify")` (L147983) = `POST …/records/modify`. VERIFIED-IN-CODE
- `zones/modify` (L148037), `subscriptions/modify` (L148078), `changes/database` (L148107, `fetchDatabaseChanges`), `changes/zone` with payload `{ zones: [...] }` (L148114, `fetchRecordZoneChanges`), `records/zip/prepare` (L148125). VERIFIED-IN-CODE
- Batch op types include `"delete"` (`_pushOperations("delete", e)` L146852) alongside createOrUpdate. VERIFIED-IN-CODE. Request JSON is standard CloudKit Web Services `{operations:[{operationType, record:{recordName, recordType, recordChangeTag, fields:{X:{value}}}}], zoneID}` — INFERRED (CloudKit JS library builds it; Notes code never hand-writes it).
- Zone: private `{zoneName:"Notes", ownerName:"currentUser"}` (`PRIVATE_ZONE_ID`, L154346-154349). Shared DB used for shared-to-me zones (`isSharedToMe` → `zoneID.database === "Shared"`, L154359). VERIFIED-IN-CODE
- Record types processed from changes: `AccountData, Note, SearchIndexes, Folder, PasswordProtectedNote, User, Users, Note_UserSpecific, PasswordProtectedNote_UserSpecific, Folder_UserSpecific, cloudkit.share, Hashtag, InlineAttachment` — L129083. VERIFIED-IN-CODE
- desiredKeys for list fetch: `TitleEncrypted, SnippetEncrypted, FirstAttachmentUTIEncrypted, FirstAttachmentThumbnail(Orientation), ModificationDate, Deleted, Folders, Folder, Attachments, ParentFolder, Note, LastViewedModificationDate, MinimumSupportedNotesVersion, IsPinned` — L166140. VERIFIED-IN-CODE

## Folder record (`recordType: "Folder"`, L153366)

- Fields (model props L153452-153471): `MinimumSupportedNotesVersion`, `Title` (dataKey `TitleEncrypted`), `ParentFolder`, `ParentModificationDate`, `TitleModificationDate`, `Deleted`, `DeletedDate`, `ImportedFromLegacy`. VERIFIED-IN-CODE
- `TitleEncrypted` encoding = `EncryptedString = [string("utf-8"), Base64]` (L138346, L153363 `FolderEncodings`) — i.e. **plain UTF-8 bytes, base64**; no real encryption client-side. VERIFIED-IN-CODE
- Built-in record names: `DefaultFolder-CloudKit` ("Notes"), `TrashFolder-CloudKit` ("Recently Deleted"), `iCloudFolder` ("All iCloud", virtual) — L128787, L154365; titles L177794-177796. VERIFIED-IN-CODE
- Trash folder auto-created if missing: on `VALIDATING_REFERENCE_ERROR` while saving a note into trash, writes `{recordType:"Folder", recordName:TRASH_FOLDER, fields:{TitleEncrypted:{value:<b64 "Recently Deleted">}}}` — L168541-168553. VERIFIED-IN-CODE
- Per-user overlay record `Folder_UserSpecific::<userRecordName>/<folderRecordName>` (L154375) — holds per-user ParentFolder/Title for shared folders (L153484, L195793). VERIFIED-IN-CODE

### Create
- `onCreateNewChildFolder`: `Folder.create({Title: "New Folder"})`; if a parent id given, `setParent({recordName, action: CloudKit.ReferenceAction.VALIDATE, zoneID})`; `t.save()` — L195756-195768. So create = records/modify createOrUpdate with `TitleEncrypted` (+ `ParentFolder` reference, action VALIDATE). Record name is a client UUID — INFERRED.
- Nested folders: yes — `getChildFolders()` filters `!Deleted && parent.recordName === this.recordName` (L153473-153477); parent = `ParentFolder` reference. VERIFIED-IN-CODE

### Rename
- `onFolderRename`: no-op if built-in, empty after trim, or unchanged; else `n.Title = r; n.save()` (for shared-root-with-user, sets Title on the UserSpecific record instead) — L195787-195797. => update `TitleEncrypted` (TitleModificationDate presumably bumped — INFERRED). VERIFIED-IN-CODE

### Delete (`Folder.deleteFolder`, L153804-153868)
1. `this.Deleted = 1`; destroy UserSpecific record. 2. Fetch all children. 3. Every note in folder → `deleteOrMoveToRecentlyDeletedAsNeeded()` (i.e. moved to TrashFolder, not flagged); child folders recurse `deleteFolder()`. 4. Finally `wantsToBeDestroyed = true; save(true)` → routed to `deleteAction` (L168500) = a real **records/modify delete** of the folder record. Conflict → reload & retry; VALIDATING_REFERENCE_ERROR → full sync & retry. Shared-with-user root → `removeMeFromShare()` instead. VERIFIED-IN-CODE
- Confirmation dialog: "Are you sure you want to delete “%{name}”?" / "All notes and any subfolders will be deleted." buttons "Delete Folder"/"Cancel"; shared variants exist (L171722-171744, L177839-177844). Built-ins can't be deleted (`isBuiltIn` check). VERIFIED-IN-CODE
- Smart folders: no `SmartFolder`/query-folder record found; only Tags (Hashtag records, "All Tags") and suggested searches. INFERRED: not supported on web.

## Note record

- Fields seen: `TitleEncrypted, SnippetEncrypted, FirstAttachmentUTIEncrypted, FirstAttachmentThumbnail(+Orientation), CreationDate, ModificationDate, Deleted, DeletedDate (L139186), Folders, Folder, Attachments, TextDataEncrypted, TextDataAsset, TextRevisionNumber, IsPinned, LastViewedModificationDate` (L129084, L139169, L168535). VERIFIED-IN-CODE
- Body: `TextDataEncrypted` = topotext protobuf, **zlib-deflated** (`Zip.encode = pako.deflate`, L130003); if > `CK_INLINE_SIZE_LIMIT`, sent as `TextDataAsset` instead, and TextDataEncrypted=null (L168535-168536). VERIFIED-IN-CODE
- Move to Recently Deleted: `moveToTrash() { moveToFolder(Folder.trashFolder) }` (L139547) — updates `Folder`/`Folders` reference to `TrashFolder-CloudKit`, sets `FoldersModificationDate` (L139545). Not a flag. VERIFIED-IN-CODE
- Permanent delete (from trash): `deletePermanently(){ this.Deleted = 1; markAttachmentsForDeletion }` (L139671-139675) then save. VERIFIED-IN-CODE
- Shared note delete: unshare, reload, moveToTrash (L139813-139826). VERIFIED-IN-CODE
- Dialog: "Deleted notes are moved to the Recently Deleted folder." / "Notes are available for 30 days…" (L177745-177746). VERIFIED-IN-CODE

## Text format (topotext proto2, L126312)
- `String{string=2, substring=3, timestamp=4, attributeRun=5, attachment=6}`; `AttributeRun{length=1, paragraphStyle=2, font=3, fontHints=5, underline=6, strikethrough=7, superscript=8, link=9, color=10, writingDirection=11, attachmentInfo=12, timestamp=13, emphasis=14}`. VERIFIED-IN-CODE
- `ParagraphStyle{style=1, alignment=2, writingDirection=3, indent=4, todo=5, paragraphHints=6, startingListItemNumber=7, blockQuoteLevel=8, uuid=9}`; checklist = `Todo{todoUUID=1, done=2}`. VERIFIED-IN-CODE
- `ParagraphStyle.style` enum = `TTParagraphStyle{TITLE=0, HEADING=1, BODY=3, FIXED=4, BULLET=100, DASH=101, NUMBER=102, TODO=103}` — L181474. Editor registry adds Subheading=2: `new Map([[0,Title],[1,Heading],[2,Subheading],[3,Body],[4,FixedWidth],[100,BulletedList],[101,DashedList],[102,NumberedList],[103,TodoList]])` — L186509-186519. VERIFIED-IN-CODE. (Note: 100 is **bulleted "•"**, 101 dashed, not dotted/dashed.) Unknown/absent `style` → Body (default style, L187848-187852).
- `fontHints` bitmask `TTFontHints{BOLD=1, ITALIC=2}` — L181513. Bold/italic are fontHints, not separate fields; underline/strikethrough = `underline`/`strikethrough` ints (1 = single, L181711, L189131-189132). HTML paste: weight>400→BOLD, italic→ITALIC, text-decoration→underline 1 / strikethrough 1, `<a>`→`link` — L181700-181714. VERIFIED-IN-CODE
- `indent`: Tab/Shift-Tab (`formatIndent`/`formatOutdent`) → `indent = max(0, indent±1)`, 0 written as null — L186905-186915. VERIFIED-IN-CODE. `blockQuoteLevel`, `emphasis`, `superscript`, `color` are decoded/round-tripped (L115703, L115725) but no web UI writes them found — INFERRED read-only.

### Checklist (style 103)
- Item = `ParagraphStyle{style:103, todo:{todoUUID, done}}`. New item: `TTToDoAttribute.create({toDoUUID: UuidModule.random(), done})` — L187021-187025. VERIFIED-IN-CODE
- `toggleChecklist` (L186856-186874): for each paragraph in selection, if not all already todo → `setStyle(id(TodoList)=103).setToDo(w(false))`; else → `setStyle(3 Body).setToDo(null)`. VERIFIED-IN-CODE
- Tick box: `toggleChecklistItemForFrameModelIsDone` reads `toDo.done || false` and calls `setChecklistItemForFrameModelIsDone(!done)` → `toDo.setDone(t)`, keeping the UUID — L186876-186893. VERIFIED-IN-CODE
- Leaving checklist via style change strips todo (L186850); new line after a done item resets `done:false` (L186717); duplicate/missing todoUUIDs (e.g. paste) get fresh random UUIDs, todo dropped if no style — L180944-180949. VERIFIED-IN-CODE
- Markdown autoformat at line start: `"[ ] "`/`"[x] "` → checklist, `"- "` → dashed, `"* "` → bulleted, numbered via `1.`-style prefix — L186690-186706. VERIFIED-IN-CODE (numbered trigger regex not read — INFERRED).

## Table attachment (`com.apple.notes.table`)
- `TABLE_UTI = "com.apple.notes.table"` (L129881). It is an **Attachment record** (not InlineAttachment) with `UTI`/`UTIUnencrypted`/`UTIEncrypted` = that UTI. New table: `Title: loc("Table.Title")` ("Table"), `MinimumSupportedNotesVersion: 2`, 2 rows x 2 cols — L137920-137935. VERIFIED-IN-CODE
- Placement in note: one `U+FFFC` char on its own line with `attributeRun.attachmentInfo{attachmentIdentifier: <attachment recordName>, typeUTI:"com.apple.notes.table"}` — L187155. VERIFIED-IN-CODE
- **Read** (L129905-129911): bytes = `MergeableDataEncrypted` (base64) ?? `encryptedValues.mergeableData` (E2E/ADP path) ?? `MergeableDataAsset.downloadedData`; then `inflateMergeableData` (zlib/gzip inflate with size limit, L130002) → `ICTableVersionedDocument.initWithData`. VERIFIED-IN-CODE
- **Write** (L167491-167498): `icTableManager.find(id).saveToArchive()` → `pako.deflate` → base64 → `MergeableDataEncrypted`; if > `CK_INLINE_SIZE_LIMIT` → `MergeableDataAsset` Blob and MergeableDataEncrypted=null. With encryption info, goes into `EncryptedValues` + `CryptoTag` + `CryptoInitializationVector` and both Mergeable fields null (L167525). `TableRevisionNumber` is client-only (deleted before save). VERIFIED-IN-CODE
- Layering: `versioned_document.Document{serializationVersion=1, repeated Version version=2}`, `Version{serializationVersion=1, minimumSupportedVersion=2, bytes data=3}` (L124694) — table serializationVersion 0 (L127115). `data` = `CRDT.Document` (crframework.proto, L124696): `{version=1 VectorTimestamp, startVersion=2, repeated DocObject object=3, repeated string keyItem=4, repeated string typeItem=5, repeated bytes uuidItem=6, ttTimestamp=7}`; `DocObject` oneof `{registerLatest=1, registerGreatest=2, registerLeast=3, set=4, orderedSet=5, dictionary=6, timestamp=7, vectorTimestamp=8, index=9, string(topotext)=10, weakReference=11, oneof=12, custom=13, stringArray=14, array=15, tsOrderedSet=16}`; `CustomObject{type=1 (→typeItem idx), repeated MapEntry{key=1 (→keyItem idx), ObjectID value=2}=3}`; `ObjectID` oneof `{integerValue=1, unsignedIntegerValue=2, doubleValue=3, stringValue=4, bytesValue=5, objectIndex=6}`; `OrderedSet{Array array=1, Dictionary set=2}`; `Array{StringArray array=1, Dictionary dictionary=2}`; `Dictionary.Element{key=1, value=2, timestamp=3, index=4}`. VERIFIED-IN-CODE
- Root object = custom type `"com.apple.notes.ICTable"` (L123079) with keys `identity` (UUID), `crTableColumnDirection`, `crRows`, `crColumns`, `cellColumns` — encode L121595, decode L123097-123101. `crRows`/`crColumns` = ordered sets of row/column UUIDs; `cellColumns` = dictionary columnUUID → dictionary rowUUID → cell (L121848-121861 `cellColumns.get(col).set(row, v)`). Cell = `CRMergeableAttributedString` (topotext string) — L123091; missing cells filled with empty strings on decode (L123102-123109). VERIFIED-IN-CODE. `crTableColumnDirection` = a register (LTR/RTL) — INFERRED.
- Edits via `ICTable.insertRow/insertColumn/set(col,row,str)`, merges via `mergeWithDocument` (CRMergeResult Updated/NoChange/Fail, L127135). VERIFIED-IN-CODE

## Menus / UI strings
- Folder ⋯ menu: "Rename Folder", "Share Folder", "Manage Shared Folder", "Delete Folder"; "New Folder" default title (L177797-177811). VERIFIED-IN-CODE
- Note list context menu: "Delete", "Pin Note"/"Unpin Note"; "Pinned" section (L177812-177815). VERIFIED-IN-CODE
- Aa menu ("Choose a style to apply to text"), order from L195020: Title(0), Heading(1), Subheading(2), Body(3), Monospaced(4, key `FixedWidth`), Bulleted List • (100), Dashed List - (101), Numbered List 1. (102). Checklist (103) is NOT in Aa — separate toolbar button "Make a checklist". Labels L177646-177656. VERIFIED-IN-CODE

## UI checklist (loc keys, L177560-177900; English)
Toolbar
- [ ] `Toolbar.Button.NewNote` "Create a note" · `.Checklist` "Make a checklist" · `.Style` "Choose a style to apply to text" · `.Table` "Insert a table" · `.Delete` "Delete this note" · `.Recover`/`.RecoverTitle` "Recover this note"/"Recover" · `.Lock` "Close all locked notes" · `.Share` "Add people to this note" · `.SharedWithYou`/`.SharedByYou` "View participants…"
- [ ] `Toolbar.Button.Bold/Italic/Underline/Strikethrough`; status `Toolbar.Status.Saving` "Saving…", `.Edited`, `.Saved`, `.MovingOneNote/MultipleNotes` "Moving %{count} note(s)…"
- [ ] Touch callout: `TouchSelectionCallout.Select`, `.SelectAll`, `.BIU`, `.Bold/.Italic/.Underline/.Strikethrough`
Table context menu
- [ ] `Table.AddColumn`, `.AddColumnBefore`, `.AddColumnAfter`, `.DeleteColumn`, `.AddRow`, `.AddRowAbove`, `.AddRowBelow`, `.DeleteRow`; `Note.ContentInfo.Table(s)` "%{count} table(s)"
Folder panel / note list
- [ ] `Folders.ContextMenu.Rename/Share/ManageShared.Title`; `FolderPanel.DeleteSelectedFolder.Title` "Delete Folder"; `FolderPanel.FolderSettings.ToolTip` "Edit or delete folders"; `FolderPanel.Tags.Label` "Tags", `.TagCloud.AllTags.Label` "All Tags"
- [ ] `NotesList.ContextMenu.Delete` / `.Pin` "Pin Note" / `.Unpin`; sections `NotesList.Section.Pinned` / `.Notes`
- [ ] Search: `Note.SearchBox.Placeholder` "Search all notes", `Note.SuggestedSearch.*` (Shared, Locked, Checklists, Tags, Drawings, Scanned Documents, Attachments)
Link dialog
- [ ] `LinkModification.AddLink.Title` "Add Link"/`.EditLink.Title`; "Link To:" placeholder "Enter a URL or Note Title"; "Name:" "Optional"; "Use note title as link name"; "Remove Link"/"Cancel"/"Done"
Empty states
- [ ] `Note.Empty.Title` "New Note", `Note.Empty.Snippet` "No additional text", `Note.Empty.List` "No Notes", `Note.RecentlyDeleted.List` "Notes are available here for 30 days…", `Note.Loading` "Loading…"
Dialogs
- [ ] `Alert.DeleteNote.*`, `Alert.DeleteSharedNote.Title` + `DeleteSharedWithYouNote/SharedByYouNote.Body`, `Alert.RecentlyDeletedNoteEditing.*` "Recently deleted notes can’t be edited.", `Alert.DeleteFolder.*` (+Shared Owner/Participant), `Alert.MoveFolder.*` "Notes is limited to 5 levels of folders.", `Alert.OpenNote.NotAvailable.*`, `Alert.SharedNoteWasDeleted.*`, `Alert.FolderShare.*` (locked/shared-to-me/move variants, "Move Anyway"), `Alert.Open{Pages,Numbers,Keynote}File.*`, `Alert.PasswordProtected*`, `Alert.WebGL.*`
- [ ] Locked notes: `Notes.PasswordProtected.*` ("This note is locked.", "Enter Password", "Password incorrect.", "Hint: %{passwordHint}", "Can’t View Note")
Keyboard
- [ ] Format shortcuts come from browser `beforeinput` inputTypes `formatBold/Italic/Underline/StrikeThrough/Indent/Outdent` (L92508-92515), i.e. platform Cmd/Ctrl+B/I/U, Tab/Shift-Tab. VERIFIED-IN-CODE. No explicit custom keymap table found — INFERRED none.
- [ ] Line-start autoformat: `[ ] `, `[x] `, `- `, `* `, numbered (L186690-186706).

## Gaps
- Resolved: style enum, table CRDT schema, checklist write path.
- Open: exact CRTable column-direction values; numbered-list autoformat regex; whether `inflateMergeableData` accepts gzip as well as zlib (only "inflate" seen); E2E `EncryptedValues` inner layout; Folder record-name generation; whether DeletedDate is written on folder delete (field exists, write not found).

## Tables: empty cells (VERIFIED on his account, 2026-10-19)
Every table in the account, whether made on an iPhone or on icloud.com, holds a kind-10 text
object for **every** cell, including ones never typed in. The object has empty text (`2: bytes 0B`)
and only the two sentinel substrings. `cellColumns` maps column → `CRDictionary` (kind 6) of elements
`{1: row object ref {6: idx}, 2: value object ref, 3: timestamp {1: {1: replica idx, 2: clock}}}`.
So an empty cell is edited exactly like a typed one (`table::with_cell`). An in-memory run over
all 10 tables took text in an empty cell and read back exactly (probe `table_cell_offline`).
Web code: `setObject_columnID_rowID` (main.js:121856) makes a column's CRDictionary only when
one is missing. Quince has not needed that path yet.

## Tables: adding/removing rows and columns (VERIFIED-IN-CODE, notes3 main.js, 2026-10-19)
- `insertRow(index)` (121764): `uuid = CRUUID.UUID()`; `rows.insert(uuid, index)` puts the uuid into the **tsOrderedSet**
  `crRows` (DocObject kind 16: a topotext-style array of elements carrying char IDs, replica and clock, plus a dictionary
  uuid→NSUUID object). No cell objects are made; `undoablyInsertContents_atRow(null, uuid)` only records undo.
  Columns are the same through `crColumns` (`insertColumn`, 121767).
- A cell gets its kind-10 text object and its `cellColumns[col][row]` dictionary element only when it is first written
  (`setObject_columnID_rowID`, 121856). That creates the column's CRDictionary if it is missing.
- `removeRowAtIndex` (121776): every column's dictionary drops the row key (`a.delete(row)`), an emptied column dictionary
  is removed, then `rows.delete(index)`. Columns mirror this (121799).
- UI actions (156592+): addRowAbove/Below, addColumnBefore/After, deleteRow(s), deleteColumn(s), deleteTable.
  Strings: "Add Row", "Add Row Above", "Add Row Below", … (177833).
- **For Quince** this needs two writers it does not have yet:
  (1) an ordered-set insert or delete (a topotext array edit plus a dictionary entry, with Quince's own replica UUID added
  to `uuidItem` and a clock above every clock seen), and
  (2) creating a cell object for a never-written cell.
  Both must be byte-proven against Apple's own encoder (round-trip a phone-made table) and tried live on a throwaway
  table before touching his.
- Measured on his 11×2 table (attachment f7e8966f…, read-only, 2026-10-19). `crRows` (kind 16) =
  `OrderedSet{1: Array{1: StringArray{1: topotext.String{2: "￼"×11, 3: substrings (charID{replica, clock}, length,
  timestamp, tombstone?, child*), 5: one attribute run per character}, 2: ArrayAttachment{1: attachmentIndex 0…10,
  2: uuid bytes}*}, 2: Dictionary uuid→NSUUID}, 2: set Dictionary}`.
  A row insert is therefore: one "￼" inserted through the topotext CRDT (the same writer notes use), a new
  ArrayAttachment (the next attachmentIndex, a fresh uuid), an NSUUID custom object and its dictionary entries, and the
  document version vector raised. Still open: how `attachmentIndex` relates to character position after deletes
  (tombstones). Settle it with a table on which a row was deleted on the phone.
- **Settled (2026-10-19, read-only, all 10 of his tables):** `ArrayAttachment.attachmentIndex` is the row's or column's
  **live position**. Deleted rows stay in the topotext as tombstoned substrings and do not count. Table 33c73e38 has rows
  substrings [(1,0,len2), (1,12,len1), (1,25,len1,TOMB)], 3 live chars, attIdx 0–2. Table f7e8966f has two tombstoned
  rows, 11 live, attIdx 0–10. So an insert at k = a topotext insert at live position k (`rewrite_text_object`, the cell
  writer, already does this for a String), every attachmentIndex ≥ k incremented, `{k, uuid}` added, and the uuid's
  NSUUID object plus its dictionary entries added. Decoder used: scratchpad tools/ordered.py.
- **Wiring (table 33c73e38, read-only):** each row or column has two NSUUID custom objects (`UUIDIndex` → uuidItem):
  an *element* (E) and an *identity* (I). In the ordered set, `Array.dictionary` holds E→I and `OrderedSet.set` holds
  E→E. Each element carries a timestamp VectorTimestamp `[(replicaIndex, clock)]`. A deleted row keeps its E→I in
  `Array.dictionary` but leaves `set`. `cellColumns` is column-I → column dictionary, and a column dictionary is
  row-I → kind-10 cell String. Document `version` (field 1) is `[(replicaIndex into uuidItem, clock)]`; here
  `[(0,6), (1,2), …, (8,4)]`, 21 uuidItems.
- **Build recipe for "add row at k":**
  1. Append Quince's replica uuid to `uuidItem` if absent. clock = (its clock in `version`, or 0) + 1.
  2. Append two uuidItems (E, I), two NSUUID objects pointing at them, the elements E→I in `Array.dictionary` and E→E in
     `set`, each stamped `[(quince, clock)]`.
  3. Insert "￼" at live position k in the StringArray's topotext String (`rewrite_text_object`). Shift attachmentIndex
     ≥ k by one and add `{k, E's uuid bytes}`.
  4. Raise Quince's entry in `version` to the clock.
  5. Optionally, per column, add row-I → a new empty kind-10 String (copy a phone-made empty cell's exact shape), so
     the new cells can be typed in at once.
  Prove it by applying the recipe to a copy of a phone-made table, then checking that Quince's own reader gets the
  grid with the row at k. Then run it live on a throwaway table and read the table back from Apple (and on the phone).
- **Built (2026-10-19):** `table::with_row` / `with_column` (one `with_member`) follow the recipe above. Each is proven
  before returning: the NSUUID object, dictionary elements and version vector are rebuilt from the table's own and
  compared byte for byte, other replicas' subclocks are copied untouched, and the result must read back as the same
  grid plus one empty line. Offline it took all 9 of his tables, at the top, middle and end, both axes (54/54), and
  the new cells took text. Live on the throwaway table 7fcc79fb (used only by deleted Quince test notes): rows at the
  top and end, a column in the middle, and text typed in each, all read back from Apple. Still unverified: the iPhone
  rendering it.
- **Delete recipe (read off table 33c73e38, whose 4th row was deleted on the phone):** the row's "￼" is tombstoned
  in the topotext; its ArrayAttachment is gone and the later indices are shifted down; E→E is removed from
  `OrderedSet.set`; the row-I keys are removed from every column dictionary (obj18 and obj22 have no key 10). Only
  E→I stays in `Array.dictionary`, and nothing else marks the delete. Web code agrees: `removeRowAtIndex` deletes from
  each column dictionary, drops an emptied column dictionary from `cellColumns`, then `rows.delete(index)` (121776).
  Column delete mirrors it: the column-I element goes from `cellColumns`. Build it on the same proven writer
  (`with_member`) as a removal, and confirm on the throwaway first.
- **Correction (2026-10-19): a table's texts share one writer list.** In a table document the topotext writer list
  (`replicaUUID` + clocks) is kept once at Document field 7 (`ttTimestamp`). No cell or ordered-set String carries
  field 4. A CharID's replica number counts from that document-level list. Quince's text writer used to add a list of
  its own to the edited text, so its characters claimed writer 1, the phone's number. That affected cell edits (shipped
  earlier) and, briefly, row inserts (1a10fbf). Fixed with `table::edit_table_text`: the document's list is lent to the
  text, Quince joins it as the next writer, and the list is written back to field 7. A text that already carries its
  own list is refused. Damage: only the orphan table f7e8966f (one cell, "a") and the throwaway 7fcc79fb, neither in a
  live note.
