import { invoke } from "@tauri-apps/api/core";

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

  reminderLists: () => invoke<ReminderList[]>("reminder_lists"),
  remindersIn: (listGuid: string, includeDone: boolean) =>
    invoke<Reminder[]>("reminders_in", { listGuid, includeDone }),
  remindersDueBetween: (from: number, to: number) =>
    invoke<Reminder[]>("reminders_due_between", { from, to }),
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

  contactsList: (search: string | null) => invoke<Contact[]>("contacts_list", { search }),

  driveSyncFolder: (folderId: string) => invoke<number>("drive_sync_folder", { folderId }),
  driveChildren: (folderId: string) => invoke<DriveItem[]>("drive_children", { folderId }),
  driveItem: (drivewsid: string) => invoke<DriveItem | null>("drive_item", { drivewsid }),
};

/** Tauri rejects with a plain string from our commands; keep it readable either way. */
export type Calendar = {
  guid: string;
  title: string;
  colour: string | null;
  ctag: string | null;
  sortOrder: number;
  enabled: boolean;
};

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
  /** Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. null if it never repeats. */
  recurFrequency: number | null;
  recurInterval: number | null;
  etag: string | null;
};

export type NoteFolder = { guid: string; title: string; sortOrder: number };

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
  lastName: string | null;
  company: string | null;
  isCompany: boolean;
  birthday: string | null;
  notes: string | null;
  etag: string | null;
  methods: ContactMethod[];
};

/** What to show, whichever name fields a contact happens to have. */
export function displayName(c: Contact): string {
  const full = [c.firstName, c.lastName].filter(Boolean).join(" ").trim();
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

export const driveName = (item: DriveItem) =>
  item.extension && !isContainer(item) ? `${item.name}.${item.extension}` : item.name;

export function messageOf(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Try again.";
}
