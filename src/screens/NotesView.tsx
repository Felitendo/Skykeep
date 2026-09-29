import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, messageOf, nestFolders, type Note, type NoteFolder } from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";

const ALL = "__all__";

const when = (seconds: number | null) => {
  if (seconds === null) return "";
  const d = new Date(seconds * 1000);
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
};

/** True when the body already opens with the title, which is Apple's usual case. */
function bodyRepeatsTitle(note: Note, body: string | null): boolean {
  const text = body?.trimStart();
  if (!text) return false;
  const title = note.title.trim();
  return title.length > 0 && text.startsWith(title);
}

/** Apple's Recently Deleted, by the record name that is the same in every language. */
const TRASH_FOLDER = "TrashFolder-CloudKit";

export function NotesView() {
  const [folders, setFolders] = useState<NoteFolder[]>([]);
  const [folder, setFolder] = useState<string>(ALL);
  const [notes, setNotes] = useState<Note[]>([]);
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { revision } = useSync();
  /** The body being typed, held apart from the note until it is saved. */
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);

  /** What is typed in the search field. While it holds anything, the list is the results. */
  const [query, setQuery] = useState("");
  const searching = query.trim().length > 0;

  const read = useCallback(async () => {
    const [found, list] = await Promise.all([
      api.noteFolders(),
      // Search reaches every folder, as Apple's does — a note is found wherever it is filed.
      searching ? api.notesSearch(query) : api.notesIn(folder === ALL ? null : folder),
    ]);
    setFolders(Array.isArray(found) ? found : []);
    setNotes(Array.isArray(list) ? list : []);
    setLoaded(true);
  }, [folder, query, searching]);

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
  }, [read, revision]);

  /**
   * Every write goes to Apple first and the library is re-read after, so nothing on screen
   * is a guess about what was stored.
   */
  const write = useCallback(
    async (run: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await run();
        await read();
        return true;
      } catch (e) {
        setError(messageOf(e));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [read]
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
        // answer under someone's cursor is how a half-saved note replaced a whole one.
        setDraft((typing) => {
          if (typing === null) setLoadedBody({ guid: openGuid, text });
          return typing;
        });
      })
      .catch((e) => {
        if (!cancelled) setError(messageOf(e));
      });
    return () => {
      cancelled = true;
    };
  }, [openGuid, revision]);

  /** What the open note says, or null while that is still unknown. */
  const body = open && loadedBody?.guid === open.guid ? loadedBody.text : null;

  const dirty = draft !== null && body !== null && draft !== body;

  /**
   * Saving is not something anyone should have to remember.
   *
   * Apple's Notes has no save button because a note is not a document you commit — it is
   * just there, and what you typed is what it says. Typing schedules a save; leaving the
   * note or the field takes it immediately, so nothing is ever waiting on a timer when you
   * walk away.
   */
  const pending = useRef<{ guid: string; body: string } | null>(null);

  const flush = useCallback(async () => {
    const next = pending.current;
    if (!next) return;
    pending.current = null;
    if (await write(() => api.noteSave(next.guid, next.body))) {
      // The draft is kept, not cleared. Clearing it handed the box back to whatever the
      // library last handed over, and a save is followed by a refresh — so for a moment the
      // box showed text older than what had just been typed, and the next keystroke was
      // typed onto that and saved over the real note. A note went from five lines to a
      // single letter that way.
      //
      // While someone is in a note, what is on their screen is the note. It is given up only
      // when they leave it.
      setSaved(true);
    }
  }, [write]);

  const type = useCallback(
    (typed: string) => {
      // Nothing may be typed into a note whose text has not arrived.
      if (!open || body === null) return;
      setDraft(typed);
      setSaved(false);
      pending.current = typed === body ? null : { guid: open.guid, body: typed };
    },
    [open, body]
  );

  // A pause in typing is the signal. Long enough not to write on every keystroke, short
  // enough that closing the window a second later has already saved.
  useEffect(() => {
    if (!pending.current) return undefined;
    const timer = window.setTimeout(() => void flush(), 900);
    return () => window.clearTimeout(timer);
  }, [draft, flush]);

  // Leaving the note takes whatever is outstanding with it, before the draft is cleared.
  useEffect(() => () => void flush(), [openGuid, flush]);

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
      setLoadedBody({ guid, text: draft.body });
      if (latest !== draft.body) {
        setDraft(latest);
        pending.current = { guid, body: latest };
      } else {
        setDraft(null);
        pending.current = null;
      }
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
    },
    [composing, commitComposing]
  );

  const editor = useRef<HTMLTextAreaElement | null>(null);
  const create = useCallback(async () => {
    if (composingRef.current) await commitComposing();
    await flush();
    setComposing({ folderGuid: folder === ALL ? null : folder, body: "" });
    setOpenGuid(null);
    // Straight into the new note, as Apple's New Note does.
    window.requestAnimationFrame(() => editor.current?.focus());
  }, [commitComposing, flush, folder, setComposing]);

  /**
   * Delete goes to Recently Deleted, as on the phone; from there, Delete Now is final and is
   * asked about first, in Apple's words.
   */
  const inTrash = open?.folderGuid === TRASH_FOLDER;
  const remove = useCallback(async () => {
    if (!open) return;
    if (
      open.folderGuid === TRASH_FOLDER &&
      !window.confirm("This note will be deleted immediately. You can’t undo this action.")
    ) {
      return;
    }
    await write(() => api.noteDelete(open.guid));
  }, [open, write]);

  /** Out of Recently Deleted and back into Notes. */
  const recover = useCallback(async () => {
    if (!open) return;
    await write(() => api.noteMove(open.guid, null));
  }, [open, write]);

  const moveTo = useCallback(
    async (guid: string) => {
      if (!open) return;
      await write(() => api.noteMove(open.guid, guid === ALL ? null : guid));
    },
    [open, write]
  );

  // A save confirmation is a moment, not a state.
  useEffect(() => {
    if (!saved) return undefined;
    const timer = window.setTimeout(() => setSaved(false), 2000);
    return () => window.clearTimeout(timer);
  }, [saved]);

  return (
    <div className="notes">
      <aside className="notes-folders" aria-label="Note folders">
        <button
          className="rem-list"
          aria-current={folder === ALL}
          onClick={() => setFolder(ALL)}
        >
          <span className="rem-list-name">All Notes</span>
        </button>
        {nestFolders(folders).map(({ folder: f, depth }) => (
          <button
            key={f.guid}
            className="rem-list"
            aria-current={folder === f.guid}
            onClick={() => setFolder(f.guid)}
            /* A folder inside another is shown inside it, the way it was filed. */
            style={depth > 0 ? { paddingLeft: `${8 + depth * 14}px` } : undefined}
          >
            <span className="rem-list-name">{f.title}</span>
          </button>
        ))}
      </aside>

      <section className="notes-list" aria-label="Notes">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>
              {searching
                ? "Search"
                : folder === ALL
                  ? "All Notes"
                  : folders.find((f) => f.guid === folder)?.title}
            </h1>
            <span className="badge">
              {notes.length} {notes.length === 1 ? "note" : "notes"}
            </span>
          </div>
          <div className="cal-actions">
            <button className="quiet" onClick={() => void create()} disabled={busy}>
              New note
            </button>
          </div>
        </header>
        <input
          className="notes-search"
          type="search"
          placeholder="Search"
          aria-label="Search notes"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
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
              { value: ALL, label: "All Notes" },
              ...folders.map((f) => ({ value: f.guid, label: f.title })),
            ]}
          />
        </div>

        {error && <p className="error" role="alert">{error}</p>}

        <ul className="note-rows">
          {notes.map((note) => (
            <li key={note.guid}>
              <button
                className="note-row"
                aria-current={openGuid === note.guid}
                onClick={() => void selectNote(note.guid)}
              >
                <span className="note-row-title">
                  {note.pinned && <span className="pin" aria-label="Pinned">●</span>}
                  {note.title}
                </span>
                <span className="note-row-meta">
                  <span className="note-row-when">{when(note.modifiedAt)}</span>
                  <span className="note-row-snippet">
                    {note.locked ? "Locked" : note.snippet?.slice(0, 60) || "No text"}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>

        {loaded && notes.length === 0 && !error && (
          <p className="cal-empty">No notes here.</p>
        )}
      </section>

      <article className="note-body" aria-label="Note">
        {/*
          * One editor for a new note and a saved one, always in the same place, so the box
          * being typed into is never taken away when a new note becomes a real one. The
          * slots above it are held open with `false` rather than removed, which is what lets
          * React keep the same element — focus, cursor and selection with it.
          */}
        {!composing && open && !open.locked && !bodyRepeatsTitle(open, body) && (
          <h2>{open.title}</h2>
        )}
        {!composing && open && !open.locked && (
          <div className="note-bar">
            <p className="note-when">
              {busy ? "Saving…" : dirty ? "Saving…" : saved ? "Saved" : when(open.modifiedAt)}
            </p>
            <div className="note-bar-actions">
              <Picker
                label={`Folder for ${open.title}`}
                value={open.folderGuid ?? ALL}
                disabled={busy}
                onChange={(guid) => void moveTo(guid)}
                options={[
                  { value: ALL, label: "No folder" },
                  ...folders.map((f) => ({ value: f.guid, label: f.title })),
                ]}
              />
              {inTrash && (
                <button className="quiet" onClick={() => void recover()} disabled={busy}>
                  Recover
                </button>
              )}
              <button className="quiet danger" onClick={() => void remove()} disabled={busy}>
                {inTrash ? "Delete Now" : "Delete"}
              </button>
            </div>
          </div>
        )}
        {composing || (open && !open.locked && open.editable && body !== null) ? (
          <textarea
            key="editor"
            ref={editor}
            className="note-text"
            aria-label={composing ? "New note" : `Body of ${open?.title ?? "note"}`}
            value={composing ? composing.body : (draft ?? body ?? "")}
            placeholder={composing ? "New Note" : undefined}
            onChange={(e) => {
              const text = e.target.value;
              if (composingRef.current) {
                setComposing((c) => (c ? { ...c, body: text } : c));
              } else {
                type(text);
              }
            }}
            onKeyDown={(e) => {
              // Nothing needs saving by hand, but the reflex deserves an answer.
              if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                e.preventDefault();
                void (composingRef.current ? commitComposing() : flush());
              }
            }}
            onBlur={() => void (composingRef.current ? commitComposing() : flush())}
            spellCheck
          />
        ) : open?.locked ? (
          <p className="cal-empty">
            This note is locked. Apple keeps it encrypted, so there is nothing here to show —
            open it on a device where you can enter its password.
          </p>
        ) : open && body === null ? (
          /* Nothing to type into until the note's own text has arrived: an empty box over a
             note whose body had not loaded would write that emptiness back. */
          <p className="note-loading" aria-live="polite">
            Opening…
          </p>
        ) : open ? (
          <>
            <pre className="note-text">{body}</pre>
            <p className="note-readonly">
              Quince can’t yet save changes to this note without risking what is in it — it
              is very long, or its copy on iCloud is out of step with your devices — so it is
              read-only here. Editing it on an Apple device is safe, and after that Quince can
              usually edit it again.
            </p>
          </>
        ) : (
          loaded && <p className="cal-empty">Select a note.</p>
        )}
      </article>
    </div>
  );
}
