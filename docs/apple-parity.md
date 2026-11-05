# Apple parity: Quince vs macOS Calendar, Reminders, Notes, Contacts, iCloud Drive

Sources: Apple's macOS user guides (Calendar, Reminders, Notes, Contacts; macOS 27 TOCs), each guide's
keyboard-shortcuts page, the iCloud.com user guide TOC, and the Mac Help page on iCloud Drive in Finder.
Read 2026-10-16. Quince code read at the same date. Linux mapping: Cmd = Ctrl; Apple chords using both Control and Command are written Control+Cmd+ and need a separate Linux mapping.

Feasibility key: **WEB** = the iCloud web service already exposes it (icloud.com does it). **WEB?** = probably
there, payload not yet verified. **LOCAL** = client-side only, no API needed. **NO** = impossible
(end-to-end encrypted or device-only).

---

## Calendar

Quince has: Day/Week/Month/Year views, drag-to-create and double-click-to-create, popover editor
(title, location, all-day, start/end, calendar, notes), calendar show/hide checkboxes, search,
delete with undo. Backend preserves alarms/guests/recurrence when it can read the existing event.

### (a) Missing features, by daily-use impact
| # | Feature | Feasible |
|---|---|---|
| 1 | Alerts on events (set/edit "15 min before", etc.). Model has none; only preserved on edit | WEB (`alarms` array in event body, calendar.rs:443) |
| 2 | Repeating events: create/edit recurrence; "this event / all future" choice on edit and delete | WEB (`recurrence`, `changeRecurring`, calendar.rs:441-442) |
| 3 | Invitees: add guests, see RSVP state, accept/decline invitations | WEB (`invitees`, icloud.com does accept/decline) |
| 4 | Drag an existing event to move it, drag its edge to resize | LOCAL + existing `event_save` |
| 5 | Quick Event / natural language ("Dinner at 6-7", "Party 6 Feb") | LOCAL parser |
| 6 | URL and attachments fields | URL WEB; attachments WEB? |
| 7 | Travel time | WEB? (CalDAV X-APPLE-TRAVEL-*; not in icloud.com UI) |
| 8 | Per-event time zone picker / Floating | WEB (`tz` already parsed, calendar.rs:155) |
| 9 | Create, rename, recolour, delete calendars; set default calendar | WEB (icloud.com does all) |
| 10 | Share calendar (private with people, public link), manage shared | WEB |
| 11 | Subscribed calendars (webcal), holidays, birthdays calendar | Subscribe WEB?; birthdays LOCAL from Contacts (occasions.rs exists) |
| 12 | Import/export .ics | LOCAL + event_save |
| 13 | Restore deleted calendars/events (icloud.com Data Recovery) | WEB? (separate recovery endpoint) |
| 14 | Availability panel, Siri suggestions, Focus filters, lunar calendars | Availability NO via web; rest LOCAL/NO |
| 15 | Print | LOCAL |

### (b) Shortcuts missing (Apple list vs CalendarView.tsx:371-395)
Have: Ctrl+Left/Right, T/Ctrl+T, Ctrl+1..4, Ctrl+N, Ctrl+F, Esc, Delete in popover.
Missing:
- Shift+Ctrl+T go to date
- Ctrl+E edit selected event; Opt+Ctrl+I inspector window; Ctrl+I info
- Tab / Shift+Tab / arrows select next/previous event (no event selection model at all)
- Ctrl+Opt+Up/Down move event 15 min (or a week in Month); Ctrl+Opt+Left/Right move a day
- Ctrl+R refresh all calendars
- Ctrl+Plus/Minus text size
- Space toggles selected calendar; Ctrl-click checkbox toggles all; Opt+Ctrl-click shows only that one
- Shift+Ctrl+N new calendar group; Opt+Ctrl+S new subscription
- Ctrl+P print; Shift+Ctrl+A availability; Ctrl+, settings
- Return closes event editor (Quince: Return submits form, fine; Esc discards rather than saving like Apple)

### (c) UI/UX differences
- Apple: a click on an event selects it; double-click opens. Quince opens the popover straight away; no selected state.
- Apple Esc/Return closes and keeps edits; Quince Esc calls `onClose` (CalendarView.tsx:1477), verify whether edits are dropped.
- Apple's Week view can be Mon-Fri only and shows 6-24 visible hours; start/end-of-day shading. Not present.
- Month view in Apple: double-click a day, type "Dinner 6-7". Quince creates an untimed draft.
- Apple has a separate inspector window option ("Open events in separate windows"). None here.
- Hidden calendars persist only in localStorage and are "forgotten on restart" in one path (CalendarView.tsx:261). Apple syncs visibility per device.

### (d) Likely bugs / edge cases
- **Silent data loss on edit when eventdetail fails.** `event_detail` returns `None` on any network error or non-2xx (calendar.rs:414-417). `put_event` then builds a fresh body with `recurrence: null, alarms: [], invitees: []` (calendar.rs:437-447) and PUTs it over an existing event. A transient failure turns a weekly meeting with guests into a one-off with no alerts. Fix: for an event with an etag, treat `None` as an error, not as "new".
- **Recurring occurrences.** `Event` carries no recurrence/occurrence id; delete posts `{guid, pGuid, etag}` (calendar.rs:371). Deleting one occurrence of a series likely deletes the series (or 404s, depending on how iCloud names instances). Needs a live test.
- Delete on 404 does a second `event_detail`; if that also fails transiently, the 404 is surfaced as an error even though the event is gone (calendar.rs:389).
- `endsAt` clamped to start+15 min (CalendarView.tsx:294): a zero-length event (common in imported ICS deadlines) gets silently lengthened on any edit.

---

## Reminders

Quince has: lists (create/rename/delete), smart lists, sections (read), search, add, tick, flag, priority,
notes, due date (with alarm written alongside, main.rs:791-806), repeat, subtasks (parent), move list,
show/hide completed, delete.

### (a) Missing features
| # | Feature | Feasible |
|---|---|---|
| 1 | Keyboard-first entry: Return creates the next reminder, Tab/Shift-Tab indent/outdent | LOCAL (edit `parent_guid` exists) |
| 2 | Drag to reorder, drag between lists, drag to nest | WEB (CloudKit list `ReminderIDs` ordering; membership code reminders.rs:706) |
| 3 | Early reminder / custom alert time separate from due date; date without time | WEB (alarm records, set_alarm_cloudkit reminders.rs:3692) |
| 4 | Tags (#tag) and tag browser; custom Smart Lists with filters | WEB (CloudKit `Hashtag` records) |
| 5 | Create/rename/move/delete sections; columns view | WEB (sections are CloudKit records; read already works) |
| 6 | URL and images on a reminder | URL WEB; images WEB? (asset upload) |
| 7 | Location-based reminders (arrive/leave) | WEB to store; triggering NO (needs device geofence) |
| 8 | Share list, assign to person, accept/leave shared lists | WEB? (CloudKit sharing; icloud.com supports it) |
| 9 | List icon/colour, list groups, pin lists, default list | WEB |
| 10 | Desktop notifications when due | LOCAL (notification queue item already in brief) |
| 11 | Grocery lists auto-categories, templates, Apple Intelligence | Grocery categorisation NO (on-device ML); templates WEB? |
| 12 | Restore recently deleted reminders (icloud.com "restore") | WEB? |
| 13 | Sort by due date/creation/priority/title | LOCAL |
| 14 | Print | LOCAL |

### (b) Shortcuts missing (RemindersView.tsx has only Enter/Esc on fields, :703, :829, :1005)
All of Apple's list is missing: Ctrl+N new reminder, Shift+Ctrl+N new list, Opt+Ctrl+N new section,
Control+Cmd+N section with selection, Ctrl+1..9 pinned list, Ctrl+] / Ctrl+[ indent/outdent, Ctrl+E / Shift+Ctrl+E
show/hide all subtasks, Shift+Ctrl+F flag, Shift+Ctrl+C complete, Shift+Ctrl+H show/hide completed,
Ctrl+T due today, Opt+Ctrl+T tomorrow, Control+Cmd+T all overdue to today, Ctrl+K this weekend,
Opt+Ctrl+K next week, Opt+Ctrl+S sidebar, Ctrl+P print. No row selection model, so none can work yet.

### (c) UI/UX differences
- Apple: the "i" button opens a details popover (notes, URL, date, time, location, flag, priority, image). Quince edits inline in the row.
- Apple shows date and time as separate toggles; Quince uses a single datetime value (`setDue`, RemindersView.tsx:328-333).
- Apple's Today/Scheduled smart lists group by time of day / date; check Quince's smart list sections match.
- Completed reminders animate out after a short delay in Apple; verify Quince does not jump rows instantly.
- Cap message "Showing the first N" (RemindersView.tsx:941) has no Apple equivalent; Apple virtualises.

### (d) Likely bugs / edge cases
- **Half-applied edit.** `reminder_edit` writes the edit, then `set_alarm_cloudkit` (main.rs:787-806). If the alarm write fails, the error returns before `refresh_reminders`, so the UI shows a failure while the date change is already on Apple, and the local library stays stale until the next sync.
- `setDue` uses `new Date(value)` (RemindersView.tsx:331): an invalid string gives `NaN` -> `Math.floor(NaN)` sent as `dueAt`; guard needed.
- Clearing a date sends `dueAt: null` -> `due_at: Some(None)` -> alarm cleared, good; but an all-day reminder (date only) has no way to be created from the UI, so Apple-created all-day reminders edited here may gain a time. Verify.
- `reminder_edit` looks up the reminder via `reminders_in(&list_guid, true)` (main.rs:778-784): if the UI's `listGuid` is stale after a move on another device, the edit fails with "no longer in the local library" instead of resyncing.

---

## Notes

Quince has: folders, pinned section, search, create, plain-text edit of title+body with byte-proof rewrite
(ckdocument.rs:643-666), move, pin, delete to Recently Deleted, restore, locked notes shown as locked.

### (a) Missing features
| # | Feature | Feasible |
|---|---|---|
| 1 | Rich text: Title/Heading/Subheading/Body/Mono styles, bold/italic/underline/strike | WEB (attribute runs in the protobuf; rebuild_note already keeps runs) |
| 2 | Checklists (tick boxes), bulleted/dashed/numbered lists | WEB (paragraph style in attribute runs) |
| 3 | Show attachments: images, PDFs, scans, drawings inline (currently U+FFFC placeholders in a textarea) | WEB to read (Attachment + Media records); drawings/scans render WEB? |
| 4 | Tables | WEB? (table is a mergeable CRDT attachment, mergeable.rs exists) |
| 5 | Links, note-to-note links (>>), Option+Ctrl+[ / ] navigation | WEB |
| 6 | Add attachments (Shift+Ctrl+A) | WEB? (asset upload) |
| 7 | Create/rename/delete folders, subfolders | WEB |
| 8 | Tags and Smart Folders | WEB (hashtag inline attachments) |
| 9 | Gallery view and Attachments browser | LOCAL once attachments are read |
| 10 | Sort options (date edited/created/title), group by date | LOCAL |
| 11 | Shared notes/folders, highlights, activity list | WEB? (CloudKit share zone) |
| 12 | Locked notes | NO without the note password; with password it is client-side decrypt (PBKDF2+AES-GCM) so technically LOCAL, but high risk |
| 13 | Audio recording + transcription, Math Notes, Quick Note | Store WEB?; transcription/maths LOCAL only |
| 14 | Export (PDF/Markdown), print | LOCAL |
| 15 | Duplicate note (Ctrl+D) | WEB (create with copied body) |

### (b) Shortcuts missing (NotesView.tsx has only Ctrl+S, Enter/Arrow/Backspace title-body glue, :631-670)
General: Ctrl+N, Ctrl+D, Shift+Ctrl+N, Control+Cmd+S sidebar, Ctrl+1/2/3 list/gallery/attachments,
Opt+Ctrl+F search, Tab between panes, Return to edit selected note, Ctrl+Return back to list, Ctrl+P.
Editing: Shift+Ctrl+A attach, Ctrl+K link, Ctrl+L divider, Opt+Ctrl+T table, Shift+Ctrl+T/H/J/B/M styles,
Shift+Ctrl+7/8/9/L lists and checklist, Ctrl+' quote, Ctrl+Plus/Minus size, Ctrl+] / Ctrl+[ and Tab/Shift+Tab list level,
Shift+Ctrl+U tick checklist item, Control+Cmd+Up/Down move item, Shift+Ctrl+. , 0 zoom. All table-navigation keys.

### (c) UI/UX differences
- Apple's editor is one rich surface; Quince is two textareas glued together (title line + body). Selecting across title and body, or undo across them, will not behave like one document.
- Apple shows a format (Aa) toolbar button, checklist button, table, attachment, share and lock buttons; none here.
- List rows in Apple show date + snippet + thumbnail of first image; Quince shows snippet only (NotesView.tsx:519).
- Apple's locked note shows a lock screen with password field; Quince shows explanatory text (NotesView.tsx:679-682).

### (d) Likely bugs / edge cases
- **Formatting at the edit point.** `replace_text` swaps the whole string and rebuilds runs (ckdocument.rs:655-663). A note with a checklist or heading, edited in the middle, has to map old runs onto new text; insertions near a style boundary will likely inherit the wrong style (e.g. new line becomes a checklist item or loses one). Needs targeted tests with styled notes.
- Attachments: deleting or typing over a U+FFFC character in the textarea removes an attachment reference while its Attachment record survives (orphan) or the run lengths shift. Should be protected in the editor.
- Save happens on blur (NotesView.tsx:645-648, 671-674) and Ctrl+S only; closing the window or switching app mid-typing relies on blur firing. Check there is also a debounce/unload flush.
- Edits overwrite Apple's current text (notes.rs:408-423): a concurrent phone edit between lookup and update is protected by the change tag, but an edit made on the phone after Quince opened the note and before save is replaced wholesale, not merged (no CRDT merge, though topotext.rs exists).

---

## Contacts

Quince has: list, search, detail, create/edit name, company, phone/email/url with labels, birthday,
photo display, delete; addresses shown read-only; update merges onto Apple's card (contacts.rs:350-368).

### (a) Missing features
| # | Feature | Feasible |
|---|---|---|
| 1 | Edit postal addresses (read-only now, ContactsView.tsx ~277) | WEB |
| 2 | Groups/lists: view, create, add/remove members | WEB (icloud.com does lists) |
| 3 | More fields: nickname, job title, department, related names, dates, social profiles, notes, phonetic names, prefix/suffix | WEB |
| 4 | Change contact photo | WEB |
| 5 | Import/export vCard (.vcf), archive | WEB (icloud.com imports/exports vCard) |
| 6 | My Card | WEB? |
| 7 | Merge duplicates / link cards | LOCAL merge + WEB writes |
| 8 | Sort by first/last, display order, company cards (Ctrl+\) | LOCAL / WEB for isCompany |
| 9 | Smart Lists | LOCAL |
| 10 | Actions: click phone/email/address to call/mail/map | LOCAL (tel:/mailto:/geo:) |
| 11 | Restore deleted contacts | WEB? (data recovery) |
| 12 | Print labels/envelopes | LOCAL |

### (b) Shortcuts missing (none implemented in ContactsView.tsx)
Ctrl+N new card, Shift+Ctrl+N new list, Opt+Ctrl+N smart list, Ctrl+S save, Ctrl+O import, Ctrl+P print,
Ctrl+L edit, Ctrl+1 / Control+Cmd+S toggle lists, Opt+Ctrl+L last import, Ctrl+] / Ctrl+[ next/previous card,
Shift+Ctrl+M My Card, Shift+Ctrl+L merge, Ctrl+\ company/person, Opt+Ctrl+I custom image, Ctrl+I separate window, Ctrl+, settings.
Also arrow keys in the list.

### (c) UI/UX differences
- Apple: three columns (lists | names | card) with A-Z section headers and index; Quince has no lists column.
- Apple edits in place on the card with "add field" menu; Quince has an edit draft with an add-method button only for phone (ContactsView.tsx ~330).
- Apple shows label pickers with standard labels (home, work, mobile, iPhone...) plus custom.

### (d) Likely bugs / edge cases
- `create` requires a name or company (contacts.rs:325); a contact with only an email/phone (common from phones) cannot be created, Apple allows it ("No Name").
- `delete` sends the locally cached `etag` (contacts.rs:384-387); after an edit on another device it will 412 -> `Conflict` with no auto-resync-and-retry path visible in the command.
- Clearing a field in the draft: `card_from(contact, existing)` merges onto the existing card; check that a removed phone is actually removed and not re-supplied from `existing`.

---

## iCloud Drive (vs Finder + icloud.com Drive)

Quince has: folder browsing with breadcrumbs, upload (picker + drag-and-drop), download, new folder,
rename, delete to Recently Deleted, move sheet.

### (a) Missing features
| # | Feature | Feasible |
|---|---|---|
| 1 | Recently Deleted view: restore, delete permanently | WEB (icloud.com does it) |
| 2 | Preview (Quick Look) for images/PDF/text/video | LOCAL after download |
| 3 | Sort by name/kind/date/size; list vs icon view | LOCAL |
| 4 | Multi-select (Shift/Ctrl-click) for move/delete/download | LOCAL + existing calls |
| 5 | Share file/folder, collaborate, copy link, manage/stop sharing | WEB (icloud.com does it) |
| 6 | Real local sync folder (Finder-style "keep downloaded", "remove download", status icons) | LOCAL + WEB; large piece of work |
| 7 | Search across Drive | WEB? (icloud.com has search) / LOCAL over synced index |
| 8 | Folder download as zip, drag out to the desktop | LOCAL |
| 9 | Desktop & Documents folders, Optimize Storage | LOCAL concept only |
| 10 | iWork/app-library folders show correctly (Pages/Numbers/Keynote packages) | WEB (read), editing NO |
| 11 | Storage usage | WEB |
| 12 | Advanced Data Protection accounts | NO (web access disabled when ADP is on) |

### (b) Shortcuts missing (DriveView.tsx has only Esc/Enter on fields)
Finder: Shift+Ctrl+N new folder, Return rename, Ctrl+Backspace move to trash, Space Quick Look, Ctrl+Up enclosing folder,
Ctrl+Down open, Ctrl+[ / ] back/forward, Ctrl+A select all, Ctrl+C/V/Opt+Ctrl+V copy/move, Ctrl+1/2/3/4 views,
Ctrl+F search, Ctrl+I get info, Ctrl+Z undo, arrow keys and type-to-select.

### (c) UI/UX differences
- No selection state; actions are per-row buttons. Finder is select-then-act.
- No back/forward history, only breadcrumbs.
- No file-kind icons/thumbnails beyond generic; no column headers (date modified, size, kind).
- Delete asks via a confirm dialog (DriveView.tsx:165); Finder moves to Trash silently with Undo.

### (d) Likely bugs / edge cases
- Deleted items are hidden locally because "Apple's listing may still carry it" (DriveView.tsx:70, 176). If the delete actually failed server-side after the optimistic hide, the item stays hidden until restart.
- Drop of a folder onto the window: Tauri gives a directory path; check `drive_upload` rejects or recurses rather than erroring on read.
- Name collisions on upload/rename: Finder offers Keep Both/Replace; verify Quince's behaviour against Apple's auto-rename.

---

## Cross-app
- No global shortcut layer: Calendar is the only screen with app-level key handling. A shared keymap (Ctrl=Cmd) plus a selection model per list is the prerequisite for most of section (b) everywhere.
- No Undo outside calendar delete.
- No settings screen (Ctrl+,) in any app.
- No print anywhere.

---
---

# Second pass (2026-10-16)

Read page by page: the full Reminders, Notes and Contacts guide TOCs (via `/toc`) and the behaviour-heavy
pages listed under each section below, plus the iCloud for Windows guide (TOC and the Drive, Photos,
Passwords, Calendars & Contacts, Bookmarks and Recover pages).

API names used below:
- **CK-Reminders** = CloudKit `ckdatabasews` private DB, container `com.apple.reminders` (what Quince uses now).
- **CK-Notes** = CloudKit, `com.apple.notes` container (what Quince uses now).
- **CalWS** = iCloud web calendar service `/ca/*` (calendar.rs).
- **ContactsWS** = iCloud web contacts service `/co/*` (contacts.rs). **CardDAV/CalDAV** = `contacts.icloud.com` / `caldav.icloud.com` with an app-specific password, the standards route.
- **DriveWS** = `drivews` + `docws` (drive.rs).
- **PhotosCK** = CloudKit `com.apple.photos.cloud` via `ckdatabasews` (what icloud.com Photos uses).
- **E2EE** = end-to-end encrypted with keys only on trusted Apple devices: impossible from a web session.

## Why Reminders feels slow (traced)

**Not optimistic, and every write is a chain of serial round trips before anything changes on screen.**

UI side (`src/screens/RemindersView.tsx`):
- All writes go through `write()` (:283-301): `await run()` (the Tauri command, network included), then `await readItems()`, and only then does the row change. The comment at :282 says so on purpose: "to Apple first, then re-read".
- During that time the row's controls are disabled (`busyGuid`, :979, :1002, :1058, :1080), so the tick box does nothing visible until the whole chain finishes.
- Add (`add`, :440-455) sets `syncing`, which **disables the New Reminder input** (:836). You cannot type the next reminder until the previous one has made the full round trip. Apple's Return-to-add-next flow is impossible at this speed.
- `busyGuid` is a single slot (:184): tick A, then B, and A finishing clears B's busy state (`finally setBusyGuid(null)`, :297). Not a slowdown, but rows can be re-enabled while their write is still running.

Backend side, per action (all sequential `await`s):

| Action | Round trips before the UI updates |
|---|---|
| Complete (`set_done`, main.rs:646-670) | 1 `records/lookup` (inside `update_record_signed`, cloudkit.rs:728) + 1 `records/modify` (reminders.rs:1668) + `refresh_reminders` (main.rs:74-101): >=1 `changes/zone` page, then one **asset download per List record in the delta**, one after another (`fetch_memberships`, reminders.rs:729-760) |
| Create (`reminder_create`, main.rs:726-738) | create (reminders.rs:1165) + `place_in_list` lookup + modify of the List record (reminders.rs:1210, 1216) + refresh. Because the List record itself changed, the refresh always downloads that list's memberships asset too |
| Delete (`reminder_delete`, main.rs:741-763) | 1 modify (reminders.rs:1351) + refresh |
| Edit with a due date (`reminder_edit`, main.rs:768-809) | edit (lookup + modify) + `set_alarm_cloudkit` (lookup + up to 3 writes, reminders.rs:3692ff) + refresh: about 6-8 serial requests |

Does it resync everything after every write? **Not normally**: `refresh_reminders(.., whole=false)` sends the stored sync token (main.rs:81-89), so it's an incremental `changes/zone`. But:
- if Apple says the token is stale, `all_records` falls back to a **full walk** of the zone (cloudkit.rs:229), up to 200 pages (cloudkit.rs:21, 239), inside the user's click;
- membership assets are re-downloaded for every changed List, serially, with no caching by asset checksum (reminders.rs:741-757).

Fixes, in order of effect:
1. Optimistic UI: flip the row locally (tick/strike, insert the new row with a temp id, remove the deleted row) and roll back on error. Keep the input enabled; queue creates.
2. Return from the command after the write; run `refresh_reminders` in the background (spawn) and let the existing `revision` tick (RemindersView.tsx:255-274) redraw.
3. Skip the extra `lookup` when the cached etag is fresh: try `modify` with the cached `recordChangeTag` and look up only on a conflict (cloudkit.rs:728).
4. Fetch membership assets concurrently and skip unchanged ones by the asset's `fileChecksum`.
5. Write alarm + edit in one `records/modify` batch (CloudKit accepts several operations per request).

## Are colours real?

- **Calendar:** real. `color` from Apple's `Collection` is stored as-is (calendar.rs:109) and drawn with `tint()` (CalendarView.tsx:101-102) on calendars, events, the popover and the year view (:459, :546, :857, :900, :1148, :1213, :1504, :1642). Events have no colour of their own; they use their calendar's, which matches Apple.
  - Gap: `tint()` accepts only `#RRGGBB`. Any other form Apple sends (e.g. `#RRGGBBAA`, which CalDAV-created calendars can carry) falls back to hard-coded `var(--sys-blue)` silently. calendar.rs:109 does not normalise the way reminders.rs does. Unverified against a live 8-digit value.
- **Reminders:** real. `Color` on the CloudKit List is parsed by `hex_colour` (reminders.rs:479, 522-548): plain hex, the JSON `daHexString`, or RGB components. Drawn via `colourOf()` (RemindersView.tsx:28-29).
  - Gap: a Color JSON holding only `ckSymbolicColorName` (no hex, no components) returns `None` -> blue.
  - Smart lists use fixed system colours (RemindersView.tsx:20-25). Apple does the same, so that's correct.
  - New lists created in Quince: `create_list_cloudkit` (reminders.rs:1552ff) writes no `Color`, and there's no colour picker. A list made here shows blue in Quince; what the phone shows is untested.
- Nothing random. The only hard-coded colours are the blue fallbacks above and the smart-list system colours.

## Reminders, deeper (pages: Add or change, Dates/locations, Smart Lists, Sections, Sort, Move)

| Apple behaviour | Quince | Feasible via |
|---|---|---|
| Return on a selected reminder adds another below it | No; input is disabled while saving (:836) | LOCAL |
| Details panel: Notes (rich: bold/italic/lists), URL, Date tick + optional Time tick, Time Zone, Repeat, End Repeat (on date), Early Reminder, List, Tags, Flag, Priority (!/!!/!!!), Location (arriving/leaving, radius, getting in/out of car), When Messaging, Image | Has title, notes (plain), date+time combined, repeat, flag, priority, list, parent | URL, early reminder, end repeat, tags, timezone: CK-Reminders. Images: CK-Reminders asset upload. Location: store CK-Reminders, trigger impossible (device geofence). When Messaging: impossible (Messages on device) |
| All-day reminder (date without time) | Not creatable (single datetime input, :328-333) | CK-Reminders (DueDateIsAllDay field) |
| Urgent alarm on iPhone | No | CK-Reminders field likely; ringing is device-side |
| Natural-language dates ("Pay bills on Saturday") | No | LOCAL parser |
| Custom repeat: hourly/daily/weekly on weekdays/monthly each or "on the"/yearly | Presets plus "custom" label (:125, :342) | CK-Reminders (RecurrenceRule records) |
| Custom Smart Lists: Tags (include/exclude, all/any), Date (today/after/range/relative incl. overdue), Time, Priority, Flag, Location, Lists; match all/any | Only the four built-ins | CK-Reminders (smart-list filter is a List record field) + LOCAL evaluation |
| Sections: add, add with selection, reorder via Edit Sections, rename, delete (deletes contents), Auto-Categorise | Reads sections only | Write: CK-Reminders. Auto-Categorise: impossible (on-device ML) |
| Columns view (sections as columns) | No | LOCAL |
| Sort By: deadline, creation date, priority, title (asc/desc); manual drag; dragging in a sorted list switches to manual | Fixed order | `SortingStyle` field is on the List record (seen in reminders.rs test, :3363): CK-Reminders |
| Move: drag to reorder (subtasks follow), drag onto sidebar list, Cmd-click multi-select, copy/paste between lists; in Today drag between sections changes time, in Scheduled changes date | Move via list picker only | CK-Reminders (ReminderIDs ordering on the List, memberships asset) |
| Show Suggestions (re-add completed reminder while typing) | No | LOCAL over library |
| Settings: Today notification time, show all-day as overdue next day | No | LOCAL |
| Templates, shopping lists with categories | No | Templates: CK-Reminders (Template records) likely. Categories: impossible (on-device ML) |
| Sharing, assign | No | CK-Reminders sharing (CloudKit shares); unverified |

## Notes, deeper (pages: Format, View, Sort and pin, Smart Folders, Delete, Import/export)

| Apple behaviour | Quince | Feasible via |
|---|---|---|
| Format menu: Title/Heading/Subheading/Body/Mono/Block quote; bold/italic/underline/strike; font, size, colour; Highlight with colours; alignment; divider (Cmd-L) | Plain text | CK-Notes: all are attribute-run fields in the note protobuf that ckdocument.rs already round-trips |
| Collapsible headings (collapse/expand sections) | No | CK-Notes (collapsed state is in the doc) + LOCAL UI |
| "New notes start with" default style; default text size; per-note zoom | No | LOCAL |
| Gallery view (double-click opens), separate note window, note counts beside folders | No gallery/window; counts exist (`note_counts`) | LOCAL |
| Sort per folder (More > Sort By) and a global default; click the date to toggle created/edited; date groups | Fixed sort | Per-folder sort is likely a Folder record field: CK-Notes (unverified). Grouping: LOCAL |
| Pinned section with drag between Pinned and Notes; Recently Deleted can't be pinned | Pin exists | Check Quince doesn't offer Pin inside Recently Deleted (NotesView.tsx:555 gates only on `!locked`) |
| Smart Folders: Tags, Date Created, Date Edited, Checklists, mentions; any/all; untagged-only; converting a folder tags its notes (irreversible) | No | CK-Notes (Smart Folder = Folder record with a query) + LOCAL evaluation |
| Recently Deleted: off devices after 30 days, gone from iCloud after 40; recover by dragging to a folder | Delete, restore and Delete Now exist | Show the days-left count: LOCAL from deletion date |
| Import Markdown/TXT/RTF/RTFD/HTML/ENEX; export PDF or Markdown; Copy as Markdown; print | No | LOCAL conversion + CK-Notes create |
| Locked notes | Shown as locked | Decrypting with the user's note password: the crypto is client-side (password-derived key), so technically possible locally. With Advanced Data Protection on: impossible. If notes use the device passcode instead of a separate password: impossible |
| Audio recording + transcript, Math, Quick Note | No | Audio attachment: CK-Notes asset. Transcription and maths: LOCAL only |

## Contacts, deeper (pages: Update info, Lists, Smart Lists, View, Merge)

| Apple behaviour | Quince | Feasible via |
|---|---|---|
| Edit card: Add (+) menu of fields (Profile, etc.), click a label to change it, Custom label, empty fields hidden, Remove button, Done | Edit draft with phone/email/url; addresses read-only | ContactsWS (the card JSON has all fields); CardDAV as a standards fallback |
| Lists: Add > New List, New List From Selection, drag contacts onto a list, Remove From List vs Delete, nested sublists by dragging a list onto a list | None | ContactsWS groups (`/co/groups/card`). Nested lists: iCloud groups are flat, so nesting is a Mac-local feature; likely impossible on iCloud |
| Smart Lists: field + comparison + value, any/all | None | LOCAL only; iCloud doesn't sync Smart Lists |
| Merge selected cards; Look for Duplicates | None | LOCAL detection + ContactsWS update + delete |
| Link cards across accounts | N/A (one account) | Not applicable |
| Distribution list (which email a list uses) | None | Mac-local |

## iCloud for Windows: the closest analogue to Quince

This is a desktop client for a non-Apple OS, so it's the right bar for Quince's scope.

| Windows feature | What Apple ships | Quince today | Feasible via |
|---|---|---|---|
| **Drive in the file manager** | iCloud Drive in File Explorer navigation pane; files on demand | Separate in-app browser; downloads to a chosen path | DriveWS for listing/download/upload. Linux mechanism: FUSE, or a real folder with a sync engine (rclone-style). No Linux equivalent of the Windows Cloud Files API, so "placeholders" need FUSE |
| Status icons | Cloud = online only; blue arrows = waiting; progress bar = downloading; green circle + tick = downloaded; white tick in green circle = pinned; blue share = you shared; orange share = shared with you | None | LOCAL (Nautilus/Dolphin emblems or FUSE xattrs). Share state: DriveWS item fields (`shareID`/sharing flags, unverified) |
| "Always keep on this device" per file/folder/whole drive; "Free up space" | Yes | No | LOCAL sync policy + DriveWS |
| Share files and folders | Yes | No | DriveWS sharing (icloud.com does it; CloudKit share under the hood) |
| Recover: Recycle Bin first, then iCloud.com Recently Deleted (30 days) or Account Settings "Restore Files" | Yes | Delete only | Recently Deleted: DriveWS (`/retrieveTrashItems`-style endpoint icloud.com uses; unverified name). Restore Files (Data Recovery): separate service, unverified |
| **Photos** in Explorer: thumbnails, double-click to download; Apple-side edits/deletes sync down; Windows edits don't sync back (upload as new); Shared Library (v14+); upload; Shared Albums with comments | Yes | **Not in Quince at all** | PhotosCK for the library (what icloud.com Photos uses): list, thumbnails, originals, upload, delete. Shared Albums: separate `sharedstreams` web service. With ADP on: impossible |
| **Passwords** app + browser extension: view/copy username, password, code, note; add with generated password; edit; 2FA codes; password history; delete; add to shared groups (but create groups only on Apple devices) | Yes | No | **Impossible from a web session.** iCloud Keychain is E2EE; Windows gets keys by enrolling as a trusted device. icloud.com exposes no Passwords. Only route: reverse-engineering the Windows client's device enrolment. Out of scope and fragile |
| **Calendars and Contacts in Outlook**: classic Outlook only, needs an @icloud.com address | Yes | Own Calendar/Contacts apps | Linux analogue: expose CalDAV/CardDAV to Thunderbird/Evolution/GNOME Online Accounts. Feasible today via `caldav.icloud.com`/`contacts.icloud.com` with an app-specific password, no Quince code needed. Quince could offer to set it up |
| **Bookmarks**: per-browser extension, initial Merge, ongoing two-way sync with Safari | Yes | No | Safari bookmarks live in CloudKit (`com.apple.safari.bookmarks`?) and are E2EE for most accounts since iOS 17-era changes. **Likely impossible**; unverified |
| Manage storage, sign out/uninstall removes local copies | Yes | Sign-out exists | Storage: `setup.icloud.com` storageUsageInfo (web) |
| Mail | Not in the guide | No | IMAP `imap.mail.me.com` with app-specific password: feasible, standard |

### Windows parity: priorities for Quince
1. Drive as a real folder on disk (FUSE or synced dir) with keep-downloaded/free-up-space and status. This is the core of the Windows client.
2. Photos (PhotosCK). Totally absent, and it's the second pillar of iCloud for Windows.
3. Drive Recently Deleted + restore.
4. CalDAV/CardDAV hand-off so Evolution/Thunderbird/GNOME see iCloud calendars and contacts (the Outlook equivalent).
5. Drive sharing.
Passwords and Bookmarks: say plainly in the UI that they're not possible, rather than leaving them absent.
