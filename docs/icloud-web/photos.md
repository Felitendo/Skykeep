# iCloud.com Photos web app: reverse-engineered notes

Source: beautified `photos3/` bundle (`main.js`, `ui.main.js` and about 95 route chunks).
Path: `scratchpad/icloud-bundles/pretty/photos3/`. Citations use `file:line`.
Read-only static analysis. Bundles were never executed.
Tags: **VERIFIED-IN-CODE** means the code shows it directly. **INFERRED** means it is a reasonable reading that I have not proven.

## 1. Container, database, zones, host

- **VERIFIED-IN-CODE**: the container is `com.apple.photos.cloud`, environment `production`. `main.js:12156` `const u = "com.apple.photos.cloud"`, `main.js:12158` `environment: "production"`. A second container, `com.apple.icloud-photos.fdb` (FoundationDB-backed), is built for the same dsid (`main.js:12264`). Shared links use `https://ckshare.icloud.com/ws/v1/<cid>/<env>/` (`main.js:12276`).
- **VERIFIED-IN-CODE**: the host is the `ckdatabasews` service URL (`main.js:12257` `this.urlForServiceName("ckdatabasews")`). The raw URL builder is at `main.js:10286`:
  ```js
  `${host}/database/1/com.apple.photos.cloud/production/${scope||"private"}/${op}?remapEnums=true&getCurrentSyncToken=true&dsid=${dsid}&clientBuildNumber=…&clientMasteringNumber=…[&sharing_url_key=…]`
  ```
  Requests are `POST` with `content-type: text/plain` and `credentials: "include"` (`main.js:10288-10294`).
- **VERIFIED-IN-CODE**: the endpoints used by the container client (`main.js:11763-11800`) are `/records/lookup`, `/records/query`, `/internal/records/query/batch?parallel=true`, `/records/modify`, `/records/copy/start|status`, `/records/zip/prepare`, `/zones/list` (GET), `/zones/modify`, `/changes/database`, `/changes/zone` and `/assets/upload`. Bootstrap calls `zones/list` with `&excludeSCZones=true` (`main.js:10310`).
- **VERIFIED-IN-CODE**: the main library zone is `{zoneName:"PrimarySync", zoneType: RegularCustomZone}` (`main.js:10413`) in the **private** database. The zone-state record is `records/lookup {records:[{recordName:"PrimarySync-0000-ZS"}]}` (`main.js:10317-10324`).
- **VERIFIED-IN-CODE**: zone name families are classified by prefix at `main.js:10193`: `CMM-*` (shared iCloud links / moments), `PrimarySync*`, `SharedSync*` (iCloud Shared Library), `Exit-SharedSync*` and `Unknown`. The zone store tracks the primary zone, the shared zone and CMM zones (`main.js:35845-35855`). Error enum: `NoPrimarySyncZone`, `InvalidPrimarySyncZone`, `DisabledPrimarySyncZone` (`main.js:36103`).
- **INFERRED**: the database for the SharedSync zone is per-zone (`zoneStore.getDatabaseScopeForZoneID`, `ui.main.js:20727`). It is probably `shared` for participants and `private` for the owner. I did not prove this.

## 2. Record types and fields

Record types listed in `main.js:6871`: `CPLAlbum`, `CPLMaster`, `CPLAsset`, `CPLLibraryInfo`, `CPLContainerRelation`, `IndexCountResult`, `CheckIndexingState`, `CPLZoneState`, `CMMRoot`, `Share`. Elsewhere: `CPLMemory`, `CPLMemoryAssetList`, `CPLPost`, `CPLTextComment` and `CPLReact`.

**CPLAsset** (zod at `main.js:14971-15024`, VERIFIED-IN-CODE). This is the per-photo "edit/state" record.
- `masterRef` (reference to CPLMaster), `isFavorite`, `isHidden`, `isDeleted`, `isExpunged`, `isSparsePrivateRecord`, `rating` (all numbers 0/1)
- `assetDate`, `assetDateEnc`, `addedDate`, `dateExpunged`, `timeZoneOffset(Enc)`, `timeZoneNameEnc`
- `duration(Enc)`, `orientation`, `adjustmentType`, `adjustmentRenderType`, `adjustedMediaMetaDataEnc`
- `captionEnc`, `extendedDescEnc`, `keywordsEnc`, `locationEnc`, `locationV2Enc`
- `assetSubtype(V2)`, `assetHDRType`, `burstFlags(Ext)`, `burstId`, `vidCompl*`, `videoFrameRate`
- `linkedShareRecordName/ZoneName`, `contributors[]`, `remappedRef`, `proxyState`
- It also carries the derivative groups `res{JPEGThumb,JPEGMed,JPEGLarge,JPEGFull,Original,VidSmall,VidMed,VidHDRMed,VidFull,...}` (the edited renders). See §4.
- Plugin fields (`main.js:15037-15044`): `dynamic_resJPEGThumbResScaledTemplate`, `dynamic_resJPEGFullResScaledTemplate`, `dynamic_resOriginalResScaledTemplate`, `dynamic_resVidComplResScaledTemplate`, `dynamic_HLSPlaylistURL`, `likedByCaller`, `likeCount`.
- `*Enc` fields have `_decoded` variants (`main.js:15050-15058`), which shows the server returns them encrypted or base64. INFERRED: they are plain bplist/base64, as in the other apps.

**CPLMaster** (`main.js:15270-15320`, VERIFIED-IN-CODE). This is the original file.
- `itemType(Enc)` (UTI, e.g. `public.jpeg`), `filenameEnc`, `originalOrientation`, `dataClassType`, `importedBy`, `importedByDisplayNameEnc`, `importedByBundleIdentifierEnc`, `originalCreationDate(Enc)`, `mediaMetaDataType/Enc`, `codec(Enc)` (`ProRes|HEVC|H264`), plus all the `res*` groups including `resOriginalRes`, `resOriginalVidComplRes` (Live Photo video), `resOriginalAltRes` and `resProvRes`.

**CPLAlbum** (`main.js:14923-14941`, VERIFIED-IN-CODE): `albumNameEnc`, `albumType`, `position`, `sortType`, `sortAscending`, `parentId` (folder), `isDeleted`, `userModificationDate`. The root folder id is `"----Root-Folder----"` (`main.js:14922`).

**CPLContainerRelation** (`main.js:15339-15344`, VERIFIED-IN-CODE): `itemId` (asset recordName), `position`, `containerId` (album recordName). The record name is `` `${itemId}-IN-${containerId}` `` (`ui.main.js:7953-7962`).

**Resource object** (`main.js:15241-15247`, VERIFIED-IN-CODE): `{fileChecksum, size, wrappingKey?, referenceChecksum, downloadURL}`. The upload variant omits `downloadURL` and adds `receipt` (`main.js:15248-15256`, `{singleFile:{...}}`).

## 3. Queries (Library, Favorites, Recents, Albums, Hidden, Recently Deleted)

The generic index query builder is at `main.js:15837-15883` (VERIFIED-IN-CODE):
```js
{ query: { recordType: <index>,
    filterBy: [
      {fieldName:"direction", comparator:"EQUALS", fieldValue:{value:"ASCENDING"|"DESCENDING", type:"STRING"}},
      // smart album only:
      {fieldName:"smartAlbum", comparator:"EQUALS", fieldValue:{value:"FAVORITE", type:"STRING"}},
      // rating only: {fieldName:"rating", ..., type:"INT64"}
      {fieldName:"startRank", comparator:"EQUALS", fieldValue:{value:<rank>, type:"INT64"}} ] },
  zoneID: {zoneName:"PrimarySync"}, resultsLimit: n, desiredKeys: [...]? }
```
- **VERIFIED-IN-CODE**: the rank is `direction === Ascending ? i : listLength - 1 - i` (`main.js:15892-15894`). A negative startRank is rejected client-side (`main.js:15812`).
- **VERIFIED-IN-CODE**: the response mixes `CPLMaster` and `CPLAsset` records, which are split by recordType (`main.js:15833-15836`). Each asset is joined to its master through `masterRef`.
- **VERIFIED-IN-CODE**: the smallest example, used for bootstrap thumbnails (`main.js:10349-10363`), is `{query:{recordType:"CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted", filterBy:[{fieldName:"direction",comparator:"EQUALS",fieldValue:{value:"DESCENDING",type:"STRING"}}]}, zoneID:{zoneName:"PrimarySync"}, resultsLimit:18}`.
- **VERIFIED-IN-CODE**: counts come from `records/query {recordType:"HyperionIndexCountLookup", filterBy:[{fieldName:"indexCountID", comparator:"IN", fieldValue:{value:["CPLAssetByAssetDateWithoutHiddenOrDeleted"], type:"STRING_LIST"}}]}` (`main.js:10329-10340`). The query-index to count-index mapping is at `main.js:7060-7102`.

| View | recordType / filter | Count index | Cite |
|---|---|---|---|
| Library (date captured) | `CPLAssetAndMasterByAssetDateWithoutHiddenOrDeleted` | `CPLAssetByAssetDateWithoutHiddenOrDeleted` | `main.js:12537`, `main.js:7062` |
| Library (date added sort) / Recents | `CPLAssetAndMasterByAddedDate` or `...ByAddedDateWithoutHiddenOrDeleted` | `CPLAssetByAddedDate[WithoutHiddenOrDeleted]` | `main.js:23935`, `main.js:7056-7058` |
| Favorites | `CPLAssetAndMasterInSmartAlbumByAssetDate` + `smartAlbum="FAVORITE"` | ~~`CPLAssetInSmartAlbumByAssetDate:FAVORITE`~~ — **checked live 2026-10-20: this id answers 0; walk the smart album instead** | `main.js:12401`, `main.js:7149` |
| Media types | same with `smartAlbum` = `LIVE, VIDEO, PANORAMA, DEPTH, EXPOSURE, TIMELAPSE, SLOMO, SCREENSHOT, ANIMATED`; `BURSTS` uses `CPLBurstStackAssetAndMasterByAssetDate`. **Membership checked live on his library: SCREENSHOT = assetSubtypeV2 3, PANORAMA = assetSubtypeV2 1** (probe/examples/photos_smart_albums.rs) | | `main.js:7149`, `main.js:15849` |
| Rated | `CPLAssetAndMasterWithRatingByAssetDate` + `rating` INT64 | | `main.js:15857` |
| Hidden | `CPLAssetAndMasterHiddenByAssetDate` | `CPLAssetHiddenByAssetDate` | `main.js:12405` |
| Recently Deleted | `CPLAssetAndMasterDeletedByExpungedDate` | `CPLAssetDeletedByExpungedDate` | `main.js:12421` |
| Album contents | `CPLContainerRelationLiveByAssetDate` (or `...LiveByPosition` for manual sort, `CPLContainerRelationByAddedDateNotHiddenOrDeletedEndpoint` for date-added) + filters `startRank`, `direction`, `parentId=<album recordName>` | `CPLContainerRelationNotDeletedByAssetDate` | `main.js:7053`, `main.js:33162-33190` |
| Album list | `CPLAlbumByPositionLive` (or `CPLAlbumByUserModificationDate` when date sort is on), optional `parentId` filter, paged with `continuationMarker` | `CPLAlbumNotDeleted` | `main.js:23974`, `ui.main.js:53840-53870` |

- **VERIFIED-IN-CODE**: an album-contents response returns CPLMaster, CPLAsset and CPLContainerRelation records (`main.js:33193-33201`).
- **INFERRED**: `desiredKeys` is passed through when the caller supplies it (`main.js:15880`). I found no fixed literal list for the grid, so it most likely defaults to all keys.

## 4. Derivative / download URLs

- **VERIFIED-IN-CODE**: each derivative group is `res<Kind>{Height,Width,Fingerprint,FileType,Res}` (`main.js:15127-15226`). Kinds: `JPEGThumb, JPEGMed, JPEGLarge, JPEGFull, Original, VidSmall, VidMed, VidHDRMed, VidFull, OriginalVidCompl, OriginalAlt, Prov`.
- **VERIFIED-IN-CODE**: the URL is `fields.res<Kind>Res.downloadURL`, with `${f}` replaced by `encodeURIComponent(filename)`, which defaults to `"unknown.jpeg"` (`main.js:16211-16222`). Example: `r.split("${f}").join(encodeURIComponent(e))`.
- **VERIFIED-IN-CODE**: scaled templates come from `pluginFields.dynamic_res{JPEGThumb,JPEGFull,Original}ResScaledTemplate`, with `${f}` set to the filename and `${m}` set to the max pixel dimension (`main.js:40385-40389` `.split("${f}")...split("${m}").join(`${t}`)`).
- **VERIFIED-IN-CODE**: the thumbnail is usable if `resJPEGThumbRes.downloadURL || resJPEGMedRes.downloadURL` (`main.js:33028`). The original is `resOriginalRes.downloadURL` plus `.size` (`main.js:40315-40327`). A QuickTime master is downloaded as m4v (HEVC) or mp4 (`main.js:16203`).
- **VERIFIED-IN-CODE**: an expired URL is refreshed by matching `fileChecksum` (`main.js:32979-32986`, kinds `resJPEGThumb, resJPEGMed, resVidSmall, resVidMed`).

## 5. Writes (`records/modify`)

The update builder is at `ui.main.js:20071-20126` (VERIFIED-IN-CODE):
```js
{ atomic: true, zoneID, operations: [{ operationType: "update",
    record: { recordName, recordChangeTag, recordType: "CPLAsset", fields: { isFavorite: {value: 1} } } }] }
```
Each field is wrapped as `{value}`. Special types: `assetDateEnc` → `{type:"TIMESTAMP", isEncrypted:true}`; `emojiStringEnc` → `{type:"STRING", isEncrypted:true}`. `parentId`, `cloudkit.thumbnailImageData` and `keyAssetIdentifier` are sent even when they are undefined, which clears them.

| Action | Fields | Cite |
|---|---|---|
| Favorite / unfavorite | CPLAsset `isFavorite: 1/0` (toggles to 1 if any selected asset is not a favorite) | `ui.main.js:20733-20738` |
| Hide / unhide | CPLAsset `isHidden: 1/0` | `ui.main.js:40969-40973` |
| Delete (to Recently Deleted) | CPLAsset `isDeleted: 1` | `ui.main.js:22739-22741` |
| Recover | CPLAsset `isDeleted: 0` | `ui.main.js:23058-23060` |
| Delete permanently | CPLAsset `isExpunged: 1` | `ui.main.js:51640-51642` |
| Delete album/folder | CPLAlbum `isDeleted: 1` | `ui.main.js:25914-25916` |
| Move between Personal and Shared Library | `isSparsePrivateRecord: 1` in PrimarySync, or `isDeleted: 1` in the source | `ui.main.js:41084-41088` |
| New album/folder | `create` CPLAlbum `{albumNameEnc, albumType, isDeleted:0, isExpunged:0, position, sortAscending:1, sortType:CreationDate, parentId?}` | `ui.main.js:54795-54808` |
| Add to album | read the last position with `CPLContainerRelationLiveByPosition` (`startRank=MAX_SAFE_INTEGER, endRank=0, parentId, direction=DESCENDING`, `resultsLimit:3`), then `create` CPLContainerRelation `{itemId, position, containerId}` named `itemId-IN-albumId` | `ui.main.js:49970-50070`, create builder `ui.main.js:19885-19901` |
| Force delete | `operationType:"forceDelete", record:{recordName}` | `ui.main.js:22869-22884` |

- **VERIFIED-IN-CODE** (2026-10-19), **Remove from album**: `removeCPLAssetsFromAlbum` (`ui.main.js:54398-54417`) sends
  `forceDelete` for `` `${asset}-IN-${album}` `` through the builder at `ui.main.js:22869-22884`
  (`{atomic:true, zoneID, operations:[{operationType:"forceDelete", record:{recordName}}]}`). It does **not** set
  `isDeleted`, and the photo stays in the library. Confirm dialog: "Remove %{count} items from album?".
- **VERIFIED-IN-CODE, album title**: `encodeFieldValue(title)` = `btoa(unescape(encodeURIComponent(title)))`, i.e.
  base64 of the UTF-8 bytes, sent as a plain `{value}` with no type (`main.js:36424-36433`).
- **VERIFIED-IN-CODE, New album** (`ui.main.js:54770-54840`): the create builder (`ui.main.js:19908-19921`) sends
  `{recordType:"CPLAlbum", fields:{k:{value}}}` with **no recordName**, so CloudKit names it. Fields:
  `albumNameEnc`, `albumType` (0 Normal, Folder for folders), `isDeleted:0`, `isExpunged:0`, `sortAscending:1`,
  `sortType:1` (CreationDate; the enum is at `main.js:7136`: 0 Manual, 1 CreationDate, 2 LastModifiedDate, 3 ImportDate, 4 TrashDate,
  5 Title, 100 Custom), and `parentId` only inside a folder. `position` = max(first existing album position − 2, 0),
  or `MAX_POSITION_VALUE` = `MAX_SAFE_INTEGER − 1024` when there are none (`ui.main.js:7562`, `7585-7597`). A new album therefore
  sorts **first**.
- **VERIFIED-IN-CODE, Rename album** (`ui.main.js:27722-27760`): `update` CPLAlbum `{albumNameEnc, userModificationDate: Date.now()}`
  with its `recordChangeTag`.
- **VERIFIED-IN-CODE, Add to album positions** (`ui.main.js:49955-50070`): the last position is read DESCENDING from
  MAX_SAFE_INTEGER (`resultsLimit:3`), or 1024 when the album is empty. Each new item gets `min(prev + 1024, MAX_SAFE_INTEGER − 1024)`,
  so items append at the end, 1024 apart.

## 6. Upload flow

There are two paths. The bundle picks one with a ramp flag (`z.a` at `ui.main.js:40430`; there is also an `isRamped` endpoint).

**V2 (`photosupload` service).** VERIFIED-IN-CODE.
- The base URL is `urlForServiceName("photosupload")` + `"photosupload/" + endpoint + "?dsid=<dsid>"`. Shared-collection uploads use `?callerCKUserId=&ownerCKUserId=` instead. Each request is `POST`, `Content-Type: text/plain;charset=UTF-8`, with a JSON body and up to 3 retries (`ui.main.js:6102-6140`). Endpoints: `createUploadUrl`, `putAsset`, `uploadStatus`, `isRamped` (`ui.main.js:6174-6179`).
1. `createUploadUrl` `{zoneName, assets:{<uuid>: <file size>}}` → `{uploadUrls:{<uuid>: url}}` (`ui.main.js:40017-40061`).
2. XHR upload of the bytes to that URL (`xhrRequestURL`, `ui.main.js:40409`). **INFERRED**: the response is the `{singleFile:{fileChecksum,size,wrappingKey,referenceChecksum,receipt}}` shape (`main.js:15248`).
3. `putAsset` `{zoneName, localTimeZoneId, importGroup?, assetBatchId?, files:[{fileName, lastModDate, timeZoneOffset, singleFileUploadRequest:<step 2 result>}]}` → `[{uploadJobId, cplMaster, cplAsset}]` or `{response:{status, errorMessage?, isRetryable?}}` (`ui.main.js:40083-40170`).
4. Poll `uploadStatus` `{uploadJobIds:[...]}` → `{<jobId>: {progress} | {errorCode}}`, where done means progress ≥ 100 (`ui.main.js:40172-40260`). Then the client fetches the record and, when the upload targets an album, runs the add-to-album step.

**V1 (`uploadimagews`).** VERIFIED-IN-CODE (`ui.main.js:50930-50981`).
- The URL is `${uploadimagews}upload?filename=<enc name>&dsid=<dsid>&lastModDate=<ms>&timezoneOffset=<min>&clientBuildNumber=…`. The response is `{isDuplicate, records:[{recordName,recordType}], remappedRef?, requestUUID}`.
- **INFERRED**: the host is `p*-uploadimagews.icloud.com`, taken from the webservices map rather than the bundle.

**Limits** (`ui.main.js:6250-6280`, VERIFIED-IN-CODE):
- V1: JPEG only, 100 MB.
- V2: JPEG, PNG, HEIF, HEIC, MOV, MP4, TIFF, GIF, M4V and WebP, up to 500 MB.
- Width up to 20480 px; area up to 419430400 px²; file names up to 255 characters.
- One upload at a time; 1000 files per batch.

## 7. UI checklist (loc key → English; the table is at `ui.main.js:55015-57245`)

Quince status (2026-10-20, later): built and proven — keys (F . H L = - I Z Cmd+.), slideshow,
sort menu, whole-photo grid toggle, Media Types (Videos, Panoramas, Screenshots by Apple's own
subtype rule), Rated, Hidden, Recently Deleted (days left, Delete All), Albums page and sidebar
branches, Add/Remove to album (menu, drag), upload panel, Download Options, Title/Caption,
Rating, Keywords, Adjust Date, place (read-only coordinates), search, filmstrip, zoom.
Writes proven live on throwaways: favorite, hide, delete/recover/expunge, album create/rename/
delete/add/remove, title+caption, rating, keywords, date, upload (incl. a 152 MB video,
byte-identical). Audit (probe photos_audit): every view's count matches Apple's.
Still open: location write (needs a geocoder — asked him), album folders, Live Photos playback
(none in his library), Memories, Shared Albums, iCloud Links, locked Hidden/Recently Deleted,
in-app video playback (needs GStreamer plugins — asked him).

Sidebar
- [ ] `SidebarNode.Library.Title` Library, `.Memories` Memories, `.Recents` Recents, `.Favorites` Favorites, `.Hidden` Hidden, `.RecentlyDeleted` Recently Deleted, `.MediaTypes` Media Types, `.Rated` Rated, `.MyAlbums` Albums, `.SharedAlbums` Shared Albums, `.ICloudLinks` iCloud Links
- [ ] `Sidebar.AddButton.Label` "Add an album or folder" → popover New Album / New Folder; `Sidebar.Folder.Loading` "Loading…", `Sidebar.Error.LoadFailed` "Failed to load"; Hide/Show sidebar
- [ ] Library switcher: `Library.Personal` Personal Library / `Library.Shared` Shared Library / `Library.Both` Both Libraries

Toolbar (`Toolbar.*`)
- [x] Upload, New album or folder ("Create a new album or folder"), Share, Download, Delete, Remove, Toggle favorite / Add to Favorites / Remove from Favorites, Info ("Get info about this photo"), More…, Sort
- [x] Select / Select All / Deselect All / Cancel / Edit / Done; Recover, Delete, Delete All (Recently Deleted)
- [ ] Zoom slider (Zoom in/out, "Fit", `%{value}%`), square vs full aspect thumbnails, "View months" (jump to date), Start slideshow, thumbnail strip show/hide, full screen enter/exit

Action / context menu (`ActionMenu.*`, pluralised `.1/.2/.Multiple`)
- [x] Favorite / Unfavorite, Hide / Unhide, Delete, Remove (from album), Recover, Delete (expunge), Download, More Download Options…, Add to Album…, Add to Shared Album…, Share…, Copy Image, Info, Edit Location, Edit Keywords, Show in Library, Start Slideshow
- [x] Album menus: New Album, New Folder, Rename Album/Folder, Delete Album/Folder, Make Album Cover, Reset Album Cover
- [ ] Move to Shared Library / Move to My Personal Library
- [ ] Sort menu: Sort By (Date Captured / Date Added), Direction (Newest First / Oldest First) (`Grid.Sort.*`)

Keyboard shortcuts (grid, `ui.main.js:27206-27291`; skipped while Cmd/Ctrl is held)
- [ ] `=` / `-` change thumbnail size
- [x] Space / Enter open the item
- [ ] `F` or `.` toggle favorite
- [x] Backspace / Delete delete or expunge
- [ ] `H` or `L` hide or unhide
- [ ] `I` info
- [ ] `Z` zoom to fit / full
- [x] One-up view (`oneup.js:6615-6635`, `oneup.js:7311-7320`): Left/Right navigate (mirrored in RTL); Escape or Cmd/Ctrl+`.` closes
- [ ] Slideshow: Left/Right/Space (`memory~slideshow.js:1417-1435`)

Dialogs (`Alert.*`)
- [x] Delete: "Delete %{count} items from all your devices?" / "These items will be deleted from iCloud Photos on all your devices." [Delete]
- [x] Expunge: "Are you sure you want to delete these %{count} items permanently?" / "This action cannot be undone." [Delete]
- [x] Hide: "Hide %{count} items?" / "These items can be found in the hidden album." [Hide]
- [x] Remove from album: "Remove %{count} items from album?" / "…will remain in your photo library." [Remove]
- [x] Delete album: "Are you sure you want to delete this album?" (+ the "items will still be visible" text); Delete folder: "…This will also delete all albums inside this folder…"
- [ ] Failures: "Unable to Delete/Hide/Remove/Recover Item(s)" + "…due to a server error."; Recover over quota: "…because your iCloud storage is full."; "Unable to create album. Try again later."
- [ ] Recently Deleted header note (`Grid.View.Header.Recently.Deleted.Note`): "Photos and videos show the days remaining before deletion…"

Empty states
- [ ] `Grid.EmptyPage` "No Photos or Videos"; `.PersonalLibrary` "Personal Library is Empty"; `.SharedLibrary` "Shared Library is Empty" (+ "Show Personal Library"); `.Folder` "No Albums or Folders"; `.AddButton` "Add Photos"
- [ ] `NoLibrary.Title` "Turn on iCloud Photos" + description
- [ ] `AlbumInSharedLibrary.Title` "Albums are not yet supported in Shared Library view on iCloud.com"
- [ ] Locked (Hidden / Recently Deleted): "Enter Your Password to View This Album" [View Album]

Uploader (`Uploader.*`)
- [ ] Titles: "Uploading %{count} Items to iCloud", "Finalizing Uploads", "%{count} Items Uploaded", "%{count} of %{count2} Items Not Uploaded"; "Keep this page open while your files upload."
- [ ] Buttons: Stop Upload, Retry / Retry All, Show/Hide Details, Add iCloud Storage
- [ ] Menu: Add to Album…, Show in Recents, Show in Library
- [x] Errors: "This item’s file type isn’t supported.", "Item is too large to upload.", "Duplicate item.", "Not enough space to upload item.", "File name too long."

## Gaps
- Upload host names (`photosupload`, `uploadimagews`) and `ckdatabasews` come from the webservices map at login, not from this bundle.
- I did not trace the step-2 V2 XHR (method, headers, body format) end to end. The `singleFile` receipt shape is INFERRED.
- Album writes: **proven live 2026-10-19** (`probe/examples/photos_albums_roundtrip.rs`, throwaway album
  CAB752C8-…, now deleted). Unnamed create → CloudKit returns an uppercase UUID recordName; add → relations
  read back as `{containerId, itemId, position:NUMBER_INT64}`; remove (forceDelete) → relation gone, asset untouched;
  rename with a UTF-8 title round-trips; delete → `isDeleted:1`. Deleting an album **leaves its relation records**
  in place (as icloud.com's own delete does), so a mirror must drop an album's items itself.
- `desiredKeys` lists, default `resultsLimit` per grid page and the thumbnail size thresholds (`f`/`p` at `main.js:40385`) are not extracted.
- Whether SharedSync uses the `shared` database for participants is not proven.
- Modifier shortcuts (e.g. Cmd+A select all) are handled outside the grid keydown handler. I only found the search-box Cmd+A (`infopanel~photossearchbox.js:164`), so this list may be incomplete.
- Memories, shared albums (`sharedstreams`) and iCloud Links (CMM) are not documented beyond their zone names.
- beta.icloud.com Photos (2638BuildBeta15, its own numbering) vs www 2638Build29: the core main.js uses the same CloudKit record types and queries (60 identifiers, none only in beta). The www-only names (CPLReact*, text comments, Hyperion) live in lazy chunks not fetched for beta. No sign that beta Photos is ahead. Checked 2026-10-19.

## Verified live (2026-10-19, read-only, his library)
- Paging: rank 0 is the **newest** asset. `direction=ASCENDING` from `startRank=k` walks toward older ones; DESCENDING walks
  back toward the newest. `resultsLimit` counts assets and masters together. A query needs the `remapEnums=true` URL
  parameter, or Apple answers "BadRequestException: Unexpected input".
- The change feed (`changes/zone` on PrimarySync) mirrors the library: 273 records (135 CPLAsset, 135 CPLMaster,
  2 CPLAlbum, 1 CPLLibraryInfo) in 8 s, with every master's derivative `downloadURL` included. From the token with no
  changes it takes 0.4 s. Quince's Photos sync is built on this (quince_icloud::photos).
- Views from the mirror match Apple: Library 134 (Apple's count says 134), Favorites 3, Videos 1, Screenshots 43, Recently
  Deleted 1. `${f}` in a downloadURL is replaced by the file name to ask for.
- Album contents: `CPLContainerRelationLiveByAssetDate` with filters startRank, direction and `parentId=<album>` (plus
  remapEnums) is accepted. Both of his albums ("wallappaers", "test") are empty by Apple's own count
  (`HyperionIndexCountLookup` on `CPLContainerRelationNotDeletedByAssetDate:<album id>` = 0). Contents are untested on a
  non-empty album, so Quince lists no albums yet.
- **Writes (VERIFIED-IN-CODE, photos3 ui.main.js, 2026-10-19):** every flag change is one `records/modify`
  `{atomic: true, zoneID, operations: [{operationType: "update", record: {recordName, recordType: "CPLAsset",
  recordChangeTag, fields: {<field>: {value}}}}]}` (builder at 20086–20095). Favorite: `toggleCPLAssetsFavorite`
  (≈20733) sets `isFavorite` to 1 when the one asset (or any of several) is not a favorite, else 0. Delete to
  Recently Deleted sets `isDeleted: 1`; Recover sets `isDeleted: 0`. **Delete Now** in Recently Deleted is a
  separate operation, `forceDelete` with `{recordName}` (22879), which is permanent. Quince does not offer it
  until asked. Album create is `create` (19896). Quince's `photos::set_flag` sends the same update through
  `ZoneClient::update_record`.
- **Upload (VERIFIED-IN-CODE, photos3 ui.main.js, 2026-10-19):** `POST ${urlForServiceName("uploadimagews")}upload
  ?filename=<encodeURIComponent(name)>&dsid=<dsid>&lastModDate=<ms>&timezoneOffset=<min>&clientBuildNumber=<build>`
  with the file's bytes as the body (XHR, credentials included; 50940–50956). The reply is
  `{isDuplicate: bool, records: [{recordName, recordType}], remappedRef?, requestUUID}` (50922). Limits (6246–6272):
  - **iCPL (v1):** JPEG only, 125 B to 100 MB, 1 to 20480 px a side, area ≤ 419,430,400 px, file name ≤ 255 characters.
  - **v2** (behind a feature flag): JPEG, PNG, HEIF, HEIC, QuickTime and others, plus WebP.
  - Shared albums: up to 500 MB, adding TIFF, GIF and M4V.
  - Pace: `maxConcurrentUploads: 1`, `batchFileLimit: 1000`.
  - Not yet traced: the v2 request itself, and which of his files v1 would refuse.
- **Upload, VERIFIED LIVE (2026-10-19, probe photos_upload_v2, a 16 KB test JPEG, then sent to Recently Deleted):**
  the old `uploadimagews …/upload` answers **410 Gone**. icloud.com now uses the `photosupload` service
  (ui.main.js 6100–6180, 40020–40260, 63937–64042). Every call is `POST {photosupload}/photosupload/<endpoint>?dsid=<dsid>`
  with a JSON body and `Content-Type: text/plain;charset=UTF-8`. Up to 3 attempts; throttling backs off 0.5 s → 10 s.
  1. `createUploadUrl` `{zoneName: "PrimarySync", assets: {<uuid>: <bytes>}}` → `{uploadUrls: {<uuid>: <url>}}`.
  2. `POST <url>` with the raw bytes and `Content-Type: text/plain` → `{singleFile: {fileChecksum, receipt,
     referenceChecksum, size, wrappingKey}, isDuplicate?}`. isDuplicate means the photo is already in the library.
  3. `putAsset` `{zoneName, localTimeZoneId: <IANA>, importGroup: <batch uuid>, files: [{fileName, lastModDate (ms),
     timeZoneOffset (min), singleFileUploadRequest: <singleFile>}]}` → `[{uploadJobId: "<master>#PrimarySync:<asset>",
     cplMaster, cplAsset, response: {status: 200, isRetryable}}]`, or `[{response: {status, errorMessage, isRetryable}}]`.
     A shared collection uses `assetBatchId` in place of `importGroup`.
  4. `uploadStatus` `{uploadJobIds: [...]}` → `{<jobId>: {progress: 0–100}}` or `{<jobId>: {errorCode}}`. Measured: 95,
     then 100 after ~5 s. Then the CPLAsset is in the zone.
  Errors: an over-quota response text contains QUOTA_EXCEEDED. Status codes include BadRequest 400, Forbidden 403 and
  AlbumNotFound 404.

## Shared Albums (sharedstreams) — mapped and proven live 2026-10-20
Not CloudKit: the `sharedstreams` webservice. Every call `POST {sharedstreams}/{dsid}/sharedstreams/{endpoint}`
(icloud.com's `useDSIDBaseURL`), `Content-Type: text/plain;charset=UTF-8`, JSON body, cookies; the usual
query params. Per album there is also an `albumlocation` base (`https://pNN-sharedstreams.icloud.com:443/<token>/sharedstreams/`).
Endpoints (main.js:20440): webgetalbumslist, webgetpendingalbums, webgetalbumview, webstream, webasseturls,
getcomments, getchanges, albumsummary, createalbum, deletealbum, updatealbum, unsubscribe, subscribe,
deletecomment, setalbumpublic, addcomment, share, unshare, webgetassets, deleteassets, webgetassetcount,
webputasset, webgetuploadurl, webuploadstatus, reportspam, webgetsharinginfos, webgetkeyasset, publicurl.
An optional `clientOrgKey` is spread into a body when known (main.js:23805).
- createalbum `{albumguid: <UUID upper>, attributes: {name, ispublic?: "1", allowcontributions?: "1"}}` → `{albumguid, albumctag}`.
- webgetalbumslist `{}` → `{albums:[{albumguid, albumctag, albumlocation, attributes:{name, ispublic, allowcontributions,
  creationDate}, iswebuploadsupported, ownerdsid, sharingtype:"owned"|…}], callerId, rootctag}`.
- webgetalbumview `{albumguids:[…]}` → albums with `sharinginfo:[{email, firstname, fullname, invitationguid, …}]`.
- webgetassets `{albumguid, offset:"0", limit:"N", albumctag?}` → `{records:[…]}` (CPLMaster/CPLAsset-shaped).
- deletealbum `{albumguid, clientOrgKey?}` → `"OK"`.
Proven: probe/examples/shared_album_roundtrip.rs (throwaway made, listed, viewed, assets read, deleted, gone).
His account has no shared albums and no pending invitations (probe/examples/shared_albums_list.rs).

## What icloud.com beta (photos3 2638BuildBeta15) actually calls — checked 2026-10-20
- Shared Albums: the endpoint table lists putAsset/getUploadURL/getUploadStatus/deleteAssets/addComment, but
  nothing in main.js calls them. The web app only *reads* a shared album (webgetassets, webgetassetcount,
  webgetsharinginfos, webgetkeyasset, webgetpendingalbums). Adding photos to a shared album is unmapped;
  his account has 0 shared albums, so reading contents can't be proven on real data either.
- Memories: CPLMemory records, listed by CPLMemoryNotIsRejectedByCreateDate. His count (HyperionIndexCountLookup)
  is 0, favourites 0 (probe/examples/photos_memories_count.rs).
- iCloud Links (CMM zones): icloud.com only resolves/accepts links others send (resolveCMM, addAcceptedCMM);
  it never creates one.
