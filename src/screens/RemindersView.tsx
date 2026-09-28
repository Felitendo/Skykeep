import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  MOST_AT_ONCE,
  messageOf,
  type Reminder,
  type ReminderList,
  type ReminderSection,
  type ReminderCounts,
  type SmartList,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";
import { Glyph, type GlyphName } from "../components/Glyph";

/** Apple's built-in views, which is how most people enter Reminders. */
/** Apple's built-in views, in Apple's order, each with the glyph and colour its app uses. */
const SMART: { kind: SmartList; title: string; glyph: GlyphName; colour: string }[] = [
  { kind: "today", title: "Today", glyph: "today", colour: "#0a84ff" },
  { kind: "scheduled", title: "Scheduled", glyph: "scheduled", colour: "#ff453a" },
  { kind: "all", title: "All", glyph: "all", colour: "#8e8e93" },
  { kind: "flagged", title: "Flagged", glyph: "flagged", colour: "#ff9f0a" },
];

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
function dueLabel(seconds: number, dated = false): { text: string; overdue: boolean } {
  const due = new Date(seconds * 1000);
  const now = new Date();
  const days = daysApart(now, due);
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

/** `YYYY-MM-DDTHH:mm` for a datetime-local input, in local time. */
function toInputValue(seconds: number | null): string {
  if (seconds === null) return "";
  const d = new Date(seconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

type Selection =
  { type: "smart"; kind: SmartList } | { type: "list"; guid: string };

export function RemindersView() {
  const [lists, setLists] = useState<ReminderList[]>([]);
  const [selection, setSelection] = useState<Selection>({
    type: "smart",
    kind: "today",
  });
  const [items, setItems] = useState<Reminder[]>([]);
  const [showDone, setShowDone] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [busyGuid, setBusyGuid] = useState<string | null>(null);
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
    const [found, tally] = await Promise.all([
      api.reminderLists(),
      api.reminderCounts(endOfToday()),
    ]);
    setLists(Array.isArray(found) ? found : []);
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

  const readItems = useCallback(async (): Promise<Reminder[]> => {
    // Search reaches every list, as Apple's does, whatever is open in the sidebar.
    const found = searching
      ? await api.remindersSearch(query)
      : selection.type === "smart"
        ? await api.remindersSmart(selection.kind, endOfToday())
        : await api.remindersIn(selection.guid, showDone);
    const fresh = Array.isArray(found) ? found : [];
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

  /** Every write goes through here: to Apple first, then re-read, so nothing on screen is a guess. */
  const write = useCallback(
    async (
      reminder: Reminder,
      run: () => Promise<void>,
    ): Promise<Reminder[] | null> => {
      setBusyGuid(reminder.guid);
      setError(null);
      try {
        await run();
        return await readItems();
      } catch (e) {
        setError(messageOf(e));
        return null;
      } finally {
        setBusyGuid(null);
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
    const fresh = await write(r, () =>
      api.reminderSetDone(r.guid, r.listGuid, wasOpen),
    );
    if (!fresh || !wasOpen || repeatLabel(r) === null) return;
    const after = fresh.find((x) => x.guid === r.guid);
    if (after && after.dueAt !== null && after.dueAt !== r.dueAt) {
      setMoved({ guid: r.guid, text: dueLabel(after.dueAt).text });
    }
  };
  const flag = (r: Reminder) =>
    write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, { flagged: !r.flagged }),
    );
  const remove = (r: Reminder) =>
    write(r, () => api.reminderDelete(r.guid, r.listGuid));
  const rename = (r: Reminder, title: string) =>
    title.trim() && title.trim() !== r.title
      ? write(r, () =>
          api.reminderEdit(r.guid, r.listGuid, { title: title.trim() }),
        )
      : Promise.resolve();
  const setDue = (r: Reminder, value: string) =>
    write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, {
        dueAt: value ? Math.floor(new Date(value).getTime() / 1000) : null,
      }),
    );
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
    return write(r, () =>
      api.reminderEdit(r.guid, r.listGuid, { parentGuid: above.guid })
    );
  };

  const outdent = (r: Reminder) =>
    write(r, () => api.reminderEdit(r.guid, r.listGuid, { parentGuid: null }));

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
    try {
      await api.reminderCreate(targetListGuid, title);
      setDraft("");
      await readItems();
    } catch (err) {
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
    for (const r of items) {
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
      // Apple's own buckets, and it shows all three whether or not anything is in them.
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
      return buckets
        .filter((b) => b.rows.length > 0 || b.heading !== null)
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

  return (
    <div className="rem">
      <aside className="rem-lists" aria-label="Reminder lists">
        {/* Apple's own arrangement: the four built-in views as a grid of tiles, each
            with its glyph and its count, above the lists you made yourself. */}
        <div className="rem-smart-grid">
          {SMART.map((smart) => (
            <button
              key={smart.kind}
              className="rem-smart"
              aria-current={
                selection.type === "smart" && selection.kind === smart.kind
              }
              aria-label={`${smart.title}, ${counts?.[smart.kind] ?? 0} reminders`}
              onClick={() => setSelection({ type: "smart", kind: smart.kind })}
            >
              <span className="rem-smart-top">
                <span className="rem-smart-glyph" style={{ background: smart.colour }}>
                  <Glyph name={smart.glyph} />
                </span>
                <span className="rem-smart-count">{counts?.[smart.kind] ?? 0}</span>
              </span>
              <span className="rem-smart-name">{smart.title}</span>
            </button>
          ))}
        </div>

        <p className="rem-lists-heading">My Lists</p>
        {lists.map((list) => (
          <div className="rem-list-row" key={list.guid}>
            {renaming === list.guid ? (
              <input
                className="rem-list-rename"
                defaultValue={list.title}
                autoFocus
                aria-label={`Rename ${list.title}`}
                onBlur={(e) => {
                  const title = e.target.value.trim();
                  setRenaming(null);
                  if (title && title !== list.title) {
                    void listWrite(() =>
                      api.reminderListRename(list.guid, title),
                    );
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
                  className="rem-list"
                  aria-current={listGuid === list.guid}
                  onClick={() =>
                    setSelection({ type: "list", guid: list.guid })
                  }
                  onDoubleClick={() => setRenaming(list.guid)}
                >
                  <span
                    className="rem-list-glyph"
                    style={{ color: list.colour || "var(--gold)" }}
                  >
                    <Glyph name="list" />
                  </span>
                  <span className="rem-list-name">{list.title}</span>
                  <span className="rem-list-count">
                    {counts?.lists.find(([guid]) => guid === list.guid)?.[1] ?? 0}
                  </span>
                </button>
                <span className="rem-list-actions">
                <button
                  className="rem-list-action"
                  aria-label={`Rename ${list.title}`}
                  onClick={() => setRenaming(list.guid)}
                >
                  ✎
                </button>
                <button
                  className="rem-list-action danger"
                  aria-label={`Delete ${list.title}`}
                  onClick={() => {
                    // Deleting a list takes its reminders, so ask before doing it.
                    if (
                      window.confirm(
                        `Delete "${list.title}" and everything in it?`,
                      )
                    ) {
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
        ))}
        {loaded && lists.length === 0 && <p className="badge">No lists yet</p>}

        {addingList ? (
          <form onSubmit={createList}>
            <input
              className="rem-list-rename"
              value={newList}
              onChange={(e) => setNewList(e.target.value)}
              placeholder="List name"
              aria-label="New list name"
              autoFocus
              onBlur={() => !newList.trim() && setAddingList(false)}
            />
          </form>
        ) : (
          <button className="rem-list add" onClick={() => setAddingList(true)}>
            <span className="rem-list-name">Add List</span>
          </button>
        )}
      </aside>

      <section className="rem-items">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>{searching ? "Search" : heading}</h1>
            {outstanding > 0 && <span className="badge">{outstanding}</span>}
          </div>
          <div className="cal-actions">
            {inList && (
              <button className="quiet" onClick={() => setShowDone((v) => !v)}>
                {showDone ? "Hide completed" : "Show completed"}
              </button>
            )}
          </div>
        </header>
        <input
          className="notes-search"
          type="search"
          placeholder="Search"
          aria-label="Search reminders"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setQuery("");
          }}
        />

        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}

        {/*
          * An account with no lists is a real account — a new one, or one whose lists have
          * all been deleted. A reminder cannot exist outside a list, so rather than the add
          * field quietly disappearing and leaving nothing to do, this says why and offers
          * the one thing that helps.
          */}
        {lists.length === 0 && loaded && (
          <div className="rem-nolists">
            <p className="rem-nolists-line">Reminders live in lists, and there aren’t any yet.</p>
            <button className="primary" onClick={() => void makeFirstList()} disabled={syncing}>
              Create a list
            </button>
          </div>
        )}

        {targetListGuid && (
          <form className="rem-add" onSubmit={add}>
            <span className="tick ghost" aria-hidden="true" />
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              placeholder="Add a reminder"
              aria-label={
                inList
                  ? `Add a reminder to ${heading}`
                  : `Add a reminder to ${targetList?.title ?? "a list"}`
              }
              disabled={syncing}
            />
            {/* Inside a list it obviously goes there. From a smart view, where it lands is
                a real choice, so it is made here rather than decided for you. */}
            {!inList && targetList && lists.length > 0 && (
              <Picker
                className="rem-add-target"
                label="List to add to"
                value={targetListGuid ?? ""}
                onChange={chooseList}
                options={lists.map((l) => ({ value: l.guid, label: l.title }))}
              />
            )}
          </form>
        )}

        {heldBack > 0 && (
          <p className="rem-more" role="status">
            Showing the first {MOST_AT_ONCE.toLocaleString()} — {heldBack.toLocaleString()} more
            are in this list.
          </p>
        )}

        {sections.map((section) => (
        <Fragment key={section.key}>
        {section.heading && (
          <h2
            className={`rem-section${section.dated ? " quiet" : ""}`}
            style={section.colour ? { color: section.colour } : undefined}
          >
            {section.heading}
          </h2>
        )}
        <ul className="rem-list-items">
          {section.rows.map(({ reminder, depth }) => {
            // The heading already names the day in the views that have one.
            const due =
              reminder.dueAt === null
                ? null
                : dueLabel(reminder.dueAt, section.dated);
            const isOpen = openGuid === reminder.guid;
            const repeat = repeatLabel(reminder);
            const justMoved = moved?.guid === reminder.guid ? moved.text : null;
            return (
              <li
                key={reminder.guid}
                className={`${reminder.completedAt ? "is-done" : ""}${
                  depth > 0 ? " is-subtask" : ""
                }`.trim() || undefined}
              >
                <button
                  className="tick"
                  role="checkbox"
                  aria-checked={Boolean(reminder.completedAt)}
                  aria-label={reminder.title}
                  disabled={busyGuid === reminder.guid}
                  onClick={() => toggle(reminder)}
                >
                  {/* Apple's empty circle is neutral wherever it appears; the colour is the
                      reminder's own list, and it only shows once the thing is done. Borrowing
                      the open list's colour painted every circle in a smart view gold, which
                      said something untrue about where each reminder lived. */}
                  {reminder.completedAt && (
                    <span
                      className="tick-fill"
                      style={{
                        background:
                          lists.find((l) => l.guid === reminder.listGuid)?.colour ||
                          "var(--gold)",
                      }}
                    />
                  )}
                </button>

                <div className="rem-text">
                  <div className="rem-title-row">
                    {priorityMark(reminder.priority) && (
                      <span
                        className="rem-priority"
                        role="img"
                        aria-label={`${
                          PRIORITIES.find((p) => p.value === reminder.priority)
                            ?.label
                        } priority`}
                      >
                        {priorityMark(reminder.priority)}
                      </span>
                    )}
                    <input
                      className="rem-title-edit"
                      defaultValue={reminder.title}
                      key={`${reminder.guid}-${reminder.title}`}
                      aria-label={`Rename ${reminder.title}`}
                      disabled={busyGuid === reminder.guid}
                      onBlur={(e) => void rename(reminder, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") e.currentTarget.blur();
                        if (e.key === "Escape") {
                          e.currentTarget.value = reminder.title;
                          e.currentTarget.blur();
                        }
                      }}
                    />
                  </div>
                  {reminder.notes && !isOpen && (
                    <span className="rem-notes">{reminder.notes}</span>
                  )}

                  {/* Under the title, where Apple puts it. Out at the end of the row it sat
                      a thousand pixels from the thing it describes on a wide window. */}
                  {(due || repeat || justMoved) && (
                    <span className="rem-meta">
                      {justMoved ? (
                        <span className="rem-moved" role="status">
                          Moved to {justMoved}
                        </span>
                      ) : (
                        <>
                          {/* Which list it came from, where the grouping does not already
                              say so — Apple puts it here, before the time. */}
                          {showListName && (
                            <span className="rem-from">
                              {lists.find((l) => l.guid === reminder.listGuid)?.title ??
                                "Reminders"}
                            </span>
                          )}
                          {due && (
                            <span className={`rem-due${due.overdue ? " overdue" : ""}`}>
                              {due.text}
                            </span>
                          )}
                          {repeat && (
                            <span className="rem-repeat" title={repeat}>
                              <span aria-hidden="true">↻</span>
                              <span className="rem-repeat-word">
                                {repeatWord(reminder)}
                              </span>
                              <span className="visually-hidden">{repeat}</span>
                            </span>
                          )}
                        </>
                      )}
                    </span>
                  )}

                  {isOpen && (
                    <div className="rem-detail">
                      <label>
                        <span>Due</span>
                        <input
                          type="datetime-local"
                          defaultValue={toInputValue(reminder.dueAt)}
                          onBlur={(e) => void setDue(reminder, e.target.value)}
                        />
                      </label>
                      <div className="rem-field">
                        <span>Nesting</span>
                        <span className="rem-nesting">
                          <button
                            className="quiet"
                            disabled={busyGuid === reminder.guid || depth > 0}
                            onClick={() => void indent(reminder)}
                          >
                            Make subtask
                          </button>
                          <button
                            className="quiet"
                            disabled={busyGuid === reminder.guid || depth === 0}
                            onClick={() => void outdent(reminder)}
                          >
                            Move out
                          </button>
                        </span>
                      </div>
                      <div className="rem-field">
                        <span>Repeat</span>
                        <Picker
                          label={`Repeat for ${reminder.title}`}
                          value={repeatKey(reminder)}
                          disabled={busyGuid === reminder.guid || reminder.dueAt === null}
                          onChange={(key) => void setRepeat(reminder, key)}
                          options={
                            repeatKey(reminder) === "custom"
                              ? [
                                  // A schedule this menu doesn't express must still be
                                  // shown, and choosing anything else would silently
                                  // replace it.
                                  { value: "custom", label: repeat ?? "Custom" },
                                  ...REPEATS.map((o) => ({ value: o.key, label: o.label })),
                                ]
                              : REPEATS.map((o) => ({ value: o.key, label: o.label }))
                          }
                        />
                      </div>
                      {reminder.dueAt === null && (
                        <p className="rem-repeat-note">
                          Give this a date before asking it to repeat.
                        </p>
                      )}
                      {repeat && (
                        <p className="rem-repeat-note">Ticking it moves it to the next one.</p>
                      )}
                      {/* A pop-up button is not a labelled field: it names itself, and
                          wrapping one in a <label> associates a label with nothing. */}
                      <div className="rem-field">
                        <span>Priority</span>
                        <Picker
                          label={`Priority for ${reminder.title}`}
                          value={String(reminder.priority)}
                          disabled={busyGuid === reminder.guid}
                          onChange={(v) => void setPriority(reminder, Number(v))}
                          options={PRIORITIES.map((p) => ({
                            value: String(p.value),
                            label: p.label,
                          }))}
                        />
                      </div>
                      <div className="rem-field">
                        <span>List</span>
                        <Picker
                          label={`List for ${reminder.title}`}
                          value={reminder.listGuid}
                          disabled={busyGuid === reminder.guid}
                          onChange={(guid) => void moveToList(reminder, guid)}
                          options={lists.map((l) => ({ value: l.guid, label: l.title }))}
                        />
                      </div>
                      <label>
                        <span>Notes</span>
                        <textarea
                          defaultValue={reminder.notes ?? ""}
                          rows={2}
                          onBlur={(e) =>
                            void setNotes(reminder, e.target.value)
                          }
                        />
                      </label>
                    </div>
                  )}
                </div>

                <span className="rem-trailing">
                  <button
                    className={`rem-flag${reminder.flagged ? " on" : ""}`}
                    aria-label={
                      reminder.flagged
                        ? `Unflag ${reminder.title}`
                        : `Flag ${reminder.title}`
                    }
                    aria-pressed={reminder.flagged}
                    disabled={busyGuid === reminder.guid}
                    onClick={() => flag(reminder)}
                  >
                    ⚑
                  </button>
                  <button
                    className="rem-more"
                    aria-label={
                      isOpen
                        ? `Close details for ${reminder.title}`
                        : `Details for ${reminder.title}`
                    }
                    aria-expanded={isOpen}
                    onClick={() => setOpenGuid(isOpen ? null : reminder.guid)}
                  >
                    ⋯
                  </button>
                  <button
                    className="rem-delete"
                    aria-label={`Delete ${reminder.title}`}
                    disabled={busyGuid === reminder.guid}
                    onClick={() => remove(reminder)}
                  >
                    ×
                  </button>
                </span>
              </li>
            );
          })}
        </ul>
        </Fragment>
        ))}

        {loaded && items.length === 0 && !error && (
          <p className="cal-empty">
            {inList
              ? showDone
                ? "Nothing in this list."
                : "All done here."
              : "Nothing here."}
          </p>
        )}
      </section>
    </div>
  );
}
