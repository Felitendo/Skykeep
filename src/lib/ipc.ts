import { convertFileSrc, invoke } from "@tauri-apps/api/core";

export type Status = { hasStoredSession: boolean; signedIn: boolean };
export type SignInResult = "complete" | "needs_two_factor";

export type TrustedPhone = { id: number; maskedNumber: string };
export type TwoFactorOptions = {
  trustedDeviceCount: number;
  phones: TrustedPhone[];
  tooManyCodesSent: boolean;
};

/** How one service's refresh ended. */
export type SyncOutcome = "ok" | "unavailable" | "failed";

export type ServiceReport = {
  service: string;
  label: string;
  outcome: SyncOutcome;
  /** What came back, or what went wrong. */
  detail: string | null;
  lastSyncedAt: number | null;
};

/** Everything one refresh pass did. */
export type Refresh = {
  services: ServiceReport[];
  /** The stalest part of the library — the only freshness the window can claim. */
  asOf: number | null;
  tookMs: number;
};

/** Emitted by the backend whenever a refresh finishes, background ones included. */
/** Where each uploading file's progress is announced. */
export const UPLOAD = "quince://upload";
export const PHOTO_UPLOAD = "quince://photo-upload";

/** One file's upload, as it moves. */
export type UploadEvent = {
  id: string;
  batch: string;
  name: string;
  /** The folders it sits in below the one it was dropped on. */
  within: string;
  sent: number;
  total: number;
  state: "preparing" | "waiting" | "sending" | "verifying" | "done" | "failed" | "cancelled" | "planned";
  /** A folder being made, rather than a file being sent. */
  folder?: boolean;
  error: string | null;
};

export const SYNCED = "quince://synced";
/** An alert was clicked: open Reminders, where that reminder is. */
export const OPEN_REMINDER = "quince://open-reminder";
/** A lapsed session signed itself back in only as far as Apple's code. */
export const NEEDS_CODE = "quince://needs-code";

export const api = {
  /** One refresh for every service. Never rejects for a single service failing. */
  syncAll: () => invoke<Refresh>("sync_all"),
  /** What the library already knows about its own freshness, fetching nothing. */
  syncStatus: () => invoke<Refresh>("sync_status"),

  status: () => invoke<Status>("auth_status"),
  /** Where a returning person goes: their library, the code Apple wants, or sign-in. */
  resume: () => invoke<"library" | "code" | "signin">("auth_resume"),
  signIn: (appleId: string, password: string) =>
    invoke<SignInResult>("auth_sign_in", { appleId, password }),
  submitCode: (code: string) => invoke<void>("auth_submit_code", { code }),
  twoFactorOptions: () => invoke<TwoFactorOptions>("auth_two_factor_options"),
  resendCode: () => invoke<void>("auth_resend_code"),
  sendSms: (phoneId: number) => invoke<void>("auth_send_sms", { phoneId }),
  submitSmsCode: (phoneId: number, code: string) =>
    invoke<void>("auth_submit_sms_code", { phoneId, code }),
  signOut: () => invoke<void>("auth_sign_out"),
  services: () => invoke<string[]>("account_services"),

  eventSave: (draft: EventWrite) => invoke<string>("event_save", { draft }),
  eventAlerts: (guid: string) => invoke<number[]>("event_alerts", { guid }),
  eventRepeat: (guid: string) => invoke<EventRepeat | null>("event_repeat", { guid }),
  eventDelete: (guid: string, scope?: EventScope) => invoke<void>("event_delete", { guid, scope: scope ?? null }),
  calendarSync: (from: string, to: string, timezone: string) =>
    invoke<SyncSummary>("calendar_sync", { from, to, timezone }),
  /** Events holding every word of the query, soonest first. */
  calendarSearch: (query: string) => invoke<CalendarEvent[]>("calendar_search", { query }),
  calendarEvents: (from: number, to: number) =>
    invoke<CalendarEvent[]>("calendar_events", { from, to }),
  calendarList: () => invoke<Calendar[]>("calendar_list"),
  photosList: (view: PhotoView) => invoke<Photo[]>("photos_list", { view }),
  photosCounts: () => invoke<[PhotoView, number][]>("photos_counts"),
  /** The albums the person made, each with how many photos it holds. */
  /** Each album, how many it shows, and its cover — the first photo it shows. */
  photosAlbums: () => invoke<[PhotoAlbum, number, string | null][]>("photos_albums"),
  photosInAlbum: (album: string) => invoke<Photo[]>("photos_in_album", { album }),
  /** A video (or a Live Photo's moving part) opened in this computer's own player. */
  /** How a video's picture is turned, as its file records it (served upright: drawn here). */
  photoVideoTurn: (id: string, which: "original" | "video") => invoke<Turn>("photo_video_turn", { id, which }),
  /** The iCloud Drive folder on this computer: whether it is up, where, and what stopped it. */
  driveFolderStatus: () => invoke<DriveFolder>("drive_folder_status"),
  driveFolderSet: (on: boolean) => invoke<DriveFolder>("drive_folder_set", { on }),
  driveFolderOpen: () => invoke<void>("drive_folder_open"),
  /** The iCloud Photos folder on this computer: the same, for photos (read-only for now). */
  photosFolderStatus: () => invoke<DriveFolder>("photos_folder_status"),
  photosFolderSet: (on: boolean) => invoke<DriveFolder>("photos_folder_set", { on }),
  photosFolderOpen: () => invoke<void>("photos_folder_open"),
  photoPlay: (id: string) => invoke<void>("photo_play", { id }),
  /** Where the window's player fetches videos from (a loopback server), or null without one. */
  videoBase: () => invoke<string | null>("video_base"),
  /** The window full screen or back; answers whether it now is. */
  windowFullscreen: (on: boolean) => invoke<boolean>("window_fullscreen", { on }),
  /** What the window shows pictures at, so the app makes them before they are shown. */
  photosPrepare: (shown: { cell: number; fit: boolean; stageW: number; stageH: number; dpr: number }) =>
    invoke<void>("photos_prepare", { shown }),
  /** Shared albums, made or joined, newest first — from Apple each time. */
  photosSharedAlbums: () => invoke<SharedAlbum[]>("photos_shared_albums"),
  photosSharedCreate: (name: string) => invoke<string>("photos_shared_create", { name }),
  photosSharedDelete: (guid: string) => invoke<void>("photos_shared_delete", { guid }),
  /** Every photo of one burst, in the order they were shot. */
  photosBurst: (burst: string) => invoke<Photo[]>("photos_burst", { burst }),
  /** Favorite, Delete (to Recently Deleted) or Recover; answers the photo as Apple now has it. */
  photosSetFlag: (ids: string[], flag: "favorite" | "deleted" | "hidden" | "expunged", on: boolean) =>
    invoke<Photo[]>("photos_set_flag", { ids, flag, on }),
  photoSetFlag: (id: string, flag: "favorite" | "deleted" | "hidden" | "expunged", on: boolean) =>
    invoke<Photo>("photo_set_flag", { id, flag, on }),
  /** The original saved to Downloads; answers where. */
  /** As icloud.com's Download Options: the original (the default), the highest quality, or
   * the most compatible. Answers where it was saved. */
  photoDownload: (id: string, want?: "originals" | "highestQuality" | "mostCompatible") =>
    invoke<string>("photo_download", { id, want: want ?? null }),
  /** Title and caption together, as Info saves them. */
  photoSetText: (id: string, title: string, caption: string) => invoke<Photo>("photo_set_text", { id, title, caption }),
  photoSetRating: (id: string, rating: number) => invoke<Photo>("photo_set_rating", { id, rating }),
  photoSetKeywords: (id: string, keywords: string[]) => invoke<Photo>("photo_set_keywords", { id, keywords }),
  photoOpenMap: (id: string) => invoke<void>("photo_open_map", { id }),
  /** When it was taken, in unix seconds. */
  photoSetDate: (id: string, takenAt: number) => invoke<Photo>("photo_set_date", { id, takenAt }),
  /** Every word in a name, title, caption, keyword, year or month; newest first. */
  photosSearch: (query: string) => invoke<Photo[]>("photos_search", { query }),
  /** Photos and videos (or folders of them) into the library. Answers the batch; progress
   * arrives as PHOTO_UPLOAD events, in the same shape as Drive's. */
  /** Into the library, and into `album` too when one is open. */
  photosUpload: (paths: string[], album?: string) => invoke<string>("photos_upload", { paths, album: album ?? null }),
  /** An album — or a folder — at the top or in the folder `parent`. */
  photosAlbumCreate: (name: string, parent?: string, folder?: boolean) =>
    invoke<PhotoAlbum>("photos_album_create", { name, parent: parent ?? null, folder: folder ?? false }),
  photosAlbumRename: (id: string, name: string) => invoke<void>("photos_album_rename", { id, name }),
  photosAlbumDelete: (id: string) => invoke<void>("photos_album_delete", { id }),
  photosAlbumAdd: (album: string, ids: string[]) => invoke<void>("photos_album_add", { album, ids }),
  photosAlbumRemove: (album: string, ids: string[]) => invoke<void>("photos_album_remove", { album, ids }),
  photosUploadCancel: (ids: string[], batch?: string) =>
    invoke<void>("photos_upload_cancel", { ids, batch: batch ?? null }),
  /** New Calendar: Apple's next unused colour; a name already in use is refused. */
  calendarCreate: (title: string) => invoke<string>("calendar_create", { title }),
  calendarUpdate: (guid: string, change: { title?: string; colour?: string }) =>
    invoke<void>("calendar_update", { guid, title: change.title ?? null, colour: change.colour ?? null }),
  calendarDelete: (guid: string) => invoke<void>("calendar_delete", { guid }),

  reminderCounts: (until: number) => invoke<ReminderCounts>("reminder_counts", { until }),
  reminderLists: () => invoke<ReminderList[]>("reminder_lists"),
  reminderGroups: () => invoke<ReminderGroup[]>("reminder_groups"),
  /** The headings inside one list, in the order they were made. */
  reminderSections: (listGuid: string) =>
    invoke<ReminderSection[]>("reminder_sections", { listGuid }),
  remindersIn: (listGuid: string, includeDone: boolean) =>
    invoke<Reminder[]>("reminders_in", { listGuid, includeDone }),
  remindersDueBetween: (from: number, to: number) =>
    invoke<Reminder[]>("reminders_due_between", { from, to }),
  reminderSetRepeat: (guid: string, listGuid: string, repeat: Repeat | null) =>
    invoke<void>("reminder_set_repeat", { guid, listGuid, repeat }),
  reminderSetDone: (guid: string, listGuid: string, done: boolean) =>
    invoke<void>("reminder_set_done", { guid, listGuid, done }),
  reminderEdit: (guid: string, listGuid: string, edit: ReminderEdit) =>
    invoke<void>("reminder_edit", { guid, listGuid, edit }),
  reminderListCreate: (title: string) => invoke<void>("reminder_list_create", { title }),
  reminderListRename: (listGuid: string, title: string) =>
    invoke<void>("reminder_list_rename", { listGuid, title }),
  reminderReorder: (listGuid: string, guid: string, before: string | null) =>
    invoke<void>("reminder_reorder", { listGuid, guid, before }),
  reminderListColour: (listGuid: string, colour: string) =>
    invoke<void>("reminder_list_colour", { listGuid, colour }),
  reminderListDelete: (listGuid: string) =>
    invoke<void>("reminder_list_delete", { listGuid }),
  /** Reminders still to do holding every word typed, across every list. */
  remindersSearch: (query: string) => invoke<Reminder[]>("reminders_search", { query }),
  remindersSmart: (kind: SmartList, until: number) =>
    invoke<Reminder[]>("reminders_smart", { kind, until }),
  reminderCreate: (
    listGuid: string,
    title: string,
    implied: { dueAt?: number; allDay?: boolean; flagged?: boolean } = {},
  ) => invoke<void>("reminder_create", { listGuid, title, ...implied }),
  reminderDelete: (guid: string, listGuid: string) =>
    invoke<void>("reminder_delete", { guid, listGuid }),

  noteFolders: () => invoke<NoteFolder[]>("note_folders"),
  noteParagraph: (guid: string, start: number, change: { style?: number; checked?: boolean; expect?: string }) =>
    invoke<void>("note_paragraph", { guid, start, ...change }),
  /** A table's cells, row by row, by the attachment id a note's text names it by. */
  noteTable: (id: string) => invoke<string[][] | null>("note_table", { id }),
  /** Change one cell of a table; answers the table as Apple now holds it. */
  /** What a note's attachment is — a picture, a document, a recording, a file. */
  noteAttachmentInfo: (id: string) => invoke<NoteAttachment>("note_attachment_info", { id }),
  /** Open a note's attachment in the app this computer uses for it. */
  noteAttachmentOpen: (id: string) => invoke<void>("note_attachment_open", { id }),
  noteTableSetCell: (id: string, row: number, column: number, text: string) =>
    invoke<string[][]>("note_table_set_cell", { id, row, column, text }),
  /** An empty row added at `at` (0 the top); answers the table as Apple now holds it. */
  noteTableAddRow: (id: string, at: number) => invoke<string[][]>("note_table_add_row", { id, at, column: false }),
  /** An empty column added at `at` (0 the leading edge). */
  noteTableAddColumn: (id: string, at: number) => invoke<string[][]>("note_table_add_row", { id, at, column: true }),
  /** The row (or column) at `at` deleted; answers the table as Apple now holds it. */
  noteTableRemove: (id: string, at: number, column: boolean) => invoke<string[][]>("note_table_remove", { id, at, column }),
  noteRuns: (guid: string) => invoke<import("../screens/notes/Formatted").Run[]>("note_runs", { guid }),
  noteFolderCreate: (title: string, parent: string | null) =>
    invoke<string>("note_folder_create", { title, parent }),
  noteFolderRename: (guid: string, title: string) => invoke<void>("note_folder_rename", { guid, title }),
  noteFolderDelete: (guid: string) => invoke<void>("note_folder_delete", { guid }),
  notesIn: (folderGuid: string | null) => invoke<Note[]>("notes_in", { folderGuid }),
  /** [notes outside Recently Deleted, [folder guid, count]…]. */
  noteCounts: () => invoke<[number, [string, number][]]>("note_counts"),
  /** Notes holding every word typed, from every folder but Recently Deleted. */
  notesSearch: (query: string) => invoke<Note[]>("notes_search", { query }),
  /**
   * One note's text, whole. Null when there is no such note any more.
   *
   * A listed note carries no body — the list only draws a title and a line of preview, and
   * carrying every word of every note into the window on each refresh is what makes a folder
   * of thousands slow.
   */
  noteBody: (guid: string) => invoke<string | null>("note_body", { guid }),
  /** Answers with the text Apple now holds — ours, or ours combined with another device's. */
  noteSave: (
    guid: string,
    body: string,
    base: string | null,
    runs: import("../screens/notes/Formatted").Run[] | null = null,
    baseRuns: import("../screens/notes/Formatted").Run[] | null = null,
  ) => invoke<string>("note_save", { guid, body, base, runs, baseRuns }),
  noteCreate: (folderGuid: string | null, body: string) =>
    invoke<string>("note_create", { folderGuid, body }),
  noteDelete: (guid: string) => invoke<void>("note_delete", { guid }),
  noteMove: (guid: string, folderGuid: string | null) =>
    invoke<void>("note_move", { guid, folderGuid }),

  /** Pinned notes lead their list, here and on every device. */
  notePin: (guid: string, pinned: boolean) => invoke<void>("note_pin", { guid, pinned }),

  /** Create when the guid is empty, otherwise save changes. Returns Apple's id for it. */
  /** `base` is the card as the form opened it: only what changed from it is written. */
  contactSave: (contact: Contact, base: Contact | null = null) => invoke<string>("contact_save", { contact, base }),
  contactDelete: (guid: string) => invoke<void>("contact_delete", { guid }),
  contactsList: (search: string | null) => invoke<Contact[]>("contacts_list", { search }),
  /** These people as a vCard file in Downloads, written by Apple. Returns the path. */
  contactsExport: (guids: string[], fileName: string) => invoke<string>("contacts_export", { guids, fileName }),
  /** Everyone in a vCard file, added to Contacts. Returns [added, could not be read]. */
  /** vCard and CSV files, any number: [imported, could not be read]. */
  /** How many came in, could not be read, and were in Contacts already. */
  contactsImport: (paths: string[]) => invoke<[number, number, number]>("contacts_import", { paths }),
  /** The card that is this person's own, if they have said. */
  contactMe: () => invoke<string | null>("contact_me"),
  contactSetMe: (guid: string) => invoke<void>("contact_set_me", { guid }),
  /** Contacts' lists — Apple's groups — in the order Apple keeps them. */
  contactLists: () => invoke<ContactList[]>("contact_lists"),
  /**
   * Make a list (no guid), rename one, add people to it or take them out, or delete it.
   * Membership goes as changes, laid over the list as Apple holds it. Returns its id.
   */
  contactListWrite: (change: { guid?: string; name?: string; add?: string[]; remove?: string[]; delete?: boolean }) =>
    invoke<string>("contact_list_write", change),

  driveSyncFolder: (folderId: string) => invoke<number>("drive_sync_folder", { folderId }),
  driveChildren: (folderId: string) => invoke<DriveItem[]>("drive_children", { folderId }),
  driveItem: (drivewsid: string) => invoke<DriveItem | null>("drive_item", { drivewsid }),
  /** Saves to Downloads; returns the path it landed at. */
  driveDownload: (drivewsid: string, item?: DriveItem) => invoke<string>("drive_download", { drivewsid, item: item ?? null }),
  /** The files opened most recently, newest first. */
  driveRecents: () => invoke<RecentItem[]>("drive_recents"),
  /**
   * Put files from this machine into a Drive folder. Paths only — the bytes are read and
   * streamed in the backend, never carried through a message.
   *
   * Resolves with the files that did not go up, one line each, so a single refusal does not
   * read as the whole upload failing.
   */
  /** Files and folders into a Drive folder. Progress arrives as UPLOAD events. */
  /** Stop uploads — these, or a whole batch. One already being filed finishes. */
  driveUploadCancel: (ids: string[], batch?: string) => invoke<void>("drive_upload_cancel", { ids, batch: batch ?? null }),
  driveUpload: (batch: string, folderId: string, paths: string[]) =>
    invoke<string[]>("drive_upload", { batch, folderId, paths }),
  driveCreateFolder: (parentId: string, name: string) =>
    invoke<void>("drive_create_folder", { parentId, name }),
  driveRename: (drivewsid: string, name: string) =>
    invoke<void>("drive_rename", { drivewsid, name }),
  driveDelete: (drivewsid: string) => invoke<void>("drive_delete", { drivewsid }),
  driveTrash: () => invoke<TrashItem[]>("drive_trash"),
  driveThumbnails: (folderId: string) => invoke<Record<string, string>>("drive_thumbnails", { folderId }),
  driveDuplicate: (drivewsid: string) => invoke<void>("drive_duplicate", { drivewsid }),
  storageInfo: () => invoke<{ usedBytes: number; totalBytes: number }>("storage_info"),
  /** Who is signed in, as Apple names them. */
  accountInfo: () => invoke<AccountInfo>("account_info"),
  /** Settings' switches, as the backend obeys them. */
  /** Write a plain diagnostic report to Downloads; answers where. Nothing is sent anywhere. */
  diagnosticsSave: () => invoke<string>("diagnostics_save"),
  settingsGet: () => invoke<{ reminderAlerts: boolean; eventAlerts: boolean }>("settings_get"),
  settingsSet: (key: "reminderAlerts" | "eventAlerts", on: boolean) => invoke<void>("settings_set", { key, on }),
  drivePutBack: (item: DriveItem) => invoke<void>("drive_put_back", { item }),
  driveDeleteForever: (item: DriveItem) => invoke<void>("drive_delete_forever", { item }),
  /** Recover, or delete for good, all of these in one request; what could not be, with why. */
  driveTrashOut: (items: DriveItem[], recover: boolean) =>
    invoke<{ drivewsid: string; reason: string }[]>("drive_trash_out", { items, recover }),
  driveMove: (drivewsid: string, destinationId: string) =>
    invoke<void>("drive_move", { drivewsid, destinationId }),
};

/** Tauri rejects with a plain string from our commands; keep it readable either way. */
export type Calendar = {
  guid: string;
  title: string;
  colour: string | null;
  ctag: string | null;
  sortOrder: number;
  enabled: boolean;
  /** False for a subscribed feed or a calendar shared for reading only. */
  writable: boolean;
  /** The one Apple puts a new event in. */
  isDefault: boolean;
};

/**
 * Where a new event should go.
 *
 * Apple names the default calendar outright, so it is asked rather than guessed. Taking the
 * first of the list is the mistake that put every new reminder in whichever list happened to
 * sort first — and the first calendar can also be one nothing may be written to.
 */
export function defaultCalendar(calendars: Calendar[]): Calendar | null {
  const writable = calendars.filter((c) => c.writable);
  return writable.find((c) => c.isDefault) ?? writable[0] ?? null;
}

/** What the app sends to create or change an event. A guid means it already exists. */
export type EventWrite = {
  guid: string | null;
  calendarGuid: string;
  title: string;
  location: string | null;
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  timezone: string | null;
  notes: string | null;
  url: string | null;
  /** Set the alerts to exactly these minutes before the start (0 at the time); left out,
   *  the event keeps the alerts Apple holds. */
  alerts?: number[];
  /** Set the repeat to this (null: never) when `repeatChanged`; otherwise it is left as it is. */
  repeat?: EventRepeat | null;
  repeatChanged?: boolean;
  /** For one occurrence of a repeating event: whether the change is for it alone or the series. */
  scope?: EventScope;
};

/** How an event repeats, in icloud.com's words. */
export type EventRepeat = { freq: "daily" | "weekly" | "monthly" | "yearly"; interval: number; count: number | null };

export type CalendarEvent = {
  guid: string;
  calendarGuid: string;
  title: string;
  location: string | null;
  /** Unix seconds, UTC. */
  startsAt: number;
  endsAt: number;
  allDay: boolean;
  timezone: string | null;
  url: string | null;
  notes: string | null;
  etag: string | null;
};

export type SyncSummary = { calendars: number; events: number };

export type ReminderList = {
  guid: string;
  title: string;
  colour: string | null;
  ctag: string | null;
  sortOrder: number;
  enabled: boolean;
  /** Apple's name for the list's glyph, such as "fitness". */
  emblem?: string | null;
  /** The group this list is filed under. */
  groupGuid?: string | null;
  /** Shown as a tile beside Today and Scheduled. */
  pinned?: boolean;
  /** How the list orders its reminders: "manual", "displayDate_asc", … */
  sorting?: string | null;
};

/** A folder of lists in the Reminders sidebar. */
export type ReminderGroup = { guid: string; title: string; sortOrder: number };

/** A heading inside a reminders list. */
export type ReminderSection = {
  guid: string;
  listGuid: string;
  title: string;
  createdAt: number | null;
};

export type SmartList = "today" | "scheduled" | "flagged" | "all";

/** What every entry in the Reminders sidebar is holding. */
export type ReminderCounts = {
  today: number;
  scheduled: number;
  flagged: number;
  all: number;
  /** [list guid, outstanding count] */
  lists: [string, number][];
};

/**
 * The most reminders any one view is handed at once.
 *
 * Kept in step with the same figure in the library. "All" on an account ten years deep is
 * genuinely unbounded, and the whole of it crossing into this window is what the plan rules
 * out; the true totals are counted separately, so a view that reaches this can say so.
 */
export const MOST_AT_ONCE = 1000;

/** How often a reminder comes back. Frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. */
export type Repeat = { frequency: number; interval: number };

/** Anything omitted is left untouched; `null` clears a field. */
export type ReminderEdit = {
  title?: string;
  notes?: string | null;
  dueAt?: number | null;
  /** With `dueAt`: a day, not a time. */
  allDay?: boolean;
  flagged?: boolean;
  completed?: boolean;
  /** Apple's scale: 0 none, 1 high, 5 medium, 9 low. */
  priority?: number;
  /** Move the reminder to another list. */
  listGuid?: string;
  /** Nest under another reminder, or null to bring it back out to the top level. */
  parentGuid?: string | null;
};

export type Reminder = {
  guid: string;
  listGuid: string;
  title: string;
  notes: string | null;
  dueAt: number | null;
  /** null while outstanding */
  completedAt: number | null;
  priority: number;
  flagged: boolean;
  /** The reminder this one is a subtask of, if any. */
  parentGuid: string | null;
  /** Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. null if it never repeats. */
  recurFrequency: number | null;
  recurInterval: number | null;
  /** The section of its list this sits in, if any. */
  sectionGuid?: string | null;
  /** Due on a day rather than at a time. */
  allDay?: boolean;
  etag: string | null;
};

export type NoteFolder = {
  guid: string;
  title: string;
  sortOrder: number;
  /** The folder this one sits inside, where it sits inside one at all. */
  parentGuid: string | null;
};

/**
 * Folders in the order they are filed, each with how deep it sits.
 *
 * Apple Notes nests folders, and flattening them turns somebody's filing into one long list
 * with every level jumbled together. A folder whose parent has been deleted is shown at the
 * top rather than dropped — it still holds notes.
 */
export function nestFolders(folders: NoteFolder[]): { folder: NoteFolder; depth: number }[] {
  const byParent = new Map<string | null, NoteFolder[]>();
  const known = new Set(folders.map((f) => f.guid));
  for (const f of folders) {
    const under = f.parentGuid && known.has(f.parentGuid) ? f.parentGuid : null;
    byParent.set(under, [...(byParent.get(under) ?? []), f]);
  }

  const out: { folder: NoteFolder; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number) => {
    for (const folder of byParent.get(parent) ?? []) {
      // A folder cannot be its own ancestor, but nothing about the data guarantees that, and
      // a cycle here would hang the window rather than show a wrong tree.
      if (seen.has(folder.guid)) continue;
      seen.add(folder.guid);
      out.push({ folder, depth });
      walk(folder.guid, depth + 1);
    }
  };
  walk(null, 0);
  for (const folder of folders) {
    if (!seen.has(folder.guid)) out.push({ folder, depth: 0 });
  }
  return out;
}

export type Note = {
  guid: string;
  folderGuid: string | null;
  title: string;
  snippet: string | null;
  body: string | null;
  /** Locked by its owner, so Apple holds it encrypted and there is nothing to show. */
  locked: boolean;
  /** False when Apple's document can't be rewritten without losing what surrounds the text. */
  editable: boolean;
  pinned: boolean;
  createdAt: number | null;
  modifiedAt: number | null;
  etag: string | null;
};

export type ContactMethod = { kind: string; label: string | null; value: string };

export type ContactList = {
  guid: string;
  name: string;
  /** The contacts in it, by guid. */
  members: string[];
  etag: string | null;
};

/** Which events a change to one day of a repeating event is for. */
export type RecentItem = DriveItem & { openedAt: number | null };

export type EventScope = "this" | "future" | "all";

export type Contact = {
  guid: string;
  firstName: string | null;
  middleName: string | null;
  lastName: string | null;
  photoUrl: string | null;
  photoSignature: string | null;
  company: string | null;
  isCompany: boolean;
  birthday: string | null;
  notes: string | null;
  etag: string | null;
  methods: ContactMethod[];
};

/** What to show, whichever name fields a contact happens to have. */
/**
 * Where the interface loads a contact's picture from.
 *
 * Its own scheme rather than a data URI over IPC: the webview fetches and caches it the way
 * it does any image, so a list of a hundred people does not push a hundred pictures through
 * a message channel every time it scrolls.
 */
/** Who is signed in. `photoContact` is their own card, when it has a picture. */
export type AccountInfo = {
  name: string | null;
  email: string | null;
  webAccessAllowed?: boolean | null;
  dataProtection?: boolean | null;
  photoContact?: string | null;
};

export type NoteAttachment = {
  id: string;
  uti: string;
  kind: "image" | "document" | "audio" | "video" | "file";
  filename: string | null;
  title: string | null;
  width: number | null;
  height: number | null;
  size: number | null;
  hasPreview: boolean;
};

/** A note attachment's picture — the file itself, or Apple's rendering of it. */
export function noteMediaSrc(id: string, part: "file" | "preview"): string {
  return convertFileSrc(`note/${id}/${part}`, "quince");
}

/** One of a photo's pictures, served from the disk cache (fetched from Apple once). */
export function pictureSrc(id: string, size: "thumb" | "medium" | "full" | "original" | "video"): string {
  return convertFileSrc(`pic/${id}/${size}`, "quince");
}

/** The longer sides a picture is made at for the screen (photos.rs FITS). */
export const FITS = [128, 256, 384, 512, 768, 1024, 1536, 2048, 2560, 3072, 3840];

/**
 * A photo's picture made `long` device pixels on its longer side — the step at or above it —
 * resampled with care by the app. The engine then draws it at its own size: shrunk by
 * WebKitGTK instead, a large picture came out jagged.
 */
export function fitSrc(id: string, long: number): string {
  const step = FITS.find((f) => f >= long) ?? FITS[FITS.length - 1];
  return convertFileSrc(`pic/${id}/fit${step}`, "quince");
}

/** A shared album (Apple's sharedstreams service). */
export type SharedAlbum = {
  guid: string;
  name: string;
  createdAt: number | null;
  /** Made by this account, as against one it was invited to. */
  owned: boolean;
  public: boolean;
  contributions: boolean;
};

export type PhotoAlbum = {
  id: string;
  name: string;
  position: number;
  changeTag: string | null;
  /** A folder of albums, not an album of photos. */
  folder?: boolean;
  /** The folder it sits in; none at the top. */
  parent?: string | null;
};

export type PhotoView = "library" | "favorites" | "videos" | "screenshots" | "panoramas" | "portrait" | "recents" | "hidden" | "rated" | "recentlyDeleted";

export type Photo = {
  id: string;
  master: string;
  /** Unix seconds. */
  takenAt: number;
  addedAt: number | null;
  kind: "image" | "video" | "live";
  width: number;
  height: number;
  duration: number | null;
  filename: string | null;
  size: number | null;
  uti: string | null;
  favorite: boolean;
  hidden: boolean;
  deleted: boolean;
  deletedAt: number | null;
  /** In Recently Deleted: when Apple deletes it for good, unix seconds. */
  expungeAt?: number | null;
  /** The burst it was shot in; a burst shows as its chosen photo. */
  burst?: string | null;
  burstPick?: boolean;
  /** How many photos its burst holds. */
  burstCount?: number;
  /** Its title and caption, as typed in Photos' Info. */
  title?: string | null;
  caption?: string | null;
  /** Its star rating, 0 (none) to 5. */
  rating?: number;
  keywords?: string[];
  /** Where it was taken, in degrees. */
  latitude?: number | null;
  longitude?: number | null;
  subtype: number;
  changeTag: string | null;
};

/** The picture of the contact with this id, served by the backend. */
export function contactPhotoSrc(guid: string): string {
  return convertFileSrc(`photo/${guid}`, "quince");
}

export function photoSrc(c: Contact): string | null {
  return c.photoUrl ? contactPhotoSrc(c.guid) : null;
}

export function displayName(c: Contact): string {
  const full = [c.firstName, c.middleName, c.lastName].filter(Boolean).join(" ").trim();
  if (full) return full;
  if (c.company?.trim()) return c.company;
  return c.methods[0]?.value ?? "No Name";
}

export type DriveItem = {
  drivewsid: string;
  parentId: string | null;
  name: string;
  extension: string | null;
  /** FOLDER, APP_LIBRARY or FILE. */
  kind: string;
  size: number | null;
  childCount: number | null;
  createdAt: number | null;
  modifiedAt: number | null;
  etag: string | null;
};

/** An item in Drive's Recently Deleted: when it goes for good, and where Recover puts it. */
export type TrashItem = DriveItem & { expiresAt: number | null; restorePath: string | null };

export const DRIVE_ROOT = "FOLDER::com.apple.CloudDocs::root";

export const isContainer = (item: DriveItem) =>
  item.kind === "FOLDER" || item.kind === "APP_LIBRARY";

/** The container an item belongs to: the middle of `<TYPE>::<zone>::<id>`. */
export const zoneOf = (drivewsid: string): string | null =>
  drivewsid.split("::")[1] || null;

/**
 * Whether files from this machine can be put in a folder.
 *
 * An app library — Keynote's, Shortcuts', a third party's — belongs to the app that made it,
 * and Apple files a document by a bare id with no zone beside it. A file meant for one would
 * land somewhere else entirely, so it is not offered.
 */
export const acceptsUploads = (folderId: string): boolean =>
  (zoneOf(folderId) ?? "com.apple.CloudDocs") === "com.apple.CloudDocs";

export const driveName = (item: DriveItem) =>
  item.extension && !isContainer(item) ? `${item.name}.${item.extension}` : item.name;

export function messageOf(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Try again.";
}

/** A film's turn as its file records it: the matrix `[a, b, c, d]`, each -1, 0 or 1. */
export type Turn = [number, number, number, number];

/**
 * How a film is drawn, turned as it was recorded, in a box the shape of the photo (`w` by
 * `h`, upright). The film is served as the camera wrote it (the backend leaves its turn to
 * the window, which WebKitGTK honoured in one way of drawing and not another); here it is
 * laid out in its own shape and turned into the box with the file's own matrix — CSS's
 * matrix() and the file's are the same reading — a phone's quarter turn, or the front
 * camera's quarter turn and mirror.
 */
export function turnStyle(turn: Turn | null | undefined, w: number, h: number): Record<string, string> {
  if (!turn || (turn[0] === 1 && turn[1] === 0 && turn[2] === 0 && turn[3] === 1) || !w || !h) return {};
  const quarter = turn[0] === 0;
  return {
    position: "absolute",
    left: "50%",
    top: "50%",
    width: quarter ? `${(100 * h) / w}%` : "100%",
    height: quarter ? `${(100 * w) / h}%` : "100%",
    maxWidth: "none",
    maxHeight: "none",
    translate: "-50% -50%",
    transform: `matrix(${turn[0]}, ${turn[1]}, ${turn[2]}, ${turn[3]}, 0, 0)`,
  };
}

/** The iCloud Drive folder on this computer (see src-tauri/src/mount.rs). */
export type DriveFolder = {
  on: boolean;
  path: string;
  problem: string | null;
  /** Files written into the folder and not yet at Apple (the Drive folder only). */
  waiting?: { files: number; sending: number; failure: string | null } | null;
};
