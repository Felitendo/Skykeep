// The README's demo library: an invented person's notes, reminders, week, contacts, files and
// photos, so the screenshots show a lived-in app without anyone's real data. Every name, place
// and address here is made up; phone numbers use the 555-01xx range kept for fiction, emails
// use example.com, and every picture is drawn by .uiverify/readme-shots.mjs.
//
// Used only when a suite passes `demoFixture: true` to installStub (stub.mjs) and adds this
// script first: `ctx.addInitScript(installDemo, { picBase, tz })`. Nothing else reads it.
//
// addInitScript SERIALIZES this function: everything it needs is inside it or in its argument.
export function installDemo({ picBase, tz }) {
  const HOUR = 3600, DAY = 86400, MIN = 60;
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const at = (days, h = 0, m = 0) => { const d = new Date(today); d.setDate(d.getDate() + days); d.setHours(h, m, 0, 0); return d.getTime() / 1000; };
  const now = Math.floor(Date.now() / 1000);
  window.__picBase = picBase;

  // ---- Calendar: a busy week, four calendars, a birthday and reminders on the schedule.
  const calendars = [
    { guid: "work", title: "Work", colour: "#0088ff", ctag: null, sortOrder: 0, enabled: true, writable: true, isDefault: true },
    { guid: "home", title: "Home", colour: "#34c759", ctag: null, sortOrder: 1, enabled: true, writable: true, isDefault: false },
    { guid: "family", title: "Family", colour: "#ff9500", ctag: null, sortOrder: 2, enabled: true, writable: true, isDefault: false },
    { guid: "fitness", title: "Fitness", colour: "#af52de", ctag: null, sortOrder: 3, enabled: true, writable: true, isDefault: false },
    { guid: "quince:birthdays", title: "Birthdays", colour: "#8e8e93", ctag: null, sortOrder: 900, enabled: true, writable: false, isDefault: false },
  ];
  let n = 0;
  const ev = (cal, title, day, h, m, minutes, location = null, extra = {}) => ({
    guid: extra.guid ?? `d${++n}`, calendarGuid: cal, title, location, startsAt: at(day, h, m), endsAt: at(day, h, m) + minutes * MIN,
    allDay: false, timezone: tz, url: null, notes: null, etag: null });
  const allDay = (cal, title, day, days = 1, guid = `a${++n}`) => ({ guid, calendarGuid: cal, title, location: null,
    startsAt: at(day), endsAt: at(day + days), allDay: true, timezone: null, url: null, notes: null, etag: null });
  const standups = [-2, -1, 0, 1, 2].map((d) => ev("work", "Team standup", d, 9, 0, 30, null, { guid: `SERIES__standup${d + 2}` }));
  const events = [
    ...standups,
    ev("work", "Design critique", -2, 11, 0, 60, "Studio B"),
    ev("fitness", "Yoga", -2, 18, 0, 60, "Northside Studio"),
    ev("work", "Roadmap planning", -1, 10, 0, 90, "Room 4"),
    ev("home", "Lunch with Sam", -1, 12, 30, 60, "Corner Café"),
    ev("home", "Dentist", -1, 16, 0, 60),
    ev("work", "Product review", 0, 10, 30, 60, "Room 4"),
    ev("family", "Lunch with Mum", 0, 12, 30, 60),
    ev("work", "1:1 with Jordan", 0, 15, 0, 45),
    ev("fitness", "Climbing", 0, 17, 30, 90, "Boulder Hall"),
    ev("work", "Customer interviews", 1, 11, 0, 90),
    ev("home", "Bike service", 1, 14, 0, 60, "Spoke & Chain"),
    ev("family", "Movie night", 1, 19, 0, 120),
    ev("work", "Sprint demo", 2, 10, 0, 60),
    ev("work", "Focus time", 2, 13, 0, 120),
    ev("family", "Swim lessons", 3, 10, 0, 60, "Leisure Centre"),
    ev("family", "Farmers market", 3, 11, 30, 90),
    ev("family", "Brunch with the Okafors", -3, 11, 0, 90),
    allDay("family", "Cabin weekend", 2, 3),
    allDay("quince:birthdays", "Maya Chen’s Birthday", 1, 1, "quince:birthdays:c3:2026"),
  ];
  const dueReminders = [
    { guid: "dr1", listGuid: "trip", title: "Book the train tickets", notes: null, dueAt: at(0, 14, 0), completedAt: null, priority: 0, etag: null },
    { guid: "dr2", listGuid: "errands", title: "Pick up the dry cleaning", notes: null, dueAt: at(1, 17, 0), completedAt: null, priority: 0, etag: null },
  ];

  // ---- Reminders: Today, colourful lists, tags, sections and a subtask or two.
  const reminderLists = [
    { guid: "home", title: "Home", colour: "#007aff", ctag: null, sortOrder: 0, enabled: true, emblem: "house" },
    { guid: "errands", title: "Errands", colour: "#ff9500", ctag: null, sortOrder: 1, enabled: true, emblem: "cart" },
    { guid: "work", title: "Work", colour: "#af52de", ctag: null, sortOrder: 2, enabled: true, emblem: "briefcase" },
    { guid: "trip", title: "Cabin Weekend", colour: "#34c759", ctag: null, sortOrder: 3, enabled: true, emblem: "tent" },
    { guid: "groceries", title: "Groceries", colour: "#ff3b30", ctag: null, sortOrder: 4, enabled: true, emblem: "basket" },
  ];
  const R = (guid, listGuid, title, dueAt, extra = {}) => ({ guid, listGuid, title, notes: null, dueAt, completedAt: null,
    priority: 0, flagged: false, parentGuid: null, recurFrequency: null, recurInterval: null, sectionGuid: null, allDay: false, etag: null, ...extra });
  const reminderItems = [
    R("r1", "home", "Call the plumber about the sink", at(0, 8, 30), { priority: 1 }),
    R("r2", "trip", "Book the train tickets", at(0, 10, 0), { flagged: true, notes: "Window seats if we can", sectionGuid: "s-before" }),
    R("r2a", "trip", "Compare the 4:10 and 5:40 trains", null, { parentGuid: "r2", sectionGuid: "s-before" }),
    R("r3", "work", "Send the Q4 slides to Jordan", at(0, 11, 30), { priority: 1 }),
    R("r4", "errands", "Return the library books", at(0), { allDay: true }),
    R("r5", "home", "Water the herbs", at(0, 18, 0), { recurFrequency: 0, recurInterval: 1 }),
    R("r6", "trip", "Pack the camping gear", at(0, 20, 0), { sectionGuid: "s-pack", flagged: true }),
    R("r6a", "trip", "Headlamps and spare batteries", null, { parentGuid: "r6", sectionGuid: "s-pack" }),
    R("r7", "errands", "Pick up the dry cleaning", at(1, 17, 0)),
    R("r8", "groceries", "Oat milk", null), R("r9", "groceries", "Basil", null), R("r10", "groceries", "Lemons", null),
    R("r11", "work", "Review the onboarding draft", at(2, 10, 0)),
    R("r12", "home", "Renew car insurance", at(9, 9, 0), { flagged: true }),
    R("r13", "trip", "Print the trail map", at(1, 19, 0), { sectionGuid: "s-before" }),
  ];
  window.__tagMap = { r2: ["trip"], r3: ["work"], r4: ["errands"], r5: ["home"], r6: ["trip", "weekend"], r7: ["errands"], r13: ["trip"] };
  window.__sections = [
    { guid: "s-before", listGuid: "trip", title: "Before we go", createdAt: null },
    { guid: "s-pack", listGuid: "trip", title: "Packing", createdAt: null },
  ];
  window.__smartLists = [
    { guid: "SL-today", kind: "today", title: null, colour: null, emblem: null, pinned: true, sorting: null, filter: null },
    { guid: "SL-1", kind: "custom", title: "This Weekend", colour: "#5856d6", emblem: null, pinned: false, sorting: null, filter: '{"hashtags":["weekend"]}' },
  ];
  window.__details = { r2: { url: null, earlyAlerts: [{ id: "a1", count: 30, unit: 0 }] } };

  // ---- Notes: folders, pinned notes, and one note with a checklist, a table and a picture.
  const noteFolders = [
    { guid: "f1", title: "Notes", sortOrder: 0 },
    { guid: "f3", title: "Travel", sortOrder: 1 },
    { guid: "f4", title: "Recipes", sortOrder: 2 },
    { guid: "f5", title: "Work", sortOrder: 3 },
    { guid: "f2", title: "Recently Deleted", sortOrder: 9 },
  ];
  const cabin = [
    ["Cabin weekend", 0],
    ["Friday to Sunday at Fern Hollow. Leave by four to miss the traffic.", null],
    ["\uFFFC", null, null, "M1"],
    ["Packing", 1],
    ["Hiking boots", 103, true], ["Rain jackets", 103, true], ["Camera and a spare battery", 103, true],
    ["Board games", 103, false], ["Coffee beans and the grinder", 103, false],
    ["Who brings what", 1],
    ["\uFFFC", null, null, "T1"],
    ["", null],
  ];
  const notes = {
    nC: { lines: cabin, folder: "f3", pinned: true, modified: now - 25 * MIN },
    nS: { lines: [["Sourdough starter", 0], ["Feed it every morning: 50 g flour, 50 g water.", null], ["Bake when it doubles in four hours", 103, false], ["Rye works too, it just smells sharper", 101]], folder: "f4", pinned: true, modified: now - 2 * DAY },
    nB: { lines: [["Book club picks", 0], ["November: The Overstory. December: Piranesi.", null], ["Ask Lena to host", 103, false]], folder: "f1", modified: now - 3 * HOUR },
    nQ: { lines: [["Q4 planning", 0], ["Ship the new onboarding by mid-November.", null], ["Hire a second designer", 103, false]], folder: "f5", modified: now - 26 * HOUR },
    nG: { lines: [["Gift ideas", 0], ["Sam: a good pour-over kettle", 101], ["Mum: the blue scarf from the market", 101]], folder: "f1", modified: now - 3 * DAY },
    nL: { lines: [["Lisbon in spring", 0], ["Tram 28 early, before the crowds. Custard tarts in Belém.", null]], folder: "f3", modified: now - 5 * DAY },
    nM: { lines: [["Design sync", 0], ["Tighter spacing in the sidebar, and calmer empty states.", null]], folder: "f5", modified: now - 8 * DAY },
    nK: { lines: [["Bike maintenance", 0], ["Chain lube every 300 km. Brake pads at 2 mm.", null]], folder: "f1", modified: now - 12 * DAY },
    nR: { lines: [["Tomato soup", 0], ["Roast the tomatoes first, with garlic and thyme.", null]], folder: "f4", modified: now - 20 * DAY },
  };
  const toNote = (guid, n) => {
    const text = n.lines.map((l) => l[0]).join("\n");
    const rest = n.lines.slice(1).map((l) => l[0]).filter((t) => t !== "￼").join(" ");
    return { guid, folderGuid: n.folder, title: n.lines[0][0], snippet: rest, body: text, locked: false, editable: true,
      pinned: !!n.pinned, createdAt: n.modified - 30 * DAY, modifiedAt: n.modified, etag: null };
  };
  const noteList = Object.entries(notes).map(([g, n]) => toNote(g, n));
  // The note that arrives from "the phone" in the hero animation (readme-shots.mjs adds it).
  const arriving = { lines: [["Farmers market", 0], ["Saturday morning, before swim lessons", null], ["Sourdough loaf", 103, false], ["Honey", 103, false], ["Apples for the pie", 103, false], ["Flowers for Maya", 103, false]], folder: "f1", modified: now };
  window.__demoArrive = () => { const list = window.__demo.noteList; if (!list.some((x) => x.guid === "nF")) { notes.nF = arriving; list.unshift(toNote("nF", arriving)); } };
  window.__table = [["Who", "Brings"], ["Sam", "Firewood and the speaker"], ["Alex", "Breakfast for Saturday"], ["Maya", "Board games and snacks"]];
  const runsOf = (n) => n.lines.map((l, i) => ({ length: [...l[0]].length + (i < n.lines.length - 1 ? 1 : 0), style: l[1], indent: 0,
    checked: l[1] === 103 ? !!l[2] : null, bold: false, italic: false, underline: false, strike: false, link: null, ...(l[3] ? { attachment: l[3] } : {}) }));

  // ---- Contacts.
  const C = (guid, firstName, lastName, methods = [], extra = {}) => ({ guid, firstName, middleName: null, lastName, photoUrl: null, photoSignature: null,
    company: null, isCompany: false, birthday: null, notes: null, etag: null, methods, ...extra });
  const contactList = [
    C("c1", "Alex", "Brennan", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-2231" }]),
    C("c2", "Priya", "Desai", [{ kind: "email", label: "home", value: "priya.desai@example.com" }]),
    C("c3", "Maya", "Chen", [
      { kind: "phone", label: "mobile", value: "+1 (555) 010-4417" },
      { kind: "phone", label: "work", value: "+1 (555) 010-9020" },
      { kind: "email", label: "home", value: "maya.chen@example.com" },
      { kind: "email", label: "work", value: "maya@studio-north.example" },
      { kind: "url", label: "homepage", value: "mayachen.example" },
    ], { company: "Studio North", birthday: "1991-10-22", notes: "Met at the climbing gym. Loves the sourdough." }),
    C("c4", "Jordan", "Ellis", [{ kind: "phone", label: "work", value: "+1 (555) 010-7788" }], { company: "Studio North" }),
    C("c5", "Noah", "Fischer", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-3304" }]),
    C("c6", "Sam", "Garcia", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-6612" }]),
    C("c7", "Lena", "Hart", [{ kind: "email", label: "home", value: "lena.hart@example.com" }]),
    C("c8", "Theo", "Ivanov", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-1180" }]),
    C("c9", "Ada", "Kim", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-5521" }]),
    C("c10", "Ruth", "Lindqvist", [{ kind: "phone", label: "home", value: "+1 (555) 010-0045" }]),
    C("c11", "Ben", "Morales", [{ kind: "email", label: "home", value: "ben.morales@example.com" }]),
    C("c12", "Chidi", "Okafor", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-8890" }]),
    C("c13", "Jamie", "Rivera", [{ kind: "email", label: "home", value: "jamie.rivera@example.com" }]),
    C("c14", "Hana", "Sato", [{ kind: "phone", label: "mobile", value: "+1 (555) 010-2675" }]),
    C("c15", null, null, [{ kind: "phone", label: "work", value: "+1 (555) 010-0100" }], { company: "Spoke & Chain Bikes", isCompany: true }),
  ];
  window.__lists = [
    { guid: "L1", name: "Family", members: ["c10", "c13"], etag: "e1" },
    { guid: "L2", name: "Climbing club", members: ["c3", "c6", "c9"], etag: "e2" },
    { guid: "L3", name: "Studio North", members: ["c3", "c4"], etag: "e3" },
  ];
  window.__me = "c13";
  window.__addresses = { c3: [{ label: "HOME", street: "48 Willow Lane", subLocality: null, city: "Brookfield", subAdministrativeArea: null,
    state: "OR", postalCode: "97000", country: "United States", countryCode: "us" }] };

  // ---- Drive: a tidy root and a folder of trip files with thumbnails.
  const F = (id, parentId, name, extension, kind, size, modified, childCount = null) => ({ drivewsid: id, parentId, name, extension, kind, size,
    childCount, createdAt: modified - 40 * DAY, modifiedAt: modified, etag: "e" });
  const drive = {
    root: [
      F("FOLDER::cabin", null, "Cabin Weekend", null, "FOLDER", null, now - 2 * HOUR, 12),
      F("FOLDER::docs", null, "Documents", null, "FOLDER", null, now - 3 * DAY, 24),
      F("FOLDER::desk", null, "Desktop", null, "FOLDER", null, now - DAY, 6),
      F("FOLDER::recipes", null, "Recipes", null, "FOLDER", null, now - 9 * DAY, 12),
      F("APPLIB::pages", null, "Pages", null, "APP_LIBRARY", null, null),
      F("APPLIB::numbers", null, "Numbers", null, "APP_LIBRARY", null, null),
      F("APPLIB::keynote", null, "Keynote", null, "APP_LIBRARY", null, null),
      F("FILE::budget", null, "Household budget", "numbers", "FILE", 412300, now - 4 * DAY),
      F("FILE::resume", null, "Portfolio", "pdf", "FILE", 2840000, now - 15 * DAY),
    ],
    "FOLDER::cabin": [
      F("FILE::lake", "FOLDER::cabin", "Lake at dawn", "jpg", "FILE", 3412000, now - 2 * HOUR),
      F("FILE::ridge", "FOLDER::cabin", "Ridge trail", "jpg", "FILE", 2980000, now - 3 * HOUR),
      F("FILE::fire", "FOLDER::cabin", "Campfire", "jpg", "FILE", 2210000, now - 5 * HOUR),
      F("FILE::dunes", "FOLDER::cabin", "Sunset over the valley", "jpg", "FILE", 3120000, now - 6 * HOUR),
      F("FILE::map", "FOLDER::cabin", "Trail map", "pdf", "FILE", 1840000, now - DAY),
      F("FILE::plan", "FOLDER::cabin", "Weekend plan", "pdf", "FILE", 220000, now - DAY),
      F("FILE::costs", "FOLDER::cabin", "Shared costs", "numbers", "FILE", 98000, now - 2 * DAY),
      F("FILE::play", "FOLDER::cabin", "Road trip songs", "txt", "FILE", 2100, now - 3 * DAY),
      F("FILE::stars", "FOLDER::cabin", "Stars over the lake", "jpg", "FILE", 2650000, now - 7 * HOUR),
      F("FILE::mist", "FOLDER::cabin", "Morning mist", "jpg", "FILE", 2870000, now - 8 * HOUR),
      F("FILE::shore", "FOLDER::cabin", "The shore", "jpg", "FILE", 3010000, now - 9 * HOUR),
      F("FOLDER::receipts", "FOLDER::cabin", "Receipts", null, "FOLDER", null, now - 2 * DAY, 4),
    ],
  };
  const driveById = new Map(Object.values(drive).flat().map((i) => [i.drivewsid, i]));
  const thumbs = Object.fromEntries(["lake", "ridge", "fire", "dunes", "map", "plan", "stars", "mist", "shore"].map((k) => [`FILE::${k}`, `${picBase}/drive/${k}`]));

  // ---- Photos: a library over three months, a video, a Live Photo, favourites and albums.
  const shapes = [[4032, 3024], [3024, 4032], [4032, 3024], [4032, 2268], [4032, 3024], [3024, 4032]];
  window.__photos = Array.from({ length: 54 }, (_, i) => {
    const [width, height] = shapes[i % shapes.length];
    const takenAt = now - Math.floor(i * 1.7 * DAY) - (i % 5) * 3 * HOUR - HOUR;
    return { id: `g${i}`, master: `mg${i}`, takenAt, addedAt: takenAt, kind: i === 4 ? "video" : i === 9 ? "live" : "image",
      width, height, duration: i === 4 ? 14.2 : i === 9 ? 2.8 : null, filename: i === 4 ? `IMG_${4100 + i}.MOV` : `IMG_${4100 + i}.HEIC`,
      size: 2400000, uti: i === 4 ? "com.apple.quicktime-movie" : "public.heic", favorite: i % 7 === 2, hidden: false, deleted: false,
      deletedAt: null, subtype: 0, changeTag: null };
  });
  window.__albums = [
    { id: "AL1", name: "Cabin Weekend", position: 0, changeTag: null, items: ["g0", "g1", "g2", "g3"] },
    { id: "AL2", name: "Sunsets", position: 1, changeTag: null, items: ["g5", "g11", "g17"] },
    { id: "AL3", name: "Garden", position: 2, changeTag: null, items: ["g7", "g8"] },
  ];

  // ---- Settings and freshness.
  const services = ["notes", "reminders", "calendar", "contacts", "drive", "photos"];
  const labels = { notes: "Notes", reminders: "Reminders", calendar: "Calendar", contacts: "Contacts", drive: "Drive", photos: "Photos" };
  const report = () => ({ services: services.map((s, i) => ({ service: s, label: labels[s], outcome: "ok", detail: null,
    lastSyncedAt: Math.floor(Date.now() / 1000) - 20 - i * 9 })), asOf: Math.floor(Date.now() / 1000) - 20, tookMs: 40 });
  window.__local = { library: 48_200_000, photos: 1_912_000_000, notes: 86_000_000, drive: 640_000_000, other: 7_400_000, total: 2_693_600_000, clearable: 2_100_000_000 };

  window.__demo = {
    events, calendars, dueReminders, reminderLists, reminderItems, noteFolders, noteList, contactList,
    // Driveitems for the stub's own search; the folders are answered below.
    driveItems: drive.root,
    answer(cmd, args) {
      // The stub's own arrays, which it shares back after taking these rows in.
      const reminderItems = window.__demo.reminderItems;
      switch (cmd) {
        case "note_runs": return notes[args.guid] ? runsOf(notes[args.guid]) : undefined;
        case "note_body": return notes[args.guid] ? notes[args.guid].lines.map((l) => l[0]).join("\n") : undefined;
        case "note_attachment_info":
          if (args.id === "T1") return { id: "T1", uti: "com.apple.notes.table", kind: "table", filename: null, title: null, width: null, height: null, size: 1200, hasPreview: false };
          if (args.id === "M1") return { id: "M1", uti: "public.jpeg", kind: "image", filename: "Porch.jpeg", title: null, width: 1600, height: 760, size: 412000, hasPreview: true };
          return undefined;
        case "reminder_counts": {
          const open = reminderItems.filter((r) => !r.completedAt && !r.parentGuid);
          const end = at(1);
          return { today: open.filter((r) => r.dueAt !== null && r.dueAt < end).length, scheduled: open.filter((r) => r.dueAt !== null).length,
            flagged: open.filter((r) => r.flagged).length, all: open.length,
            lists: reminderLists.map((l) => [l.guid, open.filter((r) => r.listGuid === l.guid).length]) };
        }
        case "reminder_groups": return [];
        case "reminders_smart": {
          const open = reminderItems.filter((r) => !r.completedAt);
          const pick = { today: (r) => r.dueAt !== null && r.dueAt < args.until, scheduled: (r) => r.dueAt !== null, flagged: (r) => r.flagged, all: () => true }[args.kind] ?? (() => true);
          const tops = open.filter((r) => !r.parentGuid && pick(r));
          // As Today shows them on a phone: the reminders due, without their undated subtasks.
          return tops;
        }
        case "reminders_in": {
          const all = reminderItems.filter((r) => r.listGuid === args.listGuid);
          return args.includeDone ? all : all.filter((r) => !r.completedAt);
        }
        case "reminder_set_done": {
          const r = reminderItems.find((x) => x.guid === args.guid);
          if (r) r.completedAt = args.done ? Math.floor(Date.now() / 1000) : null;
          return undefined;
        }
        case "reminders_in_smart_list":
          return { reminders: reminderItems.filter((r) => !r.completedAt && (window.__tagMap[r.guid] ?? []).includes("weekend")), unknown: [] };
        case "drive_children": return (drive[args.folderId] ?? drive.root).map((i) => ({ ...i }));
        case "drive_item": return driveById.get(args.drivewsid) ?? null;
        case "drive_thumbnails": return thumbs;
        case "drive_sync_folder": return (drive[args.folderId] ?? drive.root).length;
        case "drive_recents": return drive["FOLDER::cabin"].filter((i) => i.kind === "FILE").map((i, k) => ({ ...i, openedAt: now - (k + 1) * 3 * HOUR }));
        case "drive_trash": return [];
        case "account_info": return { name: "Jamie Rivera", email: "jamie.rivera@example.com", photoContact: null, webAccessAllowed: true, dataProtection: false };
        case "storage_info": return { usedBytes: 61_400_000_000, totalBytes: 200_000_000_000 };
        case "sync_status": case "sync_all": return report();
        case "photos_albums": return window.__albums.map(({ items, ...a }) => [a, items.length, items[0] ?? null]);
        default: return undefined;
      }
    },
  };
}
