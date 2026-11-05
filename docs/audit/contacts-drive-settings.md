# Audit: Contacts, Drive, Settings, Drive folder (FUSE)

Read-only audit, 2026-10-22. Nothing was built or run, so every item comes from reading the code at the line given.
Ranked by how much the user feels it.

## Ranked findings

| # | Impact | Where | Problem | Minimal fix |
|---|--------|-------|---------|-------------|
| 1 | **High: items vanish** | `src/screens/DriveView.tsx:573, 828, 764, 1107` | The `gone` set is only ever added to. Moved items keep the same `drivewsid`, so after a drag into folder X, **opening X hides them**: the rows and the count both filter by `gone`. The same happens to a deleted item recovered from Recently Deleted. This lasts until restart, because the screen stays mounted. | Remove ids from `gone` once a refresh shows they have left the folder they were deleted or moved from. Or key `gone` by `folderId:drivewsid` and clear it on `here.id` change. |
| 2 | **High: Cancel does not cancel** | `src-tauri/src/main.rs:4104-4110, 4184, 3790-3806` | Abort handles are registered inside `.map()`, which `buffer_unordered(3)` runs lazily. Only the 3 files in flight are in `uploads_running`. The tray's **Cancel** (and the per-row ✕ on a *waiting* file) aborts those 3, and every other planned file goes on uploading. `cancelled(batch)` is only checked during planning. Deleting a folder that is being uploaded into has the same gap (DriveView:771-776). | At the top of `run`, return the cancelled event if `cancelled(&batch)` or the id is in a new `cancelled_ids` set. Or create and register every `AbortHandle` before `stream::iter`. |
| 3 | **High: jank every 10 s** | `DriveView.tsx:605, 661`; `main.rs:4304-4320, 4351-4358` | Every quiet refresh (10 s poll, each sync landing, each upload landing) calls `driveThumbnails`. That reads **every** cached thumbnail with blocking `std::fs::read` on the async runtime, base64-encodes them all and ships them as `data:` URLs over IPC. A folder of 500 photos is roughly 10-25 MB of JSON parsed on the WebKit main thread, and then `setThumbs({...was,...t})` re-renders all rows. | Serve thumbnails through a URI scheme the way contact photos are served (`contactPhotoSrc`). Return only `id -> version` (or nothing) and call it only when `sameItems` was false. |
| 4 | **High: Contacts reloads on every Notes/Reminders edit** | `ContactsView.tsx:311, 382, 450`; `src/lib/outbox.ts:59-62`; `outbox.rs:1096, 1105` | `quince://outbox-changed` carries no service, and it fires twice per drain cycle while *anything* waits. Contacts stays mounted while hidden and re-reads the whole book (`contactsList`), the duplicates (`contactsDuplicates`, a second whole-book load) and the addresses on every `outbox.tick`. Typing in a note therefore keeps the library lock busy with address-book loads. | Emit the service in the payload and tick only for `"contacts"`, or fold `outbox.tick` into the `due` gate so a hidden screen reads once on return. |
| 5 | **High: mount and Drive freeze on a stalled network** | `crates/quince-auth/src/lib.rs:302-306`; `main.rs:162, 3618`; `mount.rs:61` | The Apple client has **no `connect_timeout` or read timeout**. `DRIVE_PASS` is **one global lock** held across `fetch_folder`. If one fetch hangs (Wi-Fi switch, resume from suspend), every folder open, the 10 s refresh, resyncs after writes and FUSE `list()` calls queue behind it. The window spins on "Opening ..." indefinitely, and FUSE gives up only after its own 30 s `LIST_WAIT`. | Add `.connect_timeout(10s)` and `.read_timeout(30s)` to `apple_only()`. Make the pass lock per folder (`HashMap<String, Arc<Mutex<()>>>`), since the race it guards is per folder. |
| 6 | **High: slow file manager on the mount** | `crates/quince-vfs/src/fuse.rs:1117-1131, 641-648, 568-637`; `mount.rs:234-239` | Every FUSE `lookup` calls `children()`, which takes the **global library lock** (`blocking_lock`), queries the whole folder from SQLite, re-derives names and places every inode, all to find one name. With `TTL = 1s` (fuse.rs:32), a file manager stat-ing a 2,000-item folder costs O(N²) and competes with every UI IPC call for the lock. | Cache `(folder ino -> name->ino map, built_at)` in `Inner` and invalidate it on `store_folder`, writes and rename. Raise the entry/attr TTL (for example 10 s) for Apple items. |
| 7 | **Med-High: bulk delete/move is very slow and blocks Drive** | `DriveView.tsx:768-781, 831-838`; `drive.rs:545-556, 1118-1131`; `main.rs:4238-4258, 4504-4528` | Each item is one IPC call doing about 4 sequential round trips: list the parent, the write, the in-function re-read, then `resync_drive_folder` under `DRIVE_PASS`. Moving uses 5. Deleting 50 files means about 200 requests, each resync holding the global pass lock, so navigation stalls. | Add `drive_delete_many` and `drive_move_many`: one parent listing, one `deleteItems`/`moveItems` body with all items (Apple takes arrays; `delete_forever_many` already batches), one resync per affected folder. |
| 8 | **Medium: bulk contact delete is O(N x book)** | `ContactsView.tsx:691-703`; `main.rs:3433-3446` | `removeMany` awaits `contactDelete` one at a time. Each call runs `library.contacts(None)`, which loads every card with all methods and details, to find one guid. 100 deletes in a 3k book means 100 full-book loads under the library lock, then a full re-read. | Look up one row (`SELECT ... WHERE guid=?`, like `contact_photo_source`), and add a `contacts_delete(guids)` command that records them all in one transaction. |
| 9 | **Medium: unsaved card silently discarded** | `ContactsView.tsx:813-819, 954, 257-262, 586` | While a card is being edited, **⌘N**, the **New Contact** button (`disabled={busy}` only), **Edit Contact** on another row, or a global-search reveal (`setDraft(null)`) replace the draft with no prompt. | If the draft differs from `draftBase`, `confirm()` "Discard changes?" first. Or disable New Contact while `draft` is set. |
| 10 | **Medium: full upload re-renders the whole Drive list** | `DriveView.tsx:938-952, 1604, 1110, 1520` | Upload state lives in `DriveView`, and every progress event (about 100 per file, 3 at once) re-renders the screen and maps every row. Selection uses `picked.includes` per row and in `pickedItems`, so after Ctrl-A in a 3k folder each render costs about 9M comparisons. | Move the `UPLOAD` listener and state into `UploadTray` (or a `useSyncExternalStore` store). Build `const pickedSet = useMemo(() => new Set(picked))` and use `.has`. |
| 11 | **Medium: per-keystroke whole-book IPC** | `ContactsView.tsx:287-311`; `quince-core/src/contacts.rs:750-830` | Each key in search reloads every contact, every method and all details, sorts, serializes the full cards and re-sorts in JS (`sortedContacts`). The sequence number drops stale replies, but the backend still does every one under the library lock. | Debounce about 120 ms, or have the search return only matching guids and filter the list already held in memory. Drop the second sort, since the backend already orders. |
| 12 | **Medium: downloads have no progress and no cancel; spinner lies** | `DriveView.tsx:731-743`; `main.rs:3688-3738`; `drive.rs:233-262` | Opening a file streams it to Downloads with only a row spinner: no percent, no cancel. `busyGuid` is a single value, so a second download takes the spinner from the first, and the first one's `finally` clears the second's. | Emit `sent/total` from the `bytes_stream` loop (the upload already has `Progress`), make `busyGuid` a `Set`, and add an abort handle as uploads have. |
| 13 | **Low-Med: offline folder loses all thumbnails** | `main.rs:4375-4377` | If one new file needs a thumbnail and that fetch fails, `?` returns `Err` and discards `out`, which already holds every cached thumbnail. Offline, the grid shows no pictures at all. | `match` the fetch: on `Err`, return `out` as it is. |
| 14 | **Low: counts change while searching** | `ContactsView.tsx:895, 687-690` | "All Contacts" and each list count use `contacts`, which is already filtered by the search, so the sidebar numbers shrink as you type. | Count from a separate unfiltered size (one `COUNT(*)` IPC, or keep the last unfiltered list). |
| 15 | **Low: copy rule** | `DriveView.tsx:1769` "Downloading…"; `SettingsView.tsx:636` "Syncing…"; `SettingsView.tsx:706` "Clearing…" | Button labels end in "…". (`PhotoCacheGroup` already does it right: "Clearing".) | Use "Downloading", "Syncing", "Clearing". |

## Details for the top items

### 1. Moved or recovered Drive items disappear (DriveView `gone`)
`setGone((g) => new Set([...g, ...ids]))` runs in `removeAll` (764) and `moveAll` (828). Ids are only removed when Apple *refuses* the change (786-790, 840-844). The set filters `rows` (1107) and the count (1351) in **every** folder, and `DriveView` is never unmounted (Library.tsx keeps screens alive).
Steps: drag `a.pdf` onto folder `Work` → open `Work` → it is not there, and "N items" is one short. Restarting the app "fixes" it.
Fix: after the `refresh()` at 793/846, prune ids the fresh listing no longer contains:
`setGone(g => new Set([...g].filter(id => itemsRef.current.some(i => i.drivewsid === id))))`.
Or scope entries as `${here.id}:${id}` and reset on `here.id` change.

### 2. Upload Cancel leaves most of the batch running
```rust
.map(|job| { … uploads_running().lock().map(|mut m| m.insert(job.event.id.clone(), abort)) … })
.buffer_unordered(3)
```
`map` on a stream runs as items are pulled, and `buffer_unordered(3)` pulls 3 at a time, so `drive_upload_cancel` only ever sees the files in flight. Waiting files have no handle. The tray's per-file ✕ (UploadTray.tsx:211) is offered for `state === "waiting"` and does nothing. Batch Cancel stops 3 files and the other 77 upload.
Fix (smallest): inside `run`, before `app.emit(… "sending")`, add
`if cancelled(&batch_id) || cancelled_ids().contains(&id) { return cancelled }`.
Have `drive_upload_cancel` also insert the explicit `ids` into `cancelled_ids`.

### 3. Thumbnail payload on every refresh
`refresh()` (DriveView:649-668) runs every 10 s while focused, on every sync revision, and 2 s after each landed upload. Each run calls `drive_thumbnails`, which:
- reads all cached files synchronously (`std::fs::read` in an `async fn`, blocking a Tokio worker);
- base64-encodes all of them;
- returns a `HashMap<String, String>` of data URLs.

The webview parses that on the main thread, then `setThumbs((was) => ({ ...was, ...t }))` always makes a new object and redraws the list. Because of the poll, it repeats every 10 s, and that is the periodic hitch.
Fix: register a `drive-thumb://<id>?v=<etag>` handler that serves the cached file (the same pattern as `contactPhotoSrc`). `drive_thumbnails` then only fills the cache and returns the ids that have pictures. In `refresh`, skip it when `sameItems(was, next)`.

### 4. A hidden Contacts screen re-reads on every outbox event
`useOutbox` bumps `tick` on each `quince://outbox-changed`. The backend emits that with `()` as payload, twice per drain loop (outbox.rs:1096/1105), whenever *any* service has something waiting: notes, reminders or calendar. ContactsView's three effects depend on `outbox.tick` directly, not through the `due` gate that already defers sync revisions while the screen is hidden. Every note autosave therefore costs two whole-book loads plus a whole-book duplicate scan, on the same library mutex the Notes save needs.
Fix: `app.emit(CHANGED, service)` and in `useOutbox` tick only when `payload === service`. In ContactsView, also gate the tick like `due` (`if (active) seenTick.current = outbox.tick`).

### 5. No HTTP timeouts, plus one global Drive lock
`apple_only()` sets no timeouts, and reqwest's default is none. `DRIVE_PASS` (main.rs:162) is held from `fetch_folder` to store, in `drive_sync_folder` (3618), `resync_drive_folder` (3671), the background pass (refresh.rs:366) and FUSE `list` (mount.rs:61). One half-open TCP connection after resume therefore blocks all of Drive, in the window and the mount, until the kernel gives up (minutes).
Fix: `connect_timeout(Duration::from_secs(10))` and `read_timeout(Duration::from_secs(30))` on the shared builder. The read timeout is per read, so long streamed uploads and downloads still work. Make the pass lock per folder id, which keeps the stated guarantee (the re-read after a write lands last *for that folder*).

### 6. FUSE `lookup` rebuilds the folder each time
`lookup → child → children → store.children → data.0.blocking_lock() + drive_children(folder)`, then `named(&items)` and `inodes.place` for **every** entry, and only then `.find(name)`. `getattr` is cheap, but lookups repeat every second (`TTL = 1s`). Nautilus or Dolphin listing a big folder does N lookups after `readdir`, giving O(N²) SQLite rows, each one waiting on the same mutex the UI's IPC uses. With `n_threads = 4`, four such lookups fill the pool.
Fix: keep a per-folder `HashMap<String, (u64, DriveItem)>` built by `children()` with a short validity, invalidated by `store_folder` (mount.rs list), `change()`, `rename` and create/unlink. Lookups then cost O(1) without the library lock. Raise `TTL` for Apple items.

### 9. Unsaved contact edits lost without a prompt
`startNew()` (223-228) and `edit(c)` (229-243) overwrite `draft` unconditionally. They are reachable during editing from ⌘N (813), the always-enabled header button (954), a row's "Edit Contact" (586) and `useReveal` (257). Escape is a deliberate Cancel, as in Apple's Contacts, and can stay.
Fix: a `leaveDraft()` helper that returns `true` immediately when `JSON.stringify(draft) === JSON.stringify(draftBase.current)` and no address edits are pending, and otherwise `await confirm({ title: "Discard changes?", action: "Discard", danger: true })`.

## Checked and fine (not listed)
- Contact rows: `memo` + stable `rowActions` + `content-visibility: auto`. Drive list rows use the same `content-visibility: auto` pattern.
- `Face` decodes before drawing and has no fade.
- Downloads go to `.part`, then hard-link, so a broken download never leaves a truncated file. Uploads stream from disk.
- Drive folder open is cached first with a token guard against stale navigation. List writes are queued with read-back on refusal.
- FUSE writes are journaled; replace keeps the old version in Recently Deleted; conflicted copies are made.
- Settings: folder status runs in `spawn_blocking`, the 2 s poll stops when Settings unmounts, and the export state survives navigation.

## Summary
Biggest user-visible bugs: moved or recovered Drive items stay hidden (#1), and upload Cancel only stops 3 files (#2). Both are small, local fixes.
Biggest smoothness costs: thumbnail data-URL payloads every 10 s (#3), whole-address-book reloads triggered by other apps' outbox events (#4), and O(N²) FUSE lookups on the global library lock (#6).
Stability: add HTTP timeouts and make `DRIVE_PASS` per folder (#5), so one stalled request can't freeze Drive and the mount.
