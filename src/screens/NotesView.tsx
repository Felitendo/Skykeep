import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { usePending } from "../lib/pending";
import { useErrorToast } from "../components/Toast";
import { useConfirm } from "../components/Confirm";
import { EmptyState } from "../components/EmptyState";
import { SearchField } from "../components/SearchField";
import { PaneTitle } from "../components/PaneTitle";
import { Unsent, Waiting } from "../components/Unsent";
import { useOutbox } from "../lib/outbox";
import { ChevronLeft, Folder, FolderOpen, FolderPlus, ListChecks, Pin, PinOff, Share, SquarePen, Trash2 } from "lucide-react";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { editorDate, groupNotes, listDate, mergeEdits, splitTitle } from "./notes/model";
import { api, messageOf, nestFolders, type Note, type NoteFolder } from "../lib/ipc";
import { RowMenu } from "../components/RowMenu";
import { FormattedBody, type Run } from "./notes/Formatted";
import { carryRuns, runsFit } from "./notes/rich";
import { NoteEditor, refreshTables, type EditorHandle, type EditorStatus } from "./notes/NoteEditor";
import { FormatPanel } from "./notes/FormatPanel";
import { NotePicture } from "./notes/NotePicture";
import { OPEN_NOTE_EVENT } from "./notes/links";
import { useReveal } from "../lib/reveal";
import { Arriving } from "../components/EmptyState";
import { useArriving, useSync } from "../lib/sync";
import { Picker } from "../components/Picker";

const ALL = "__all__";

/** A new note's first line set as its title, as Notes sets it, unless it was given a style. */
function titled(text: string, runs: Run[]): Run[] {
  const end = text.indexOf("\n");
  const firstLine = end < 0 ? text.length : end + 1;
  const out: Run[] = [];
  let at = 0;
  for (const r of runs) {
    const from = at;
    at += r.length;
    if (from >= firstLine || r.style !== null) { out.push(r); continue; }
    const inside = Math.min(at, firstLine) - from;
    out.push({ ...r, length: inside, style: 0 });
    if (r.length > inside) out.push({ ...r, length: r.length - inside });
  }
  return out;
}

/** Apple's Recently Deleted, by the record name that is the same in every language. */
const TRASH_FOLDER = "TrashFolder-CloudKit";
/** Apple's own "Notes" folder: it can be neither renamed nor deleted. */
const DEFAULT_FOLDER = "DefaultFolder-CloudKit";

/** A note's title or snippet as words: without the marks where attachments sit (U+FFFC) or
 *  a line breaks (U+2028), which drew as boxes. The backend strips them too; this covers
 *  what was stored before it did. */
const wordsOnly = (text: string) => text.replace(/[\uFFFC\u2028]/g, " ").replace(/\s+/g, " ").trim();

const FOLDERS_KEY = "quince.notes.foldersWidth";
const clampFolders = (w: number) => Math.round(Math.max(160, Math.min(420, w)));
function keptFoldersWidth(): number | null {
  try {
    const n = Number(window.localStorage.getItem(FOLDERS_KEY));
    return n > 0 ? clampFolders(n) : null;
  } catch {
    return null;
  }
}
function keepFoldersWidth(w: number) {
  try {
    window.localStorage.setItem(FOLDERS_KEY, String(w));
  } catch {
    /* a width not kept is only a width not kept */
  }
}
/** The width that shows every folder's whole name: the widest row's name as it would run
 * untruncated, with the row's icon, count and padding around it. */
function fitFolders(aside: HTMLElement | null): number | null {
  if (!aside) return null;
  let widest = 0;
  aside.querySelectorAll<HTMLElement>(".notes-folder").forEach((row) => {
    const name = row.querySelector<HTMLElement>(".notes-folder-name");
    if (!name) return;
    const others = row.scrollWidth - name.clientWidth;
    widest = Math.max(widest, others + name.scrollWidth);
  });
  if (!widest) return null;
  const pad = parseFloat(getComputedStyle(aside).paddingLeft) + parseFloat(getComputedStyle(aside).paddingRight);
  return clampFolders(widest + pad + 2);
}

export function NotesView() {
  // The folders' column: as wide as it was left, or — the first time — as wide as the longest
  // folder name needs, within reason.
  const notesRef = useRef<HTMLDivElement | null>(null);
  const foldersRef = useRef<HTMLElement | null>(null);
  const [foldersWidth, setFoldersWidth] = useState<number | null>(keptFoldersWidth);

  const [folders, setFolders] = useState<NoteFolder[]>([]);
  const kept = useRef(foldersWidth !== null);
  useLayoutEffect(() => {
    if (kept.current) return;
    const fit = fitFolders(foldersRef.current);
    // Never narrower than the column was made, and never more than a third of a wide window.
    if (fit) setFoldersWidth(Math.max(208, Math.min(fit, 320)));
  }, [folders]);
  const [folderMenu, setFolderMenu] = useState<{ folder: NoteFolder; x: number; y: number; alignRight: boolean } | null>(null);
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  const [folder, setFolder] = useState<string>(ALL);
  const [notes, setNotes] = useState<Note[]>([]);
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  /**
   * In a narrow window only one column shows, as on iPhone: the list, or — once a note is
   * picked or begun — that note, with a way back. A wide window shows both and ignores this.
   */
  const [detail, setDetail] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [confirm, confirmSheet] = useConfirm();
  const { revision } = useSync();
  const arriving = useArriving("notes");
  // Tables on screen keep up with a phone's edits, as the note's text does.
  useEffect(() => {
    if (revision) refreshTables();
  }, [revision]);
  /** The body being typed, held apart from the note until it is saved. */
  const [draft, setDraft] = useState<string | null>(null);
  /** `draft`, for effects that read it without re-running on every keystroke. */
  const draftRef = useRef<string | null>(null);
  draftRef.current = draft;
  /** Counts every change made in the editor, formatting as much as text. */
  const [edits, setEdits] = useState(0);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  /** What is typed in the search field. While it holds anything, the list is the results. */
  const [query, setQuery] = useState("");
  const searching = query.trim().length > 0;

  /** Notes outside Recently Deleted, and how many each folder holds. */
  const [counts, setCounts] = useState<{ all: number; per: Map<string, number> }>({
    all: 0,
    per: new Map(),
  });

  const { hold, over } = usePending<Note>();
  /** Edits kept here and not yet at Apple, and any Apple turned down. */
  const outbox = useOutbox("notes");
  const read = useCallback(async () => {
    api
      .noteCounts()
      .then(([all, per]) => setCounts({ all, per: new Map(per) }))
      .catch(() => {
        // Counts are a courtesy; the list is what matters.
      });
    const [found, list] = await Promise.all([
      api.noteFolders(),
      // Search reaches every folder, as Apple's does — a note is found wherever it is filed.
      searching ? api.notesSearch(query) : api.notesIn(folder === ALL ? null : folder),
    ]);
    setFolders(Array.isArray(found) ? found : []);
    setNotes(over(Array.isArray(list) ? list : []));
    setLoaded(true);
  }, [folder, query, searching, over]);

  // Reads the library and nothing else. `revision` ticks when a refresh finishes, whether
  // someone pressed for it or the timer came round, and that is the screen's whole
  // relationship with the network.
  useEffect(() => {
    let cancelled = false;
    read().catch((e) => {
      if (!cancelled) {
        setError(messageOf(e));
        setLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
    // `outbox.tick`: an edit sent, or turned down, changes what the library holds.
  }, [read, revision, outbox.tick]);

  /**
   * Every write goes to Apple first and the library is re-read after, so nothing on screen
   * is a guess about what was stored.
   */
  /**
   * A pin, a move, a delete shows the moment it is asked for, as on the phone; then it goes
   * to Apple and the list is re-read. A refusal re-reads, which puts the note back, and says
   * why.
   */
  const write = useCallback(
    async (run: () => Promise<void>, predict?: (rows: Note[]) => Note[]) => {
      setBusy(true);
      setError(null);
      if (predict) setNotes((rows) => predict(rows));
      // Kept over any refresh until Apple has it; let go before the read that confirms it.
      const release = predict ? hold(predict) : () => {};
      try {
        await run();
        release();
        await read();
        return true;
      } catch (e) {
        release();
        setError(messageOf(e));
        read().catch(() => {});
        return false;
      } finally {
        release();
        setBusy(false);
      }
    },
    [read, hold]
  );

  /** Mirrors `composing` for effects that must not re-run when it changes. */
  const composingRef = useRef<{ folderGuid: string | null; body: string } | null>(null);

  // Keep a note open across refreshes where possible; fall back to the first — but never
  // while a new note is being written, which has nothing open behind it on purpose.
  useEffect(() => {
    if (composingRef.current) return;
    if (notes.length === 0) return setOpenGuid(null);
    setOpenGuid((current) =>
      current && notes.some((n) => n.guid === current) ? current : notes[0].guid
    );
  }, [notes]);

  // A link to another note, followed: that note opened — from All iCloud when the folder in
  // front does not hold it — or, when it is not in this account's notes, said so.
  const following = useRef<string | null>(null);
  const follow = (guid: string) => {
    if (notes.some((n) => n.guid === guid)) {
      setOpenGuid(guid);
      setDetail(true);
      return;
    }
    following.current = guid;
    if (folder !== ALL) setFolder(ALL);
    else void read();
  };
  const followRef = useRef(follow);
  followRef.current = follow;
  useEffect(() => {
    const on = (e: Event) => followRef.current((e as CustomEvent<string>).detail);
    window.addEventListener(OPEN_NOTE_EVENT, on);
    return () => window.removeEventListener(OPEN_NOTE_EVENT, on);
  }, []);
  // A note found by the search across everything: opened as a link to it opens it.
  useReveal("notes", (r) => followRef.current(r.id));
  useEffect(() => {
    const guid = following.current;
    if (!guid || folder !== ALL || notes.length === 0) return;
    following.current = null;
    if (notes.some((n) => n.guid === guid)) {
      setOpenGuid(guid);
      setDetail(true);
    } else {
      setError("That note isn’t in your notes. It may have been deleted, or it belongs to someone else.");
    }
  }, [notes, folder]);

  const open = useMemo(
    () => notes.find((n) => n.guid === openGuid) ?? null,
    [notes, openGuid]
  );

  // A different note means a different draft. Switching away from unsaved typing would
  // otherwise carry it into the next note and save it over that one.
  //
  // Except for the note just created from what is being typed: that is the same note, and
  // what was typed while it was being created is still to be saved onto it.
  const adopting = useRef<string | null>(null);
  useEffect(() => {
    if (adopting.current !== null && adopting.current === openGuid) {
      adopting.current = null;
      return;
    }
    setDraft(null);
    setSaved(false);
  }, [openGuid]);

  /**
   * The open note's text, fetched on its own.
   *
   * A listed note carries no body, so this is the only thing that knows what the note says —
   * and until it arrives nothing may be edited or saved. An editor showing an empty box over
   * a note whose text had not loaded yet would write that emptiness back the moment anyone
   * touched it, which is the worst thing this app could do.
   *
   * Kept beside the guid it belongs to, so a slow fetch for one note can never be mistaken
   * for the body of the next one opened.
   */
  const [loadedBody, setLoadedBody] = useState<{ guid: string; text: string } | null>(null);

  useEffect(() => {
    if (!openGuid) return setLoadedBody(null);
    let cancelled = false;
    setLoadedBody((current) => (current?.guid === openGuid ? current : null));
    api
      .noteBody(openGuid)
      .then((text) => {
        if (cancelled || text === null) return;
        // Never over what is being typed. A refresh follows every save, and adopting its
        // answer under someone's cursor is how a half-saved note replaced a whole one. Typing
        // already kept, with nothing new since, is not being typed: what Apple made of it —
        // merged with another device's edit, once sent — is taken in.
        setDraft((typing) => {
          const settled = typing !== null && !pending.current && base.current?.guid === openGuid && typing === base.current.text;
          if (typing === null || settled) setLoadedBody({ guid: openGuid, text });
          return typing;
        });
      })
      .catch((e) => {
        if (!cancelled) setError(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, [openGuid, revision, outbox.tick]);

  /** How the open note is set, from Apple's attribute runs, read with its body. */
  const [loadedRuns, setLoadedRuns] = useState<{ guid: string; runs: Run[] } | null>(null);
  const loadedRunsRef = useRef(loadedRuns);
  loadedRunsRef.current = loadedRuns;
  /** The line the cursor is in and its marks, for the toolbar to show as on. */
  const [status, setStatus] = useState<EditorStatus | null>(null);
  const [formatAt, setFormatAt] = useState<{ x: number; y: number } | null>(null)
  useEffect(() => {
    setFormatAt(null);
    if (!openGuid) return setLoadedRuns(null);
    let cancelled = false;
    api.noteRuns(openGuid).then(
      (runs) => { if (!cancelled) setLoadedRuns({ guid: openGuid, runs: Array.isArray(runs) ? runs : [] }); },
      () => {},
    );
    return () => { cancelled = true; };
  }, [openGuid, revision, outbox.tick]);

  /** What the open note says, or null while that is still unknown. */
  const body = open && loadedBody?.guid === open.guid ? loadedBody.text : null;
  const runs = open && loadedRuns?.guid === open.guid ? loadedRuns.runs : null;
  /**
   * Whether this note's formatting can be written from the editor: only when the runs read
   * describe the text exactly. Otherwise the editor shows it plain and saves the text alone,
   * which keeps Apple's formatting where it is — never a plain copy written over it.
   */
  const formatSafe = body !== null && runsFit(body, runs);
  /** The editor, to hand it what Apple holds and to run the toolbar's commands. */
  const editorRef = useRef<EditorHandle | null>(null);
  /** A new note's id once it is made, so the editor it was typed in stays the one shown. */
  const adoptedKey = useRef<string | null>(null);
  /** The formatting on screen now, beside `draft` — the text on screen now. */
  const draftRuns = useRef<Run[] | null>(null);

  const dirty = draft !== null && body !== null && draft !== body;

  /**
   * Saving is not something anyone should have to remember.
   *
   * Apple's Notes has no save button because a note is not a document you commit — it is
   * just there, and what you typed is what it says. Typing schedules a save; leaving the
   * note or the field takes it immediately, so nothing is ever waiting on a timer when you
   * walk away.
   */
  const pending = useRef<{ guid: string; body: string; runs: Run[] | null } | null>(null);
  /**
   * The text Apple last confirmed for the open note: what an edit began from. Sent with each
   * save, so the save can tell an edit made on another device since and keep both.
   */
  const base = useRef<{ guid: string; text: string; runs?: Run[] | null } | null>(null);
  /** One save at a time: a second, started while the first was out, could land last with
   *  older text. Saves chain behind this. */
  const inFlight = useRef<Promise<void>>(Promise.resolve());

  const flush = useCallback(() => {
    const run = inFlight.current.then(async () => {
      const next = pending.current;
      if (!next) return;
      pending.current = null;
      const from = base.current?.guid === next.guid ? base.current.text : null;
      const fromRuns = base.current?.guid === next.guid ? (base.current.runs ?? null) : null;
      let saved: string | null = null;
      const ok = await write(async () => {
        saved = await api.noteSave(next.guid, next.body, from, next.runs, next.runs ? fromRuns : null);
      });
      if (!ok || saved === null) {
        // Not sent: the edit goes back in line (unless newer typing already replaced it), to
        // go with the next pause, the next sync or leaving the note — never dropped.
        if (!pending.current) pending.current = next;
        return;
      }
      const kept: string = saved;
      const keptRunsForBase = next.runs ? (kept === next.body ? next.runs : carryRuns(next.body, next.runs, kept)) : null;
      base.current = { guid: next.guid, text: kept, runs: keptRunsForBase };
      setSaved(true);
      // What Apple now holds becomes what this note reads, text and formatting together, so
      // what is on screen and what was saved agree without a refresh.
      const keptRuns = next.runs ? (kept === next.body ? next.runs : carryRuns(next.body, next.runs, kept)) : null;
      adopted.current = `${next.guid}\u0000${kept}\u0000${JSON.stringify(keptRuns ?? (loadedRunsRef.current?.guid === next.guid ? loadedRunsRef.current.runs : null))}`;
      setLoadedBody({ guid: next.guid, text: kept });
      if (keptRuns) setLoadedRuns({ guid: next.guid, runs: keptRuns });
      // Apple holds `kept` — ours, or ours with another device's edit combined. What is on
      // screen takes it in: unchanged if nothing was typed since, combined if something was.
      // The draft is never handed back to the library's copy, which can lag a save, and was
      // how a five-line note once became a single letter.
      if (kept !== next.body) {
        setDraft((typing) => {
          const now = typing ?? next.body;
          const merged = now === next.body ? kept : (mergeEdits(next.body, now, kept) ?? now);
          if (merged !== now) {
            // Another device's words, taken into the editor with the formatting moved along.
            const laid = draftRuns.current ? carryRuns(now, draftRuns.current, merged) : null;
            draftRuns.current = laid;
            editorRef.current?.replace(merged, laid);
            if (merged !== kept) pending.current = { guid: next.guid, body: merged, runs: next.runs ? laid : null };
          }
          return merged;
        });
      }
    });
    inFlight.current = run.catch(() => {});
    return run;
  }, [write]);

  // An edit that could not be sent goes again when a sync has just succeeded — Apple is
  // reachable — rather than waiting for more typing.
  useEffect(() => {
    if (pending.current) void flush();
  }, [revision, flush]);

  const type = useCallback(
    (typed: string, typedRuns: Run[]) => {
      // Nothing may be typed into a note whose text has not arrived.
      if (!open || body === null) return;
      // The first keystroke in a note marks what it read as the base of the edit.
      if (base.current?.guid !== open.guid) base.current = { guid: open.guid, text: body, runs: formatSafe ? runs : null };
      draftRuns.current = typedRuns;
      setDraft(typed);
      setEdits((n) => n + 1);
      setSaved(false);
      // Compared only when the text is back to what Apple holds — never on an ordinary key,
      // which in a long note would be a second walk of the whole thing per keystroke.
      const unchanged = typed === body && (!formatSafe || JSON.stringify(typedRuns) === JSON.stringify(runs));
      pending.current = unchanged ? null : { guid: open.guid, body: typed, runs: formatSafe ? typedRuns : null };
    },
    [open, body, runs, formatSafe]
  );

  // A pause in typing is the signal. Long enough not to write on every keystroke, short
  // enough that closing the window a second later has already saved.
  useEffect(() => {
    if (!pending.current) return undefined;
    const timer = window.setTimeout(() => void flush(), 900);
    return () => window.clearTimeout(timer);
    // `edits` as well as the text: a change of style alone leaves the text as it was.
  }, [draft, edits, flush]);

  // Leaving the note takes whatever is outstanding with it, before the draft is cleared; the
  // next note's edit begins from what that note reads, never from this one's.
  // Only the note being left loses its base. Compared with the note now open instead, this
  // closure's `openGuid` is the old one: a new note made from a draft (null → its guid) had
  // its base wiped the moment it became real, and the next save merged from a lagging copy —
  // which wrote the whole text again on every save.
  useEffect(() => {
    const leaving = openGuid;
    return () => {
      void flush().then(() => {
        if (leaving && base.current?.guid === leaving) base.current = null;
      });
    };
  }, [openGuid, flush]);
  useEffect(() => {
    if (base.current && base.current.guid !== openGuid) base.current = null;
  }, [openGuid]);

  /**
   * A note nobody has typed into is not a note. Apple discards an empty new one the moment
   * you look away rather than syncing a "New Note" placeholder to every device, and that
   * had been the actual bug: pressing New note wrote junk to iCloud immediately, even if
   * nothing was ever typed.
   *
   * So composing happens locally first. It becomes a real, synced note the moment there is
   * something worth keeping — a pause after typing, or leaving it — and simply evaporates
   * if you never type at all.
   */
  const [composing, setComposingState] = useState<{
    folderGuid: string | null;
    body: string;
  } | null>(null);
  const setComposing = useCallback(
    (
      next:
        | { folderGuid: string | null; body: string }
        | null
        | ((
            c: { folderGuid: string | null; body: string } | null
          ) => { folderGuid: string | null; body: string } | null)
    ) => {
      setComposingState((current) => {
        const value = typeof next === "function" ? next(current) : next;
        composingRef.current = value;
        return value;
      });
    },
    []
  );

  /**
   * Turn what is being typed into a real note, once, without taking the page away.
   *
   * It used to swap the box being typed into for a different one: for as long as the create
   * took, nothing was open — "Select a note" — and focus fell out onto the window, so the
   * next keystroke went nowhere and Ctrl+A selected the whole app instead of the note. The
   * box that went away could also fire its blur and create the note a second time, which is
   * where three "Untitled" notes came from. Now the same box stays, a create in flight is not
   * started again, and the note it makes is adopted in place, with anything typed during the
   * create saved onto it afterwards.
   */
  const creating = useRef(false);
  const commitComposing = useCallback(async () => {
    const draft = composingRef.current;
    if (!draft || creating.current) return;
    if (!draft.body.trim()) {
      // Nothing was ever typed. There is nothing to keep, so nothing is sent.
      setComposing(null);
      return;
    }
    creating.current = true;
    setBusy(true);
    setError(null);
    try {
      const guid = await api.noteCreate(draft.folderGuid, draft.body);
      const latest = composingRef.current?.body ?? draft.body;
      adopting.current = guid;
      // The same editor goes on as this note: nothing remounts under the cursor.
      adoptedKey.current = guid;
      setLoadedBody({ guid, text: draft.body });
      base.current = { guid, text: draft.body };
      // The note is made from its text; its formatting follows straight after — the first
      // line a title, as Notes makes it, and whatever was set while composing.
      const typedRuns = draftRuns.current && runsFit(latest, draftRuns.current) ? titled(latest, draftRuns.current) : null;
      setDraft(latest);
      setEdits((n) => n + 1);
      pending.current = { guid, body: latest, runs: typedRuns };
      // In the list at once, so the note is open — and its editor still there — before the
      // list is read again.
      const now = Math.floor(Date.now() / 1000);
      setNotes((ns) => ns.some((n) => n.guid === guid) ? ns : [{
        guid, folderGuid: draft.folderGuid, title: splitTitle(latest).title || "New Note", snippet: null, body: null,
        locked: false, pinned: false, createdAt: now, modifiedAt: now, editable: true, etag: null,
      } as Note, ...ns]);
      setOpenGuid(guid);
      setComposing(null);
      await read();
    } catch (e) {
      // The write failed, not the typing — it stays on screen to try again.
      setError(messageOf(e));
    } finally {
      creating.current = false;
      setBusy(false);
    }
  }, [read, setComposing]);

  // The same pause-then-save rhythm as an existing note, up until there is a real note to
  // hand it to.
  useEffect(() => {
    if (!composing || !composing.body.trim()) return undefined;
    const timer = window.setTimeout(() => void commitComposing(), 900);
    return () => window.clearTimeout(timer);
  }, [composing, commitComposing]);

  /** Finish composing before doing anything that would otherwise abandon it mid-thought. */
  const selectNote = useCallback(
    async (guid: string) => {
      if (composing) await commitComposing();
      setOpenGuid(guid);
      setDetail(true);
    },
    [composing, commitComposing]
  );

  const create = useCallback(async () => {
    if (composingRef.current) await commitComposing();
    await flush();
    setComposing({ folderGuid: folder === ALL ? null : folder, body: "" });
    setOpenGuid(null);
    setDetail(true);
    // Straight into the new note, as Apple's New Note does.
    window.requestAnimationFrame(() => editorRef.current?.focus());
  }, [commitComposing, flush, folder, setComposing]);

  /**
   * Delete goes to Recently Deleted, as on the phone; from there, Delete Now is final and is
   * asked about first, in Apple's words.
   */
  const inTrash = open?.folderGuid === TRASH_FOLDER;
  /**
   * Folders, as Notes manages them. Each shows its change at once — a new "New Folder" ready
   * to be named, a new name, a folder gone — and a refusal re-reads and says why.
   */
  const newFolder = async (parent: string | null) => {
    const standIn = `pending-folder-${Date.now()}`;
    setFolders((fs) => [...fs, { guid: standIn, title: "New Folder", sortOrder: 0, parentGuid: parent } as NoteFolder]);
    try {
      const guid = await api.noteFolderCreate("New Folder", parent);
      await read();
      setRenamingFolder(guid);
    } catch (e) {
      setFolders((fs) => fs.filter((f) => f.guid !== standIn));
      setError(messageOf(e));
    }
  };
  const renameFolder = async (f: NoteFolder, title: string) => {
    setRenamingFolder(null);
    const name = title.trim();
    if (!name || name === f.title) return;
    setFolders((fs) => fs.map((x) => (x.guid === f.guid ? { ...x, title: name } : x)));
    try {
      await api.noteFolderRename(f.guid, name);
      await read();
    } catch (e) {
      setError(messageOf(e));
      read().catch(() => {});
    }
  };
  const deleteFolder = async (f: NoteFolder) => {
    // Apple's own words for it.
    const sure = await confirm({
      title: `Delete \u201c${f.title}\u201d?`,
      body: "All notes and any subfolders will be deleted.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    setFolders((fs) => fs.filter((x) => x.guid !== f.guid && x.parentGuid !== f.guid));
    if (folder === f.guid) setFolder(ALL);
    try {
      await api.noteFolderDelete(f.guid);
      await read();
    } catch (e) {
      setError(messageOf(e));
      read().catch(() => {});
    }
  };

  const remove = useCallback(async () => {
    if (!open) return;
    if (
      open.folderGuid === TRASH_FOLDER &&
      !(await confirm({
        title: "Delete this note now?",
        body: "It will be deleted immediately. You can’t undo this action.",
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    const gone = open.guid;
    // The note below opens in its place, or the one above at the end, as in Notes: the order
    // is the list's as shown, pinned notes first.
    const order = groupNotes(notes).flatMap((g) => g.notes.map((n) => n.guid));
    const at = order.indexOf(gone);
    const next = order[at + 1] ?? order[at - 1] ?? null;
    setOpenGuid(next);
    await write(() => api.noteDelete(gone), (rows) => rows.filter((n) => n.guid !== gone));
  }, [open, write, notes, confirm]);

  /** Out of Recently Deleted and back into Notes — Apple's "Notes" folder where there is one. */
  const recover = useCallback(async () => {
    if (!open) return;
    const back = folders.some((f) => f.guid === DEFAULT_FOLDER) ? DEFAULT_FOLDER : null;
    const guid = open.guid;
    await write(() => api.noteMove(guid, back), (rows) => rows.filter((n) => n.guid !== guid));
  }, [open, write, folders]);

  const moveTo = useCallback(
    async (guid: string) => {
      if (!open) return;
      const to = guid === ALL ? null : guid;
      const moving = open.guid;
      await write(
        () => api.noteMove(moving, to),
        // Out of the folder on screen; in All it stays, filed somewhere else.
        (rows) => folder === ALL
          ? rows.map((n) => (n.guid === moving ? { ...n, folderGuid: to } : n))
          : rows.filter((n) => n.guid !== moving),
      );
    },
    [open, write, folder]
  );

  const pin = useCallback(async () => {
    if (!open) return;
    const guid = open.guid;
    const pinned = !open.pinned;
    await write(
      () => api.notePin(guid, pinned),
      (rows) => rows.map((n) => (n.guid === guid ? { ...n, pinned } : n)),
    );
  }, [open, write]);

  // A save confirmation is a moment, not a state.
  useEffect(() => {
    if (!saved) return undefined;
    const timer = window.setTimeout(() => setSaved(false), 2000);
    return () => window.clearTimeout(timer);
  }, [saved]);

  const canEdit = Boolean(composing) || Boolean(open && !open.locked && open.editable && body !== null);

  /** Every change in the editor: to the note being composed, or to the open one. */
  const apply = (next: string, nextRuns: Run[]) => {
    if (composingRef.current) {
      draftRuns.current = nextRuns;
      setComposing((c) => (c ? { ...c, body: next } : c));
    } else type(next, nextRuns);
  };

  // What Apple holds, taken into the editor when nothing is being typed — a change made on
  // the phone arrives in the open note without it being reopened.
  // Keyed to Apple's copy alone: a save, a keystroke or a re-render is never a reason to
  // put the editor back to an older text.
  const adopted = useRef<string | null>(null);
  useEffect(() => {
    if (!open || body === null) return;
    const key = `${open.guid}\u0000${body}\u0000${JSON.stringify(runs)}`;
    if (adopted.current === key) return;
    adopted.current = key;
    // Only over nothing new: no typing at all, or none since what Apple last confirmed.
    if (pending.current || (draftRef.current !== null && draftRef.current !== base.current?.text)) return;
    editorRef.current?.replace(body, formatSafe ? runs : null);
    draftRuns.current = formatSafe ? runs : null;
    if (draftRef.current !== null && draftRef.current !== body) {
      setDraft(null);
      base.current = null;
    }
  }, [open, body, runs, formatSafe]);

  /** The note as it is on screen, typing not yet sent included, saved as a Markdown file. */
  const exportNote = async () => {
    if (!open || body === null) return;
    // Read off the editor itself: the very text on screen, with its formatting.
    const shown = editorRef.current?.current();
    const text = shown?.text ?? draft ?? body;
    const set = shown?.runs ?? runs ?? [];
    const name = (text.split("\n")[0].trim() || "Note").replace(/[\\/:*?"<>|]/g, "-").slice(0, 80);
    let path: string | null;
    try {
      path = await saveFileDialog({ defaultPath: `${name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
    } catch (e) {
      setError(messageOf(e));
      return;
    }
    if (!path) return;
    try {
      await api.noteExportMarkdown(text, set, path);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const leave = () => {
    editorRef.current?.flushChange();
    void (composingRef.current ? commitComposing() : flush());
  };

  // Another folder is another list, and a narrow window shows that list, not a note from the last.
  useEffect(() => setDetail(false), [folder]);

  const sections = useMemo(() => groupNotes(notes), [notes]);
  const heading = searching
    ? "Search"
    : folder === ALL
      ? "All iCloud"
      : (folders.find((f) => f.guid === folder)?.title ?? "Notes");
  const saveFailed = error !== null;

  return (
    <div
      className={`notes${detail ? " is-detail" : ""}`}
      ref={notesRef}
      style={foldersWidth ? ({ "--folders-w": `${foldersWidth}px` } as React.CSSProperties) : undefined}
    >
      <NotePicture />
      <div
        className="notes-folders-edge"
        role="separator"
        aria-orientation="vertical"
        aria-label="Folder column width"
        title="Drag to resize. Double-click to fit the names."
        onPointerDown={(e) => {
          const box = notesRef.current?.getBoundingClientRect();
          if (!box) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          const move = (m: PointerEvent) => setFoldersWidth(clampFolders(m.clientX - box.left));
          const up = () => {
            window.removeEventListener("pointermove", move);
            window.removeEventListener("pointerup", up);
            kept.current = true;
            setFoldersWidth((w) => {
              if (w) keepFoldersWidth(w);
              return w;
            });
          };
          window.addEventListener("pointermove", move);
          window.addEventListener("pointerup", up);
        }}
        onDoubleClick={() => {
          const w = fitFolders(foldersRef.current);
          if (w) {
            kept.current = true;
            setFoldersWidth(w);
            keepFoldersWidth(w);
          }
        }}
      />
      <aside className="notes-folders" aria-label="Note folders" ref={foldersRef}>
        <p className="side-heading">iCloud</p>
        <button className="source-row notes-folder" aria-current={folder === ALL} onClick={() => setFolder(ALL)}>
          <FolderOpen className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
          <span className="source-name notes-folder-name">All iCloud</span>
          <span className="count notes-folder-count">{counts.all || ""}</span>
        </button>
        {nestFolders(folders)
          .filter(({ folder: f }) => f.guid !== TRASH_FOLDER)
          .map(({ folder: f, depth }) => {
            const own = f.guid !== DEFAULT_FOLDER;
            if (renamingFolder === f.guid) {
              return (
                <input
                  key={f.guid}
                  className="field notes-folder-rename"
                  defaultValue={f.title}
                  aria-label={`Rename ${f.title}`}
                  autoFocus
                  onFocus={(e) => e.currentTarget.select()}
                  onBlur={(e) => void renameFolder(f, e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") { e.currentTarget.value = f.title; e.currentTarget.blur(); }
                  }}
                  style={depth > 0 ? { paddingLeft: `${8 + depth * 16}px` } : undefined}
                />
              );
            }
            const openMenu = (x: number, y: number, alignRight: boolean) =>
              setFolderMenu({ folder: f, x, y, alignRight });
            return (
              <div
                key={f.guid}
                className={`notes-folder-row${folderMenu?.folder.guid === f.guid ? " is-menu" : ""}`}
                onContextMenu={(e) => {
                  e.preventDefault();
                  // Apple's own "Notes" folder has no menu: it cannot be renamed or deleted.
                  if (own) openMenu(e.clientX, e.clientY, false);
                }}
              >
                <button
                  className="source-row notes-folder"
                  aria-current={folder === f.guid}
                  onClick={() => setFolder(f.guid)}
                  style={depth > 0 ? { paddingLeft: `${8 + depth * 16}px` } : undefined}
                >
                  <Folder className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
                  <span className="source-name notes-folder-name" title={f.title}>{f.title}</span>
                  <span className="count notes-folder-count">{counts.per.get(f.guid) || ""}</span>
                </button>
                {own && (
                  <button
                    className="icon-button small round row-more notes-folder-more"
                    aria-label={`Actions for ${f.title}`}
                    aria-haspopup="menu"
                    onClick={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      openMenu(r.right, r.bottom + 4, true);
                    }}
                  >
                    <span aria-hidden="true">•••</span>
                  </button>
                )}
              </div>
            );
          })}
        {folders.some((f) => f.guid === TRASH_FOLDER) && (
          <button
            className="source-row notes-folder"
            aria-current={folder === TRASH_FOLDER}
            onClick={() => setFolder(TRASH_FOLDER)}
          >
            <Trash2 className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
            <span className="source-name notes-folder-name">Recently Deleted</span>
            <span className="count notes-folder-count">{counts.per.get(TRASH_FOLDER) || ""}</span>
          </button>
        )}
        <button className="side-add" onClick={() => void newFolder(null)}>
          <FolderPlus size={15} strokeWidth={2} aria-hidden="true" />
          New Folder
        </button>
        {folderMenu && (
          <RowMenu
            at={folderMenu}
            onClose={() => setFolderMenu(null)}
            entries={[
              { label: "Rename Folder", run: () => setRenamingFolder(folderMenu.folder.guid) },
              { label: "New Folder", run: () => void newFolder(folderMenu.folder.guid) },
              null,
              { label: "Delete Folder", danger: true, run: () => void deleteFolder(folderMenu.folder) },
            ]}
          />
        )}
      </aside>

      <section className="notes-list" aria-label="Notes">
        <header className="pane-head notes-list-head">
          <PaneTitle count={`${notes.length} ${notes.length === 1 ? "note" : "notes"}`}>{heading}</PaneTitle>
          <button
            className="icon-button"
            aria-label="New note"
            title="New Note"
            onClick={() => void create()}
            disabled={busy}
          >
            <SquarePen size={18} strokeWidth={2} aria-hidden="true" />
          </button>
        </header>
        <SearchField
          className="notes-search"
          placeholder="Search"
          aria-label="Search notes"
          value={query}
          onChange={setQuery}
          onKeyDown={(e) => {
            if (e.key === "Escape") setQuery("");
          }}
        />

        {/* Shown only when the folder column has folded away. */}
        <div className="notes-folder-picker">
          <Picker
            label="Folder"
            value={folder}
            onChange={setFolder}
            options={[
              { value: ALL, label: "All iCloud" },
              ...folders.map((f) => ({ value: f.guid, label: f.title })),
            ]}
          />
        </div>

        <Unsent refused={outbox.refused} what="an edit to a note" onResend={(r) => void outbox.resend(r)} onDiscard={(r) => void outbox.discard(r)} />

        <div className="note-rows" role="list">
          {sections.map((section) => (
            <Fragment key={section.key}>
              <h2 className="notes-section">{section.heading}</h2>
              {section.notes.map((note) => (
                <div role="listitem" key={note.guid}>
                  <button
                    className="note-row"
                    aria-current={openGuid === note.guid}
                    onClick={() => void selectNote(note.guid)}
                  >
                    <span className="note-row-title">{wordsOnly(note.title) || "New Note"}</span>
                    <span className="note-row-meta">
                      <span className="note-row-when">{listDate(note.modifiedAt)}</span>
                      <span className="note-row-snippet">
                        {note.locked ? "Locked" : wordsOnly(note.snippet ?? "").slice(0, 80) || "No additional text"}
                      </span>
                    </span>
                  </button>
                </div>
              ))}
            </Fragment>
          ))}
        </div>

        {loaded && notes.length === 0 && !error && !(arriving && !searching) && (
          <EmptyState className="notes-empty">{searching ? "No results." : "No notes here."}</EmptyState>
        )}
        {notes.length === 0 && !searching && (
          <Arriving service="notes" what="notes" className="notes-empty" />
        )}
      </section>

      <article className="note-body" aria-label="Note">
        {(composing || open) && (
          <div className="note-bar">
            <button
              className="compact-back"
              onClick={() => {
                // Leaving the note takes what was typed with it, as leaving any note does.
                void (composingRef.current ? commitComposing() : flush());
                setDetail(false);
              }}
            >
              <ChevronLeft size={18} strokeWidth={2.25} aria-hidden="true" />
              <span>{heading}</span>
            </button>
            <p className="note-when">
              {busy || (!saveFailed && (dirty || composing?.body.trim()))
                ? "Saving…"
                : open && outbox.waiting.has(open.guid)
                  ? <Waiting shown />
                  : saved
                  ? "Saved"
                  : editorDate(composing ? Math.floor(Date.now() / 1000) : (open?.modifiedAt ?? null))}
            </p>
            {!composing && open && !open.locked && (
              <div className="toolbar-group note-bar-actions">
                {!inTrash && open.editable && (
                  <>
                    <button
                      className={`tool notes-aa${formatAt ? " is-on" : ""}`}
                      aria-label="Format"
                      aria-haspopup="dialog"
                      aria-expanded={Boolean(formatAt)}
                      title="Choose a style to apply to text"
                      disabled={!canEdit}
                      // Keeps the cursor where it is in the note while the panel is open.
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        setFormatAt((at) => (at ? null : { x: r.left + r.width / 2, y: r.bottom + 6 }));
                      }}
                    >
                      Aa
                    </button>
                    <button
                      className="tool"
                      aria-label="Make a checklist"
                      aria-pressed={Boolean(status?.checklist)}
                      title="Make a checklist"
                      disabled={!canEdit}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => editorRef.current?.toggleChecklist()}
                    >
                      <ListChecks size={18} strokeWidth={2} aria-hidden="true" />
                    </button>
                    {formatAt && (
                      <FormatPanel
                        at={formatAt}
                        status={status}
                        onClose={() => setFormatAt(null)}
                        onStyle={(style) => editorRef.current?.setStyle(style)}
                        onMark={(mark) => editorRef.current?.toggleMark(mark)}
                        onQuote={() => editorRef.current?.toggleQuote()}
                        onColour={(c) => editorRef.current?.setColour(c)}
                        onIndent={(by) => editorRef.current?.indent(by)}
                        onMove={(by) => editorRef.current?.moveLines(by)}
                      />
                    )}
                  </>
                )}
                {!inTrash && <Picker
                  pullDown
                  label={`Folder for ${open.title}`}
                  value={open.folderGuid ?? ALL}
                  disabled={busy}
                  onChange={(guid) => void moveTo(guid)}
                  options={[
                    // "No folder" only for an account without Apple's default "Notes" folder;
                    // with one, it was a second "Notes" that meant something else.
                    ...(folders.some((f) => f.guid === DEFAULT_FOLDER)
                      ? []
                      : [{ value: ALL, label: "Notes" }]),
                    ...folders
                      .filter((f) => f.guid !== TRASH_FOLDER)
                      .map((f) => ({ value: f.guid, label: f.title })),
                  ]}
                />}
                {!inTrash && !open.locked && (
                  <button
                    className="tool"
                    aria-label="Export as Markdown"
                    title="Export as Markdown"
                    onClick={() => void exportNote()}
                    disabled={body === null}
                  >
                    <Share size={17} strokeWidth={2} aria-hidden="true" />
                  </button>
                )}
                {!inTrash && (
                  <button
                    className="tool"
                    aria-label={open.pinned ? "Unpin Note" : "Pin Note"}
                    aria-pressed={open.pinned}
                    title={open.pinned ? "Unpin Note" : "Pin Note"}
                    onClick={() => void pin()}
                    disabled={busy}
                  >
                    {open.pinned ? (
                      <PinOff size={17} strokeWidth={2} aria-hidden="true" />
                    ) : (
                      <Pin size={17} strokeWidth={2} aria-hidden="true" />
                    )}
                  </button>
                )}
                {inTrash && (
                  <button className="tool has-label is-accent" onClick={() => void recover()} disabled={busy}>
                    Recover
                  </button>
                )}
                {inTrash ? (
                  <button className="tool has-label danger" onClick={() => void remove()}>
                    Delete Now
                  </button>
                ) : (
                  <button
                    className="tool"
                    aria-label="Delete"
                    title="Delete"
                    onClick={() => void remove()}
                  >
                    <Trash2 size={17} strokeWidth={2} aria-hidden="true" />
                  </button>
                )}
              </div>
            )}
          </div>
        )}

        {/*
          * One editor for a new note and a saved one, always in the same place, so the box
          * being typed into is never taken away when a new note becomes a real one. The
          * first line is a title field and the rest a body, but they are one text: Enter in
          * the title splits it, Backspace at the start of the body joins it, and what is
          * saved is always the two put back together.
          */}
        {canEdit ? (
          <div className="note-editor"
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                e.preventDefault();
                leave();
              }
            }}
          >
            <NoteEditor
              // One editor per note, so nothing typed in one can be shown in another.
              key={composing || (open && adoptedKey.current === open.guid) ? "composing" : open?.guid}
              ref={editorRef}
              text={composing ? composing.body : (draft ?? body ?? "")}
              runs={composing ? draftRuns.current : formatSafe ? runs : null}
              editable
              autoFocus={Boolean(composing)}
              label={composing ? "New note" : `${open?.title ?? "Note"}`}
              placeholder={composing ? "New Note" : undefined}
              onChange={apply}
              onStatus={setStatus}
              onBlur={leave}
            />
          </div>
        ) : open?.locked ? (
          <EmptyState>
            This note is locked. Apple keeps it encrypted, so there is nothing here to show —
            open it on a device where you can enter its password.
          </EmptyState>
        ) : open && body === null ? (
          /* Nothing to type into until the note's own text has arrived: an empty box over a
             note whose body had not loaded would write that emptiness back. */
          <EmptyState busy className="note-loading">Opening…</EmptyState>
        ) : open ? (
          <div className="note-editor is-readonly">
            <h1 className="note-title">{splitTitle(body ?? "").title}</h1>
            {runs && body !== null && runsFit(body, runs) ? (
              <div className="note-text note-read">
                <FormattedBody text={body} runs={runs} skip={splitTitle(body).title.length + 1} />
              </div>
            ) : (
              <pre className="note-text">{splitTitle(body ?? "").rest}</pre>
            )}
            <p className="note-readonly">
              Pomander can’t yet save changes to this note without risking what is in it — it
              is very long, or its copy on iCloud is out of step with your devices — so it is
              read-only here. Editing it on an Apple device is safe, and after that Pomander can
              usually edit it again.
            </p>
          </div>
        ) : (
          loaded && <EmptyState className="note-pick">Select a note.</EmptyState>
        )}
      </article>
      {confirmSheet}
    </div>
  );
}
