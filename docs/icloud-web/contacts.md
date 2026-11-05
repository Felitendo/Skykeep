# iCloud.com Contacts web API (`/co/`) — reverse-engineered

Source: `pretty/contacts2/main.js` (build 2638Build32), read-only grep. Line numbers refer to `contacts2/main.js`.
Tags: **VERIFIED-IN-CODE** / **INFERRED**.

## 0. Transport (`Ri`, L30752–30790; `Fi`, L30835–30845) — VERIFIED-IN-CODE
- Service name `"contacts"`; URL from `xi.getWebServices().urlForServiceName("contacts", path)` (L30776).
- Header `{"Content-Type": "text/plain"}` with a JSON-stringified body (`ji`, L30829–30831; `Fi` L30838–30840).
- **PUT/DELETE are tunnelled**: real method becomes POST and `?method=PUT|DELETE` is appended. L30773 `"PUT" !== n.method && "DELETE" !== n.method || (s = Ni(s, "method", n.method), c = {...n, method: "POST"})`.
- Auto query params when set: `syncToken`, `prefToken`, `locale` (formatted `en_US`), `order` (`first,last` | `last,first`, L20948–20949). L30763–30770.
- Body size cap 2 MiB, 10 MiB for `co/import`. L30754 `t.startsWith("co/import") ? 10485760 : 2097152`.
- Timeouts 30 s default, 90 s for list/search/delete/export, 190 s import (L30832–30833, L31523).
- Error code map: 401 AUTH_FAILED, 403 WEB_ACCESS_DISABLED, 507 STORAGE_FULL, 520 SCHEMA_INVALID; 420/421/422 treated specially (`la`, L31043–31060).

## 1. Endpoints — VERIFIED-IN-CODE
| Call | Method | Body | Line |
|---|---|---|---|
| `co/startup` | GET | — → `StartupResponse` (`oa`) | L31173–31178 |
| `co/changeset` | POST | (no body; uses syncToken/prefToken query) → `ChangesetResponse` | L30978–30984 |
| `co/contacts/card/?offset=N&limit=M` | GET | → `{contacts: Contact[]}` | L31358–31364 |
| `co/contacts/card/?search=<q>` | GET | → `{contacts}` | L31370–31376 |
| `co/contacts/card/` | POST | `{contacts:[Contact]}` create | L31382–31384 |
| `co/contacts/card/?method=PUT` | POST | `{contacts:[Contact minus normalized/dateCreated/dateModified]}` | L31396–31398, L31224–31230 |
| `co/contacts/card/?method=DELETE` | POST | `{contacts:[{contactId}]}` | L31414–31420 |
| `co/groups/card/` | POST | `{groups:[{groupId,name,contactIds}]}` | L31453–31462 |
| `co/groups/card/?method=PUT` | POST | `{groups:[{groupId,etag,name,contactIds}]}` | L31472–31482 |
| `co/groups/card/?method=DELETE` | POST | `{groups:[{groupId}]}` | L31490–31495 |
| `co/mecardid` | **PUT** (→ POST `?method=PUT`) | `{"meCardId": "<contactId>"}` | L31501–31510 |
| `co/import/` | POST | raw vCard text → `{…changeset, importedContactIds[]}` | L31517–31531, L31239–31241 |
| `co/export/` | POST | `{contactIdsToExport:[...], exportFileName}` → vCard text | L31546–31570 |

No `co/mecard` (only `co/mecardid`) — VERIFIED-IN-CODE (grep).

## 2. Startup / changeset response `ta` (L30962–30970) + `oa` (L30985–30990) — VERIFIED-IN-CODE
```json
{ "syncToken", "prefToken", "meCardId": "…|null", "isGuardianRestricted", "restricted",
  "contacts": [Contact], "groups": [Group], "collections": [{ "collectionId", "etag", "groupsOrder": [groupId] }],
  "contactsOrder": [contactId], "headerPositions": {"A": 0, ...},
  "deletes": { "collectionIds": [], "contactIds": [], "groupIds": [] },
  "errors": [ { "contactId"|"groupId"|"collectionId" (exactly one), "etag", "errorCode": 409, "errorMessage" } ] }
```
Startup adds `sectionHeaders, sectionIndices, localizedSectionHeaders, localizedSectionIndices`. Collection `Ui` L30847–30851; deletes `ea` L30957–30961; errors `Ji` L30944–30956.

## 3. Contact shape `Yi` (L30906–30933) — VERIFIED-IN-CODE
`contactId (client crypto.randomUUID(), L26227), etag?, prefix?, firstName?, middleName?, lastName?, suffix?, nickName?, phoneticFirstName?, phoneticLastName?, companyName?, phoneticCompanyName?, isCompany?, department?, jobTitle?, birthday?: string, notes?, photo?: {url, signature?, crop?:{x,y,width,height}, "VND-63-MEMOJI-DETAILS"?}|null, emailAddresses?/phones?/urls?/relatedNames?: [{label?, field: string}], streetAddresses?: [{label?, field:{street, city, state, postalCode, country, countryCode (lowercased), subLocality, subadministrativeArea}}], dates?: [{label?, field?}], profiles?: [{label?, field?, user?, userId?, displayname?}], IMs?: [{label?, field:{userName, IMService?}}], normalized?, dateCreated?, dateModified?, isGuardianApproved?, whitelisted?, order?` (schema is `.passthrough()`, so unknown keys are kept).
Label values (e.g. `HOME`, `WORK`, `_$!<Mobile>!$_`) not enumerated in this pass — gap.

## 4. Group shape `Qi` (L30935–30943) — VERIFIED-IN-CODE
`{ groupId, name?, contactIds?: [contactId], etag?, isGuardianApproved?, whitelisted?, headerPositions? }`
- **Create**: client generates `groupId = crypto.randomUUID()` (L34546), POSTs `{groupId, name, contactIds}` (L31458–31462), then updates `groupsOrder` locally (L34557).
- **Rename / membership**: both are the same full-replace PUT `{groupId, etag, name, contactIds}` — membership is the **entire contactIds list**, no add/remove deltas on the wire (L31477–31482). The UI's "Add to List"/"Remove from List" compute the new list client-side — INFERRED from the reducer action `{groupId, addContactIds}` (L34555–34556).
- **Delete**: `{groups:[{groupId}]}` with `?method=DELETE` (L31490–31495).
- Conflicts: 412 on update triggers a `co/changeset` refetch (`Oa`, L31231–31237); per-item `errors[].errorCode` 409/412 handled (L31404–31406).
- Bulk delete >5 contacts: re-polls changeset at 1 s/4 s/16 s to confirm (`Sa`, L31238; L31425–31440).

## 5. UI menus / dialogs — VERIFIED-IN-CODE
Groups are called **Lists** in UI.
- List row menu "List actions": Rename List, Delete List; delete confirm `Delete "%{name}"?` "...will be deleted from every device..." (L34567–34572). New List dialog: "New List", "New list name", Create, default "Untitled List" (L34573–34577).
- Contact list (L47642–47705): Sort (Sort By), Select / "More options": Select All Contacts, Make This My Card, Add to List → "New List…", Remove from List, New Contact, Delete Contact(s) confirm, Remove-or-Delete prompt when in a list ("delete the card … or remove it from the list"), Import vCard, Share: Export vCard…, Print Contact(s)…. Context menu title "Contact actions". Export filenames "%{name1} and %{name2}.vcf", "iCloud vCards.vcf".
- Settings toolbar button (L34566). Sort order `first,last` / `last,first` (L20948–20949), sent as `order` query.

## Gaps
- `zi.b` preprocess applied to contacts (L30934) not inspected (likely normalisation).
- Lazy chunks `1–4.main.js` (editor UI) not mined for field label enums / custom labels.
- Pagination limits used by the app for `offset/limit` not extracted.
- Contacts beta (Beta34) vs www (32): identical API endpoint set (11 paths); the differences are UI only (checked 2026-10-19).

## Labels (VERIFIED-IN-CODE, beta contacts2 Beta34, 2026-10-19)
The editor offers a fixed list per field; each value is stored in Apple's uppercase spelling.
- Phone: `MOBILE, IPHONE, APPLE WATCH, HOME, WORK, SCHOOL, MAIN, HOME FAX, WORK FAX, PAGER`
- Email: `HOME, WORK, SCHOOL, ICLOUD` (default `HOME`)
- URL: `HOMEPAGE, HOME, WORK, SCHOOL` (default `HOMEPAGE`)
- Address: `HOME, WORK, SCHOOL` (default `HOME`)
- Dates: `ANNIVERSARY`
Anything else is a custom label, stored as typed. Quince's editor offers the same lists (ContactsView `LABELS`, 2026-10-19).
