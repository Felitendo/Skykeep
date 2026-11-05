# iCloud.com Calendar web API (`/ca/`) — reverse-engineered

Source: `pretty/calendar/main.js` (build 2634Build23), read-only grep of the beautified bundle.
Line numbers refer to that file. Tags: **VERIFIED-IN-CODE** = literally present; **INFERRED** = deduced, not proven.

## 0. Transport

- All paths are built by `Kl(base, segments[], query)` → `base/seg1/seg2?k=v&...`; undefined query values dropped, segments `encodeURI`d. L64592–64600 `return e + "/" + t.map(function(e) { return encodeURI(e) }).join("/") + "?" + r` — VERIFIED-IN-CODE
- Requests go through `vI({path, serviceName: Ul, fetchOptions, zodSchema})` → `Zl.ckRequest(...)` (L78441–78470). `Ul` is the calendar service host (the `calendarws` gateway) — INFERRED (name not resolved).
- Writes are always HTTP **POST** with `methodOverride=PUT|DELETE` in the **query string**, not the body. L82566–82575 `case "removed": m = { methodOverride: "DELETE" } ... case "dirty": m = { methodOverride: "PUT" }` — VERIFIED-IN-CODE
- Optimistic concurrency: `ifMatch=<etag>` query param. L82606 `n.etag ? { ifMatch: n.etag } : {}` — VERIFIED-IN-CODE
- Common query params: `lang`, `usertz`, `requestID` (monotonic counter `qx.nextId()`), `startDate`, `endDate` (current visible window, so the server returns a ChangeSet scoped to it), optional `delegator`. L82600–82612 — VERIFIED-IN-CODE
- Attachment upload header `{"content-type": "text/plain"}` (`Xy`, L73146–73148) — VERIFIED-IN-CODE
- GUID prefix stripping `vb()`: removes `-sharedinvitation-` / `-delegator-` markers before putting a guid in a path. L73155–73156, L73251–73256 — VERIFIED-IN-CODE
- Date tuples (`qg`) are 7-number arrays `s.tuple([s.number() x7])` L72668. Order `[yyyymmdd, y, m, d, h, min, minutesOfDay]` — VERIFIED L73697 (see §12).

## 1. Endpoint list

Base-path enum, L64590 — VERIFIED-IN-CODE:
`ca/availability, ca/freebusy, ca/nextavailable, ca/collectiondetail, ca/collections, ca/collectionsetting, ca/share, ca/sharereply, ca/eventdetail, ca/events, ca/attachment, ca/idle, ca/inboxitem, ca/serverpreferences, ca/allcollections, ca/notifications, json/sync, co/addressbook, ws/v1/autocomplete`
Plus literal paths: `ca/delegation` (L83926), `ca/delegatordetails` (L85696), `ca/delegationorder` POST (L85732), `ca/delegate` GET/POST (L85769, L85829).

**There is no `ca/startup` in this build.** grep for `ca/startup` = 0 hits — VERIFIED-IN-CODE. Startup = parallel GETs of `ca/allcollections` + `ca/events` + `ca/notifications`, results merged (`Object.assign`). L83060 `i = [wj, _j, Ej], e.a(2, jj(i, r, n))`; merge L83172 `return t.ok ? Object.assign({}, e, t.value) : e` — VERIFIED-IN-CODE
Startup query: `startDate, endDate, usertz, lang, clientVersion, [bday=true if showBirthdays], requestID`. L83496–83504 — VERIFIED-IN-CODE
Startup response schema `Sj` = `{Event[], Alarm[], Collection[], InboxItem[]?, Invitee[]?}` (partial). L83043–83049 — VERIFIED-IN-CODE

| Endpoint | Method | Purpose | Line |
|---|---|---|---|
| `GET ca/events?bday&startDate&endDate&lang&usertz&requestID[&delegator]` | GET | range fetch, response `Ty` (ChangeSet updates) | L82508–82531 VERIFIED |
| `GET ca/eventdetail/<pGuid>/<guid>?lang&usertz` | GET | full event (invitees, alarms, recurrence, attachments) | L82483–82506 VERIFIED |
| `POST ca/events/<pGuid>/<guid>[/<changeRecurring>]?…&methodOverride` | POST | create / PUT update / DELETE | L82577, L82600–82618 VERIFIED |
| `POST ca/attachment/<pGuid>/<guid>?X-name&X-type&ctag&ifMatch…` | POST | upload raw file body | L82634–82659 VERIFIED |
| `GET ca/collectiondetail/<collectionGUID>` | GET | one calendar | L83707–83720 VERIFIED |
| `POST ca/collections/<guid>?methodOverride` | POST | calendar create/rename/delete | L83746–83770 VERIFIED |
| `POST ca/collectionsetting?methodOverride=PUT` | POST | order/visibility/default calendar | L83785–83803 VERIFIED |
| `POST ca/share/<guid>?methodOverride=PUT` | POST | share/publish calendar | L83821–83849 VERIFIED |
| `POST ca/sharereply/<inboxGuid>?methodOverride=PUT` | POST | accept/decline calendar share | L83866–83890 VERIFIED |
| `POST ca/inboxitem/<inboxGuid>?methodOverride=DELETE` body `{}` | POST | dismiss notification | L87611–87628 VERIFIED |
| `POST ca/idle?startDate&endDate&usertz&lang&clientVersion&requestID` | POST | long-poll for changes, response `{ChangeSet}` | L87280–87290, body L87547 VERIFIED |
| `GET/POST ca/serverpreferences` | GET/POST | server prefs | L78700–78730 VERIFIED |
| `POST json/sync` body `{"service-id":"appleprefs","apps":[{"app-id":Ul,"registry-version":""}]}` | POST | KVS prefs | L78648–78660 VERIFIED |
| `ca/availability` GET/POST, `ca/freebusy` POST, `ca/nextavailable` POST | | invitee availability | L79217–79330 VERIFIED |

## 2. Event save / delete (`nj`, L82550–82625) — VERIFIED-IN-CODE

Path segments: `g = n.changeRecurring ? [n.pGuid, f, n.changeRecurring] : [n.pGuid, f]` (L82577). So **"this / future / all" is a 3rd path segment**, e.g. `ca/events/<cal>/<guid>/future`. Enum `eb`: L73152–73153 `e.this = "this", e.all = "all", e.future = "future"`.

Body for create/update (L82588–82598):
```json
{
  "Event": { ...event fields, [startDate,endDate = masterStartDate/masterEndDate when changeRecurring=="all" and !recurrenceMaster] },
  "Invitee": [ ...inviteeDetails ],        // only if present
  "Alarm":   [ ...alarmDetails ],          // only if present
  "Recurrence": [ recurrenceDetails ],     // single-element array
  "ClientState": { "Collection": [ { "guid": pGuid, "ctag": collection.ctag }, /* + oldpGuid entry when moving calendars */ ] }
}
```
- ClientState built L86858–86866 (`f = { Collection: [{ ctag: d.ctag, guid: i.pGuid }] }`, plus `oldpGuid` entry when moving).
- `all` on a non-master occurrence: L82579 `"all" === n.changeRecurring && !n.recurrenceMaster && n.masterEndDate && n.masterStartDate && (y = {startDate: n.masterStartDate, endDate: n.masterEndDate})`.
- `addedFromDelegate` gets a fresh `guid: crypto.randomUUID()` (L82581).
- Delete body (reason `removed`): `{ Event: isJunk ? {...event} : {}, ClientState }` with `methodOverride=DELETE` (L82585–82587).
- Moving calendars: set `oldpGuid` on the event (L91446 `t.oldpGuid = w.guid`) — VERIFIED-IN-CODE.
- Response schema `Ky = {ChangeSet: Dy, guid?, pGuid?}` (L72996–73000); `Dy = {UpcomingAlarm?, fullState?, updates?: Ty, inserts?: Ty, deletes?: [{pGuid?, guid}]}` (L72930–72940).
- `Ty` (ChangeSet payload) = `{Alarm?, Event?, InboxItem?, Invitee?, Recurrence?, Collection?, Participant?}` arrays (L72920–72929).

### "This / future / all" UI → param mapping — VERIFIED-IN-CODE
- Edit dialog "Change repeating event?" (L67964): `onAllEvents` → `changeRecurring: g ? "all" : "future"` (g = isRecurrenceMaster); `onOnlyThisEvent` → `{recurrenceException: true, hadRecurrence: true, changeRecurring: "this"}` (L104766–104780).
- Delete dialog: same mapping (L104822–104852). Reply to invite: `"all"` / `"this"` (L104862–104874).
- Reducer: if master and `future` is chosen, it's upgraded to `all` (L75944 `f.payload.recurrenceMaster && a === eb.future ? f.payload.changeRecurring = eb.all`).

## 3. Event shape `uy` (L72688–72735) — VERIFIED-IN-CODE
`guid, pGuid, oldpGuid?, etag, title?, location?, description?, url?, tz|null, tzname?, startDateTZOffset?, allDay, duration (number), startDate, endDate, localStartDate, localEndDate, createdDate, lastModifiedDate (all qg tuples), masterStartDate?, masterEndDate?, icon, readOnly, hasAttachments, attachments: Attachment[], alarms: string[] (alarm guids), invitees?: string[] (invitee guids), recurrence?: string|null (recurrence guid), recurrenceMaster, recurrenceException, hadRecurrence?, changeRecurring?, eventStatus?, isJunk?, organizer?, transparent? (Show as Free/Busy), extendedDetailsAreIncluded, shouldShowJunkUIWhenAppropriate, birthday* fields, delegator?`.
Event references alarms/invitees/recurrence **by guid**; the objects live in separate top-level arrays.

Client sanitises title/location/description/url to plain text before save (L91484–91500); allDay end is bumped +1 day midnight (L91480–91483). Empty title defaults to "New Event" (L91508).

## 4. Alarm shape `ly` + measurement `cy` (L72736–72751) — VERIFIED-IN-CODE
```json
{ "guid": "<eventGuid>:<uuid>", "pGuid": "<eventGuid>", "messageType": "message",
  "isLocationBased": false,
  "measurement": { "weeks":0, "days":0, "hours":0, "minutes":15, "seconds":0, "before": true },
  "onDate": [/* qg tuple, absolute alarm; mutually exclusive with measurement */] }
```
- Creation L91536–91543 `guid: g.payload.guid + ":" + crypto.randomUUID(), messageType: "message", pGuid: "" + g.payload.guid, isLocationBased: !1`.
- Alarm `pGuid` is the **event guid**, not the calendar. On-date vs measurement: L91528–91534.
- UI max 2 alerts ("Alert", "2nd Alert", L68125/L68130; L91535 `alarms.length <= 2`).
- Presets `Zy` L73004–73087: None, 5/15/30 min, 1/2 h, 1/2 days before, At time of event (`before:false`, all zero). All-day presets `Qy = [1 day, 2 days]` with "(%{time})" labels L67806/L67809.
- Default alarm on new event from pref `defaultEventAlarm` (Cy, L72880) applied L104609.

## 5. Recurrence shape `_y` (L72838–72852) — VERIFIED-IN-CODE
```json
{ "guid": "<eventGuid>*MME-RID", "pGuid": "<eventGuid>",
  "freq": "daily|weekly|monthly|yearly", "interval": 1,
  "count": 5,            // "After N Times"
  "until": [qg tuple],   // "On Date"
  "weekDays": ["MO","WE"], "frequencyDays": "1st|2nd|3rd|4th|last"?, 
  "byDay": [15],         // day-of-month numbers (not weekdays!)
  "byMonth": [3], "weekStart": "SU"?, "recurrenceMasterStartDate"?, "recurrenceMaster"? }
```
- New recurrence guid: L91434 `var t = g.payload.guid + "*MME-RID"`; modifying reuses `g.payload.recurrence` (L91430). Removing dispatches `eventRecurrenceRemoved` (L91442) — wire form of removal INFERRED (probably event saved with `recurrence: null` and no Recurrence array).
- Picker defaults L74150–74180: weekly → `weekDays:[today]`; monthly "each" → `byDay:[date]`, monthly "on the" → `frequencyDays + weekDays`; yearly → `byMonth:[month]` (+ optional on-the). `frequencyDays` values from `zy = ["1st","2nd","3rd","4th","last"]` L72985 (string; format of multi values INFERRED).
- Never / After Number of Times / On Date end options L68279–68289.

## 6. Invitee shape `wy` (L72824–72837) — VERIFIED-IN-CODE
`{ guid: "<eventGuid>:<email or canonicalPhone>", pGuid: "<eventGuid>", commonName?, email?, phoneNumber?, isMe, isOrganizer, inviteeStatus: "ACCEPTED|TENTATIVE|DECLINED|NEEDS-ACTION", isSenderMe?, sentBy?, role?, cutype? }`
Built L91514–91523 (`guid: g.payload.guid + ":" + (n ? AA(n) : t)`, `inviteeStatus: "NEEDS-ACTION"`). Edit = new invitee with `oldGuid` (L91524). Status enum `ry` L72676.
Participant (calendar sharing) `by` L72810–72823. Phone canonicalisation via `ws/v1/autocomplete/canonicalizePhoneNumbers` POST (L84964) — VERIFIED.
"Save & Send" prompt for events with invitees (L67960–67963).

## 7. Attachment shape `sy` (L72680–72687) — VERIFIED-IN-CODE
`{ id?, name?, pGuid, size?: string, type?: "mime/type", url? }`
- Upload: `POST ca/attachment/<pGuid>/<eventGuid>?X-name=<file.name>&X-type=<mime|application/octet-stream>&ctag&lang&usertz&requestID&ifMatch&startDate&endDate`, body = raw File, header `content-type: text/plain`; response `{ChangeSet}`. L82626–82660.
- Limits: 20 MB per file, 20 attachments per event. L100906 `size: String(20)`, L100917 `amount: String(20)` — VERIFIED-IN-CODE.
- Delete: `eventAttachmentDeleted` removes entry from `event.attachments` locally and marks event dirty (L75412–75425); no dedicated delete endpoint exists → sent via normal event PUT — INFERRED.

## 8. Collection (calendar) `dy` (L72752–72780) — VERIFIED-IN-CODE
`guid, ctag, etag, title?, order?, color?, symbolicColor?, enabled, visible, isDefault, isFamily, readOnly, objectType, supportedType: "Event|Todo|Freebusy|null", shareTitle, isPublished, publishedUrl?, prePublishedUrl, isPrivatelyShared, sharedUrl?, participants?, description?, createdDate, lastModifiedDate, extendedDetailsAreIncluded, shouldShowJunkUIWhenAppropriate, delegator?`
- Save body: `{ Collection: <collection>, ClientState: { Collection: [{guid, ctag}], fullState: false } }`; delete body `{}` (L83760–83769).
- collectionsetting body `{ order, visible, DefaultCalendar }` (L83797–83801).
- share body `{ guid, shareTitle, shareType: "published"|"shared"|null, publishedUrl, sendNotification:false, delta:{add:[],delete:[],modify:[],reinvite:[]} }` (L83840–83847).
- sharereply body `{ guid, status ("DECLINED" if isJunk), order, shareURL, shareTitle, [isJunk:true] }` (L83882–83890).

## 9. ca/idle (push/poll)
Body: `{ ClientState: { Collection: [{ctag, guid}...], alarmRange: 60, fullState: true, userTime: 1234567890 } }` (L87547–87557), query includes `clientVersion`. — VERIFIED-IN-CODE

## 10. Preferences `Cy` (L72864–72889) — VERIFIED-IN-CODE
`visibleDays, startingWeekday, showHoursAtATime, startWithTodayInWeekView, showBirthdays, showAlternateCalendar, alternateCalendar, showDeclinedEvents?, dateFormat, timeFormat, dateSeparator, timeSeparator, isCalendarListVisible, selectedCollection, defaultCalendar, alarmsByDefault, defaultEventAlarm (measurement), enableTimeZone, lastTimeZoneSet[], hideEvents, hideEventsNum, emailParticipantsSharingChange, promptEventUpdates, viewingMode?`. Read from KVS (`Zl.getPreferences("calendar")`, L82830).

## 11. UI menus & dialogs (strings) — VERIFIED-IN-CODE
- Toolbar "+" menu: New Event / New Calendar / New Subscription… / Refresh Calendar (L67917–67920).
- Event inspector (L68085–68137): Title, "Location or Video Call", all-day, from/to, quick durations 30/60/90/120/150/180 min, sections Invitees, Calendar, Status, All Day, Starts, Ends, Time Zone, Repeat, End, Alert, 2nd Alert, Show as (Free/Busy), Organizer, Attachments, URL, Notes; buttons Cancel/OK/Save/Add/Delete/Accept/Decline/Maybe/Report as Junk. Titles: Add Event / Edit Event / Event Details.
- Repeat picker (L68279–68289): Never, Every Day, Every Week, Every Month, Every Year, Custom...; End: Never / After Number of Times (#{input} Time(s)) / On Date.
- Custom recurrence modal (L68540–68576): Daily/Weekly/Monthly/Yearly, "Every #{input} Week(s) on:", weekday toggles, Monthly Each vs On The, 1st…last, month grid.
- Alert picker: presets in §4 (L67799–67809).
- Invitee row actions: Edit, Send Email..., Remove (L68290–68292).
- Dialogs: Change repeating event? (Only This Event / All Future Events / All Events / Only Selected Events) L67964–67969; Delete repeating event? L67970–67973; Reply to this event or all events? L67974–67975; Send event update to invitees? Save & Send L67960; Delete and Notify L67933–67937; Decline and Notify L67942–67946; Report as Junk L67929–67932; calendar Delete / Stop Sharing / Leave Shared L67927–67959 (can't delete last owned calendar, L67957).
- No event right-click context-menu strings (Duplicate/Copy/Cut/Paste) found — VERIFIED-IN-CODE (grep 0 hits besides share "Copy").

## 12. Pass 2 additions — VERIFIED-IN-CODE unless tagged

- **Service name**: `Ul = Vl.CALENDAR` (L64584), `Vl.CALENDAR = "calendar"` (L64582, siblings `cksharews`, `contacts`, `keyvalue`, `feedbackws`). So requests use the `calendar` key of the webservices map (host typically `pXX-calendarws.icloud.com`, INFERRED from the webservices response, not in this bundle).
- **Common transport params** added by the CloudOS kit, not per-call: `clientBuildNumber`, `clientMasteringNumber`, `clientId` (L65413 `Ad(t, "clientBuildNumber", this.buildNumber), Ad(t, "clientMasteringNumber", r), Ad(t, "clientId", n)`) and `dsid` (L65476 `e.url.setQueryParameter("dsid", t)`).
- **Date tuple semantics resolved**: L73697 `ew = function(e,t,n,r,i){ return [1e4*e + 100*t + n, e, t, n, r, i, 60*r + i] }` → `[yyyymmdd, year, month(1-based), day, hour, minute, minutesSinceMidnight]`. The 7th element is minutes-of-day, **not** duration (duration is the separate `duration` field). Supersedes the INFERRED note in §0.
- **eventdetail response**: `GET ca/eventdetail/<pGuid>/<guid>?lang&usertz[&delegator]` parsed with `zodSchema: Ty, schemaName: "changeSetUpdates"` (L82483–82506) → same `{Event[], Alarm[], Invitee[], Recurrence[], Collection?, Participant?, InboxItem?}` arrays as a ChangeSet `updates` block; then sharedinvitation-marker transformer applied. The guid has `vb()` stripping applied.
- **Time zones / all-day**: `tz` may be an IANA name, `"UTC"`, `"Floating"`, `null` or `""`. Resolution `Sb` L73380 `null === n || "Floating" === n ? r : Mw(n) ? n : <GMT±startDateTZOffset> : r` — floating/null renders in user tz; else falls back to `"GMT"+startDateTZOffset`. Picker offers `OY = ["UTC","Floating"]` (L104255). All-day saves bump `endDate` to next-day 00:00 (L91466–91469); when an event switches from all-day to timed, `tz` is set to the user tz (L91470 `e.payload.allDay && !n.allDay && (i = t)`). Whether the server stores all-day as `"Floating"` — INFERRED (likely, not proven).
- **New-event template** (L91290–91305): `{guid:"", title:"", readOnly:false, etag:"", extendedDetailsAreIncluded:false, shouldShowJunkUIWhenAppropriate:true, tz:"", icon:0, recurrenceException:false, duration:60, allDay:false, startDateTZOffset:"", pGuid:"", hasAttachments:false, birthdayIsYearlessBday:false, alarms:[], invitees:[], recurrence:"", ...}`.
- Notification action enum L74855: `ACCEPT, DECLINE, MAYBE, OK, DECLINE_CALENDAR, JOIN_CALENDAR, JUMP_TO_EVENT, REPORT_AS_JUNK`.

## 13. UI checklist (loc key → English)

Toolbar / view (L67888–67921)
- [ ] `Toolbar.Button.Day/Week/Month/List` — Day / Week / Month / List view switch
- [ ] `Toolbar.Button.Today` — Go to today; `Toolbar.Menu.Previous/Next.Tooltip` — Previous/Next day, week, or month
- [ ] `Toolbar.Button.AddCalendar.Tooltip` — Add Calendar; "+" menu: New Event / New Calendar / New Subscription… / Refresh Calendar
- [ ] Actions menu (`Toolbar.Button.Action.*`): Go to Today, Go to Date…, Preferences…, Show/Hide Declined Events, Delete Event(s), Send Invitations…, Stop Sharing, Calendar Info, Edit/Done, Enable/Disable Local Storage, Help and Support
- [ ] Sidebar: Show calendars / Show inbox / Open actions menu (`Sidebar.Toolbar.Button.*`)

Calendar list (L67990–68010)
- [ ] Sections Subscriptions / Delegates; Birthdays calendar (+ info "Open Contacts", loading popup)
- [ ] Per-calendar action menu: Edit Calendar, Events affect availability, Delete Calendar, Edit/Delete Shared Calendar
- [ ] Inline rename placeholder "New Calendar", untitled → "Untitled", visibility checkbox, color well (palette + "Custom color...")
- [ ] Sharing popup: Private Calendar (Sharing With:, Add Person…, View & Edit / View Only, Invite Again, Remove Person, Stop Sharing), Public Calendar (Link / Email / Copy → Copied), Family Calendar, Leave Calendar
- [ ] Validation: invalid email/phone, already added, can't share with yourself, phone cannot receive invitations

Event inspector / pickers: see §11 (fields, repeat, custom recurrence, alerts, invitee row Edit/Send Email.../Remove).

Dialogs / errors
- [ ] Repeating edit / delete / reply dialogs, Save & Send, Delete and Notify, Decline and Notify, Report as Junk ("Junk invitations are automatically deleted… cannot be undone.") — §11
- [ ] Delete declined event ("…no longer appear when you choose “Show Declined Events”…")
- [ ] Calendar delete / last calendar can't be deleted / leave shared / duplicate name "A calendar with the name “%{title}” already exists."
- [ ] Quota: "Creating a new calendar would exceed the maximum number of allowed calendars."; join error variants
- [ ] Event save failed "Couldn't save changes to “%{title}”."
- [ ] Attachments: too large ("must be less than %{size}MB"), too many ("maximum of %{amount} attachments per event"), upload failed
- [ ] Availability: "No Available Time" (`AvailabilityModal.NoAvailableTime`)

Keyboard
- [ ] Delete key deletes selected event(s) (`Toolbar.Button.Action.Delete.Shortcut` "Del", backspace glyph on Mac, L67890)
- [ ] Escape cancels drag/drop (L107781); Enter/Escape commit/cancel inline edits (L113546–113547)
- No other global shortcut registry found for calendar actions (INFERRED from grep; no `key === "t"`-style handlers).

Empty states: no dedicated "No Events" / empty-list string found in the en table — VERIFIED-IN-CODE by grep (0 hits).

## Gaps
- Exact calendarws host comes from the webservices `calendar` entry at login, not this bundle; auth is cookie-based via `Zl.ckRequest`.
- Server-side tz value for all-day events ("Floating" vs user tz) not proven.
- Recurrence removal wire format and attachment removal are INFERRED (no explicit endpoint).
- `frequencyDays` encoding for multiple positions unverified.
- Inbox / notification-panel strings not enumerated.

## Invitations (VERIFIED-IN-CODE, calendar main.js, 2026-10-19)
- Statuses: `ACCEPTED | TENTATIVE | DECLINED | NEEDS-ACTION` (main.js:72676).
- Accept/Maybe/Decline in the event inspector (main.js:105007) dispatch
  `eventResponseSelectedAndSaved {eventGuid, status, changeRecurring}` (76141). That runs the
  **ordinary event save** (`nj`, the `ca/events` PUT) with `inviteeDetails` where my invitee
  (`isMe`) carries the new `inviteeStatus` (86909–86935). There is no separate reply endpoint.
  On a recurring event, `changeRecurring` is `all`/`future` as for any edit.
- Inbox: `GET ca/notifications` returns `InboxItem[]` (schema main.js:72890):
  `{guid, method: REQUEST|CANCEL|REPLY|RESOURCE_CREATE|RESOURCE_UPDATE|RESOURCE_DELETE|ACCEPTED|DECLINED, objectType: Event|Collection, pGuid, collectionGuid?, lastModifiedEpoch, invitees?, initiatedBy*, acceptedBy*, shareTitle?, shareURL?, title?}`.
  An Event item merges in the event's own fields.
- Dismiss an inbox item: `POST ca/inboxitem/{guid}?methodOverride=DELETE&lang&usertz&requestID&startDate&endDate` with body `{}` (87603).
- Shared-calendar invitation (objectType Collection): `POST ca/collections/{guid}?methodOverride=PUT` with body
  `{guid, status: ACCEPTED|DECLINED, order, shareURL, shareTitle, isJunk?}`. Junk forces DECLINED (83860).
- Inbox text keys: `Notification.Description.EVENT_REQUEST | EVENT_UPDATE | EVENT_CANCEL | EVENT_REPLY_<status>` (118049).

## Calendars: create / rename / colour / delete (VERIFIED-IN-CODE, calendar main.js, 2026-10-19)
- `aP` (83721): `POST ca/collections/{guid}?lang&usertz&requestID&startDate&endDate[&ifMatch=<etag>][&delegator]`
  - **create** (`reason: "added"`): no methodOverride; body `{Collection: <collection>, ClientState: {Collection: [{guid, ctag}], fullState: false}}`
  - **rename / colour / visibility** (`"dirty"`): `methodOverride=PUT`, the same body, `ifMatch` = the collection etag
  - **delete** (`"removed"`): `methodOverride=DELETE`, body `{}`
  - Response schema `saveCollectionBasicResponseDetail`.
- New collection defaults (`yC`, 77328): `{title: "", guid, order: 0, visible: true, color, symbolicColor, shareTitle: "",
  supportedType: "Event", objectType: "personal", extendedDetailsAreIncluded: false, readOnly: false}`.
  - guid is `crypto.randomUUID()` (lowercase) (115638).
  - color is the first palette colour no existing calendar uses (115640).
- Palette (`r2`, 113150): red `#ff2d55`, orange `#ff9500`, yellow `#ffcc00`, green `#65db39`, blue `#34aadc`, purple `#cc73e1`,
  brown `#a2845e`, each with its `symbolicColor` name.
- Title: sanitised and **must be unique**. On a duplicate the UI shows `M0(title)` ("A calendar with the name … already exists") and does not save (115700).
- Order, visibility and default calendar: `POST ca/collectionsetting?methodOverride=PUT` (`oP`, 83777) with `{order, visible, defaultCollectionGuid}`.
