# Engine audit: sync, outbox, push, auth, SQLite

Read-only audit, 2026-10-22. Nothing was built or run, so every item comes from reading the code. Line numbers are for the tree as it stands today.

## Ranked findings

| # | Impact | Where | Problem | Minimal fix |
|---|--------|-------|---------|-------------|
| 1 | **High: app-wide freeze** | `crates/quince-auth/src/lib.rs:302-306`, `src-tauri/src/main.rs:955-959`, `:1569` | The single Apple HTTP client has no `connect_timeout`, `timeout`/`read_timeout` or `tcp_keepalive`. `keep_signed_in` and the heartbeat hold the **auth mutex** across `heartbeat().await`. After suspend/resume or a Wi-Fi change, a pooled socket can hang for the kernel's retransmit limit (roughly 15 min or more). | Build the client with `.connect_timeout(10s).read_timeout(30s).tcp_keepalive(30s).pool_idle_timeout(90s)`, and give large downloads and uploads their own per-request `.timeout()`. Also wrap `heartbeat()` in `tokio::time::timeout(20s)`. |
| 2 | **High: false "Apple signed you out"** | `quince-auth/src/lib.rs:514-528`, `:960-963`, `:474`, `:509`, `:624`, `:606`; `main.rs:968-984` | `validate` and `accountLogin` turn every non-2xx reply (503, 502, 500) into `Error::Protocol`. `resume` gives up only when **both** errors are `Network`, so an Apple 5xx outage drops into SRP sign-in and then into `clear_challenge_state(); Ok(false)`. `keep_signed_in` then emits `NEEDS_SIGNIN` and a desktop notification. `sign_in_inner` also clears `session_token` and `sign_in` persists that. | Map 5xx and 429 from `validate` and `accountLogin` to a transient error such as `Error::Unavailable`. In `resume`, return `Err` when either error was transient, before trying SRP. When `resume` calls `sign_in`, restore `before` on any error other than `BadCredentials`. |
| 3 | **High: every screen waits on sync** | `main.rs:117` (`Data(Mutex<Library>)`), `refresh.rs:607-622`, `notes.rs:268-346` (`quince-icloud`) | One `tokio::Mutex<Library>` serves every read command. A store holds it for the whole apply, and a full pass decodes every note (gzip, protobuf and topotext) inside that lock. A daily full Photos, Notes or Reminders pass, or a first sync, leaves Notes, Reminders and Contacts blank or frozen. Typing a note autosave (`note_save`, `main.rs:3083`) also queues behind it. | Serve read-only commands from `Library::open_reader` handles. That reader already exists for search (`main.rs:1315-1341`), and WAL makes it safe. Move note decoding (`note_from_record`) into `fetch_cloudkit`, which holds no lock. |
| 4 | **High: slow edits and drags** | `main.rs:128-135`, `:2606`, `:2734-2811`; `reminders_cmds.rs:209` | `resync_reminders` passes `whole = true`, so each reorder drag, tag add or remove, repeat, list create, rename, colour or delete waits for a **full zone walk** (up to 200 pages). It then runs serial list-membership downloads (`reminders.rs:740-770`, one request after another) and a whole-library apply under the lock, without `block_in_place`. | Use `refresh_reminders(…, false)`. A delta from the stored token already includes every record that moved, tombstones too, and an expired cursor falls back to a full walk by itself (`cloudkit.rs:249-257`). Fetch memberships with `buffer_unordered(8)` as Notes does. |
| 5 | **Medium-high: re-render storm** | `main.rs:5645-5651`, `refresh.rs:205` and every other `landed()` call; `src/lib/sync.tsx:128-132` | Each service landing emits `SYNCED` even when nothing changed, which makes 7 events per 5-minute pass, plus one per focus and one per push. Each event bumps a global `revision`, and Notes, Reminders, Contacts and Drive then re-read whole lists, the open note or reminder and the tables (`NotesView.tsx:219,359,377,469`). Each re-read takes the library lock, behind the next service's store. | In `store()`, compare `conn.total_changes()` before and after the apply. Emit only when rows changed, and include the service name in the payload so a screen re-reads only for its own service. |
| 6 | **Medium: duplicate passes** | `refresh.rs:174-500`; `sync.tsx:139-147`; `push.rs:250-275` | Pass locks only queue requests; they never merge them. Focus in each window (each window has its own throttle), the 5-minute timer, every push burst and each window's mount all run another full pass once the current one finishes. Each pass costs a heartbeat, a full calendar window, a contacts check and a Drive root read. | Use a per-service "in flight" flag. A second request waits on the running pass and returns its report, setting a dirty bit that causes at most one more pass afterwards. |
| 7 | **Medium: calendar refresh is ~12 serial requests** | `crates/quince-icloud/src/sync.rs:84-98` | For each 28-day chunk of the 5-month window it calls `/ca/startup` **and** `/ca/events` one after the other, but `startup.calendars` is only used from the first chunk. This runs on every ctag change, which is polled every 4 s while focused. `calendar_sync` (`main.rs:1609-1625`) does the same for the window the UI asks for, and its apply runs without `block_in_place`. | Call `startup` once and fetch the remaining chunks with `/ca/events` alone, concurrently (`try_join_all`). Better still, re-fetch only the calendars whose ctag moved. |
| 8 | **Medium: unbounded memory** | `src-tauri/src/photos.rs:21-40` | `LINKS` gains an entry (six long signed URLs) for every master on every Photos pass. Entries expire logically after 45 min (`:48`) but are never removed, so after one full pass the map holds the whole library for the life of the process. | In `remember()`, call `map.retain(\|_, (_, at)\| at.elapsed() < LINK_LIFE)` before inserting. |
| 9 | **Medium: memory spike on full Photos walk** | `crates/quince-icloud/src/photos.rs:37`, `:96-120`, `:400-405` | `PASS_PAGES = 100` × 500 records keeps about 50k `serde_json::Value` CPLMaster and CPLAsset records in memory at once, plus held records. This happens again every 24 h (`refresh.rs:65` `FULL_SYNC_EVERY` covers photos). My estimate is several hundred MB of peak RSS for a large library. I did not measure it. | Lower `PASS_PAGES` to about 10, since the walk already resumes across passes. Consider a weekly rather than daily full read for Photos. |
| 10 | **Medium: blocking keyring on every sync** | `quince-auth/src/lib.rs:537-539`, `:350-372`; `session.rs:531-557` | Each heartbeat, so each sync and focus, calls `persist()`. That makes synchronous Secret Service D-Bus writes (several chunks, deletes of old names, then a read-back), all on a tokio worker while holding the auth lock. | Persist only when the serialized session or cookie jar differs from the last write (keep a hash), and run it in `spawn_blocking`. |
| 11 | **Medium: Files app freezes during sync** | `src-tauri/src/mount.rs:237`, `:275`; `mount_photos.rs:128`, `:239`; `photo_cache.rs:464,481` | FUSE `readdir` and `lookup` call `data.0.blocking_lock()`, so Nautilus hangs for the length of any sync store. | Read from a `Library::open_reader` handle held by the mount (read-only, WAL). |
| 12 | **Low-medium: live channel can stall silently** | `crates/quince-icloud/src/courier.rs:536-546`; `push.rs:478-486` | The registry `POST` has no timeout. During the 11-hour renewal inside `run()`, a hang stops `next()` from being polled while `LIVE` is still `true`, so pushes are lost and the poller only safety-checks every 2 min. | Add `.timeout(ANSWER)` to the registry request. Item 1 also covers this. |
| 13 | **Low: heavy work for a small question** | `main.rs:4735-4762` | `warm_contact_photos` runs after every pass. It loads every contact with every contact method and calls `stat()` on each photo path **while holding the library lock**. | Query only `guid, photo_signature WHERE photo_url IS NOT NULL`, drop the lock, then check the files. |

## Details on the top items

### 1. No network timeouts, and the auth lock is held across the network

`apple_only()` (`lib.rs:1073`) sets only a resolver and a proxy, and `Client::with` adds cookies, the user agent and default headers. Nothing bounds a request. The only timeouts in the engine are in push (`push.rs:114`), the subscription fetch (`main.rs:1997`) and one 300 ms probe (`cloudkit.rs:1664`).

This is how it spreads:
- `sync_all`, `sync_some` and the 5-minute timer all `await keep_signed_in`. That function locks `auth.client` (`main.rs:955`) and awaits `heartbeat()`. When `validate` fails, `heartbeat()` falls through to `resume()`, which can mean up to three requests or a full SRP sign-in.
- While that lock is held, the following all queue: `cloudkit_context` (every write and the outbox), `calendar_context`, `drive_context`, `account_info`, `account_services`, `storage_info`, the 4-second calendar and contacts watchers (`push.rs:113,214`), the push listener and the poller.
- On a laptop the trigger is ordinary. After resume from sleep, hyper reuses a pooled connection whose peer is gone. With no keepalive and no read timeout, the request waits for TCP retransmission to give up.

To the user this looks like Skykeep stuck on "Updating" after opening the lid, with edits never leaving and the push channel dead until restart.

Fix in `Client::with`:
```rust
let http = apple_only()
    .cookie_provider(Arc::clone(&jar))
    .user_agent(UA)
    .default_headers(defaults)
    .connect_timeout(Duration::from_secs(10))
    .read_timeout(Duration::from_secs(30))   // per read, so big bodies still stream
    .tcp_keepalive(Duration::from_secs(30))
    .pool_idle_timeout(Duration::from_secs(90))
    .build()?;
```
Also wrap the call at `main.rs:959` in `tokio::time::timeout(Duration::from_secs(20), guard.heartbeat())` and treat an elapsed timeout as `Err(_) => {}`.

### 2. An Apple outage reads as a sign-out

Path: `heartbeat` → `validate` 503 → `Err(Protocol)` → `resume()` → `validate` 503 again → `account_login` 503 → `Err(Protocol)`. The guard at `lib.rs:474` requires both errors to be `Network`, so the code moves on to `sign_in`. One of two things follows:
- `CoolingDown`, if an earlier failure set a cooldown. That falls into `Err(_) => {}`.
- SRP init gets a 503. `sign_in_inner` has already run `clear_challenge_state()` (`:624`), which removes `session_token`. The failure is counted towards the cooldown, and `sign_in` persists the session without its token (`:606`).

Both end at `:509`, which returns `Ok(false)`. `keep_signed_in` then sets `LIVE = false`, emits `NEEDS_SIGNIN` and posts "Apple signed Skykeep out", even though nothing was signed out. The stored `session_token` is now gone, so the cheap `accountLogin` recovery no longer works later.

Fix: a transient class for HTTP 5xx and 429 in `validate` and `account_login`. In `resume`, use `if transient(&validated) || transient(&logged_in) { return Err(...) }`. In the `sign_in` branch of `resume`, restore `before` on every error except `BadCredentials`.

### 3. One mutex for every reader

The search path already proves the fix: `read_for_search` (`main.rs:1321`) uses `Library::open_reader` because WAL readers never block on the writer. Every other read command (`notes_in`, `note_body`, `reminders_in`, `contacts_list`, `calendar_events`, `drive_children`, the `quince://` handlers that read rows) still does `data.0.lock().await`.

`store()` correctly moves the network work outside the lock. But the apply itself, inside `block_in_place`, holds the lock for as long as the rows take to write, and for Notes that includes decoding every body (`note_from_record`, called from `apply_cloudkit_whole`). FTS triggers fire on every insert (`schema.rs:640-660`), which adds to the time.

Minimal fix: a small pool of reader connections (or a thread-local one) for read-only commands, keeping `Data` only for writes. That also covers items 11 and 13.

### 4. Reminder list operations do a full walk

`resync_reminders` passes `whole = true`. The comment says a list change moves other records too, but a delta from the stored token returns every record changed since that token, including those others. A full walk adds nothing except cost: every page, one membership download per list in series, and a whole-zone prune and apply under the lock, which in this path is not even wrapped in `block_in_place`. The drag or tag command resolves only after all of that.

Fix: `refresh_reminders(http, root, params, data, false)`. Keep the daily full pass in `refresh.rs` as the floor.

### 5. `SYNCED` is emitted with nothing changed

`refresh::on_landed` (`main.rs:5645`) emits `refresh::status` for every service as it lands. A clean 5-minute pass therefore emits 6 `SYNCED` events plus the final one. `sync.tsx` bumps `revision` on each event regardless of content, and every open screen re-reads its whole list and its open item. `NotesView.tsx:469` also re-runs `[revision, flush]`. That is the probable source of "it redraws for no reason" and of lock pile-ups.

Fix in `store()`: record `library.conn().total_changes()` before and after the apply, return `changed: bool` in `ServiceReport`, and call `landed()` only when it is true. Include `service` in the payload so a screen bumps only for its own service.

## Checked and fine (not findings)

- Outbox amend versus in-flight send: `outbox_take` bumps `tries` before sending, and `amend` and `withdraw` require `tries = 0` (`quince-core/src/outbox.rs:95-155`).
- Pass atomicity (`Library::atomically`), the account-owner guard in `store()`, outbox replay after each pass, and the busy and 503 retry with a cap (`cloudkit.rs:124-166`).
- Expired cursors start the walk over, and a partial walk never prunes (`ZoneWalk::whole`).
- Push socket: ping every 60 s, dead after 30 s, connect bounded by `ANSWER`, and the protobuf reader checks bounds.
- Indexes cover the hot list queries (notes by folder, reminders by list, photos by taken, outbox by service). WAL with `synchronous=NORMAL` is set.
- Offline start keeps `webservices` from the keyring, so the timer and watchers still run once the network is back.

## Summary

1. Two issues make the app look broken: there are no HTTP timeouts while the auth lock is held across the network (it freezes after sleep), and an Apple 5xx outage is misread as a sign-out.
2. Most visible slowness comes from the one library mutex shared by sync and UI reads, full Reminders walks after every list edit, and `SYNCED` re-render storms when nothing changed.
3. The fixes are small and local: client timeouts, a transient error class, reader connections, a delta resync, change-gated events, and a pruned `LINKS` map.
