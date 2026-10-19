// The IPC stub and sample data the browser suites share: the shapes the Rust commands
// return, with sample rows so every screen has something real-shaped to draw.
export const SERVICES = ["account","archivews","calendar","ckdatabasews","ckdeviceservice",
                  "cksharews","contacts","contactsweb","developerapi","docws","drivews",
                  "findme","geows","iwmb","iworkexportws","iworkthumbnailws","keyvalue",
                  "mail","mcc","mccgateway","mpp","notes","photos","photosupload",
                  "premiummailsettings","push","reminders","settings","sharedlibrary",
                  "sharedstreams","ubiquity","uploadimagews"];

/* addInitScript SERIALIZES this function — closures do not cross into the page, so every
   value it needs must arrive through the single argument. */
/* Fixture events, so the agenda's rendering is verified without inventing rows in anyone's
   real library. Times are unix seconds relative to now. */
export function installStub({ screen, services, twoFactor, calendarFixture, notesFixture, contactsFixture, driveFixture, remindersFixture }) {
  const HOUR = 3600, DAY = 86400;
  // Today at 9 in the morning, on this machine's clock. Counted from UTC's midnight it was
  // yesterday for the first hours after local midnight in a zone east of UTC — and the
  // week's events fell into last week.
  const base = Math.floor(new Date(new Date().setHours(0, 0, 0, 0)).getTime() / 1000) + 9 * HOUR;
  // Local midnight, n days from today: where an all-day day begins, as the app makes them.
  const localDay = (n) => { const d = new Date(); d.setHours(0, 0, 0, 0); d.setDate(d.getDate() + n); return d.getTime() / 1000; };
  const events = !calendarFixture ? [] : [
    { guid: "e1", calendarGuid: "work", title: "Design review", location: "Studio",
      startsAt: base + HOUR, endsAt: base + 2 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e2", calendarGuid: "home", title: "Dentist", location: null,
      startsAt: base + 5 * HOUR, endsAt: base + 6 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e3", calendarGuid: "work", title: "Sprint planning", location: "Room 2",
      startsAt: base + DAY + 2 * HOUR, endsAt: base + DAY + 3 * HOUR, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    // One day of a weekly series, as Apple names an occurrence: the series' id and its start.
    { guid: "SERIES__20261020T040000Z", calendarGuid: "work", title: "Weekly standup", location: null,
      startsAt: base + 2 * HOUR, endsAt: base + 2 * HOUR + 1800, allDay: false,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "e4", calendarGuid: "personal", title: "Flight to Hyderabad", location: null,
      startsAt: base + 3 * DAY, endsAt: base + 4 * DAY, allDay: true,
      timezone: "Asia/Kolkata", url: null, notes: null, etag: null },
    { guid: "quince:birthdays:c1:2026", calendarGuid: "quince:birthdays", title: "Zoya Ahuja’s Birthday",
      location: null, startsAt: localDay(2), endsAt: localDay(3), allDay: true,
      timezone: null, url: null, notes: null, etag: null },
    { guid: "quince:holidays:diwali:2026", calendarGuid: "quince:holidays", title: "Diwali",
      location: null, startsAt: localDay(5), endsAt: localDay(6), allDay: true,
      timezone: null, url: null, notes: null, etag: null },
  ];
  const dueReminders = !calendarFixture ? [] : [
    { guid: "r1", listGuid: "l1", title: "Water the plants", notes: null,
      dueAt: base + DAY + 8 * HOUR, completedAt: null, priority: 1, etag: null },
    { guid: "r2", listGuid: "l2", title: "Evening run", notes: null,
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, etag: null },
    { guid: "r3", listGuid: "l1", title: "Plan tomorrow", notes: null,
      dueAt: base + 13 * HOUR, completedAt: base + 13 * HOUR, priority: 0, etag: null },
  ];
  const noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 1 },
  ];
  const noteList = !notesFixture ? [] : [
    { guid: "n1", folderGuid: "f1", title: "Curtains",
      snippet: "Bathroom accessories like mirror holder…",
      body: "Curtains\nBathroom accessories like mirror holder, soap holder, shampoo, clothes hanger\nFan\nwall clock\nBed\nDining table\nTea filter\nKnife\nWashing Machine cover",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1788500000, etag: null },
    { guid: "n2", folderGuid: "f1", title: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES",
      snippet: "📌 SECTION A: 2-MARK HIGH-YIELD…",
      body: "🔴 UNIT 1: SOFTWARE TESTING METHODOLOGIES\n\n📌 SECTION A: 2-MARK HIGH-YIELD QUESTIONS\n\nSoftware testing is the process of evaluating a system to find whether it satisfies the specified requirements, and to detect the differences between expected and actual results.",
      locked: false, editable: true, pinned: true, createdAt: null, modifiedAt: 1787900000, etag: null },
    { guid: "n3", folderGuid: "f1",
      title: "These sources outline the fundamental principles and methodologies",
      snippet: "of **Software Testing Methodologies…",
      body: "These sources outline the fundamental principles and methodologies of **Software Testing Methodologies (CSEN3311)**, emphasizing its evolution from simple debugging to a structured, process-oriented discipline. The material details the **Software Testing Life Cycle (STLC)**, covering essential phases from initial planning and test design to execution and post-implementation review.",
      locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1787800000, etag: null },
  ];
  const contactList = !contactsFixture ? [] : [
    { guid: "c1", firstName: "Zoya", lastName: "Ahuja", company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "mobile", value: "+91 90000 00001" },
                { kind: "email", label: "home", value: "zoya@example.com" }] },
    { guid: "c2", firstName: "Amma", lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 98765 43210" }] },
    { guid: "c3", firstName: null, lastName: null, company: "Northfield University",
      isCompany: true, birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: "work", value: "+91 891 284 0000" },
                { kind: "url", label: null, value: "northfield.example" }] },
    { guid: "c4", firstName: null, lastName: null, company: null, isCompany: false,
      birthday: null, notes: null, etag: null,
      methods: [{ kind: "phone", label: null, value: "+91 90000 11111" }] },
  ];
  const driveItems = !driveFixture ? [] : [
    { drivewsid: "FOLDER::a", parentId: null, name: "Downloads", extension: null,
      kind: "FOLDER", size: null, childCount: 3, createdAt: 1785000000,
      modifiedAt: 1788000000, etag: null },
    { drivewsid: "APPLIB::b", parentId: null, name: "Keynote", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "APPLIB::c", parentId: null, name: "Shortcuts", extension: null,
      kind: "APP_LIBRARY", size: null, childCount: null, createdAt: 1780000000,
      modifiedAt: null, etag: null },
    { drivewsid: "FILE::d", parentId: null, name: "Semester Plan", extension: "pdf",
      kind: "FILE", size: 2411724, childCount: null, createdAt: 1787000000,
      modifiedAt: 1788600000, etag: null },
  ];
  const reminderLists = !remindersFixture ? [] : [
    { guid: "l1", title: "Daily", colour: "#63da38", ctag: null, sortOrder: 0, enabled: true },
    { guid: "l2", title: "GYM", colour: "#cc73e1", ctag: null, sortOrder: 1, enabled: true },
    { guid: "l3", title: "Life Goals", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true },
  ];
  const reminderItems = !remindersFixture ? [] : [
    { guid: "r1", listGuid: "l1", title: "Plan tomorrow", notes: null,
      dueAt: base - 2 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r2", listGuid: "l1", title: "Pick up the parcel", notes: null,
      dueAt: base + DAY + 7 * HOUR, completedAt: null, priority: 0, flagged: true, parentGuid: null, etag: null },
    { guid: "r3", listGuid: "l2", title: "Evening run", notes: "Legs today",
      dueAt: base + 6 * HOUR, completedAt: null, priority: 0, flagged: false, parentGuid: null, etag: null },
    { guid: "r4", listGuid: "l1", title: "Water the plants", notes: null,
      dueAt: base - DAY, completedAt: base - DAY, priority: 0, flagged: false, parentGuid: null, etag: null },
  ];
  // Shaped like a real account rather than a tidy one: the default is not the first, and one
  // of them is a subscribed feed that nothing may be written to.
  const calendars = [
    { guid: "work", title: "Work", colour: "#cc73e1", ctag: null, sortOrder: 0, enabled: true,
      writable: true, isDefault: false },
    { guid: "home", title: "Home", colour: "#0088ff", ctag: null, sortOrder: 1, enabled: true,
      writable: true, isDefault: true },
    { guid: "personal", title: "Asha", colour: "#1badf8", ctag: null, sortOrder: 2, enabled: true,
      writable: true, isDefault: false },
    // As the app adds them: drawn from contacts and Apple's holiday feed, read-only.
    { guid: "quince:birthdays", title: "Birthdays", colour: "#8e8e93", ctag: null, sortOrder: 900,
      enabled: true, writable: false, isDefault: false },
    { guid: "quince:holidays", title: "India Holidays", colour: "#34c759", ctag: null, sortOrder: 901,
      enabled: true, writable: false, isDefault: false },
  ];
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    // As Tauri builds it on Linux (tauri/scripts/core.js): the whole path escaped.
    convertFileSrc: (path, protocol = "asset") => `${protocol}://localhost/${encodeURIComponent(path)}`,
    invoke: async (cmd, args) => {
      switch (cmd) {
        case "auth_status":
          return { hasStoredSession: screen === "library", signedIn: screen === "library" };
        case "auth_resume":      return screen === "library" ? "library" : "signin";
        case "auth_sign_in":     return screen === "code" ? "needs_two_factor" : "complete";
        case "auth_submit_code": return null;
        case "account_services": return services;
        case "calendar_events": {
          window.__eventReads = (window.__eventReads ?? 0) + 1;
          // A heavy account, for measuring: `window.__manyEvents = n` adds n events spread over
          // the year around today, several a day.
          const n = window.__manyEvents ?? 0;
          if (!n) return events;
          const extra = (window.__extra ??= Array.from({ length: n }, (_, i) => ({
            guid: `many${i}`, calendarGuid: i % 2 ? "work" : "home", title: `Event ${i}`, location: null,
            startsAt: base + Math.floor((i - n / 2) / 8) * DAY + (8 + (i % 8)) * HOUR, endsAt: base + Math.floor((i - n / 2) / 8) * DAY + (9 + (i % 8)) * HOUR,
            allDay: false, timezone: "Asia/Kolkata", url: null, notes: null, etag: null })));
          return events.concat(extra);
        }
        case "calendar_search": {
          const words = String(args?.query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
          return events.filter((e) => words.every((w) => `${e.title} ${e.location ?? ""}`.toLowerCase().includes(w)))
            .sort((a, b) => a.startsAt - b.startsAt);
        }
        case "calendar_list": return calendars;
        case "event_repeat": return String(args.guid).startsWith("SERIES__") ? { freq: "weekly", interval: 1, count: null } : null;
        case "event_alerts": return args.guid === "e2" ? [15] : [];
        case "event_save": (window.__eventSaves ??= []).push(args.draft); return "E-new";
        case "event_delete": (window.__eventDeletes ??= []).push(args); return null;
        case "calendar_sync": return { calendars: calendars.length, events: events.length };
        case "reminder_counts": return { today: 4, scheduled: 7, flagged: 2, all: 12, lists: [["L1", 5], ["L2", 3], ["lists", 4]] };
        case "reminder_lists": return !remindersFixture ? [] : reminderLists;
        case "reminders_in": {
          if (!remindersFixture || remindersFixture === "lists") return [];
          // Opt-in for measuring a switch: each list's own reminders, after a real read's wait.
          if (window.__slowItems) await new Promise((r) => setTimeout(r, window.__slowItems));
          return window.__filterLists ? reminderItems.filter((r) => r.listGuid === args.listGuid) : reminderItems;
        }
        case "reminders_due_between": return calendarFixture ? dueReminders : [];
        case "reminder_create":
          // Kept, as Apple keeps it, with what the view implied.
          reminderItems.push({ guid: `made-${reminderItems.length}`, listGuid: args.listGuid, title: args.title,
            notes: null, dueAt: args.dueAt ?? null, allDay: args.allDay ?? false, completedAt: null, priority: 0,
            flagged: args.flagged ?? false, parentGuid: null, etag: null });
          return null;
        case "reminder_delete": return null;
        case "reminder_set_done": return null;
        case "reminder_set_repeat": return null;
        case "reminder_edit": {
          (window.__edits ??= []).push(args.edit);
          // Kept, as Apple keeps it, so the read after a write shows the write.
          const r = reminderItems.find((x) => x.guid === args.guid);
          if (r) for (const [k, v] of Object.entries(args.edit)) if (k !== "listGuid") r[k] = v;
          return null;
        }
        case "reminder_reorder": {
          (window.__reorders ??= []).push(args);
          const i = reminderItems.findIndex((x) => x.guid === args.guid);
          const [moved] = reminderItems.splice(i, 1);
          const at = reminderItems.findIndex((x) => x.guid === args.before);
          reminderItems.splice(at < 0 ? reminderItems.length : at, 0, moved);
          return null;
        }
        case "reminder_list_colour": {
          (window.__listColours ??= []).push([args.listGuid, args.colour]);
          const hex = { purple: "#C077DB", red: "#FF3B30", blue: "#007AFF" }[args.colour];
          const list = reminderLists.find((l) => l.guid === args.listGuid);
          if (list && hex) list.colour = hex;
          return null;
        }
        case "reminder_list_create": case "reminder_list_rename": case "reminder_list_delete":
          return null;
        case "reminders_smart": return !remindersFixture || remindersFixture === "lists" ? [] : reminderItems.filter(r => !r.completedAt);
        case "note_paragraph": (window.__paragraphs ??= []).push(args); return null;
        case "note_runs": {
          if (window.__savedRuns?.[args.guid]) return window.__savedRuns[args.guid];
          if (args.guid === "nM") {
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            return [plain(5), { ...plain(1), attachment: "M1" }, plain(1), { ...plain(1), attachment: "M2" }, plain(4)];
          }
          if (args.guid === "nT") {
            // "Groceries", a table on its own line, then a last line: the table's run names it.
            const plain = (length) => ({ length, style: null, indent: 0, checked: null, bold: false, italic: false, underline: false, strike: false, link: null });
            // "end" coloured mint on a phone (field 14).
            return [plain(10), { ...plain(1), attachment: "T1" }, plain(1), { ...plain(3), emphasis: 4 }];
          }
          // Curtains, set as a phone sets one: a title, then a dashed list and a ticked item.
          if (args.guid !== "n1") return [];
          const body = (noteList.find((n) => n.guid === "n1") ?? {}).body ?? "";
          const lines = body.split("\n");
          const runs = lines.map((l, i) => ({ length: l.length + (i < lines.length - 1 ? 1 : 0),
            style: i === 0 ? 0 : i === 1 ? 103 : 101, indent: 0, checked: i === 1 ? true : null,
            bold: false, italic: false, underline: false, strike: false, link: null }));
          return runs;
        }
        case "note_folders": return !notesFixture ? [] : (window.__noteFolders ??= noteFolders.slice()).slice();
        case "note_folder_create": {
          const guid = `f-${(window.__noteFolders ??= noteFolders.slice()).length + 1}`;
          window.__noteFolders.push({ guid, title: args.title, sortOrder: 0, parentGuid: args.parent ?? null });
          (window.__folderCalls ??= []).push(["create", args.title]);
          return guid;
        }
        case "note_folder_rename":
          window.__noteFolders = (window.__noteFolders ?? noteFolders.slice()).map((f) => (f.guid === args.guid ? { ...f, title: args.title } : f));
          (window.__folderCalls ??= []).push(["rename", args.guid, args.title]);
          return null;
        case "note_folder_delete":
          window.__noteFolders = (window.__noteFolders ?? noteFolders.slice()).filter((f) => f.guid !== args.guid);
          (window.__folderCalls ??= []).push(["delete", args.guid]);
          return null;
        case "note_save": {
          (window.__noteSaves ??= []).push(args);
          // A test can play the other device: what Apple answers is what it now holds.
          const kept = window.__phoneEdit ? window.__phoneEdit(args) : args.body;
          // Kept, as Apple keeps it: the next read of the note gives what was saved.
          const n = noteList.find((x) => x.guid === args.guid);
          if (n) n.body = kept;
          if (args.runs && kept === args.body) (window.__savedRuns ??= {})[args.guid] = args.runs;
          return kept;
        }
        case "note_delete": return null;
        case "note_move":
          if (window.__slowMove) await new Promise((r) => setTimeout(r, window.__slowMove));
          return null;
        case "note_pin": {
          const n = noteList.find((x) => x.guid === args?.guid);
          if (n) n.pinned = args.pinned;
          (window.__pins ??= []).push([args?.guid, args?.pinned]);
          return null;
        }
        case "note_create": return "Note/new";
        case "notes_in":
          // Opt-in: a note holding a table, as a phone makes one.
          if (window.__mediaNote && !noteList.some((n) => n.guid === "nM"))
            noteList.unshift({ guid: "nM", folderGuid: "f1", title: "Trip", snippet: "", body: "Trip\n\uFFFC\n\uFFFC\nend",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789100000, etag: null });
          if (window.__tableNote && !noteList.some((n) => n.guid === "nT"))
            noteList.unshift({ guid: "nT", folderGuid: "f1", title: "Groceries", snippet: "", body: "Groceries\n\uFFFC\nend",
              locked: false, editable: true, pinned: false, createdAt: null, modifiedAt: 1789000000, etag: null });
          return !notesFixture ? [] : noteList.map((n) => ({ ...n }));
        case "note_attachment_info":
          return args.id === "M1"
            ? { id: "M1", uti: "public.jpeg", kind: "image", filename: "beach.jpeg", title: null, width: 1200, height: 800, size: 271872, hasPreview: true }
            : { id: "M2", uti: "org.openxmlformats.spreadsheetml.sheet", kind: "file", filename: "Budget.xlsx", title: null, width: null, height: null, size: 9100, hasPreview: false };
        case "note_attachment_open": (window.__opened ??= []).push(args.id); return null;
        case "note_table": if (String(args.id).startsWith("M")) return null; return (window.__table ??= [["Item", "Qty"], ["Milk", "2"], ["", ""]]).map((r) => r.slice());
        case "note_table_set_cell": {
          (window.__cellWrites ??= []).push(args);
          await new Promise((r) => setTimeout(r, 300));
          if (window.__cellFail) throw "Apple refused that change.";
          window.__table[args.row][args.column] = args.text;
          return window.__table.map((r) => r.slice());
        }
        case "note_body": return (noteList.find((n) => n.guid === args?.guid) ?? {}).body ?? null;
        case "note_counts": {
          const per = new Map();
          for (const n of noteList) per.set(n.folderGuid, (per.get(n.folderGuid) ?? 0) + 1);
          return [noteList.length, [...per.entries()]];
        }
        case "contact_save": (window.__contactSaves ??= []).push(args); return "C-new";
        case "contact_delete": (window.__contactDeletes ??= []).push(args); return null;
        case "contacts_list": return !contactsFixture ? [] : contactList;
        case "contact_me": return window.__me ?? (contactsFixture ? "c1" : null);
        case "contact_set_me": await new Promise((r) => setTimeout(r, 600)); window.__me = args.guid; return null;
        case "contacts_export": (window.__exports ??= []).push(args); return `/home/test/Downloads/${args.fileName}`;
        case "contacts_import": (window.__imports ??= []).push(args); return [2, 0];
        case "plugin:dialog|open": return window.__pick ?? null;
        case "contact_lists": return !contactsFixture ? [] : (window.__lists ??= [
          { guid: "L1", name: "Family", members: ["c2"], etag: "e1" },
        ]).map((l) => ({ ...l, members: [...l.members] }));
        case "contact_list_write": {
          // Slow on purpose, as Apple is: the screen must not wait for this to redraw.
          (window.__listWrites ??= []).push(args);
          await new Promise((r) => setTimeout(r, 700));
          if (window.__listFail) throw "Contacts refused the list.";
          const lists = (window.__lists ??= [{ guid: "L1", name: "Family", members: ["c2"], etag: "e1" }]);
          if (!args.guid) {
            const guid = `L${lists.length + 1}`;
            lists.push({ guid, name: args.name ?? "Untitled List", members: args.add ?? [], etag: "e" });
            return guid;
          }
          const l = lists.find((x) => x.guid === args.guid);
          if (args.delete) { window.__lists = lists.filter((x) => x.guid !== args.guid); return args.guid; }
          if (args.name) l.name = args.name;
          l.members = l.members.filter((m) => !(args.remove ?? []).includes(m));
          for (const m of args.add ?? []) if (!l.members.includes(m)) l.members.push(m);
          return l.guid;
        }
        case "drive_thumbnails": return driveFixture ? { "FILE::d": "data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='80' height='60'><rect width='80' height='60' fill='%23eee'/></svg>" } : {};
        case "drive_duplicate": (window.__dups ??= []).push(args.drivewsid); return null;
        case "storage_info": return { usedBytes: 182196857, totalBytes: 5368709120 };
        case "diagnostics_save": (window.__diagnostics = (window.__diagnostics ?? 0) + 1); return "/home/user/Downloads/Quince diagnostics 2026-10-18 20.00.txt";
        case "account_info": return { name: "Asha Rao", email: "asha@example.com", photoContact: window.__mePhoto ?? null, webAccessAllowed: window.__webAccess ?? true, dataProtection: false };
        case "settings_get": return { reminderAlerts: true, eventAlerts: true, ...(window.__settings ?? {}) };
        case "settings_set":
          if (window.__settingsFail) throw "Couldn't save that setting.";
          (window.__settings ??= {})[args.key] = args.on;
          return null;
        case "drive_children": return !driveFixture ? [] : driveItems;
        case "drive_recents": {
          if (!driveFixture) return [];
          const now = Math.floor(Date.now() / 1000);
          const f = (id, name, ext, opened) => ({ drivewsid: `FILE::r${id}`, parentId: "FOLDER::x", name, extension: ext, kind: "FILE",
            size: 34145, childCount: null, createdAt: now - 9e6, modifiedAt: now - 9e6, etag: "e", openedAt: opened });
          return [f(1, "Invoice-0001", "pdf", now - 600), f(2, "Boarding pass", "pdf", now - 86400 - 3600), f(3, "Lease", "pdf", now - 40 * 86400)];
        }
        case "drive_trash": return !driveFixture ? [] : (window.__trash ??= [
          { drivewsid: "FILE::t1", parentId: null, name: "Old notes", extension: "pdf", kind: "FILE",
            size: 177000, childCount: null, createdAt: 1786000000, modifiedAt: 1786000000, etag: "t1",
            expiresAt: Math.floor(Date.now() / 1000) + 21 * 86400 - 60, restorePath: "Downloads/Old notes.pdf" },
          { drivewsid: "FILE::t2", parentId: null, name: "Wallpaper", extension: "jpg", kind: "FILE",
            size: 1500000, childCount: null, createdAt: 1786000000, modifiedAt: 1786000000, etag: "t2",
            expiresAt: Math.floor(Date.now() / 1000) + 2 * 86400 - 60, restorePath: "Wallpaper.jpg" },
        ]).slice();
        case "drive_trash_out": {
          // One request for all of them, as Recover All / Delete All send it.
          (window.__trashBatches ??= []).push({ recover: args.recover, ids: args.items.map((i) => i.drivewsid) });
          const ids = new Set(args.items.map((i) => i.drivewsid));
          window.__trash = (window.__trash ?? []).filter((i) => !ids.has(i.drivewsid));
          for (const i of args.items) (window.__trashCalls ??= []).push([args.recover ? "drive_put_back" : "drive_delete_forever", i.drivewsid]);
          return [];
        }
        case "drive_put_back": case "drive_delete_forever":
          window.__trash = (window.__trash ?? []).filter((i) => i.drivewsid !== args.item.drivewsid);
          (window.__trashCalls ??= []).push([cmd, args.item.drivewsid]);
          return null;
        case "drive_sync_folder":
          // A folder deleted on another device: the backend forgets it and says so.
          if (args.folderId === window.__driveGone) throw "This folder was deleted or moved on another device.";
          return driveItems.length;
        case "drive_download": return "/home/user/Downloads/file.pdf";
        case "drive_move": (window.__moves ??= []).push(args); await new Promise((r) => setTimeout(r, 600)); return null;
        case "drive_create_folder": case "drive_rename": case "drive_delete": return null;
        case "drive_item": return null;
        case "auth_two_factor_options": return twoFactor;
        case "auth_resend_code":    return null;
        case "auth_send_sms":       return null;
        case "auth_submit_sms_code": return null;
        case "auth_sign_out":    return null;
        // One refresh covers every service, so there is one command to stub rather than
        // a pair per screen.
        case "sync_all":
        case "sync_status":
          return {
            services: [
              { service: "calendar", label: "Calendar", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 120 },
              { service: "reminders", label: "Reminders", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 60 },
              { service: "notes", label: "Notes", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 90 },
              { service: "contacts", label: "Contacts", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 300 },
            ],
            asOf: Math.floor(Date.now() / 1000) - 300,
            tookMs: 40,
          };
        // The event plugin is how a finished refresh reaches the screens. Nothing emits in
        // a plain browser, so listening is a no-op that hands back an unsubscribe id.
        case "plugin:event|listen": {
          // Kept, so a test can play a finished refresh in: `window.__emit(event, payload)`.
          const all = (window.__listeners ??= {});
          (all[args.event] ??= []).push(args.handler);
          window.__emit ??= (event, payload) => (window.__listeners[event] ?? []).forEach((h) => typeof h === "function" && h({ event, payload, id: 0 }));
          return 0;
        }
        case "plugin:event|unlisten":
          return null;
        default: throw new Error(`unstubbed command: ${cmd}`);
      }
    },
  };
}

/**
 * Open a screen from the sidebar the way a person would — in a narrow window the sidebar is
 * put away, so it is brought back first.
 */
export async function openScreen(page, name) {
  const toggle = page.getByRole("button", { name: "Show Sidebar" });
  if (await toggle.isVisible()) {
    await toggle.click();
    await page.waitForTimeout(320);
  }
  await page.locator(".nav-item", { hasText: name }).click();
  // Each app's code arrives on its own; the screen is open once it is drawn.
  const root = { Calendar: ".cx", Notes: ".notes", Reminders: ".rx", Contacts: ".contacts", "iCloud Drive": ".drive" }[name];
  if (root) await page.locator(root).first().waitFor({ state: "attached", timeout: 10000 });
}
