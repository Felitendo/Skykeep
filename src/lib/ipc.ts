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
export const SYNCED = "quince://synced";

export const api = {
  /** One refresh for every service. Never rejects for a single service failing. */
  syncAll: () => invoke<Refresh>("sync_all"),
  /** What the library already knows about its own freshness, fetching nothing. */
  syncStatus: () => invoke<Refresh>("sync_status"),

  status: () => invoke<Status>("auth_status"),
  resume: () => invoke<boolean>("auth_resume"),
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
  eventDelete: (guid: string) => invoke<void>("event_delete", { guid }),
  calendarSync: (from: string, to: string, timezone: string) =>
    invoke<SyncSummary>("calendar_sync", { from, to, timezone }),
  calendarEvents: (from: number, to: number) =>
    invoke<CalendarEvent[]>("calendar_events", { from, to }),
  calendarList: () => invoke<Calendar[]>("calendar_list"),

  reminderCounts: (until: number) => invoke<ReminderCounts>("reminder_counts", { until }),
  reminderLists: () => invoke<ReminderList[]>("reminder_lists"),
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
  reminderListDelete: (listGuid: string) =>
    invoke<void>("reminder_list_delete", { listGuid }),
  remindersSmart: (kind: SmartList, until: number) =>
    invoke<Reminder[]>("reminders_smart", { kind, until }),
  reminderCreate: (listGuid: string, title: string) =>
    invoke<void>("reminder_create", { listGuid, title }),
  reminderDelete: (guid: string, listGuid: string) =>
    invoke<void>("reminder_delete", { guid, listGuid }),

  noteFolders: () => invoke<NoteFolder[]>("note_folders"),
  notesIn: (folderGuid: string | null) => invoke<Note[]>("notes_in", { folderGuid }),
  noteSave: (guid: string, body: string) => invoke<void>("note_save", { guid, body }),
  noteCreate: (folderGuid: string | null, body: string) =>
    invoke<string>("note_create", { folderGuid, body }),
  noteDelete: (guid: string) => invoke<void>("note_delete", { guid }),
  noteMove: (guid: string, folderGuid: string | null) =>
    invoke<void>("note_move", { guid, folderGuid }),

  /** Create when the guid is empty, otherwise save changes. Returns Apple's id for it. */
  contactSave: (contact: Contact) => invoke<string>("contact_save", { contact }),
  contactDelete: (guid: string) => invoke<void>("contact_delete", { guid }),
  contactsList: (search: string | null) => invoke<Contact[]>("contacts_list", { search }),

  driveSyncFolder: (folderId: string) => invoke<number>("drive_sync_folder", { folderId }),
  driveChildren: (folderId: string) => invoke<DriveItem[]>("drive_children", { folderId }),
  driveItem: (drivewsid: string) => invoke<DriveItem | null>("drive_item", { drivewsid }),
  /** Saves to Downloads; returns the path it landed at. */
  driveDownload: (drivewsid: string) => invoke<string>("drive_download", { drivewsid }),
  /**
   * Put files from this machine into a Drive folder. Paths only — the bytes are read and
   * streamed in the backend, never carried through a message.
   *
   * Resolves with the files that did not go up, one line each, so a single refusal does not
   * read as the whole upload failing.
   */
  driveUpload: (folderId: string, paths: string[]) =>
    invoke<string[]>("drive_upload", { folderId, paths }),
  driveCreateFolder: (parentId: string, name: string) =>
    invoke<void>("drive_create_folder", { parentId, name }),
  driveRename: (drivewsid: string, name: string) =>
    invoke<void>("drive_rename", { drivewsid, name }),
  driveDelete: (drivewsid: string) => invoke<void>("drive_delete", { drivewsid }),
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
};

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

/** How often a reminder comes back. Frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. */
export type Repeat = { frequency: number; interval: number };

/** Anything omitted is left untouched; `null` clears a field. */
export type ReminderEdit = {
  title?: string;
  notes?: string | null;
  dueAt?: number | null;
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
export function photoSrc(c: Contact): string | null {
  return c.photoUrl ? convertFileSrc(`photo/${c.guid}`, "quince") : null;
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
