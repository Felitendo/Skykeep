# Shell and cross-app UX audit

Scope: `src/App.tsx`, `src/screens/Library.tsx`, `src/components/*`, `src/lib/*`, `src/styles/*.css`,
`src-tauri/src/apps.rs`, and the window and tray code in `src-tauri/src/main.rs`. This was a
read-only audit: nothing was built or run. Every line number below was checked against the
source on 2026-10-22.

## Ranked findings

| # | Impact | Where | Problem | Minimal fix |
|---|--------|-------|---------|-------------|
| 1 | High | `src-tauri/src/main.rs:1544-1546`, `:1501`; callers `Library.tsx:238,241`, `Account.tsx:45` | `account_services` and `account_info` `lock().await` the auth client. That mutex is held across Apple round trips by `auth_resume` (`:894-900`), by `keep_signed_in` on every `sync_all`/`sync_some` (`:955-959`) and by the heartbeat (`:1569-1577`). At launch the window is shown with an empty sidebar ("Loading…") and a blank content pane until Apple answers. | Answer both commands from a snapshot, as `auth_status` already does with `try_lock` and `auth.known` (`:871-874`). Add the service keys and the account facts to `SessionFacts`. |
| 2 | High | `src/lib/sync.tsx:74,104,128-131`; `main.rs:4558,4582,5645-5650`; `refresh.rs:205…442` | `revision` is one global counter. It ticks once for each service that lands (6), again for the `SYNCED` emitted by `sync_all`, and again in `refresh()`'s `finally`. That is 8 or more ticks per pass, sent to every window. Notes, Reminders and Calendar re-read on every tick even while hidden (`NotesView.tsx:147,219,359,377,469`; `RemindersView.tsx:539,544,828,838,853,869`; `CalendarView.tsx:566`). The result is dozens of IPC reads and re-renders per pass, mostly for services that did not change. | Keep a revision per service, derived from `report.services[x].lastSyncedAt` (`useRevision("notes")`). Drop the bump in `finally` unless the call threw. Gate hidden screens the way Photos, Contacts and Drive already do (`seen`/`active`). |
| 3 | High | `src/lib/sync.tsx:113-125`; `refresh.rs:174,216,265,311,366,405` | Every window's `SyncProvider` runs a full `sync_all` when it mounts. Opening one app from its launcher (`--app notes`) therefore walks all six services, Photos included, and first takes the client lock for a heartbeat (feeds #1). A second pass queues behind the per-service `*_PASS` mutexes and runs again instead of joining the first. | Coalesce on the backend: if a pass is running, wait for it and return its result; if the last pass finished less than about 60 s ago, return `refresh::status`. In the UI, skip the launch refresh when `report.asOf` is recent. |
| 4 | Medium | `src/App.tsx:100-110`; no `.lapsed` rule in any stylesheet | The "Apple signed you out" notice is a `.toast` drawn in normal flow before `.library`, outside `.toast-host`. `#root` and `.library` are both `height:100%`, so the notice pushes the whole library down by about 40 px. The sidebar foot (account, Update Now) and the toast area are cut off at the bottom, and the notice sits unstyled at the top left. | Send it through `toast({ tone: "error", duration: null, action: { label: "Sign In", … } })`, or give `.toast.lapsed` a fixed position inside the toast host's area. |
| 5 | Medium (data) | `src/components/ImportSheet.tsx:36,40,42,69-73,161,216,219` | (a) `key={current.file + queue.length}`: a second file arriving during "Adding" remounts the sheet. Add is enabled again and a second click imports every event twice. (b) Escape during an import calls `onDone` (Shell's Escape is not disabled while busy). When `add()` finishes it calls `onDone()` again, and `done` is `q.slice(1)`, so the **next queued file's sheet is dropped unseen**. | Give each arrival a stable id, key on it, and have `done(id)` remove that arrival by identity. Ignore Escape while `busy`. |
| 6 | Medium | `main.rs:4878` (`window_ready`, a sync command on the GTK main thread), `:5420` (CloseRequested), `:5059-5091` | `tray_reachable` opens a new blocking zbus session connection and makes a call (1 s timeout) every time. In `window_ready` it is evaluated **before** `login::stay_hidden`, so every normal launch pays a D-Bus round trip on the main thread before `show_window`. Every window close does the same, and all windows freeze while it waits. | Reorder to `login::stay_hidden(..) && tray_reachable(..)`. Cache host presence (refresh it from ksni's watcher callbacks or a background task) and read an `AtomicBool` here. |
| 7 | Medium | `src/App.tsx:21-26`; `Library.tsx:241-261,286-293,495` | `window_ready` fires as soon as `phase === "library"`. At that point `services()` has not answered (see #1) and the chunk for the selected screen has not even started loading (`Suspense fallback={null}`). The window maps onto a blank content pane. | At module load, read `LAST_KEY` and call `apps[last].fetch()` straight away. Send `window_ready` from `Library` once `ready` is true and that chunk has resolved, keeping the 3 s fallback. |
| 8 | Medium (flicker) | `src/styles/app.css:530,535` | In a narrow window (≤900 px) the sidebar slides in with a 300 ms `transform` transition on a full-height, shadowed layer. This is the exact large-layer motion that `tokens.css:105-108` records as flickering under WebKitGTK on Hyprland. | Make it instant (`transition: none`, toggle `visibility`), or fade it with the page-driven `fadeIn` from `lib/fade.ts`. |
| 9 | Medium | `Library.tsx:366-377` with `:297-299` | Arrow keys in the sidebar call `choose()` on every press, and every screen ever chosen stays mounted. Holding ↓ mounts all six apps (the Photos grid, the Notes editor) and fires all their first reads at once, so input stalls. | Move only the focus on arrow keys. Select after about 150 ms with no further arrow press (clear a timer on each key), or on Enter/Space. |
| 10 | Medium (a11y) | `Library.tsx:389` | `tabIndex={selected === key ? 0 : -1}`. With Settings in front, no app row has tabIndex 0, so Tab cannot reach the app list. | `tabIndex={selected === key \|\| (!services.includes(selected ?? "") && index === 0) ? 0 : -1}` |
| 11 | Low-Med | `GlobalSearch.tsx:204-208`; `Library.tsx:86-92,107-115` | Ctrl+K, Ctrl+, , Ctrl+/ and Ctrl+R/F5 fire while a modal sheet (Confirm, Import, Shortcuts) is open. Ctrl+K takes the focus out of the modal into the field behind the veil. Ctrl+, swaps the screen under an open sheet. | Return early when `document.querySelector('[aria-modal="true"]')` is present (or use a narrowed `overlayOpen()`). |
| 12 | Low (flicker) | `app.css:606,612`; `SignIn.tsx:87`; `CodeEntry.tsx:78` | The sign-in and code sheets still use a compositor CSS keyframe (`sheet-in`) with `backwards` fill. That is the kind of animation `lib/fade.ts` was written to replace. On first launch the window maps with the sheet at opacity 0. | Remove `.enter-sheet` or call `fadeIn(ref.current)` in a layout effect. |
| 13 | Low (perf) | `Face.tsx:275` | `decoding="sync"` on every avatar decodes on the main thread during paint, which causes jank while Contacts and search rows scroll in. | Use `decoding="async"` and set `ready` after `img.decode()` resolves. The photo is already hidden until it is ready (`.face-photo.is-loading`), so no half-drawn frame shows. |
| 14 | Low (perf) | `lib/fade.ts:363-374` | `fadeSheetsIn` watches the entire `body` subtree. Every element inserted anywhere (photo tiles, editor spans, search rows) runs `matches` and `querySelectorAll`. | Observe `document.body` with `{ childList: true }` only (all sheets are portalled to body). Fade the in-veil sheet swap in `ImportSheet` with `fadeIn` from Shell's layout effect. |
| 15 | Low (copy) | `ImportSheet.tsx:220,284`; `SettingsView.tsx:636,706`; `Account.tsx:59,66` (text of the `.sidebar-sync` button) | Button labels end in "…" ("Adding 2 of 5…", "Adding…", "Syncing…", "Clearing…", "Updating…"). This breaks the copy rule. | "Adding 2 of 5", "Adding", "Syncing", "Clearing", "Updating". Put a spinner beside the label if motion is needed. |
| 16 | Low | `QuickCapture.tsx:96`; `GlobalSearch.tsx:196-198` | Enter in quick capture before the lists load (`target === ""`) does nothing and says nothing. A search that fails is swallowed, so typing shows no results and gives no reason. | Show "Loading lists" in the status line and keep the text so Enter can be pressed again. On a search error, show "Search isn't available right now" in the panel. |
| 17 | Low (cleanup) | `Library.tsx:197-226`; `components/SyncStatus.tsx`; `app.css:1121`, `calendar.css:577` | The rail layout effect is dead (`void from`), yet `toggleRail` still forces a synchronous layout (`getBoundingClientRect` and `getComputedStyle`) on every click. `SyncStatus` is never imported. The `picker-in` and `cx-pop` keyframes are unused. | Delete them. |

## Details: top items

### 1. Startup and new app windows wait on Apple behind the auth mutex
`App.tsx:45-46` deliberately starts `api.resume()` and opens the library at once, so that "a
round trip to Apple … is never what stands between a launch and the window". The library's first
two calls do wait for it, though:

```rust
// main.rs:1544
async fn account_services(auth: State<'_, Auth>) -> UiResult<Vec<String>> {
    let client = auth.client.lock().await;          // held by auth_resume across resume_session().await
```
`account_info` (`:1501`) does the same. `services()` is what sets `ready`, `selected` and the
first `kept` screen (`Library.tsx:241-259`). Until it returns, the sidebar shows only "Loading…"
and the content pane is empty, in a window `window_ready` has already shown. A new app window has
the same problem with `keep_signed_in`, which holds the lock for a heartbeat at the start of every
`sync_all`. The comment on `LIVE` (`main.rs:1210-1212`) records the same class of stall ("kept
every app window waiting about five seconds"), but that fix covered only `auth_resume`.

`webservices` is part of the stored `Session` (`quince-auth/src/session.rs:83`), so the answer is
available before the resume finishes. Snapshot it into `SessionFacts` and update the snapshot in
`signed_in`. Then answer with `try_lock`, falling back to the snapshot, as `auth_status` does.

### 2. One global `revision` makes every screen re-read 8 or more times per pass
- `main.rs:5645-5650`: every `landed(service)` emits a full `SYNCED`, 6 per pass.
- `main.rs:4558`: `sync_all` emits `SYNCED` again, then returns.
- `sync.tsx:74`: `finally { setRevision(n => n + 1) }` adds one more.

Every window gets every event. In each window, Notes re-runs `read()`, `noteBody`, `noteRuns` and
`refreshTables` on each tick, even while Reminders is in front, and Reminders re-runs six
effects. A pass where nothing changed still sends 8 or more waves of IPC through all mounted
screens in every open window, which also re-renders them during scrolling. Photos, Contacts and
Drive already defer while hidden (`PhotosView.tsx:833-837`); the other three do not.

Fix: in `SyncProvider`, expose `useRevision(service)` as a counter that moves only when that
service's `lastSyncedAt` or outcome changes, and let screens depend on it. Remove the
unconditional bump in `finally`, keeping it only for the `catch` path, where no event arrives.

### 3. Every window runs its own full sync when it opens
`SyncProvider`'s mount effect always calls `refresh()` → `sync_all`. The main window and each app
window (`apps.rs:150-160`) each have their own provider. Opening Notes from the launcher, or
re-making a window after it was put away for 10 minutes, starts a six-service pass and contends
for the auth lock (#1). The backend serialises passes per service (`*_PASS` mutexes) but never
merges them, so the second caller does all the network work again. Merging the passes on the
backend fixes this for every caller, including the 30 s focus-return refresh in each window.

### 4. The "signed out" notice breaks the layout
```tsx
// App.tsx:100
{lapsed && (<div className="toast lapsed" role="alert"> … </div>)}
<Library onSignOut={signOut} />
```
`.toast` only styles a card (`kit.css:380`); positioning comes from `.toast-host`. No stylesheet
has a `.lapsed` rule. The card therefore takes up space at the top of `#root`, and
`.library { height: 100% }` overflows below it. This happens every time Apple ends the session,
which is exactly when the person most needs the window to look intact.

### 5. ImportSheet can import twice or drop a queued file
- `key={current.file + queue.length}`: a second `.ics` opened while the first is "Adding 3 of 40"
  remounts `EventsSheet`, so `busy` is reset and Add is live again. The first loop is still
  running.
- `Shell` passes Escape to `onCancel = onDone` with no `busy` guard. When the running `add()`
  finishes it calls `onDone()` (`:161`), which does `setQueue(q => q.slice(1))` and removes the
  **next** file's sheet before anyone sees it.

Fix: give each `Arrival` an id when it is queued, use `done = (id) => setQueue(q => q.filter(a => a.id !== id))`,
key on the id, and pass `onCancel={busy ? () => {} : onDone}`.

### 6. Blocking D-Bus on the main thread when showing and closing windows
`window_ready` is a synchronous `#[tauri::command]`, so it runs on the main thread. It evaluates
`tray_reachable(&app) && login::stay_hidden(..)`, and `status_notifier_host()` builds a new
blocking session-bus connection and makes a property call each time. `stay_hidden` is false
except when the app was started at login, so the D-Bus work is wasted on almost every launch, and
it happens just before the first show. The `CloseRequested` handler does the same work on every
close. Swapping the operands removes the cost at launch. Caching the result removes the cost at
close.

### 7. The first visible frame is an empty shell
Even once #1 is fixed, the screen's code is not requested until `services()` resolves and
`screenFor()` renders. The other chunks are fetched 300 ms after `ready`. Starting the last
screen's `import()` at module load runs the download and parse in parallel with `auth_status`.
Moving `window_ready` to the point where `ready` is true and that chunk has resolved means the
window maps showing real content.

### 8. Narrow-window sidebar motion
`.sidebar` at ≤900 px: `transition: transform var(--spring-quick-d) var(--spring-quick)`. The
`.sidebar` element is 196 px wide, nearly the window's height, carries a shadow, and is a
`contain: layout paint` layer. `tokens.css:105` names "the sidebar" among the layers that
flickered. Make it instant; the scrim already is.

### 9 and 10. Keyboard in the sidebar
Selection follows focus (`move()` calls `choose()`), and `keptRef` never forgets a screen, so
arrowing through the list builds every app. In Settings the roving tabindex has no stop at all.
Both are one-line fixes (see the table).

## Checked and fine
These were verified and are not listed above:
- The toast timers (pause on hover/focus, count only while visible).
- RowMenu menu aim and its Escape handling.
- Picker placement before paint.
- The `useModalFocus` trap and focus return.
- The page-driven `fadeIn`/`fadeOut`.
- Screens kept mounted with `hidden` plus `inert`, and the Portal gating.
- Out-of-order search answers being dropped.
- The window colour following the theme (`tint.ts`).
- The 3 s fallback show for hidden windows.
- Putting app windows away and freeing them after 10 minutes.
- Progress bars driven by `transform`.
- Small spinners on small layers only.

## Summary
The two biggest problems are both in the sync path. Launch and new app windows wait on Apple behind the auth mutex (#1, #3), and one global `revision` makes every mounted screen in every window re-read 8 or more times per pass (#2).
The visible glitches are the unpositioned "signed out" notice (#4), the blank first frame (#7), and the narrow-window sidebar slide on a large layer (#8).
The one data-safety bug is in ImportSheet: a remount or an Escape mid-import can import events twice or silently drop the next queued file (#5). The rest are small fixes for keyboard, accessibility and copy.
