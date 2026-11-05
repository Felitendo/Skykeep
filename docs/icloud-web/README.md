# icloud.com web-app reference (local only, do not commit)

Taken from icloud.com's publicly served JS bundles on 2026-10-17. No sign-in was
needed. The bundles were only grepped and never executed.

## How the bundles are found
- Shell: `https://www.icloud.com/` loads `/system/icloud.com/2638Build27/en-us/main.js`.
  That file holds the child-app configs (`applicationName:"notes3"`, `"reminders2"`, ...).
  It builds child URLs as `/applications/<app>/current/<path>` (VERIFIED-IN-CODE).
- Each child app: `https://www.icloud.com/applications/<app>/current/en-us/index.html`
  redirects to a versioned build, which lists its scripts.

| App | Bundle app name | Build | Files |
|---|---|---|---|
| Drive | iclouddrive | 2638Build17 | main.js + vendors + 40 lazy chunks |
| Notes | notes3 (CloudKit com.apple.notes) | 2638Build17 | main.js (single 7.5MB) |
| Reminders | reminders2 (CloudKit com.apple.reminders) | 2638Build16 | main.js |
| Calendar | calendar (/ca/) | 2634Build23 | main.js |
| Contacts | contacts2 (/co/) | 2638Build32 | main.js + N.main.js chunks |
| Photos | photos3 (CloudKit com.apple.photos.cloud, zone PrimarySync) | 2638Build29 | main.js + ui.main.js + 95 chunks |

Local copies (scratchpad, temporary):
`/tmp/claude-1000/-home-yash-Projects/2540edda-3efd-42b2-8acc-a7662fcef7d7/scratchpad/icloud-bundles/`
(`b/` = raw files, `pretty/` = js-beautify output; line numbers in the docs refer to `pretty/`).
The scratchpad is temporary. Re-download if it has been cleared. Build numbers change often.

## beta.icloud.com (checked 2026-10-19)
`https://beta.icloud.com/` serves newer builds: the shell is `2638BuildBeta18`. Compared app by app:
Drive **Beta31** (www 17), Contacts **Beta34** (www 32), Photos Beta15 (www 29, a separate numbering line), and
Notes, Reminders and Calendar at the same build number. The beta Drive uses a new docws `v1/item` API (see drive.md).
Beta bundles are in `~/.cache/quince-icloud-re/beta/`. From now on, prefer the beta build wherever it is newer.

## Index
| Doc | Covers |
|---|---|
| [reminders.md](reminders.md) | List/Reminder records; create, complete, due and all-day writes; Alarm/AlarmTrigger; RecurrenceRule; ResolutionTokenMap JSON; Attachment(URL)/Hashtag/Assignment; Priority; UI checklist (§12) |
| [calendar.md](calendar.md) | Event save body; alarms and recurrence; this/future/all path segment; eventdetail; invitees; attachments; all-day and tz; query params; UI checklist (§13) |
| [notes.md](notes.md) | Paragraph style enum, checklist (style 103 + todo), Aa menu, fontHints, com.apple.notes.table CRDT, UI checklist |
| [drive.md](drive.md) | item copy, download/batch and thumbnail_token, trash dateExpiration, storageUsageInfo quota, shortcuts, UI checklist |
| [photos.md](photos.md) | Container/zone, CPL record types, smart-album queries, derivative URLs, favourite/hide/delete writes, upload, UI checklist |
| [contacts.md](contacts.md) | Contacts (unchanged in this pass) |
| [mail.md](mail.md) | Mail: IMAP-backed mailws2 API, thread search/get/move/flag, send, Mail Drop, feasibility |
| [findmy.md](findmy.md) | Find My: fmipservice initClient/refreshClient, Play Sound, Lost Mode, Erase, auth token; feasibility |
| [shell-auth-account.md](shell-auth-account.md) | Sign-in methods (SRP, 2FA, passkey, security key…), session/validate, account and plan calls; Quince gaps |
| [proto/](proto/README.md) | Apple's own CRDT / topotext / versioned_document protobuf schemas |

Static analysis also ran through REA `analyze-javascript-application` (output in scratchpad `rea-out/`). The documented findings come from grep/sed on `pretty/`.

## Still unknown
- **All:** no live traffic has been captured. CloudKit JS is not bundled in reminders2, so the `records/modify` `operationType` strings there are INFERRED. Real hostnames (calendarws, photosupload, uploadimagews, setup) come from the webservices map at sign-in.
- **Reminders:** the List `Name` setter (`u.b`). All-day DueDate: the reader expects UTC midnight, the writer seems to produce local midnight. Native-only shapes (early reminder TimeInterval, RecurrenceRule creation, Hashtag/Assignment writes). Assignment `Status` and Vehicle `Event` enums.
- **Calendar:** whether all-day events are stored as `"Floating"`; how recurrence and attachments are removed; multi-position `frequencyDays`; empty-state and inbox strings; any shortcuts beyond Delete/Escape/Enter.
- **Notes:** `crTableColumnDirection` values; the `1.` auto-numbering rule; gzip vs zlib acceptance; `EncryptedValues` inner layout; Folder record naming; whether folder delete writes `DeletedDate`.
- **Drive:** callers of retrieveTrashDetails/deleteItemsInTrash; folder-as-zip download; `thumbnail_token` URL lifetime; fetch-layer query params.
- **Photos:** the newer upload path's byte-upload step and response; the remove-from-album body; decoding `*Enc` fields; `desiredKeys` and page sizes; Shared Library database scope.

Tags: **VERIFIED-IN-CODE** means it was seen in a bundle. **INFERRED** means it was
reasoned from nearby code or naming and has not been confirmed against live traffic.

## Seen live (2026-10-19, signed-in browsing, reads only; account number shown as `{dsid}`)
Hosts come from the webservices map (`p177-` is this account's partition).
| App | Calls on open |
|---|---|
| Sign-in | `idmsa /appleauth/auth/verify/device/key[/challenge]` (trusted-device key), `setup /setup/ws/1/accountLogin`, `validate` (421 = session gone) |
| Account | `setup /setup/ws/1/storageUsageInfo`, `/setup/web/family/getFamilyDetails`, `gatewayws /acsegateway/v1/accounts/{dsid}/plans/icloud/pricing`, `…/web/plans/next-larger-plan`, `…/v3/…/subscriptions/features/cloud.storage/plan-summary` |
| Shared settings | `keyvalueservice /json/sync` (POST, repeatedly) |
| Calendar | `GET /ca/state`, `/ca/allcollections`, `/ca/events`, `/ca/notifications` (invitations inbox), `/ca/serverpreferences/`, `/ca/alarmtriggers/` |
| Reminders | `GET /rd/state`, `/rd/alarmtriggers/`; CloudKit `com.apple.reminders` zones/list, records/query, records/lookup |
| Notes | CloudKit `com.apple.notes` records/query |
| Contacts | `GET /co/startup`, `/co/addressbook`, `/co/mecard/`; `POST /co/changeset`; `contactsweb /v1/co/me/interactions` |
| Drive | `docws GET /v1/enumerate/{apps,files/root,folders/root}`, `/ws/_all_/list/enumerate/{favorites,recentDocs,sharedByMe,sharedToMe}`, `/ws/_all_/list/changes/favorites`, `POST /ws/com.apple.CloudDocs/download/batch`; `drivews POST /retrieveItemDetails[InFolders]`; CloudKit `com.apple.clouddocs` subscriptions/list |
| Photos | CloudKit `com.apple.photos.cloud` private+shared zones/list, records/query, records/lookup; thumbnails from `cvws-h2…/B/<token>/public.jpeg`; `sharedstreams /{dsid}/sharedstreams/webgetalbumslist` |
| Mail | `mccgateway /mailws2/v1/thread/search`, `/mailws2/v1/geqs/query`, `/mailis/v1/mail/{recommendations,sync/VIP,sync/RECOMMENDATIONS}`, `/mpp/v1/auth-tokens/keys/latest`; `mcc /cc/mail/v1/account/{dsid}/preference/{web/all,recents,vips}`, `/unified/rule` |

Note: the new Drive web app reads through `docws /v1/enumerate/*` as well as `drivews`.
No writes were captured. The recording and its browser profile were deleted afterwards.
