import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { Notice } from "../components/Notice";
import { Portal } from "../components/Portal";
import { TimeField } from "../components/TimeField";
import { ChevronLeft } from "lucide-react";
import {
  api,
  MOST_AT_ONCE,
  messageOf,
  type Reminder,
  type ReminderGroup,
  type ReminderList,
  type ReminderSection,
  type ReminderCounts,
  type SmartList,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";
import { Glyph, emblemGlyph, type GlyphName } from "../components/Glyph";

/** Apple's built-in views, in Apple's order, each with the glyph and system colour its app uses. */
const SMART: { kind: SmartList; title: string; glyph: GlyphName; colour: string }[] = [
  { kind: "today", title: "Today", glyph: "today", colour: "var(--sys-blue)" },
  { kind: "scheduled", title: "Scheduled", glyph: "scheduled", colour: "var(--sys-red)" },
  { kind: "all", title: "All", glyph: "all", colour: "var(--sys-gray)" },
  { kind: "flagged", title: "Flagged", glyph: "flagged", colour: "var(--sys-orange)" },
];

/** A list's own colour, or Apple's blue for one that has none. */
const colourOf = (list: ReminderList | undefined | null) =>
  list?.colour && /^#[0-9a-f]{6}$/i.test(list.colour) ? list.colour : "var(--sys-blue)";

/** Groups the viewer has folded shut, remembered on this computer. */
const FOLDED_KEY = "quince.reminders.folded";
function readFolded(): Set<string> {
  try {
    const raw = window.localStorage.getItem(FOLDED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

/** End of today in the viewer's own timezone — the interface knows it, the backend doesn't. */
function endOfToday(): number {
  const d = new Date();
  return Math.floor(
    new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime() / 1000,
  );
}

/**
 * How many days apart two moments are, counted on a calendar rather than in milliseconds.
 *
 * Dividing by 86,400,000 is wrong on the two days a year a clock moves: a day is then
 * twenty-three hours or twenty-five, and something due tomorrow evening comes out as today.
 * Counting dates is right wherever the viewer is, including the places that never change.
 */
function daysApart(from: Date, to: Date): number {
  const noon = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), 12).getTime();
  return Math.round((noon(to) - noon(from)) / 86_400_000);
}

/**
 * When a reminder is due, said the way Apple says it.
 *
 * `dated` means the heading above already names the day — under "Tomorrow", Apple writes
 * "9:30 PM" and not "Tomorrow 9:30 PM". Saying it twice is the sort of thing that makes a
 * screen read as almost-right.
 */
function dueLabel(seconds: number, dated = false, allDay = false): { text: string; overdue: boolean } {
  const due = new Date(seconds * 1000);
  const now = new Date();
  const days = daysApart(now, due);
  // A day has no time to show, and is overdue only once the day itself has passed.
  if (allDay) {
    if (dated || days === 0) return { text: dated ? "" : "Today", overdue: false };
    if (days === 1) return { text: "Tomorrow", overdue: false };
    return { text: due.toLocaleDateString(undefined, { day: "numeric", month: "short" }), overdue: days < 0 };
  }
  const time = due.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  if (dated) {
    return { text: time, overdue: days < 0 || (days === 0 && due.getTime() < now.getTime()) };
  }
  if (days < 0)
    return {
      text: due.toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
      }),
      overdue: true,
    };
  if (days === 0)
    return { text: `Today ${time}`, overdue: due.getTime() < now.getTime() };
  if (days === 1) return { text: `Tomorrow ${time}`, overdue: false };
  return {
    text: due.toLocaleDateString(undefined, { day: "numeric", month: "short" }),
    overdue: false,
  };
}

/** Apple's scale, and the marks its own app puts before a title. */
const PRIORITIES = [
  { value: 0, label: "None", mark: "" },
  { value: 1, label: "High", mark: "!!!" },
  { value: 5, label: "Medium", mark: "!!" },
  { value: 9, label: "Low", mark: "!" },
] as const;

const priorityMark = (priority: number) =>
  PRIORITIES.find((p) => p.value === priority)?.mark ?? "";

const REPEAT_UNITS = ["day", "week", "month", "year"] as const;

/** What Apple's own repeat menu offers. "never" is the absence of a rule, not a rule. */
const REPEATS = [
  { key: "never", label: "Never", repeat: null },
  { key: "0:1", label: "Every day", repeat: { frequency: 0, interval: 1 } },
  { key: "1:1", label: "Every week", repeat: { frequency: 1, interval: 1 } },
  { key: "1:2", label: "Every 2 weeks", repeat: { frequency: 1, interval: 2 } },
  { key: "2:1", label: "Every month", repeat: { frequency: 2, interval: 1 } },
  { key: "3:1", label: "Every year", repeat: { frequency: 3, interval: 1 } },
] as const;

/** Which choice a reminder's stored rule corresponds to, or a description of one we don't
 *  offer — Apple allows schedules this menu deliberately doesn't try to express. */
function repeatKey(r: Reminder): string {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return "never";
  const key = `${r.recurFrequency}:${r.recurInterval ?? 1}`;
  return REPEATS.some((option) => option.key === key) ? key : "custom";
}

/** "Every day", "Every 2 weeks" — or null when this happens only once. */
function repeatLabel(r: Reminder): string | null {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return null;
  const unit = REPEAT_UNITS[r.recurFrequency];
  if (!unit) return "Repeats";
  const every = r.recurInterval ?? 1;
  return every > 1 ? `Every ${every} ${unit}s` : `Every ${unit}`;
}

/**
 * The word Apple puts beside the repeat arrow: Daily, Weekly, Monthly, Yearly.
 *
 * The arrow alone says something repeats without saying how often, which is the one thing
 * worth knowing about it at a glance.
 */
function repeatWord(r: Reminder): string | null {
  if (r.recurFrequency === null || r.recurFrequency === undefined) return null;
  if ((r.recurInterval ?? 1) > 1) return repeatLabel(r);
  return ["Daily", "Weekly", "Monthly", "Yearly"][r.recurFrequency] ?? "Repeats";
}

/**
 * An all-day date as Apple keeps it is that day's midnight in UTC. Shown and grouped here at
 * the same day's local midnight — read as a moment, the 17th's UTC midnight is the evening of
 * the 16th west of Greenwich, and a day reminder would drift into the day before.
 */
const dayToLocal = (utc: number) => {
  const d = new Date(utc * 1000);
  return Math.floor(new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()).getTime() / 1000);
};
const dayToUtc = (value: string) => {
  const [y, m, d] = value.split("-").map(Number);
  return Math.floor(Date.UTC(y, m - 1, d) / 1000);
};
const asShown = (r: Reminder): Reminder =>
  r.allDay && r.dueAt !== null ? { ...r, dueAt: dayToLocal(r.dueAt) } : r;


type Selection =
  { type: "smart"; kind: SmartList } | { type: "list"; guid: string };

export function RemindersView() {
  const [lists, setLists] = useState<ReminderList[]>([]);
  /** The list whose Show List Info sheet is open. */
  const [infoFor, setInfoFor] = useState<ReminderList | null>(null);
  const [groups, setGroups] = useState<ReminderGroup[]>([]);
  const [folded, setFolded] = useState<Set<string>>(readFolded);
  const toggleFolded = useCallback((guid: string) => {
    setFolded((was) => {
      const next = new Set(was);
      if (next.has(guid)) next.delete(guid);
      else next.add(guid);
      try {
        window.localStorage.setItem(FOLDED_KEY, JSON.stringify([...next]));
      } catch {
        // Without storage the fold still works; it is forgotten on restart.
      }
      return next;
    });
  }, []);
  const [selection, setSelection] = useState<Selection>({
    type: "smart",
    kind: "today",
  });
  const [items, setItems] = useState<Reminder[]>([]);
  const [showDone, setShowDone] = useState(false);
  const [syncing, setSyncing] = useState(false);
  /** Every reminder with a write still on its way — a set, since several can be at once. */
  const [busy, setBusy] = useState<ReadonlySet<string>>(() => new Set());
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [newList, setNewList] = useState("");
  const [addingList, setAddingList] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { revision } = useSync();
  /** What each sidebar entry is holding. Apple puts a number on every one. */
  const [counts, setCounts] = useState<ReminderCounts | null>(null);
  /** The headings inside the list that is open, when a real list is open. */
  const [listSections, setListSections] = useState<ReminderSection[]>([]);
  /** A repeating reminder that was just ticked forward, and where it landed. */
  const [moved, setMoved] = useState<{ guid: string; text: string } | null>(
    null,
  );

  const inList = selection.type === "list";
  const listGuid = inList ? selection.guid : null;

  const readLists = useCallback(async () => {
    const [found, tally, folders] = await Promise.all([
      api.reminderLists(),
      api.reminderCounts(endOfToday()),
      // An older library without groups is still a working one.
      api.reminderGroups().catch(() => [] as ReminderGroup[]),
    ]);
    setLists(Array.isArray(found) ? found : []);
    setGroups(Array.isArray(folders) ? folders : []);
    setCounts(tally);
    setLoaded(true);
  }, []);

  useEffect(() => {
    if (selection.type !== "list") return setListSections([]);
    let cancelled = false;
    api
      .reminderSections(selection.guid)
      .then((found) => {
        if (!cancelled) setListSections(Array.isArray(found) ? found : []);
      })
      .catch(() => {
        // Without sections the list still shows every reminder, just without headings.
        if (!cancelled) setListSections([]);
      });
    return () => {
      cancelled = true;
    };
  }, [selection, revision]);

  /** What is typed in the search field. While it holds anything, the pane is the results. */
  const [query, setQuery] = useState("");
  const searching = query.trim().length > 0;
  /**
   * In a narrow window one column shows at a time, as on iPhone: the lists, or the list that
   * was picked, with a way back. Searching shows the results under the search field.
   */
  const [detail, setDetail] = useState(false);

  const readItems = useCallback(async (): Promise<Reminder[]> => {
    // Search reaches every list, as Apple's does, whatever is open in the sidebar.
    const found = searching
      ? await api.remindersSearch(query)
      : selection.type === "smart"
        ? await api.remindersSmart(selection.kind, endOfToday())
        : await api.remindersIn(selection.guid, showDone);
    const fresh = (Array.isArray(found) ? found : []).map(asShown);
    setItems(fresh);
    return fresh;
  }, [selection, showDone, query, searching]);

  // Both reads come from the library. `revision` ticks when the shared refresh finishes —
  // pressed, or on the timer — so a reminder ticked on the phone appears here without this
  // screen ever touching the network.
  useEffect(() => {
    let cancelled = false;
    readLists().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [readLists, revision]);

  useEffect(() => {
    readItems().catch((e) => setError(messageOf(e)));
  }, [readItems, revision]);

  useEffect(() => {
    if (!moved) return undefined;
    const timer = window.setTimeout(() => setMoved(null), 2600);
    return () => window.clearTimeout(timer);
  }, [moved]);

  const patch = (guid: string, change: Partial<Reminder>) => (rows: Reminder[]) =>
    rows.map((x) => (x.guid === guid ? { ...x, ...change } : x));

  /**
   * Every write goes through here. What it will do shows at once — a tick, a flag, a row
   * gone — the way the phone answers a tap; then it goes to Apple and the list is re-read,
   * so what stays on screen is what Apple kept. Waiting on the network before showing the
   * tap made every add, tick and delete feel a second behind. A refusal re-reads, which
   * puts the row back as it was, and says why.
   */
  const write = useCallback(
    async (
      reminder: Reminder,
      run: () => Promise<void>,
      predict?: (rows: Reminder[]) => Reminder[],
    ): Promise<Reminder[] | null> => {
      setBusy((was) => new Set(was).add(reminder.guid));
      setError(null);
      if (predict) setItems((rows) => predict(rows));
      try {
        await run();
        return await readItems();
      } catch (e) {
        setError(messageOf(e));
        readItems().catch(() => {});
        return null;
      } finally {
        setBusy((was) => {
          const next = new Set(was);
          next.delete(reminder.guid);
          return next;
        });
      }
    },
    [readItems],
  );

  /**
   * Ticking a repeating reminder never closes it — Apple rolls it to the next occurrence.
   * The row therefore comes back unticked with a later date, which looks exactly like
   * nothing happened, so the new date is called out for a moment.
   */
  const toggle = async (r: Reminder) => {
    const wasOpen = !r.completedAt;
    const fresh = await write(
      r,
      () => api.reminderSetDone(r.guid, r.listGuid, wasOpen),
      // A repeating one rolls on rather than closing, so only Apple's answer can show it.
      repeatLabel(r) === null
        ? patch(r.guid, { completedAt: wasOpen ? Math.floor(Date.now() / 1000) : null })
        : undefined,
    );
    if (!fresh || !wasOpen || repeatLabel(r) === null) return;
    const after = fresh.find((x) => x.guid === r.guid);
    if (after && after.dueAt !== null && after.dueAt !== r.dueAt) {
      setMoved({ guid: r.guid, text: dueLabel(after.dueAt).text });
    }
  };
  const flag = (r: Reminder) =>
    write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { flagged: !r.flagged }),
      patch(r.guid, { flagged: !r.flagged }),
    );
  const remove = (r: Reminder) =>
    write(
      r,
      () => api.reminderDelete(r.guid, r.listGuid),
      // A subtask goes with its parent, as on the phone.
      (rows) => rows.filter((x) => x.guid !== r.guid && x.parentGuid !== r.guid),
    );
  const rename = (r: Reminder, title: string) =>
    title.trim() && title.trim() !== r.title
      ? write(
          r,
          () => api.reminderEdit(r.guid, r.listGuid, { title: title.trim() }),
          patch(r.guid, { title: title.trim() }),
        )
      : Promise.resolve();
  /** A date, and whether it has a time — the phone's Date and Time switches. */
  const setDue = (r: Reminder, day: string, time: string | null) => {
    const allDay = Boolean(day) && time === null;
    const dueAt = !day ? null : allDay ? dayToUtc(day) : Math.floor(new Date(`${day}T${time}`).getTime() / 1000);
    return write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { dueAt, allDay }),
      patch(r.guid, { dueAt: allDay && dueAt !== null ? dayToLocal(dueAt) : dueAt, allDay }),
    );
  };
  const setNotes = (r: Reminder, notes: string) =>
    write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, { notes: notes.trim() || null }),
    );
  const setRepeat = (r: Reminder, key: string) => {
    const chosen = REPEATS.find((option) => option.key === key);
    if (!chosen) return Promise.resolve(null);
    return write(r, () => api.reminderSetRepeat(r.guid, r.listGuid, chosen.repeat));
  };
  const setPriority = (r: Reminder, priority: number) =>
    write(r, () => api.reminderEdit(r.guid, r.listGuid, { priority }));
  /** Nest a reminder under the one above it, the way Tab does in Apple's app. */
  const indent = (r: Reminder) => {
    const flat = threaded.map((t) => t.reminder);
    const index = flat.findIndex((x) => x.guid === r.guid);
    // Only under something above it, and never under another subtask: Apple nests one deep.
    const above = flat
      .slice(0, index)
      .reverse()
      .find((x) => !x.parentGuid);
    if (!above) return Promise.resolve(null);
    return write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { parentGuid: above.guid }),
      patch(r.guid, { parentGuid: above.guid }),
    );
  };

  const outdent = (r: Reminder) =>
    write(
      r,
      () => api.reminderEdit(r.guid, r.listGuid, { parentGuid: null }),
      patch(r.guid, { parentGuid: null }),
    );

  const moveToList = (r: Reminder, listGuid: string) =>
    listGuid === r.listGuid
      ? Promise.resolve(null)
      : write(r, () => api.reminderEdit(r.guid, r.listGuid, { listGuid }));

  /** List writes go to Apple first, then the whole set is re-read. */
  const listWrite = useCallback(
    async (run: () => Promise<void>) => {
      setSyncing(true);
      setError(null);
      try {
        await run();
        await readLists();
        await readItems();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setSyncing(false);
      }
    },
    [readLists, readItems],
  );

  /**
   * The first list on an account that has none.
   *
   * Named the way Apple names the one it makes for a new account, so the two accounts end up
   * looking the same rather than this one carrying a name only this app would have chosen.
   */
  async function makeFirstList() {
    await listWrite(() => api.reminderListCreate("Reminders"));
  }

  async function createList(e: React.FormEvent) {
    e.preventDefault();
    const title = newList.trim();
    if (!title) return;
    await listWrite(() => api.reminderListCreate(title));
    setNewList("");
    setAddingList(false);
  }

  /**
   * A smart list is a view, not somewhere to keep things, so adding from Today or All has
   * to file the reminder in a real list — as Apple's own app does.
   *
   * Which one was previously just "the first one", which is why everything landed in
   * Assignments with no explanation. Apple's default-list setting is a client preference it
   * does not publish anywhere this app can read, so rather than guess at it, the choice is
   * shown and it is yours: whichever list you last added to, changeable right there.
   */
  const [preferredList, setPreferredList] = useState<string | null>(() => {
    try {
      return window.localStorage.getItem("quince.reminders.defaultList");
    } catch {
      return null;
    }
  });

  const chooseList = useCallback((guid: string) => {
    setPreferredList(guid);
    try {
      window.localStorage.setItem("quince.reminders.defaultList", guid);
    } catch {
      // A browser with storage switched off still works; it just forgets the choice.
    }
  }, []);

  const targetListGuid =
    listGuid ??
    (preferredList && lists.some((l) => l.guid === preferredList)
      ? preferredList
      : (lists[0]?.guid ?? null));
  const targetList = lists.find((l) => l.guid === targetListGuid) ?? null;

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const title = draft.trim();
    if (!title || !targetListGuid) return;
    setSyncing(true);
    setError(null);
    // What the view implies, as on the phone: typed into Today or Scheduled it is due today,
    // into Flagged it is flagged — so it stays where it was typed, rather than vanishing.
    const kind = selection.type === "smart" ? selection.kind : null;
    const today = new Date();
    const pad2 = (n: number) => String(n).padStart(2, "0");
    const todayKey = `${today.getFullYear()}-${pad2(today.getMonth() + 1)}-${pad2(today.getDate())}`;
    const implied =
      kind === "today" || kind === "scheduled"
        ? { dueAt: dayToUtc(todayKey), allDay: true }
        : kind === "flagged"
          ? { flagged: true }
          : {};
    // The row is there the moment Return is pressed, and the field is ready for the next.
    const standIn = `pending:${Date.now()}`;
    setDraft("");
    const shownHere = inList || kind === "today" || kind === "scheduled" || kind === "flagged" || kind === "all";
    if (shownHere) setItems((rows) => [
      ...rows,
      {
        guid: standIn, listGuid: targetListGuid, title, notes: null,
        dueAt: implied.dueAt !== undefined ? dayToLocal(implied.dueAt) : null,
        allDay: implied.allDay ?? false,
        completedAt: null, priority: 0, flagged: implied.flagged ?? false, parentGuid: null,
        recurFrequency: null, recurInterval: null, etag: null,
      } as Reminder,
    ]);
    try {
      await api.reminderCreate(targetListGuid, title, implied);
      await readItems();
    } catch (err) {
      setItems((rows) => rows.filter((r) => r.guid !== standIn));
      setDraft((d) => d || title);
      setError(messageOf(err));
    } finally {
      setSyncing(false);
    }
  }

  const heading = inList
    ? (lists.find((l) => l.guid === listGuid)?.title ?? "Reminders")
    : (SMART.find((s) => s.kind === (selection as { kind: SmartList }).kind)
        ?.title ?? "Reminders");
  const outstanding = items.filter((r) => !r.completedAt).length;

  /**
   * How many this view actually holds, when that is more than it was handed.
   *
   * The counts come from the library rather than from the list on screen, so they stay true
   * even where the list is only the first thousand of them. Saying nothing would be the
   * dishonest part — a list that quietly stops is worse than one that says where it stopped.
   */
  const heldBack = (() => {
    if (items.length < MOST_AT_ONCE || !counts) return 0;
    const total = inList
      ? (counts.lists.find(([guid]) => guid === listGuid)?.[1] ?? 0)
      : counts[(selection as { kind: SmartList }).kind];
    return Math.max(0, total - items.length);
  })();

  /**
   * Reminders in Apple's order, with each subtask directly beneath its parent.
   *
   * The library returns a flat list sorted by date and priority, which is right for the
   * top level and wrong for what hangs off it: a subtask sorted away from its parent reads
   * as an unrelated fragment. This re-threads them without re-sorting either level.
   */
  const threaded = useMemo(() => {
    const children = new Map<string, Reminder[]>();
    for (const r of items) {
      if (!r.parentGuid) continue;
      const bucket = children.get(r.parentGuid);
      if (bucket) bucket.push(r);
      else children.set(r.parentGuid, [r]);
    }
    const known = new Set(items.map((r) => r.guid));
    const out: { reminder: Reminder; depth: number }[] = [];
    // Open reminders first, in their list's own order; completed ones beneath them, the most
    // recently completed first, as Reminders shows them. In the list's stored order they came
    // out scattered among the open ones, in no order a person could follow.
    const open = items.filter((r) => !r.completedAt);
    const done = items
      .filter((r) => r.completedAt)
      .sort((a, b) => (b.completedAt ?? 0) - (a.completedAt ?? 0));
    for (const r of [...open, ...done]) {
      // A subtask whose parent is not in this view — filtered out, or in another list —
      // has to appear at the top level rather than vanish.
      if (r.parentGuid && known.has(r.parentGuid)) continue;
      out.push({ reminder: r, depth: 0 });
      for (const child of children.get(r.guid) ?? []) {
        out.push({ reminder: child, depth: 1 });
      }
    }
    return out;
  }, [items]);
  /**
   * The rows arranged the way Apple arranges them.
   *
   * Every one of Apple's smart lists is grouped, and this app showed each of them as one flat
   * run of reminders — which is most of why the screen felt unlike the real thing even when
   * the same reminders were on it. Today breaks by time of day, Scheduled by date, and the
   * ones that gather from everywhere break by the list each reminder came from.
   *
   * A real list is left alone: Apple does not group inside one either.
   */
  /**
   * Whether a row should name the list it came from.
   *
   * Apple shows it wherever the grouping does not already say so: in Today and Scheduled,
   * which gather from everywhere, but not under a list heading where it would repeat.
   */
  const showListName =
    !searching &&
    !inList &&
    (selection as { kind: SmartList }).kind !== "all" &&
    (selection as { kind: SmartList }).kind !== "flagged";

  const sections = useMemo(() => {
    if (searching) {
      // Results gather from every list, so the list each came from is the grouping — as in
      // All.
      const byList = new Map<string, typeof threaded>();
      for (const row of threaded) {
        byList.set(row.reminder.listGuid, [...(byList.get(row.reminder.listGuid) ?? []), row]);
      }
      return [...byList.entries()].map(([guid, rows]) => {
        const list = lists.find((l) => l.guid === guid);
        return {
          key: guid,
          heading: list?.title ?? "Reminders",
          colour: list?.colour ?? null,
          dated: false,
          rows,
        };
      });
    }
    if (inList) {
      // Inside a real list the grouping is the list's own sections, which is how Apple shows
      // it: reminders outside any section first with no heading, then each section in the
      // order it was made. A reminder placed in a section that is not here — one deleted
      // since — falls back to the loose ones rather than vanishing.
      const known = new Set(listSections.map((s) => s.guid));
      const loose = threaded.filter(
        (row) => !row.reminder.sectionGuid || !known.has(row.reminder.sectionGuid)
      );
      return [
        { key: "loose", heading: null, colour: null, dated: false, rows: loose },
        ...listSections.map((section) => ({
          key: section.guid,
          heading: section.title,
          colour: null,
          dated: false,
          rows: threaded.filter((row) => row.reminder.sectionGuid === section.guid),
        })),
      ];
    }

    const kind = (selection as { kind: SmartList }).kind;
    const now = new Date();
    const listOf = (guid: string) => lists.find((l) => l.guid === guid);

    if (kind === "today") {
      // Apple's own buckets, each shown only when something is in it: empty "Morning" and
      // "Afternoon" headings above the one reminder due tonight read as missing reminders.
      const buckets: { key: string; heading: string | null; rows: typeof threaded }[] = [
        { key: "loose", heading: null, rows: [] },
        { key: "morning", heading: "Morning", rows: [] },
        { key: "afternoon", heading: "Afternoon", rows: [] },
        { key: "tonight", heading: "Tonight", rows: [] },
      ];
      for (const row of threaded) {
        const due = row.reminder.dueAt;
        // Anything overdue, or with no time at all, sits above the buckets — which is where
        // Apple puts it.
        if (due === null || daysApart(now, new Date(due * 1000)) < 0) {
          buckets[0].rows.push(row);
          continue;
        }
        const hour = new Date(due * 1000).getHours();
        buckets[hour < 12 ? 1 : hour < 17 ? 2 : 3].rows.push(row);
      }
      // Each part of the day in time order, as Today reads; the list's own order put 9:30
      // above 8:30. A subtask keeps its place under its parent.
      for (const b of buckets.slice(1)) {
        if (b.rows.some((r) => r.depth > 0)) continue;
        b.rows.sort((x, y) => (x.reminder.dueAt ?? 0) - (y.reminder.dueAt ?? 0));
      }
      return buckets
        .filter((b) => b.rows.length > 0)
        // The buckets are all today, so a row under one needs only its time. The loose rows
        // above are overdue or undated and still have to say which day they mean.
        .map((b) => ({ ...b, colour: null, dated: b.heading !== null }));
    }

    if (kind === "scheduled") {
      const byDay = new Map<number, typeof threaded>();
      const overdue: typeof threaded = [];
      for (const row of threaded) {
        const due = row.reminder.dueAt;
        if (due === null) continue;
        const days = daysApart(now, new Date(due * 1000));
        if (days < 0) overdue.push(row);
        else {
          const bucket = byDay.get(days);
          if (bucket) bucket.push(row);
          else byDay.set(days, [row]);
        }
      }
      const out = overdue.length
        ? [{ key: "overdue", heading: "Overdue", colour: null, dated: false, rows: overdue }]
        : [];
      for (const days of [...byDay.keys()].sort((a, b) => a - b)) {
        const when = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
        out.push({
          key: `d${days}`,
          heading:
            days === 0
              ? "Today"
              : days === 1
                ? "Tomorrow"
                : when.toLocaleDateString(undefined, {
                    weekday: "short",
                    day: "numeric",
                    month: "short",
                  }),
          colour: null,
          dated: true,
          rows: byDay.get(days) ?? [],
        });
      }
      return out;
    }

    // All and Flagged gather from everywhere, so the list each came from is the grouping.
    const byList = new Map<string, typeof threaded>();
    for (const row of threaded) {
      const bucket = byList.get(row.reminder.listGuid);
      if (bucket) bucket.push(row);
      else byList.set(row.reminder.listGuid, [row]);
    }
    return [...byList.entries()]
      .sort(
        ([a], [b]) =>
          (listOf(a)?.sortOrder ?? 0) - (listOf(b)?.sortOrder ?? 0) ||
          (listOf(a)?.title ?? "").localeCompare(listOf(b)?.title ?? ""),
      )
      .map(([guid, rows]) => ({
        key: guid,
        heading: listOf(guid)?.title ?? "Reminders",
        colour: listOf(guid)?.colour ?? null,
        // Grouped by list rather than by day, so a row still has to name its day.
        dated: false,
        rows,
      }));
  }, [threaded, inList, selection, lists, listSections, searching]);

  const countOf = (guid: string) => counts?.lists.find(([g]) => g === guid)?.[1] ?? 0;
  const openList = inList ? (lists.find((l) => l.guid === listGuid) ?? null) : null;
  const smartOpen =
    selection.type === "smart" ? SMART.find((x) => x.kind === selection.kind) : undefined;
  /** The colour the open view is drawn in: its list's, or its built-in view's. */
  const accent = searching
    ? "var(--text)"
    : openList
      ? colourOf(openList)
      : (smartOpen?.colour ?? "var(--sys-blue)");

  /**
   * The sidebar in Apple's order: a list or group at the top level wherever it was walked,
   * with a group's lists beneath it. Pinned lists are tiles instead, as on the phone.
   */
  const pinned = lists.filter((l) => l.pinned);
  const topLevel = [
    ...lists
      .filter((l) => !l.pinned && !(l.groupGuid && groups.some((g) => g.guid === l.groupGuid)))
      .map((list) => ({ kind: "list" as const, order: list.sortOrder, list })),
    ...groups.map((group) => ({ kind: "group" as const, order: group.sortOrder, group })),
  ].sort((a, b) => a.order - b.order);
  const inGroup = (guid: string) => lists.filter((l) => !l.pinned && l.groupGuid === guid);

  const listRow = (list: ReminderList, nested: boolean) => (
    <div className={`rx-list-row${nested ? " nested" : ""}`} key={list.guid}>
      {renaming === list.guid ? (
        <input
          className="rx-rename"
          defaultValue={list.title}
          autoFocus
          aria-label={`Rename ${list.title}`}
          onBlur={(e) => {
            const title = e.target.value.trim();
            setRenaming(null);
            if (title && title !== list.title) {
              void listWrite(() => api.reminderListRename(list.guid, title));
            }
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
            if (e.key === "Escape") {
              e.currentTarget.value = list.title;
              e.currentTarget.blur();
            }
          }}
        />
      ) : (
        <>
          <button
            className="rx-list"
            aria-current={listGuid === list.guid}
            onClick={() => {
              setQuery("");
              setSelection({ type: "list", guid: list.guid });
              setDetail(true);
            }}
            onDoubleClick={() => setRenaming(list.guid)}
          >
            <span className="rx-badge" style={{ background: colourOf(list) }}>
              <Glyph name={emblemGlyph(list.emblem)} />
            </span>
            <span className="rx-list-name">{list.title}</span>
            <span className="rx-list-count">{countOf(list.guid)}</span>
          </button>
          <span className="rx-list-actions">
            <button
              className="rx-icon-button"
              aria-label={`Show List Info for ${list.title}`}
              title="Show List Info"
              onClick={() => setInfoFor(list)}
            >
              ⓘ
            </button>
            <button
              className="rx-icon-button"
              aria-label={`Rename ${list.title}`}
              title="Rename"
              onClick={() => setRenaming(list.guid)}
            >
              ✎
            </button>
            <button
              className="rx-icon-button danger"
              aria-label={`Delete ${list.title}`}
              title="Delete"
              onClick={() => {
                // Deleting a list takes its reminders, so ask before doing it.
                if (window.confirm(`Delete "${list.title}" and everything in it?`)) {
                  void listWrite(() => api.reminderListDelete(list.guid));
                }
              }}
            >
              ×
            </button>
          </span>
        </>
      )}
    </div>
  );

  const tile = (
    key: string,
    title: string,
    glyph: GlyphName,
    colour: string,
    count: number,
    current: boolean,
    choose: () => void,
  ) => (
    <button
      key={key}
      className="rx-tile"
      aria-current={current}
      aria-label={`${title}, ${count} reminders`}
      style={{ "--tile": colour } as React.CSSProperties}
      onClick={() => {
        setQuery("");
        choose();
        setDetail(true);
      }}
    >
      <span className="rx-tile-glyph">
        <Glyph name={glyph} />
      </span>
      <span className="rx-tile-count">{count}</span>
      <span className="rx-tile-name">{title}</span>
    </button>
  );

  /** Where a reminder typed at the foot of the view is filed, when that is a choice. */
  const addRow = targetListGuid && !searching && (
    <form className="rx-add" onSubmit={add}>
      <span className="rx-tick ghost" aria-hidden="true">
        <Glyph name="plus" />
      </span>
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        placeholder="New Reminder"
        aria-label={
          inList ? `Add a reminder to ${heading}` : `Add a reminder to ${targetList?.title ?? "a list"}`
        }
      />
      {/* From a built-in view, where it lands is a real choice, so it is shown — but only
          once something is being typed, not as a permanent control on the screen. */}
      {!inList && targetList && lists.length > 1 && draft.trim() && (
        <Picker
          className="rx-add-target"
          label="List to add to"
          value={targetListGuid ?? ""}
          onChange={chooseList}
          options={lists.map((l) => ({ value: l.guid, label: l.title }))}
        />
      )}
    </form>
  );

  return (
    <div
      className={`rx${detail && !searching ? " is-detail" : ""}${searching ? " is-searching" : ""}`}
      style={{ "--accent": accent } as React.CSSProperties}
    >
      <aside className="rx-side" aria-label="Reminder lists">
        <label className="rx-search">
          <Glyph name="search" />
          <input
            type="search"
            placeholder="Search"
            aria-label="Search reminders"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setQuery("");
            }}
          />
        </label>

        <div className="rx-tiles">
          {SMART.map((smart) =>
            tile(
              smart.kind,
              smart.title,
              smart.glyph,
              smart.colour,
              counts?.[smart.kind] ?? 0,
              !searching && selection.type === "smart" && selection.kind === smart.kind,
              () => setSelection({ type: "smart", kind: smart.kind }),
            ),
          )}
          {pinned.map((list) =>
            tile(
              list.guid,
              list.title,
              emblemGlyph(list.emblem),
              colourOf(list),
              countOf(list.guid),
              !searching && listGuid === list.guid,
              () => setSelection({ type: "list", guid: list.guid }),
            ),
          )}
        </div>

        <p className="rx-side-heading">My Lists</p>
        <div className="rx-lists">
          {topLevel.map((entry) =>
            entry.kind === "list" ? (
              listRow(entry.list, false)
            ) : (
              <div className="rx-group" key={entry.group.guid}>
                <button
                  className="rx-group-row"
                  aria-expanded={!folded.has(entry.group.guid)}
                  onClick={() => toggleFolded(entry.group.guid)}
                >
                  <span className="rx-group-glyph">
                    <Glyph name="folder" />
                  </span>
                  <span className="rx-list-name">{entry.group.title}</span>
                  <span className="rx-chevron">
                    <Glyph name="chevron" />
                  </span>
                </button>
                {!folded.has(entry.group.guid) &&
                  inGroup(entry.group.guid).map((list) => listRow(list, true))}
              </div>
            ),
          )}
        </div>
        {loaded && lists.length === 0 && <p className="rx-side-empty">No lists yet</p>}

        {addingList ? (
          <form className="rx-new-list" onSubmit={createList}>
            <input
              className="rx-rename"
              value={newList}
              onChange={(e) => setNewList(e.target.value)}
              placeholder="List name"
              aria-label="New list name"
              autoFocus
              onBlur={() => !newList.trim() && setAddingList(false)}
            />
          </form>
        ) : (
          <button className="rx-add-list" onClick={() => setAddingList(true)}>
            <Glyph name="plus" />
            Add List
          </button>
        )}
      </aside>

      <section className="rx-main">
        <button className="compact-back" onClick={() => setDetail(false)}>
          <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
          <span>Lists</span>
        </button>
        <header className="rx-head">
          <h1>{searching ? `Results for “${query.trim()}”` : heading}</h1>
          <span className="rx-head-count" aria-label={`${outstanding} to do`}>
            {outstanding}
          </span>
        </header>
        {inList && (
          <div className="rx-subhead">
            <button className="rx-link" onClick={() => setShowDone((v) => !v)}>
              {showDone ? "Hide Completed" : "Show Completed"}
            </button>
          </div>
        )}

        {error && (
          <Notice message={error} onDismiss={() => setError(null)} />
        )}

        {/* An account with no lists: a reminder cannot exist outside one, so the one thing
            that helps is offered instead of an add field with nowhere to go. */}
        {lists.length === 0 && loaded && (
          <div className="rem-nolists">
            <p className="rem-nolists-line">Reminders live in lists, and there aren’t any yet.</p>
            <button className="primary" onClick={() => void makeFirstList()} disabled={syncing}>
              Create a list
            </button>
          </div>
        )}

        {heldBack > 0 && (
          <p className="rx-capped" role="status">
            Showing the first {MOST_AT_ONCE.toLocaleString()} — {heldBack.toLocaleString()} more
            are in this list.
          </p>
        )}

        {sections.map((section) => (
          <Fragment key={section.key}>
            {section.heading && (
              <h2
                className={`rx-section${section.dated ? " is-quiet" : ""}`}
                style={section.colour ? { color: section.colour } : undefined}
              >
                {section.heading}
              </h2>
            )}
            <ul className="rx-rows">
              {section.rows.map(({ reminder, depth }) => {
                const due =
                  reminder.dueAt === null ? null : dueLabel(reminder.dueAt, section.dated, reminder.allDay);
                const isOpen = openGuid === reminder.guid;
                const repeat = repeatLabel(reminder);
                const justMoved = moved?.guid === reminder.guid ? moved.text : null;
                const home = lists.find((l) => l.guid === reminder.listGuid);
                const mark = priorityMark(reminder.priority);
                return (
                  <li
                    key={reminder.guid}
                    className={`rx-row${reminder.completedAt ? " is-done" : ""}${
                      depth > 0 ? " is-subtask" : ""
                    }${isOpen ? " is-open" : ""}`}
                  >
                    <button
                      className="rx-tick"
                      role="checkbox"
                      aria-checked={Boolean(reminder.completedAt)}
                      aria-label={reminder.title}
                      disabled={busy.has(reminder.guid)}
                      style={{ "--tick": colourOf(home) } as React.CSSProperties}
                      onClick={() => toggle(reminder)}
                    />

                    <div className="rx-text">
                      <div className="rx-title-row">
                        {mark && (
                          <span
                            className="rx-priority"
                            role="img"
                            aria-label={`${
                              PRIORITIES.find((p) => p.value === reminder.priority)?.label
                            } priority`}
                          >
                            {mark}
                          </span>
                        )}
                        <input
                          className="rx-title"
                          defaultValue={reminder.title}
                          key={`${reminder.guid}-${reminder.title}`}
                          aria-label={`Rename ${reminder.title}`}
                          disabled={busy.has(reminder.guid)}
                          onBlur={(e) => void rename(reminder, e.target.value)}
                          onKeyDown={(e) => {
                            // As in Reminders on a Mac: Return finishes it and starts the next,
                            // Tab tucks it under the one above, Shift-Tab brings it back out.
                            if (e.key === "Enter") {
                              e.preventDefault();
                              e.currentTarget.blur();
                              window.setTimeout(() => document.querySelector<HTMLInputElement>(".rx-add input")?.focus(), 0);
                            } else if (e.key === "Escape") {
                              e.currentTarget.value = reminder.title;
                              e.currentTarget.blur();
                            } else if (e.key === "Tab" && inList) {
                              e.preventDefault();
                              const typed = e.currentTarget.value;
                              void rename(reminder, typed);
                              void (e.shiftKey ? (reminder.parentGuid ? outdent(reminder) : null) : (reminder.parentGuid ? null : indent(reminder)));
                            }
                          }}
                        />
                      </div>
                      {reminder.notes && !isOpen && (
                        <span className="rx-notes">{reminder.notes}</span>
                      )}
                      {(due || repeat || justMoved || (showListName && home)) && (
                        <span className="rx-meta">
                          {justMoved ? (
                            <span className="rx-moved" role="status">
                              Moved to {justMoved}
                            </span>
                          ) : (
                            <>
                              {showListName && (
                                <span className="rx-from">{home?.title ?? "Reminders"}</span>
                              )}
                              {due && (
                                <span className={`rx-due${due.overdue ? " overdue" : ""}`}>
                                  {due.text}
                                </span>
                              )}
                              {repeat && (
                                <span className="rx-repeat" title={repeat}>
                                  <span aria-hidden="true">↻</span> {repeatWord(reminder)}
                                  <span className="visually-hidden">{repeat}</span>
                                </span>
                              )}
                            </>
                          )}
                        </span>
                      )}

                      {isOpen && (
                        <div className="rx-detail">
<DueField reminder={reminder} onChange={(day, time) => void setDue(reminder, day, time)} />
                          <div className="rx-field">
                            <span>Repeat</span>
                            <Picker
                              label={`Repeat for ${reminder.title}`}
                              value={repeatKey(reminder)}
                              disabled={busy.has(reminder.guid) || reminder.dueAt === null}
                              onChange={(key) => void setRepeat(reminder, key)}
                              options={
                                repeatKey(reminder) === "custom"
                                  ? [
                                      // A schedule this menu doesn't express must still be
                                      // shown, and choosing anything else would replace it.
                                      { value: "custom", label: repeat ?? "Custom" },
                                      ...REPEATS.map((o) => ({ value: o.key, label: o.label })),
                                    ]
                                  : REPEATS.map((o) => ({ value: o.key, label: o.label }))
                              }
                            />
                          </div>
                          {reminder.dueAt === null && (
                            <p className="rx-hint">Give this a date before asking it to repeat.</p>
                          )}
                          <div className="rx-field">
                            <span>Priority</span>
                            <Picker
                              label={`Priority for ${reminder.title}`}
                              value={String(reminder.priority)}
                              disabled={busy.has(reminder.guid)}
                              onChange={(v) => void setPriority(reminder, Number(v))}
                              options={PRIORITIES.map((p) => ({
                                value: String(p.value),
                                label: p.label,
                              }))}
                            />
                          </div>
                          <div className="rx-field">
                            <span>List</span>
                            <Picker
                              label={`List for ${reminder.title}`}
                              value={reminder.listGuid}
                              disabled={busy.has(reminder.guid)}
                              onChange={(guid) => void moveToList(reminder, guid)}
                              options={lists.map((l) => ({ value: l.guid, label: l.title }))}
                            />
                          </div>
                          <label className="rx-field">
                            <span>Notes</span>
                            <textarea
                              defaultValue={reminder.notes ?? ""}
                              rows={2}
                              onBlur={(e) => void setNotes(reminder, e.target.value)}
                            />
                          </label>
                          <div className="rx-detail-actions">
                            {depth > 0 ? (
                              <button
                                className="rx-link"
                                disabled={busy.has(reminder.guid)}
                                onClick={() => void outdent(reminder)}
                              >
                                Move Out of Subtasks
                              </button>
                            ) : (
                              <button
                                className="rx-link"
                                disabled={busy.has(reminder.guid)}
                                onClick={() => void indent(reminder)}
                              >
                                Make Subtask
                              </button>
                            )}
                            <button
                              className="rx-link danger"
                              aria-label={`Delete ${reminder.title}`}
                              disabled={busy.has(reminder.guid)}
                              onClick={() => remove(reminder)}
                            >
                              Delete
                            </button>
                          </div>
                        </div>
                      )}
                    </div>

                    <span className="rx-trailing">
                      <button
                        className="rx-icon-button"
                        aria-label={
                          isOpen ? `Close details for ${reminder.title}` : `Details for ${reminder.title}`
                        }
                        aria-expanded={isOpen}
                        onClick={() => setOpenGuid(isOpen ? null : reminder.guid)}
                      >
                        <Glyph name="info" />
                      </button>
                      <button
                        className={`rx-icon-button rx-flag${reminder.flagged ? " on" : ""}`}
                        aria-label={
                          reminder.flagged ? `Unflag ${reminder.title}` : `Flag ${reminder.title}`
                        }
                        aria-pressed={reminder.flagged}
                        disabled={busy.has(reminder.guid)}
                        onClick={() => flag(reminder)}
                      >
                        <Glyph name="flagged" />
                      </button>
                    </span>
                  </li>
                );
              })}
            </ul>
          </Fragment>
        ))}

        {addRow}

        {loaded && items.length === 0 && !error && lists.length > 0 && (
          <p className="rx-empty">
            {searching
              ? "No reminders match."
              : inList
                ? showDone
                  ? "Nothing in this list."
                  : "All done here."
                : "No Reminders"}
          </p>
        )}
      </section>
      {infoFor && (
        <ListInfo
          list={infoFor}
          onClose={() => setInfoFor(null)}
          onDone={(title, colour) => {
            const list = infoFor;
            setInfoFor(null);
            // Shown at once; Apple's answer confirms or puts it back.
            setLists((ls) => ls.map((l) => (l.guid === list.guid
              ? { ...l, title: title || l.title, colour: colour ? LIST_COLOURS.find((c) => c.key === colour)!.hex : l.colour }
              : l)));
            void listWrite(async () => {
              if (title && title !== list.title) await api.reminderListRename(list.guid, title);
              if (colour) await api.reminderListColour(list.guid, colour);
            });
          }}
        />
      )}
    </div>
  );
}

/** Apple's twelve list colours, in its picker's order, keyed as CloudKit names them. */
const LIST_COLOURS = [
  { key: "red", hex: "#FF3B30" }, { key: "orange", hex: "#FF9500" }, { key: "yellow", hex: "#FFCC00" },
  { key: "green", hex: "#34C759" }, { key: "lightBlue", hex: "#51AAF2" }, { key: "blue", hex: "#007AFF" },
  { key: "indigo", hex: "#5856D6" }, { key: "pink", hex: "#EA426A" }, { key: "purple", hex: "#C077DB" },
  { key: "brown", hex: "#A2845E" }, { key: "gray", hex: "#5B6770" }, { key: "rose", hex: "#D9A69F" },
] as const;

/** Show List Info, as icloud.com's: the list's mark, its name, and Apple's colours. */
function ListInfo({ list, onClose, onDone }: {
  list: ReminderList;
  onClose: () => void;
  onDone: (title: string, colour: string | null) => void;
}) {
  const [title, setTitle] = useState(list.title);
  const current = LIST_COLOURS.find((c) => c.hex.toLowerCase() === (list.colour ?? "").toLowerCase())?.key ?? null;
  const [colour, setColour] = useState<string | null>(current);
  const shown = LIST_COLOURS.find((c) => c.key === colour)?.hex ?? list.colour ?? "#007AFF";
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const done = () => onDone(title.trim(), colour !== current ? colour : null);
  return (
    <Portal>
    <div className="modal-veil" onClick={onClose}>
      <div className="modal-sheet rx-info" role="dialog" aria-label={`\u201c${list.title}\u201d Info`} onClick={(e) => e.stopPropagation()}>
        <h2>“{list.title}” Info</h2>
        <span className="rx-info-mark" style={{ background: shown }} aria-hidden="true">
          <Glyph name="list" />
        </span>
        <input
          className="rx-info-name"
          value={title}
          aria-label="List name"
          style={{ color: shown }}
          autoFocus
          onChange={(e) => setTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && done()}
        />
        <div className="rx-info-colours" role="radiogroup" aria-label="Colour">
          {LIST_COLOURS.map((c) => (
            <button
              key={c.key}
              role="radio"
              aria-checked={colour === c.key}
              aria-label={c.key}
              className={`rx-info-swatch${colour === c.key ? " is-on" : ""}`}
              style={{ background: c.hex }}
              onClick={() => setColour(c.key)}
            />
          ))}
        </div>
        <div className="rx-info-actions">
          <button className="quiet" onClick={onClose}>Cancel</button>
          <button className="primary" onClick={done} disabled={!title.trim()}>Done</button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

/**
 * The phone's date controls: a date, and a Time switch. Off, the reminder is due on the
 * day — all day, no alert; on, at the time chosen. Each change is saved when it is made.
 */
function DueField({ reminder, onChange }: { reminder: Reminder; onChange: (day: string, time: string | null) => void }) {
  const at = reminder.dueAt === null ? null : new Date(reminder.dueAt * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = at ? `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}` : "";
  const time = at && !reminder.allDay ? `${pad(at.getHours())}:${pad(at.getMinutes())}` : null;
  return (
    <div className="rx-field">
      <span>Date</span>
      <div className="rx-due">
        <input
          type="date"
          className="rx-due-day"
          aria-label={`Date for ${reminder.title}`}
          defaultValue={day}
          key={`d${reminder.dueAt}`}
          // A new date keeps the time already set; a first date is a day, as on the phone.
          onBlur={(e) => e.target.value !== day && onChange(e.target.value, e.target.value ? time : null)}
        />
        <label className="rx-switch">
          <input
            type="checkbox"
            role="switch"
            aria-label={`Time for ${reminder.title}`}
            checked={time !== null}
            disabled={!day}
            onChange={(e) => onChange(day, e.target.checked ? "09:00" : null)}
          />
          <span className="rx-switch-track" aria-hidden="true" />
          <span>Time</span>
        </label>
        {time !== null && at && (
          <TimeField
            label={`Time of day for ${reminder.title}`}
            hours={at.getHours()}
            minutes={at.getMinutes()}
            onChange={(h, m) => onChange(day, `${pad(h)}:${pad(m)}`)}
          />
        )}
      </div>
    </div>
  );
}
