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

  // Keep a note open across refreshes where possible; fall back to the first.
  useEffect(() => {
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
  useEffect(() => {
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
  const [composing, setComposing] = useState<{ folderGuid: string | null; body: string } | null>(
    null
  );

  const commitComposing = useCallback(async () => {
    const draft = composing;
    if (!draft) return;
    if (!draft.body.trim()) {
      // Nothing was ever typed. There is nothing to keep, so nothing is sent.
      setComposing(null);
      return;
    }
    setComposing(null);
    setBusy(true);
    setError(null);
    try {
      const guid = await api.noteCreate(draft.folderGuid, draft.body);
      await read();
      setOpenGuid(guid);
    } catch (e) {
      setError(messageOf(e));
      // The write failed, not the typing — put it back rather than lose it.
      setComposing(draft);
    } finally {
      setBusy(false);
    }
  }, [composing, read]);

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

  const create = useCallback(async () => {
    if (composing) await commitComposing();
    setOpenGuid(null);
    setComposing({ folderGuid: folder === ALL ? null : folder, body: "" });
  }, [composing, commitComposing, folder]);

  const remove = useCallback(async () => {
    if (!open) return;
    await write(() => api.noteDelete(open.guid));
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
        {composing ? (
          <textarea
            key="composing"
            className="note-text"
            aria-label="New note"
            autoFocus
            value={composing.body}
            onChange={(e) => setComposing((c) => (c ? { ...c, body: e.target.value } : c))}
            onBlur={() => void commitComposing()}
            placeholder="New Note"
            spellCheck
          />
        ) : open ? (
          open.locked ? (
            <p className="cal-empty">
              This note is locked. Apple keeps it encrypted, so there is nothing here to
              show — open it on a device where you can enter its password.
            </p>
          ) : (
            <>
              {/* Apple derives a note's title from its first line, so the body already
                  contains it. Rendering both shows the same sentence twice. */}
              {!bodyRepeatsTitle(open, body) && <h2>{open.title}</h2>}

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
                  <button className="quiet danger" onClick={() => void remove()} disabled={busy}>
                    Delete
                  </button>
                </div>
              </div>

              {/*
                * Nothing is shown to type into until the note's own text has arrived. An
                * empty box over a note whose body had not loaded would write that emptiness
                * back the moment anyone touched it.
                */}
              {body === null ? (
                <p className="note-loading" aria-live="polite">
                  Opening…
                </p>
              ) : open.editable ? (
                <textarea
                  className="note-text"
                  aria-label={`Body of ${open.title}`}
                  value={draft ?? body}
                  onChange={(e) => type(e.target.value)}
                  onKeyDown={(e) => {
                    // Nothing needs saving by hand, but the reflex deserves an answer.
                    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                      e.preventDefault();
                      void flush();
                    }
                  }}
                  onBlur={() => void flush()}
                  spellCheck
                />
              ) : (
                <>
                  <pre className="note-text">{body}</pre>
                  <p className="note-readonly">
                    This note holds formatting Quince can't rewrite without losing it, so it
                    stays read-only here. Editing it on an Apple device is safe.
                  </p>
                </>
              )}
            </>
          )
        ) : (
          loaded && <p className="cal-empty">Select a note.</p>
        )}
      </article>
    </div>
  );
}
