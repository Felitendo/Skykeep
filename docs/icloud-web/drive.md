# iCloud web Drive (iclouddrive) — reverse-engineered facts

Source: `iclouddrive/` bundles, build 2638Build17, beautified copies in the session
scratchpad (`icloud-bundles/pretty/iclouddrive/`). Line numbers refer to the beautified
files. Read-only grep; nothing executed, no sign-in. Tags: **VERIFIED-IN-CODE** (seen
literally) / **INFERRED** (deduced, not seen directly).

## Ids and constants

- Drive ids are `TYPE::zone::docwsid`, TYPE ∈ `FILE, FOLDER, SHARED_FILE, SHARED_FOLDER, FILE_IN_SHARED_FOLDER, FOLDER_IN_SHARED_FOLDER`; the special id `TRASH_ROOT` is also valid — main.js L835-840 `const O = ["FILE","FOLDER","SHARED_FILE",...]` / `if ("TRASH_ROOT" === e) return !0; const t = e.split("::"); return 3 === t.length`. VERIFIED-IN-CODE
- Root = `FOLDER::com.apple.CloudDocs::root`, trash = `TRASH_ROOT`, search = `SEARCH_RESULTS` — main.js L622-624. VERIFIED-IN-CODE
- Two client modes: `DRIVE_SERVICE_UNIFIED` (drivews) and `DRIVE_SERVICE_ENUMERATE` (docws `/v1/enumerate`); in ENUMERATE mode root id is `"root"` and trash is `"trash"` — main.js L12757-12760, L12809, L13253 `const e = L === T.ENUMERATE ? s : d`. Which mode is live for an account is decided at runtime (gap). VERIFIED-IN-CODE

## drivews (`urlForServiceName("drivews")`)

Transport: every call is `POST https://<drivews host>/<name>`, body `JSON.stringify(payload)`, header `Content-Type: text/plain` — main.js L1102-1124 `post(e,t,...) { return this.sendRequest(e,t,n,i,o,"POST") }`, `headers: {"Content-Type": "text/plain"}`. Full list of known paths — main.js L1080 `v = new Set(["/retrieveSharedItemDetailsInFolders", "/retrieveItemDetailsInFolders", "/retrieveItemDetails", "/retrieveTrashDetails", "/createFolders", "/deleteItemsInTrash", "/deleteItems", "/moveItemsToTrash", "/moveItems", "/renameItems", "/shareFolder", "/moveItemsFromTrash"])`. VERIFIED-IN-CODE. Plus `putBackItemsFromTrash` (L8236). VERIFIED-IN-CODE
HTTP errors: `402→AccountNotMigrated, 421→Unauthenticated, 423→PCSMissing, 424→PCSInvalid`, ≥500 Server — main.js L1153-1158. Read paths use an 180 s timeout (`AbortSignal.timeout(18e4)`, L1127). VERIFIED-IN-CODE

### Generic batched item op (module 2231, main.js L20327-20395)
Used for `moveItems`, `moveItemsToTrash`, `moveItemsFromTrash`, `deleteItems`, `putBackItemsFromTrash`, `renameItems`:
```
{ items: [ { drivewsid, shareID?  /* only when item.share === "TO_ME" */,
             etag, clientId /* = item id */ } ], ...extra }
```
- `renameItems` item = `{drivewsid, shareID?, etag, name, extension /* files only */}`; `putBackItemsFromTrash` item = `{drivewsid, shareID?, etag}` — L20349-20357 `case "renameItems": return {...n, etag:o, name:a, extension: ...}`. VERIFIED-IN-CODE
- Response `{ items: [{ drivewsid, status, etag, parentId, name, type, ... }] }`; `status === "OK"` is success. On `ETAG_CONFLICT` with unchanged `parentId`, the client retries once with the returned `etag` — L20364-20372. VERIFIED-IN-CODE. Other status seen: `UNSHARE_ERROR` (L8166), `ID_INVALID` (L14180). VERIFIED-IN-CODE
- `moveItems` extra = `{destinationDrivewsId, shareID? /* dest TO_ME share */}`; when source is trash and dest isn't, it uses `moveItemsFromTrash` with `{destinationDrivewsId}` — main.js L19586-19590. VERIFIED-IN-CODE

### retrieveItemDetailsInFolders
- Body is an **array**: `[{drivewsid, shareID?, partialData:false, includeHierarchy:true}]` (or `{item_id}` variant) — main.js L14130-14152. VERIFIED-IN-CODE
- Response array of folder objects (zod `Tn`, L14095-14121): `docwsid, drivewsid, item_id, parentId, zone, type, name, etag, assetQuota, fileCount, shareCount, shareAliasCount, directChildrenCount, shareID, share, owner, favorite_order, dateModified, dateChanged, status, syncState, numberOfItems, hierarchy:[{drivewsid,etag,name,extension}], items:[{drivewsid,item_id,name,type,...passthrough}]`. Error entries `{status, shareID?}`; `"ID_INVALID"` + shareID ⇒ unshared. VERIFIED-IN-CODE
- Trash listing in UNIFIED mode = this call with `drivewsid: "TRASH_ROOT"` — `fetchTrash` → `fetchFolderContentCore({folderId: d /*TRASH_ROOT*/})` → `zn({drivewsid: e.folderId})`, main.js L14509-14517, L14382-14386. VERIFIED-IN-CODE. `/retrieveTrashDetails` and `/deleteItemsInTrash` exist in the path set but no caller found (gap).

### retrieveItemDetails
`{ items: [{drivewsid, includeHierarchy, shareID?}] }` → `{ items: [...] }` — main.js L1455-1466. VERIFIED-IN-CODE

### createFolders (createFolder~95b965f1.js L203-219)
```
{ destinationDrivewsId, shareID?, folders: [{ name, clientId }] }
→ { folders: [{ status:"OK"|"ETAG_CONFLICT", etag, parentId, drivewsid, ... }] }
```
Retries once on `ETAG_CONFLICT`. VERIFIED-IN-CODE

### Delete / trash
- "Delete" on a normal item = `moveItemsToTrash` (main.js L8155 `gn.a("moveItemsToTrash", e)`); App Libraries and shared-to-me items instead go to `deleteItems` immediately (L8130-8178: `e || t ? o.push(s) : r.push(s)` … `yn(o, i, "deleteSharedToMe"|...)`). VERIFIED-IN-CODE
- "Delete Forever…" (purge) and "Delete All" in trash = `deleteItems` with source `deleteFromTrash` after a confirm alert — main.js L12432 `"delete" === n ? yn(e, t, "deleteFromTrash")`, L8076. VERIFIED-IN-CODE
- Recover = `putBackItemsFromTrash`; optimistic destination computed from item `restorePath` (split on `/`, walk from root) — main.js L8236, L8270-8279 `if (!e.restorePath) return j.x; const t = e.restorePath.split("/")`. VERIFIED-IN-CODE

### shareFolder
`{ drivewsid, ckShareOperation: { operationType:"create", record:{...fields, "cloudkit.type":{value:"com.apple.folder"}}, zoneID } }` → `{status:"OK", ckShareResult}` — buildShareModalConfigs~88ce8791.js L188-203, main.js L11308 `P.post("shareFolder", e)`. Only used for unshared folders (`createShareFnOverride: isFolder && !m.share`). Files and existing shares go through CloudKit JS on `ckdatabasews` (main.js L6547 `o("ckdatabasews")`), `databaseScope` Shared if `share==="TO_ME"` else Private. VERIFIED-IN-CODE

## docws (`urlForServiceName("docws")`)

| Purpose | Path / body | Cite | Tag |
|---|---|---|---|
| Download URL | `POST ws/<zone>/download/batch[/<ownerRecordName>]?token=<validateToken>`, body `[{document_id}]` (batched ≤30, or single `{document_id}`) → `[{data_token:{url}, package_token?, thumbnail_token?, manifest_token?, document_id}]` | main.js L19772 `` `ws/${c}/download/batch${l}` ``, L20833, L20787-20794 | VERIFIED |
| Download (iWork lib) | `GET /ws/<zone>/download/by_id[_etag][/<owner>]?document_id&dsid&publish=false&redirect=true&token[&double_etag]`; shared-by-link `/ws/none/download/by_token?sharing_token=` | vendors~main~64216fa6.js L24519-24545 | VERIFIED (shared lib; Drive UI uses batch) |
| Package download | `package_token.url` → filename gets `.zip` | main.js L6545-6556 | VERIFIED |
| Duplicate | `POST v1/item/copy/<item_id>?dsid=<dsid>` body `{info_to_update:{}}` → `{item_id}`; then `GET ws/<zone>/list/lookup_by_id?document_id=<item_id>&unified_format=true` → `{docwsid, drivewsid, item_id, etag, parentId, type:"FILE", size, dateChanged, dateModified, zone, name, extension, urls:{url_download,url_thumbnail}}` | main.js L19857-19880, L19689-19706, L12576 | VERIFIED |
| Lookup | `POST ws/<zone>/list/lookup_by_id[/<owner>]` body `[{etag, document_id}]` → `[{item_id, document_id, parent_id, zone, size, type, name, extension, ctime, mtime, share_id, share_info{shared, shared_publicly, share_type}, sharing_token, short_guid, status{error_message,status_code}}]` | main.js L19881-19897, L19714-19736 | VERIFIED |
| Short GUID (share URL) | `POST ws/<zone>/update/create_short_guid[/<owner>]?errorBreakdown=true&document_id=` → `{short_guid, etag}` | main.js L19898-19907, L19709 | VERIFIED |
| Update doc | `POST ws/<zone>/update/documents?errorBreakdown=true` (or `ws/<shareZone>/update/shared/<owner>`) → `{status, results:[{document:{item_id, document_id, etag, name, short_guid, size}, status}]}`; retried ×5 on 409 `ZONE_BUSY` | main.js L19814-19836, L19744-19759 | VERIFIED |
| Mark opened | update/documents body `{document_id, lastOpenedTime: Date.now(), command:"modify_file"}` | main.js L6798-6803 | VERIFIED |
| Enumerate | `GET /v1/enumerate/{files|folders}/<folderId>?limit=200&unified_format=true` → `{drive_item:[...]}`; apps `v1/enumerate/apps` | main.js L14052-14080, L14524 | VERIFIED |
| Favorites | `ws/_all_/list/enumerate/favorites?limit&unified_format=true`; delta `ws/_all_/list/changes/favorites?limit=50&nextPage=<continuationMarker>` (304 unchanged, 205 reset) | main.js L13962, L14002 | VERIFIED |
| Recents | `ws/<appId>/list/recents` | vendors~main~64216fa6.js L21262 | VERIFIED |
| Zone | `ws/com.apple.CloudDocs/internal/create_zone?create_metadata=true&errorBreakdown=true&idempotent=true` | main.js L19850 | VERIFIED |

### Upload (3 steps) — main.js L17746-17960, uploadFiles~5ad45e42.js L170-212
1. `POST ws/<zone>/upload/web?token=<validateToken>` (shared folder: `ws/<zoneName>/<ownerRecordName>/<recordName>` appended) body `{filename, type:"FILE"|"PACKAGE", content_type, size}` → `[{url, document_id, owner_id, owner?}]` (`WireCWSUploadUrlDetails`, main.js L20056-20063). VERIFIED-IN-CODE
2. XHR `POST <url>` with **raw file body** (not multipart), `Content-Type: <file.type>` — uploadFiles chunk L212 `s.open("POST", e.toString()), s.setRequestHeader("Content-Type", r.type) ... s.send(r)`. Response `{singleFile:{fileChecksum, wrappingKey, referenceChecksum, receipt?, size}}` or package form `{hexBrSyntheticChecksum, ckManifest, brManifest, ckSectionAssets}`; client checks `singleFile.size === file.size` (main.js L20038-20067, L17874-17879). VERIFIED-IN-CODE
3. `update/documents` body:
```
{ document_id, path:{ starting_document_id: <folder docwsid | shareID.recordName>, path: <filename> },
  allow_conflict: true, file_flags:{is_writable:true,is_executable:false,is_hidden:false},
  mtime, btime, command:"add_file",
  data:{ signature, wrapping_key, reference_signature, receipt, size } }
```
Signatures are base64url→base64 converted (`l()`, L20032-20036). 413 ⇒ out of quota. VERIFIED-IN-CODE. No `sendFile` endpoint found (gap/none).

### Thumbnails / preview
- Thumbnail = `thumbnail_token.url` from download/batch; else, for png/jpg/jpeg/gif/bmp < 1 MiB, the `data_token.url` itself — main.js L4268-4274. URLs cached 10 min (`x = 6e5`, L20797). VERIFIED-IN-CODE
- iWork thumbnails: `<iworkthumbnailws>/iw/ws/<dsid>/thumbnail?document_id&etag&width&height&zone` — vendors~main~64216fa6.js L29153. VERIFIED-IN-CODE. Export: `/iw/export-ws/<dsid>/<action>` (L24604). VERIFIED-IN-CODE

## Trash "time left"

- Field: **`dateExpiration`** (ISO string) on trashed items — FolderContentsGrid~RecentsGrid~SharedGrid~988faf90.js L152-154 `let { dateExpiration: t } = e`. VERIFIED-IN-CODE
- Computation: `ms = Date.parse(dateExpiration) - Date.now()`; ≤0 → "0 minutes"; `floor(ms/day)` ≥1 → "1 day"/"%{count} days"; else hours; else minutes. Class `time-remaining warn` when < 4 days (`s = 4 * a`) — same file L147-172. VERIFIED-IN-CODE
- Items whose `dateExpiration` is ≥15 days in the past are hidden (`1296e6` ms) — main.js L20464-20467. VERIFIED-IN-CODE
- List column "Time Left" (`FolderContentsList.ColumnHeading.TimeLeft`, main.js L16388). Sidebar "Recently Deleted (%{totalNumberOfItems})" (L16353-16354). Empty: "No items" (L16128). VERIFIED-IN-CODE
- 30-day retention itself not stated in code; server sets `dateExpiration`. INFERRED

## ⋯ item action menu (ItemActionMenu~988faf90.js, module 2173)

Modes: `grid-list-item`, `quick-look`, `compact-toolbar`, `middle-toolbar`, `regular-toolbar`. `inTrash = browse && (cwd==="TRASH_ROOT" || ancestors include it)`. Order and conditions (VERIFIED-IN-CODE, chunk L350-640; labels from main.js L16107-16123, L16200-16202):

0. Header (multi-select only, disabled): "2 Selected" / "%{count} Selected" / "… (%{size})" when no folders.
1. **Open** — not trash, mode grid-list-item, any openable (folder/app lib/known ext); enabled if 1 item, not committing.
2. **Preview** — not quick-look, any item previewable (ext or has download urls); enabled if 1.
3. **Get Info** — not quick-look; enabled if exactly 1. — divider —
4. **Download a Copy…** — not trash, grid-list-item, any file; enabled if all files. — divider —
5. **Share…** / **Manage Shared File|Folder** — not trash, not toolbar modes, any shareable (not app library / homeroom); enabled if 1 item under root.
6. **Send a Copy…** — not trash, Mail service active, not regular-toolbar, any file; enabled if all files (iWork: exactly 1). — divider —
7. **Show in Enclosing Folder** — search results only; enabled if 1.
8. **Duplicate** — not trash/search/quick-look, any file; enabled if 1 file, not read-only share, parent loaded.
9. **Rename** — not trash/search/quick-look; enabled if 1, not app lib, not read-only share.
10. **Move to Folder…** — not trash/search; enabled if all movable and none read-only shared. — divider —
11. **Delete Selected** (destructive) — not trash/search/toolbars; enabled if not read-only and (browse & not committing, or 1 id).
12. **Recover** — trash only. — divider —
13. **Delete Forever…** (destructive) — trash only.
14. **Report Content** — country `au`/`gb` and all items are public TO_ME shares. — divider before.

Action ids: `open, info, rename, download, collaborate, email, send, view, move, duplicate, delete, recover, purge, report, enclosing_folder` (main.js L4403). `Open, Send, Download, Report` run sync; others async (chunk L330). VERIFIED-IN-CODE

## Toolbars, menus, dialogs

- Sort menu "Sort by": Name, Kind, Date, Size (SortMenu chunk; main.js L16389-16397). View menu "View as": Grid, List (ViewMenu chunk; L16398-16400). List columns: Name, Kind, Size, Date, Shared, Time Left (L16383-16388). VERIFIED-IN-CODE
- Trash footer (TrashFooter~2e6c9896.js): partial selection → **Recover** + **Delete** (destructive); otherwise **Recover All** + **Delete All**; disabled while loading/empty/in-flight. VERIFIED-IN-CODE
- Purge confirm: "Are you sure you want to delete “%{name}”?" / "these %{count} files|folders|items?" + "…will be immediately deleted from iCloud Drive and all your iCloud devices." buttons Delete/Cancel (main.js L15297-15306, L12408-12432). VERIFIED-IN-CODE
- Delete-to-trash confirm only for shared items, Desktop/Documents folders, App Libraries (`Alert.Delete.*`, L15332-15393). Plain items trash without a dialog. INFERRED (no generic key exists)
- Rename: inline text field (InlineTextField/RenamingTextArea chunks); validation alerts `Alert.EmptyName.*`, `Alert.HiddenName.*` (leading dot), `Alert.InvalidName.*`, `Alert.CannotRenameItem.*` ("The name “%{name}” is already taken."). VERIFIED-IN-CODE
- Move dialog: header "Choose a new location to move this item|these items" / "Item(s) will be moved to “%{selectionName}” on %{rootName}", buttons Move/Cancel; shared-item warnings `Alert.Move.*` (main.js L16507-16513). VERIFIED-IN-CODE
- New folder default name "New Folder" (`Common.NewFolder`). VERIFIED-IN-CODE

## Storage quota ("%{size} available")

- Drive does not call the quota endpoint itself: `QuotaController.fetchStorageQuota()` RPCs the parent shell (`QuotaServiceFromChildProtocol` → `fetchStorageQuota`, replies `didFetchStorageQuota`/`failedToFetchStorageQuota`) — vendors~main~64216fa6.js L1132-1144, L43896-43915. VERIFIED-IN-CODE. Refetched after uploads/deletes (main.js L1860, L10314). VERIFIED-IN-CODE
- Shell request: `POST <account/setup host>/setup/ws/1/storageUsageInfo`, `credentials:"include"`, no body — shell `js/main.js` (minified, byte offsets) @645734 `new r.QuotaLink(Object(s.j)("account","setup/ws/1/storageUsageInfo"),…)`, @650092 `this.fetch(this.quotaServiceUrl,{method:"POST",credentials:"include"})`. VERIFIED-IN-CODE
- Response (zod, same place):
```
{ storageUsageByMedia: [{ mediaKey: "photos"|"mail"|"backup"|"messages"|"docs"|"family",
                          displayLabel, displayColor, usageInBytes }],
  storageUsageInfo: { compStorageInBytes, usedStorageInBytes, totalStorageInBytes, commerceStorageInBytes },
  quotaStatus: { overQuota, haveMaxQuotaTier, "almost-full", paidQuota },   // booleans
  familyStorageUsageInfo?: { mediaKey, displayLabel, displayColor, usageInBytes,
                             familyMembers: [{appleId, dsid:number, id, firstName, fullName, lastName, usageInBytes}] } }
```
VERIFIED-IN-CODE. Parse failure ⇒ previous value kept silently.
- Status bar text: `max(0, total - used)` → formatter → `StatusBar.Quota` "%{size} available"; with a selection `StatusBar.Text.SelectionAndQuota` "%{selectionInfo}, %{quotaSize}" — FolderStatusMessage~d3fe4a5e.js L68-82, main.js L16351-16352. VERIFIED-IN-CODE
- Formatter (main.js module 1873, L3889-3934): **decimal** (divide by 1000 while ≥1000, up to 5 steps), rounding `Math.round(x*10^p)/10^p` with default precision bytes 0, kB 0, MB 1, GB 2, TB/PB 2; key `NumberFormatter.ItemSize` with `{number, sizeUnit}`. So 5 GB plan with 1.23e9 used → "3.77 GB available". VERIFIED-IN-CODE (the shared `bytesToHumanizedFormat` in vendors L27221 is binary/1024 and only used by upsell banners).
- Banners: `quotaStatus["almost-full"]` → "Your iCloud Storage is Almost Full" / "If you run out of storage, new files and edits won't be synced to iCloud."; `overQuota` → "Your iCloud Storage is Full" / "New files and edits won't be synced." (main.js L16498-16501). Upload 413 → "Not enough space to upload."; duplicate → `Alert.Duplicate.OutOfQuota.Title`. VERIFIED-IN-CODE

## Thumbnail fetch details

- Thumbnail URL is used **verbatim** from `download/batch` → `thumbnail_token.url` (zod `WireDocWsDownloadUrlsResponse`, main.js L20766-20777); no width/height template is substituted for normal files. VERIFIED-IN-CODE
- No `thumbnail_token` + image extension: client downloads `data_token.url` after a 500 ms debounce and downscales it client-side at 0.5 (`Object(u.a)(e, .5)`), caches `{url, etag}` per item id — main.js L4202-4221. VERIFIED-IN-CODE
- Export pane reuses `urls.thumbnail_token.url` (L8331-8339). Width/height params only exist on the iWork thumbnail service (see above). VERIFIED-IN-CODE

## Keyboard shortcuts (Mod = Cmd on macOS, Ctrl elsewhere — main.js L5170-5175)

- [ ] Enter: open the single selected item (selection mode: toggle) — main.js L18342-18355
- [ ] Backspace: move selection to trash; in Recently Deleted it opens purge confirm — L18356
- [ ] Mod+A: select all — L18357-18361
- [ ] Mod+ArrowUp: go to parent folder — L18337-18341
- [ ] Alt+Space: toggle current item in selection — L18362-18364
- [ ] Arrow keys: move focus/selection in grid/list (FolderContentsGrid~8f07ba20.js L310, List chunk L96); sidebar Up/Down (SourceList~988faf90.js L317-327)
- [ ] Space on focused ⋯ button opens the action menu (Grid chunk L460); Space closes Quick Look (main.js L11489). Space-to-open-Quick-Look: INFERRED
- [ ] Rename field: Enter commits, Escape cancels (InlineTextField~abffe217.js L73-76)
- [ ] Search box: Enter runs search (DriveSearchBox~988faf90.js L140)
- [ ] Drag files from OS onto window/subfolder = upload; drag items onto folder = move; drag onto trash = trash (main.js L18319-18330)
All VERIFIED-IN-CODE unless marked.

## UI checklist (English from main.js L15183-16580)

Sidebar
- [ ] Recents / Browse / Shared / Favorites / Recently Deleted (`FilesGrid.SourceList.Source.*`, L16192-16196); "Recently Deleted (%{totalNumberOfItems})"; favorites "Loading more favorites…", "See more favorites on your device."
- [ ] Toggle: "Show Sidebar" / "Hide Sidebar"

Folder toolbar (`FolderToolbar.Button.*`, L16204-16233)
- [ ] New Folder, Upload ("Upload More" / disabled "Uploading Not Available"), Delete, Download, Email, Move ("Move Items"), Rename, Info, More, Share File|Folder|Document|Spreadsheet|Presentation / Manage Shared …
- [ ] View toggle "Show items as icons or in a list": Grid / List; Sort menu "Sort by" Name/Kind/Date/Size
- [ ] New iWork: "Create Document|Spreadsheet|Presentation" (`Menu.New.Document.*`)
- [ ] Selection mode (touch): Select, Select All, Deselect All, Done, "Select Items"/"1 Item Selected"/"%{number} Items Selected"

⋯ / context menu — see "⋯ item action menu" above (Open, Preview, Get Info, Download a Copy…, Share…/Manage Shared, Send a Copy…, Show in Enclosing Folder, Duplicate, Rename, Move to Folder…, Delete Selected, Recover, Delete Forever…, Report Content; header "%{count} Selected (%{size})"; tooltip "Actions")

Trash toolbar/footer
- [ ] Recover / Recover All / Delete / Delete All; Time Left column; "No items"

Status bar
- [ ] "0 items" / "1 item" / "%{n} items"; "%{numberOfSelectedItems} of %{totalNumberOfItems} selected"; "Loading…"; "%{size} available"; "Path Bar"

List columns
- [ ] Name, Kind, Size, Date, Shared, Time Left

Empty / error states
- [ ] Root: "Your iCloud Drive is empty." / "You can upload files by dragging them here\nor by using an iCloud Drive enabled app."
- [ ] Folder: "This folder is empty" / "You can upload files by dragging them here."
- [ ] Trash: "No items"; Search: "No results"
- [ ] Recents: "No items" / "Files which you’ve opened recently will appear here."; error "There was a problem loading your recently opened files." / "Please try again later."
- [ ] Shared: "No items" / "Shared files will appear here."; error "There was a problem loading your shared files."
- [ ] Generic: "An unexpected error has occured." / "Please try again later."

Search
- [ ] Location filter: "Anywhere in Drive", "Shared With Me", "“%{folder}”"; tokens "Name Contains “%{text}”" → "Name: %{text}", "Kind: %{text}"

Recents grouping
- [ ] Today, Yesterday, Previous 7 Days, Previous 30 Days, then month names

Dialogs
- [ ] Purge confirm (`Alert.Purge.*` L15297-15331), Delete-to-trash for shared/Desktop/Documents/App Libraries (`Alert.Delete.*` L15332-15488)
- [ ] Move picker (`MoveItems.*` L16507-16513) + shared-move warnings (`Alert.Move.*` L15394-15599, `DragDrop.Dialog.Move.FileShared.*` "Move Anyway" L15638-15649)
- [ ] Rename validation: `Alert.EmptyName/HiddenName/InvalidName/CannotRenameItem` (L15184-15196)
- [ ] Transaction errors per op: "There was a problem creating folder “%{name}”." etc. (`Alert.TransactionError.*` L15197-15234)
- [ ] Download failures: "“%{fileName}” couldn’t be downloaded." / "This file has been deleted and is no longer available." (L15235-15246)
- [ ] Can’t open: "Can’t Open File" / "%{appName} cannot open this file. Would you like to download it to this device instead?" → Download File / Cancel (L15629-15636); iWork in shared folder "This document can’t be opened." (L15600-15601)
- [ ] Share folder containing shared items: "Are you sure you want to share this folder?" (L15307-15309)
- [ ] Duplicate out of quota: "Not enough free space to duplicate file." (L15276); row status "Waiting to duplicate…" (L16564)

Uploads (L16520-16580)
- [ ] Card "Uploads" + Close; "Uploading %{count} items to iCloud", "%{count} Items Uploaded", "%{count} of %{count2} Items Not Uploaded", "Retry %{count} Items", "Keep this page open while your files upload."
- [ ] Per item: Pending, Finalizing, Canceled, Retry, "Cancel upload"; row "Waiting to upload…", "Uploading…", "Finalizing upload…"; "%{transferredSize} of %{totalSize}"
- [ ] Errors: "Filename can’t begin with a period.", "Invalid filename.", "Duplicate filename.", "Can't upload.", "Not enough space to upload.", "Can't upload folders.", "File type not supported.", "Not a supported %{appLibraryName} file.", "File can't be larger than 10 GB."

## Gaps
- No callers found for `/retrieveTrashDetails`, `/deleteItemsInTrash`, `/retrieveSharedItemDetailsInFolders` (paths only). Trash restore/purge endpoints are `putBackItemsFromTrash` / `deleteItems` (above); no extra trash-only fields beyond `dateExpiration`, `restorePath`.
- Trash item fields other than `dateExpiration`, `restorePath` are passthrough (not in a zod schema).
- Download-folder-as-zip endpoint not found; folders aren't downloadable from the menu (Download requires all files).
- Query params added by the fetch service (`clientBuildNumber`, `dsid`, etc.) not traced.
- `storageUsageInfo` host: resolved via `urlForServiceName("account", …)` (setup.icloud.com is INFERRED, not literal in code). Whether the POST needs a body or query params beyond cookies: none seen.
- `thumbnail_token.url` contents/expiry are server-defined; no size parameter is client-controlled for non-iWork files.
- Space-to-Quick-Look binding not located precisely.

## Newer docws "unified" API (VERIFIED-IN-CODE iclouddrive main.js; seen live 2026-10-19)
The current web app reads the root through docws as well as drivews:
- `GET docws /v1/enumerate/files/{id}?limit=200&unified_format=true` and `/v1/enumerate/folders/{id}`.
  `{id}` is `root` for the top level (main.js:14053, 14077). The response is `DocwsUnifiedEnumerateResponse {drive_item: DriveItem[]}`.
- `GET docws /v1/enumerate/apps`: app libraries.
- `GET docws ws/{zone}/list/lookup_by_id?document_id={id}&unified_format=true` (19875).
- Favorites: `ws/_all_/list/enumerate/favorites?limit` and `ws/_all_/list/changes/favorites` (13963, 14002).
  Paginated `{continuationMarker, documents[], status: CONSISTENT|MORE_AVAILABLE}`. A body `{statusCode: 205}` means "reset and enumerate again".
- Shared: `ws/_all_/list/enumerate/sharedByMe` and `sharedToMe` (19067).
- **DriveItem (unified format)** (13676): `docwsid, drivewsid, zone, shortGUID?, isChainedToParent, dateChanged?, item_id,
  parentId?, type? (FILE_UNKNOWN|FILE|FOLDER|APP_LIBRARY|ALIAS|FINDER_ALIAS|SYMLINK|LINKED_FILE|SH…), name?, etag?, extension?,
  dateModified?, favorite_order?, share? (TO_ME|FROM_ME), share_info {shared_publicly}, shareID {shareName, recordName, zoneID},
  size?, lastOpenTime?, urls?, assetQuota?, fileCount?, shareCount?, shareAliasCount?, directChildrenCount?`. Paginated documents add `owner {firstName,lastName,ownerRecordName}`.
- Upload in this API (13650): `{item_id?, parent_item_id, size, name, content_type}` returns `DocwsDriveItemUploadResponse {item_id, url}`, and the item is created as `{item, bounce_if_exists}`.
- Errors: `{errorMessage, serverErrorCode, requestUuid}`.

Quince uses drivews `retrieveItemDetailsInFolders` and `retrieveItemDetails`, which are still served and still used by the web app.
Worth adopting: `lastOpenTime` (Recents) and `favorite_order` (Favorites).

## beta.icloud.com Drive (2638BuildBeta31, against www 2638Build17): the v1 item API (VERIFIED-IN-CODE, 2026-10-19)
Source: `~/.cache/quince-icloud-re/beta/iclouddrive/main.pretty.js:19035–19300`.
Every call goes to the **docws** host (`urlForServiceName("docws")`). The defaults are `POST`, `Content-Type: text/plain`
(the body is still JSON) and `credentials: include`. `sharing_url_key` is passed when the item is in a shared folder.
| Action | Call |
|---|---|
| Create folder | `POST v1/item?unified_format=true` with `{item: {item_info: {type: "FOLDER", name, parent_item_id}}, bounce_if_exists: false}` |
| Create file | `POST v1/item?unified_format=true` with `{item: {item_id, item_info: {type: "FILE", name, parent_item_id, data}}, bounce_if_exists: false}` |
| Upload slot | `POST v1/item/upload` with `{item_id?, parent_item_id, size, name, content_type}`, returning `{item_id, url}` |
| Duplicate | `POST v1/item/copy/{itemId}?dsid&unified_format=true` with `{info_to_update: {}}` |
| Rename / move / anything | `PUT v1/item/{itemId}` with `{info_to_update: {…}}` |
| Many at once | `PUT v1/items` with `{item_ids: [...], drive_item_update_request: {info_to_update: {…}}}`, returning `{drive_items_with_status: [{status_code: 0 OK or 1 ERROR, status_message, drive_items: {drive_item: []}}]}`. In shared folders it is done one item at a time. |
| **Trash** | `info_to_update: {parent_item_id: "trash"}` (single or many). There is no separate trash call any more. |
| **Move** | `info_to_update: {parent_item_id: <dest>}` |
| List | `GET v1/enumerate/{files or folders}/{parentId}?limit&unified_format=true`. The host is rewritten to the account partition `pNN-`. |
| Download | `GET v1/download/{itemId}?redirect=false`, returning a URL. Batched downloads build `v1/download/...` URLs directly. |
| Apps | `GET v1/enumerate/apps` |
| Lookup | `POST ws/{zone}/list/lookup_by_id` with `[{etag, document_id}]` |
| Short link | `ws/{zone}/update/create_short_guid/...?document_id&errorBreakdown=true` |
Errors: `{summary, status, serverErrorCode, reason}`.

**For Quince:** drivews (`retrieveItemDetailsInFolders`, `moveItemsToTrash`, `createFolders`, …) is still what www uses
and still works. When this beta reaches www, Drive could move to `v1/items`. That is one call with per-item status
for trash and move, which is simpler than drivews's etag dance. Check that it works on a throwaway item before switching.
