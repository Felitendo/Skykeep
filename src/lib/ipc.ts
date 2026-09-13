import { invoke } from "@tauri-apps/api/core";

export type Status = { hasStoredSession: boolean; signedIn: boolean };
export type SignInResult = "complete" | "needs_two_factor";

export type TrustedPhone = { id: number; maskedNumber: string };
export type TwoFactorOptions = {
  trustedDeviceCount: number;
  phones: TrustedPhone[];
  tooManyCodesSent: boolean;
};

export const api = {
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

  calendarSync: (from: string, to: string, timezone: string) =>
    invoke<SyncSummary>("calendar_sync", { from, to, timezone }),
  calendarEvents: (from: number, to: number) =>
    invoke<CalendarEvent[]>("calendar_events", { from, to }),
  calendarList: () => invoke<Calendar[]>("calendar_list"),
  calendarLastSynced: () => invoke<number | null>("calendar_last_synced"),

  remindersSync: () => invoke<[number, number]>("reminders_sync"),
  reminderLists: () => invoke<ReminderList[]>("reminder_lists"),
  remindersIn: (listGuid: string, includeDone: boolean) =>
    invoke<Reminder[]>("reminders_in", { listGuid, includeDone }),
  remindersLastSynced: () => invoke<number | null>("reminders_last_synced"),
  reminderSetDone: (guid: string, listGuid: string, done: boolean) =>
    invoke<void>("reminder_set_done", { guid, listGuid, done }),
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

export type Reminder = {
  guid: string;
  listGuid: string;
  title: string;
  notes: string | null;
  dueAt: number | null;
  /** null while outstanding */
  completedAt: number | null;
  priority: number;
  etag: string | null;
};

export function messageOf(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong. Try again.";
}
