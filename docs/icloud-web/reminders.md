# iCloud.com Reminders web app: reverse-engineered notes

Source: beautified `reminders2/main.js` (build 2638Build16), CloudKit container `com.apple.reminders`.
Path: `scratchpad/icloud-bundles/pretty/reminders2/main.js`. Line numbers refer to that file.
Read-only static analysis. Bundles were never executed.
Tags: **VERIFIED-IN-CODE** means the code shows it directly. **INFERRED** means it is a reasonable reading that I have not proven.

## 1. Transport, zones, record types

- **VERIFIED-IN-CODE**: the service host is `ckdatabasews.<servicesDomain>`. L24238: `d.set("ckdatabasews", { url: "https://ckdatabasews." + l.servicesDomain`.
- **VERIFIED-IN-CODE**: the app uses the embedded CloudKit JS library (`oi.create({... databaseType, partition, services:{fetch: ckFetch}})`) and builds Public, Private and Shared databases (L95500-95520). Private URLs match `"/" + ckEnvironment + "/private/"` (L95528).
- **INFERRED**: the bundle contains no literal `records/modify` or `changes/zone` strings, because CloudKit JS assembles the endpoint paths internally. The endpoints match standard CK web services: `records/modify`, `changes/zone`, `records/lookup`. The decode work in `quince-cloudkit-reminders` memory independently confirms `changes/zone`.
- **VERIFIED-IN-CODE**: the zone is `"Reminders"` (L58400 `i = "Reminders"`, L58443, L95783-95824). Shared lists use the share's `zoneID` (L108350 `v = t.share, y = v ? v.zoneID`).
- **VERIFIED-IN-CODE**: the record types are `Account` (L47771), `Reminder` (L49301), `Hashtag` (L56555), `Alarm` (L56589), `AlarmTrigger` (L56716), `RecurrenceRule` (L56913), `Assignment` (L57873), `cloudkit.share` (L58364), `Attachment` (L105812) and `List` (L108211).
- **VERIFIED-IN-CODE**: record names are prefixed. `RECORD_NAME_PREFIX: "Reminder/"` (L49302) and `"List/"` (L108212), each followed by a UUID (`List/` + `Object(l.a)()`, L108278).

## 2. List record (`List`)

The model is at L108208-108380.

| Field | Encoding | Cite |
|---|---|---|
| `Name` | `type: ArrayBuffer, serviceValueSetter: u.b` (bytes, most likely encrypted or bytes-string, see INFERRED below) | L108318 |
| `Color` | **JSON string** (`getServiceValueFromDataValue: JSON.stringify`). It is parsed back with JSON. Default is blue. | L108307-108313 |
| `BadgeEmblem` | `type: String` | L108304 |
| `Deleted`, `Imported`, `IsGroup`, `IsLinkedToAccount` | boolean (`Object(y.a)()`) | L108314-108317 |
| `ParentList` | reference, `action: VALIDATE, strong, readOnly`. This is the **list group** parent. | L108322 |
| `ReminderIDs` | JSON string array of reminder UUIDs **without** the `Reminder/` prefix. If it is too large, `ReminderIDsAsset` (an `application/json` Blob) is used instead. | L108330-108378 |
| `ResolutionTokenMap` | JSON string | L108380 |

- **VERIFIED-IN-CODE**: the ResolutionTokenMap keys for a list are `reminderIDsMergeableOrdering`, `name`, `parentAccount`, `badgeEmblem` and `color` (L108213-108219).
- **INFERRED**: `Name` is an ArrayBuffer with a custom setter, which probably makes it an ENCRYPTED_BYTES field. This fits the memory note that ENCRYPTED_BYTES is actually gzip/plain bytes. I did not trace `u.b`.

### Color JSON (L58044-58102)

**VERIFIED-IN-CODE**: each palette entry is frozen as follows:
```js
{ daSymbolicColorName: i /* or "custom" */, ckSymbolicColorName: n,
  daHexString: t.rrggbb(), red: t.r, green: t.g, blue: t.b, alpha: 1, colorRGBSpace: 2 }
```
The palette is listed in the table. The columns are the RGB value, `ckSymbolicColorName` and `daSymbolicColorName`. An entry without a `daSymbolicColorName` gets `"custom"`.

| RGB | ck | da |
|---|---|---|
| 255,59,48 | red | red |
| 255,149,0 | orange | orange |
| 255,204,0 | yellow | yellow |
| 52,199,89 | green | green |
| 81,170,242 | lightBlue | custom |
| 0,122,255 | blue | blue |
| 88,86,214 | indigo | custom |
| 234,66,106 | pink | custom |
| 192,119,219 | purple | purple |
| 162,132,94 | brown | custom |
| 91,103,112 | gray | custom |
| 217,166,159 | rose | custom |

- **VERIFIED-IN-CODE**: the default color is the `blue` entry (L58074-58078).
- **VERIFIED-IN-CODE**: colors are resolved to hex by `l(e)` (L58090-58102). It lowercases `ckSymbolicColorName` and then `daSymbolicColorName`. If either one matches a palette entry, it returns that entry's `daHexString`. Otherwise it falls back to `e.daHexString`. Equality is `l(a) === l(b)` (L58086). In practice the symbolic name wins over the hex.
- **VERIFIED-IN-CODE**: `red`/`green`/`blue` are written as 0-255 ints, because they are `t.r` from `new Color(255,59,48)`. The hex is `rrggbb` (`c()` at L58080 strips any leading `#`).
- **Not present**: `isUserGenerated` is never referenced (0 grep hits). The web app neither writes nor reads it. **INFERRED**: native clients may add it. If you round-trip, preserve unknown keys.

### BadgeEmblem (L69159-69200)

- **VERIFIED-IN-CODE**: the allowed values (`cl`, L69159) are `default`, `bookmarks1-2`, `celebration1-2`, `education1-5`, `finance1-3`, `fitness`, `sport1-6`, `food`, `wine`, `health1-2`, `lifestyle1-2`, `location1-3`, `media1-5`, `nature1-2`, `people1-3`, `pet1-3`, `shopping1-4`, `transport1-4`, `vacation`, `weather1-5`, `work1-5`, `concept1-3`, `symbol1-7` and `emoji`.
- **VERIFIED-IN-CODE**: the renderer first tries `JSON.parse(badgeEmblem).Emoji` (L69176 `Object(fs.c)(a, hs.t.any()).Emoji`). If that works, it draws the emoji over the list color. If it fails, it uses the SVG icon for the name, falling back to `default` (`ul`, L69161). A plain string is therefore a symbol name, and an emoji emblem is the JSON string `{"Emoji":"<char>"}`.
- **VERIFIED-IN-CODE**: `ll(a,b)` (L69165) treats a missing value and `"default"` as equal.

### Show List Info dialog (L71520-71580)

- **VERIFIED-IN-CODE**: the dialog seeds its state from the list's `Name`, `Color` (blue if missing) and `BadgeEmblem` (`"default"` if missing) (L71525-71526). The color picker is keyed by `ckSymbolicColorName` (L71403), so it offers palette entries only and has no custom picker. The emblem picker grid shows `cl`.
- **VERIFIED-IN-CODE**: for a new list it calls `writeDataProperty(l,"Name",d)`, then `"Color"` with the full palette object, then `"BadgeEmblem"`, then `saveNewReminderList` (L71572-71574).
- **VERIFIED-IN-CODE**: for an existing list it writes only the fields that changed, each through `updatePropertyAndResolutionTokenForModel(List, pk, ResolutionTokenMapField.Name|Color|BadgeEmblem, value)` (L71580). Every edit therefore also bumps the ResolutionTokenMap entry for that field.
- **VERIFIED-IN-CODE**: a new list starts as `Name: "New List"` (localized), `Color: blue`, `IsLinkedToAccount: true`, `ReminderIDs: []` and a fresh ResolutionTokenMap, in the `remindersZone` (L108275-108293).

## 3. Reminder record (`Reminder`)

The model is at L49299-49720.

- **VERIFIED-IN-CODE**: the fields are `AllDay`, `Completed`, `Flagged` and `Imported` (booleans); `CompletionDate`, `CreationDate`, `DueDate`, `StartDate` and `LastModifiedDate` (Number, ms epoch); `Priority` (Number); `TimeZone` (String); `List` (reference, VALIDATE, strong); `ParentReminder` (reference); `Deleted` (bool); `ImportedICSData` (bytes); `ResolutionTokenMap` (JSON string); `TitleDocument`/`NotesDocument` (+`…Asset`). Cite L49592-49720.
- **VERIFIED-IN-CODE**: the ResolutionTokenMap keys are `allDay`, `titleDocument`, `notesDocument`, `parentReminder`, `priority`, `icsDisplayOrder`, `creationDate`, `list`, `flagged`, `completed`, `completionDate`, `lastModifiedDate`, `recurrenceRuleIDs`, `dueDate` and `timeZone` (L49303-49319).
- **VERIFIED-IN-CODE**: TitleDocument and NotesDocument hold a CRDT (`isCRDT` returns true for these two, L49321). On read, `N()` (L49270) applies `l.inflate(base64decode(str))` (zlib) to get a `TTMergeableStringVersionedDocument`. On write, `x()` archives it, deflates it and base64-encodes it. If the result is larger than half the field limit, it is sent as `TitleDocumentAsset` (an `application/octet-stream` Blob) instead (L49704-49720). A new reminder gets an empty `TTMergeableAttributedString`.
- **VERIFIED-IN-CODE**: `Deleted` arrives on the wire as `1`. The service value only wins if it is newer than the local copy: `return o < u.LastModifiedDate ? n : !!l` (L49601-49619).
- **VERIFIED-IN-CODE**: `ParentReminder` is written as `{recordName, zoneID, action}`. To clear it, the app sends `null`, but only if the original record had a parent (L49658-49690). Subtasks are the non-deleted reminders whose `ParentReminder` equals the pk (L49325-49331).
- **VERIFIED-IN-CODE**: a new reminder gets `recordName "Reminder/"+uuid`, `List`, `CreationDate`, `LastModifiedDate`, empty Title/Notes docs and a ResolutionTokenMap (L49333-49355). Ordering does not live on the reminder. It is the parent List's `ReminderIDs`.
- **VERIFIED-IN-CODE**: `Priority` uses the EventKit values. L73424: `PRIORITY_NONE = 0, PRIORITY_LOW = 9, PRIORITY_MEDIUM = 5, PRIORITY_HIGH = 1`. The labels come from `ip()` (L73402-73411): "Low Priority", "Medium Priority", "High Priority", and "None" as the fallback (L88721). **INFERRED**: the web UI only displays priority, because I found no writer.

## 4. Show Completed, delete, groups

- **VERIFIED-IN-CODE**: **Show Completed is local UI state, not a list field.** It lives in a React reducer as `showCompletedReminders: {}` keyed by list pk (L62621-62645), toggled by `SetShowCompletedReminders` (L73202). Nothing writes it to CloudKit. **INFERRED**: it is not persisted, apart from possible sessionStorage use at L93231, which I did not trace.
- **VERIFIED-IN-CODE**: **Deleting a list is a soft delete.** `TransactionalReminderList.delete` (L72199-72206) does three things. It writes `Deleted` to `true` (`writeDataProperty(storeId,key,previousValue,nextValue)`, L71907). It sets `IsLinkedToAccount` to `false`. It removes the list id from the Account's `ListIDsMergeableOrdering` CRDT (`removeReminderList`, L71995-72005). A list is viewable only if `IsLinkedToAccount && !Deleted` (L70620).
- **INFERRED**: the list-delete path does not touch the reminders inside the list. They become unreachable because their List is Deleted. Reminder delete is separately a `Deleted` flag write (L71968, L72529).
- **VERIFIED-IN-CODE**: **List groups** use `IsGroup` (bool) plus `ParentList` (a reference on child lists). Expanded state is local (`expandedGroups`, L62629). Groups get no action menu (`!n.IsGroup && ...`, L71049). **INFERRED**: the web app cannot create groups, because I found no writer for `IsGroup` or `ParentList` (`ParentList` is `readOnly`).

## 5. UI menus and dialogs

- **VERIFIED-IN-CODE**: the list menu (L73155-73175) contains "Show List Info" (`ReminderList.Menu.ListAppearance`), "Show Completed" or "Hide Completed" (a toggle) and "Delete List" (destructive). The sidebar row also has its own Delete List menu (L71060-71079).
- **VERIFIED-IN-CODE**: the error alerts are `Alert.Error.ReminderListDeleteMessage` ("Unable to Delete Reminder List"), `ReminderListCreateMessage` and `ReminderDeleteMessage` (L60241-60250).
- **VERIFIED-IN-CODE**: the reminder detail reducer (L77870-77890) handles the actions `modifyFlag`, `deleteLocation` and `deleteDate` (alarm triggers). The detail panel has the fields Notes, Due Date, Tags, Location, List and Attachments (strings at L60286-60299, L60421).
- **VERIFIED-IN-CODE**: there is no right-click context menu for reminders. Indent and outdent are Cmd/Ctrl+`]` and Cmd/Ctrl+`[` (L89696-89720). See §12.

## 6. ResolutionTokenMap (all record types)

The helpers are module `2042` (L33938-33986) and `2043` (L33997-34034).

- **VERIFIED-IN-CODE**: the field value is a JSON **string** (`getServiceValueFromDataValue: JSON.stringify`, L49689-49696) that wraps a `map` object. Each key in the map is a lowercase token name from `ResolutionTokenMapField` (for example `dueDate`, `completed`, `reminderIDsMergeableOrdering`).
- **VERIFIED-IN-CODE**: a new token is `{counter: 1, modificationTime: u(), replicaID: <UUID>}` (L34001-34005). A bump is `{...old, counter: old.counter + 1, modificationTime: u()}` (L34008-34013). A bump keeps the old `replicaID`. Rollback decrements `counter` and deletes the entry when it reaches 0 (L34016-34020, L33965-33971).
- **VERIFIED-IN-CODE**: `modificationTime` is **seconds since the Apple reference date (2001-01-01Z)**, as a float. L34032: `return (Date.now() - 9783072e5) / 1e3`. `replicaID` is an uppercase UUID (L34041 `Object(r.a)().toUpperCase()`).
- **VERIFIED-IN-CODE**: tokens are compared by `counter`, then `modificationTime` (with an epsilon), then `replicaID` using the UUID TTCompare (L34022-34029). The higher token wins the merge.
- **VERIFIED-IN-CODE**: when a record is created, every field in `ResolutionTokenMapField` gets a fresh token (`i(e)` L33949-33955, called at L49351 and L108290). A missing token is backfilled with a new one (`s()` L33976-33985).
- **VERIFIED-IN-CODE**: `updatePropertyAndResolutionTokenForModel(Model, pk, field, value)` (L71896-71906) does two things. It writes the field (`field.name`, for example `DueDate`) and it bumps `map[field.value]` (for example `dueDate`). `writeReminderFields` (L72267-72276) uses the token path for any key that has a token and a plain write otherwise. It always bumps `lastModifiedDate` as well.

Exact wire value:
```json
"ResolutionTokenMap": {"value": "{\"map\":{\"dueDate\":{\"counter\":2,\"modificationTime\":813931200.123,\"replicaID\":\"6F1C…-UUID\"},\"completed\":{\"counter\":1,\"modificationTime\":813900000.5,\"replicaID\":\"…\"}}}"}
```
(The values here are illustrative, but the shape is the one in the code.)

## 7. Reminder writes

### Create (L49333-49355, L72212-72229, L72142-72197)
- **VERIFIED-IN-CODE**: a new Reminder carries `recordName "Reminder/"+UUID`, `List` (ref), `CreationDate` and `LastModifiedDate` (ms epoch, both `Date.now()`), empty `TitleDocument` and `NotesDocument` CRDTs, and a full fresh `ResolutionTokenMap`. A child reminder ("create after" a subtask) also gets `ParentReminder` through the token path (L90642).
- **VERIFIED-IN-CODE**: the same transaction inserts the reminder's id into the parent List's `ReminderIDs` at `start` (0), `end` (length) or `after` a given pk (`insertReminder`, L72142-72155). It then bumps the list token `reminderIDsMergeableOrdering` (L72176-72196). `PlaceholderAtEnd` and `PlaceholderAtPosition` are local-only flags (L72214, L90656).
- **VERIFIED-IN-CODE**: a new Reminder or List is committed in an **atomic** batch. Parents are ordered before children (`ot()` L92458-92466, `orderTheAtomicChanges` L92416-92425, `n.atomic = a` L92373). Updates that touch `Deleted`, `Completed`, List `ReminderIDs` or Account `ListIDsMergeableOrdering` are also atomic. Everything else goes in a non-atomic batch.

### Complete (L72230-72265)
- **VERIFIED-IN-CODE**: `writeCompletionProperty` writes exactly three fields per reminder, each through the token path: `CompletionDate` = `Date.now()` (or `undefined` when un-completing), `Completed` = bool, `LastModifiedDate` = `Date.now()` (L72232). **List ordering (`ReminderIDs`) is not touched.**
- **VERIFIED-IN-CODE**: completing a parent cascades to every incomplete child. Un-completing restores only the children whose `CompletionDate` equals the parent's (`childrenPKsToUpdateCompletion`, L72277-72289).
- **VERIFIED-IN-CODE**: **recurring reminders are not completed by a field write.** The app runs a server-side query with the virtual record type `CompleteRecurringReminder` (L72449-72498):
```json
{"recordType":"CompleteRecurringReminder","filterBy":[
 {"comparator":"EQUALS","fieldName":"Reminder","fieldValue":{"value":{"recordName":"Reminder/<id>","action":"VALIDATE"},"type":"REFERENCE"}},
 {"comparator":"EQUALS","fieldName":"RecurrenceRule","fieldValue":{"value":{"recordName":"RecurrenceRule/<id>","action":"VALIDATE"},"type":"REFERENCE"}},
 {"comparator":"EQUALS","fieldName":"TimeZone","fieldValue":{"value":"<IANA tz>","type":"STRING"}}]}
```
  The query is sent to `zoneID {zoneName, ownerRecordName}` and the records it returns are ingested. That means the server rolls the due date forward.
- **VERIFIED-IN-CODE**: other server-side virtual queries are `TreeDeletion` (`recordIds` REFERENCE, for list delete, L70716), `LookupValidatingReferences` (L92596), `Lists` (L92671, L95732), `SearchIndexes` (L95766) and `reminderList` (with filters `List`, `includeCompleted` INT64 and `LookupValidatingReference` INT64 1, L58476-58502).

### Due date and all-day (L90583-90612, L75059-75080, L88490-88582)
- **VERIFIED-IN-CODE**: the date picker produces `dateComponentData = {year, month, day, hour, minute, second: 0, timeZone: {identifier}}` (L88553-88567). The identifier is the browser's IANA zone (`tC()` L83704, `Intl.DateTimeFormat().resolvedOptions().timeZone`).
- **VERIFIED-IN-CODE**: `handleEditDate` first deletes the existing date Alarm and its AlarmTrigger and clears `DueDate`, `AllDay` and `TimeZone`. It then sets `DueDate = Mg(components)`, `AllDay = allDay` and `TimeZone = components.timeZone.identifier`. **A timed date (`!allDay`) also creates an Alarm plus an AlarmTrigger of type `Date`. An all-day date creates no alarm** (L90606-90609). When the date is removed, `RecurrenceRuleIDs = []` and the RecurrenceRule records are deleted (L90591-90601).
- **VERIFIED-IN-CODE**: `Mg()` converts wall-clock components in the given zone to UTC ms: `Date.UTC(y, m-1, d, h, min, s)` minus the zone offset (L75059-75078).
- **VERIFIED-IN-CODE**: on read, an all-day `DueDate` is treated as a **floating UTC-midnight** value. L88497: `t && e.setMinutes(e.getMinutes() + e.getTimezoneOffset())` (applied when `allDay`). A timed reminder displays the AlarmTrigger's `DateComponentsData` in preference to `DueDate` (L89069-89081).
- **INFERRED**: for all-day the web writer builds midnight components in the *local* zone and runs them through `Mg`, which gives local midnight rather than UTC midnight. The reader expects UTC midnight. **For Quince, write all-day `DueDate` = UTC midnight of the date, `AllDay = 1`, and `TimeZone` = the IANA zone, which matches the reader and EventKit's floating semantics.** Verify this against a natively created all-day reminder.

## 8. Alarm and AlarmTrigger (L56589-56870)

**Alarm** (`Alarm/<UUID>`):
| Field | Value | Cite |
|---|---|---|
| `AlarmUID` | the same UUID as the record name suffix | L56598, L56620 |
| `Reminder` | REFERENCE, VALIDATE, strong | L56634-56639 |
| `TriggerID` | the AlarmTrigger record name **without** the `AlarmTrigger/` prefix | L56643-56662 |
| `DueDateResolutionTokenAsNonce` | `counter * 1e11 + modificationTime` of the reminder's `dueDate` token | L56603-56607 |
| `CreationDate`, `LastModifiedDate` | ms epoch | L56614-56615 |
| `Deleted`, `Imported` | false | L56621-56622 |

**AlarmTrigger** (`AlarmTrigger/<UUID>`):
- **VERIFIED-IN-CODE**: `Alarm` is a REFERENCE (VALIDATE, strong) and `Deleted` is set. `Type` is one of `Date`, `Location`, `TimeInterval` or `Vehicle` (L56873-56875).
- `Date`: `DateComponentsData` holds base64 of JSON `{year,month,day,hour,minute,second,timeZone:{identifier}}` (L56785-56797), and `Imported: 0`.
- `Location`: `Latitude`, `Longitude` (encrypted DOUBLE), `Radius` (m), `Proximity` (0 NONE / 1 ENTER = arriving / 2 LEAVE = leaving, L75124), `Title`, `Address` and `ReferenceFrameString: "1"` (encrypted STRING), plus `LocationUID`. It may also carry `MapKitHandle` and `Routing` (bytes) and `ContactLabel`.
- `Vehicle`: `Event` (Number, enter or exit car).
- `TimeInterval`: `TimeInterval` (Number). **INFERRED**: this is a relative or early alarm. The web app never creates one.
- **VERIFIED-IN-CODE**: the encrypted setters set `fields[k].isEncrypted = true` and type `STRING` or `DOUBLE` (L56880-56894).
- **VERIFIED-IN-CODE**: the links are `Reminder.AlarmIDs` (alarm UUIDs without prefix, L49567) → `Alarm.TriggerID` → `AlarmTrigger`, and back-references go `AlarmTrigger.Alarm` → `Alarm.Reminder`. On delete, the app sets `Deleted = true` on both records and clears `Alarm.Reminder` and `AlarmTrigger.Alarm` (L90507-90515).
- **INFERRED**: there is no "early reminder" field in this bundle. Native clients most likely model it as a second Alarm of type `TimeInterval`.

## 9. RecurrenceRule (L56899-57040)

- **VERIFIED-IN-CODE**: the fields are `Frequency`, `Interval`, `OccurrenceCount`, `EndDate` (ms), `FirstDayOfTheWeek` (Number), `Imported` (Number), `Deleted`, `Reminder` (REFERENCE, VALIDATE). The fields `DaysOfTheWeek`, `DaysOfTheMonth`, `DaysOfTheYear`, `WeeksOfTheYear`, `MonthsOfTheYear` and `SetPositions` each hold **base64(JSON)**.
- **VERIFIED-IN-CODE**: `Frequency` maps 0 = daily, 1 = weekly, 2 = monthly, 3 = yearly, 4 = hourly, 5 = minutely and 6 = secondly (`_m`, L76871-76887).
- **VERIFIED-IN-CODE**: each `DaysOfTheWeek` entry is `{dayOfTheWeek: 1..7 (1 = Sunday), weekNumber?: n}` (`wm` L76888-76906, `Cm` L76915-76929). The `nth` value comes from `weekNumber` or the first `SetPositions` entry. `-1` means last (L76933-76947).
- **VERIFIED-IN-CODE**: the link is `Reminder.RecurrenceRuleIDs` (UUIDs without prefix). The app uses only `[0]` (L71697, L89063).
- **INFERRED**: the web app only reads and deletes RecurrenceRules (it has `lE.delete` and no create path). Its NewFeature string says it "complete[s] recurring reminders". Native clients create the rules.

## 10. Hashtag, Assignment, Attachment/URL

- **VERIFIED-IN-CODE** Hashtag (`Hashtag/…`, L56553-56568): `Name` (String, readOnly, no `#`), `Deleted` and `Reminder` (REFERENCE, readOnly). The reminder lists them in `HashtagIDs`. The UI renders `"#" + Name` (L89133). **INFERRED**: the web UI is read-only for tags.
- **VERIFIED-IN-CODE** Assignment (`Assignment/…`, L57871-57897): `AssignedDate` (Number), `EncryptedAssigneeIdentifier` and `EncryptedOriginatorIdentifier` (String), `OwningReminderIdentifier` (String), `Status` (bool), `MinimumSupportedAppVersion`, `Imported`, `Deleted` and `Reminder` (ref, readOnly). These are listed in `AssignmentIDs`, and only `[0]` is used (L89082).
- **VERIFIED-IN-CODE** Attachment (`Attachment/…`, L105799-105910): `Type` is one of `File`, `Image` or `URL` (L105785). The other fields are `URL` (base64 string, decoded and validated, L105888-105899), `FileName` (base64 string), `FileAsset` and `MetadataAsset` (assets; the metadata is a bplist with `icon` or `image` data), `Metadata` (base64 bplist), `FileSize`, `Width`, `Height`, `UTI`, `Imported`, `Deleted` and `Reminder` (ref, readOnly). These are listed in `Reminder.AttachmentIDs`. **The reminder "URL" is therefore an Attachment record of `Type: "URL"`, not a Reminder field.** Every service setter on this model is a no-op, so the web app is read-only for attachments.

## 11. CloudKit JS request building

- **VERIFIED-IN-CODE**: the CloudKit JS library is **not** in this bundle. `newRecordsBatch` is only called (L92374) and never defined, and `operationType` appears only in app error contexts. The app calls `db.newRecordsBatch({zoneID,…, atomic}).createOrUpdate(records).delete(records).commit()` (L92372-92385). A record counts as a create when it has no `recordChangeTag`, and as an update when it has one (`rt` and `it`, L92450-92457). The serialized record is `{recordName, recordType, recordChangeTag, fields:{K:{value[,type,isEncrypted]}}}` (L108970-108980, L56700 setter `e.fields[t].value = n`). After a commit, the app saves `httpResponse.body.syncToken` for the zone (L92412-92413).
- **INFERRED** (standard CK JS behaviour): `createOrUpdate` emits `operationType: "create"` when there is no change tag and `"update"` (with `recordChangeTag`) when there is one. It posts `records/modify` with `{operations, zoneID, atomic}`. Quince should send `update` with the last-seen `recordChangeTag` and handle `CONFLICT` by refetching and merging through the ResolutionTokenMap.

## 12. UI checklist

Menus and actions:
- [ ] List menu: Show List Info (`ReminderList.Menu.ListAppearance`), Show Completed / Hide Completed (`…ShowCompleted` / `…HideCompleted`), Delete List (`…DeleteList`) (L73155-73175)
- [ ] Sidebar row menu: Delete List (L71060-71079); no menu on groups
- [ ] "Add List" button (`Button.AddList`); new list default name "New List"
- [ ] List Info dialog: title "List Info" / "“%{name}” Info", "List Color" picker (12 swatches), "List Icon" emblem grid, Done/Cancel
- [ ] Delete confirmation: "Delete list “%{listName}”?" + "This will delete all reminders in this list." ; shared variants "Delete shared list “%{listName}”?" with SharedByMe / SharedToMe descriptions
- [ ] "New Reminder" row (`ReminderList.NewReminder`); placeholder title "New Reminder", notes placeholder "Notes"
- [ ] Details popover ("Details", Save/Cancel): "Remind me on a Day" switch, Date + Time fields, Repeat (read-only text) + End ("Never"), Location ("Remind at Location", Arriving/Leaving, Radius "%{radius} m", "Current Location", "Getting in/out of Car", "Custom…", search "Search or Enter Address", Confirm), Priority (display), List, Tags, URL, Images, "Assign Reminder"
- [ ] Shared labels: "Shared by %{name}", "Shared with …" variants, fallback "This list is shared"
- [ ] Touch selection callout: Select / Select All

Keyboard (L89458-89720, L66096-66121):
- [ ] Row: Enter = edit title / open details, Escape = return focus to row, Space = toggle complete (no Ctrl)
- [ ] List: ArrowUp/ArrowDown move selection (Shift = extend, Cmd/Ctrl = move item), Home/End, Cmd/Ctrl+A select all, Ctrl+Space toggle selection, Backspace delete selected
- [ ] Cmd/Ctrl+`]` indent (make subtask), Cmd/Ctrl+`[` outdent
- [ ] Click modifiers: Shift-click extend, Cmd/Ctrl-click inclusive select
- [ ] Menus: arrows, Home/End, Enter/Space activate, type-ahead by letter
- [ ] No right-click context menu (only `preventDefault` on calendar cells, L87376)

Empty and loading states:
- [ ] "No List Selected", "No Reminders", "All Items Completed", "Loading Reminders…", "Loading…"
- [ ] Sidebar empty: "No Lists" + "Use Reminders on iOS, iPadOS, or macOS to create a new list."
- [ ] Dates: Today/Tomorrow/Yesterday (+ ", %{time}"), "%{date}, Overdue"; recurrence strings `Reminder.Date.Recurring.*` (Daily/Weekly/Monthly/Yearly, "Every %{interval} …", nth weekday)

Error alerts (`Alert.Error.*`, L60200-60260): ReminderList Add/Create/Update/Delete/Accept, Reminder Create/Update/Delete/Completion, Reminder.Save ("Unable to Save Changes"), ReminderCompletionRecurringDescription ("This is a repeating reminder. Complete repeating reminders using Reminders for iOS, iPadOS, and macOS."), Location errors, WebGL alert.

## Gaps

- I have not captured a real `records/modify` body, because the CK JS library is external to this bundle. The `operationType` values are INFERRED.
- I did not resolve the `Name` serviceValueSetter (`u.b`) on List.
- The all-day write semantics conflict: the writer produces local midnight and the reader expects UTC midnight. Confirm against a native record.
- Native clients may set early-reminder (`TimeInterval`) alarms, `Priority`, hashtags, assignments and RecurrenceRule creation. The web app writes none of these, so the exact native shapes still need to be observed.
- I did not trace the `Status` and `Event` enum values (Assignment and Vehicle).
