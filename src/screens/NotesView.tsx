import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Notice } from "../components/Notice";
import { flushSync } from "react-dom";
import { ChevronLeft, Folder, FolderOpen, Pin, PinOff, SquarePen, Trash2 } from "lucide-react";
import { LongTextarea } from "../components/LongTextarea";
import { editorDate, groupNotes, joinTitle, listDate, splitTitle } from "./notes/model";
import { api, messageOf, nestFolders, type Note, type NoteFolder } from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Picker } from "../components/Picker";

const ALL = "__all__";

/** Apple's Recently Deleted, by the record name that is the same in every language. */
const TRASH_FOLDER = "TrashFolder-CloudKit";

export function NotesView() {
  const [folders, setFolders] = useState<NoteFolder[]>([]);
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
  const { revision } = useSync();
  /** The body being typed, held apart from the note until it is saved. */
  const [draft, setDraft] = useState<string | null>(null);
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
      setDetail(true);
    },
    [composing, commitComposing]
  );

  const editor = useRef<HTMLTextAreaElement | null>(null);
  const bodyEditor = useRef<HTMLTextAreaElement | null>(null);
  const create = useCallback(async () => {
    if (composingRef.current) await commitComposing();
    await flush();
    setComposing({ folderGuid: folder === ALL ? null : folder, body: "" });
    setOpenGuid(null);
    setDetail(true);
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

  const pin = useCallback(async () => {
    if (!open) return;
    await write(() => api.notePin(open.guid, !open.pinned));
  }, [open, write]);

  // A save confirmation is a moment, not a state.
  useEffect(() => {
    if (!saved) return undefined;
    const timer = window.setTimeout(() => setSaved(false), 2000);
    return () => window.clearTimeout(timer);
  }, [saved]);

  /** What the editor holds right now, whoever is writing it. */
  const text = composing ? composing.body : (draft ?? body ?? "");
  const { title: titleLine, rest } = splitTitle(text);
  const hadBreak = text.includes("\n");
  const canEdit = Boolean(composing) || Boolean(open && !open.locked && open.editable && body !== null);

  /** Every change to either half of the editor becomes one change to the whole note. */
  const apply = (next: string) => {
    if (composingRef.current) setComposing((c) => (c ? { ...c, body: next } : c));
    else type(next);
  };
  /**
   * Make a change that moves the cursor into the other half, all at once. Waiting a frame to
   * move focus left a gap a fast typist fills: the letter after Enter landed back in the
   * title. The change is applied and drawn synchronously, and focus moves in the same turn.
   */
  const moveWith = (next: string, el: HTMLTextAreaElement | null, at: number) => {
    flushSync(() => apply(next));
    if (!el) return;
    el.focus();
    el.setSelectionRange(at, at);
  };
  const place = (el: HTMLTextAreaElement | null, at: number) => {
    if (!el) return;
    el.focus();
    el.setSelectionRange(at, at);
  };

  // The title grows onto more lines as it is typed, as a heading does, rather than scrolling.
  useLayoutEffect(() => {
    const el = editor.current;
    if (!el) return;
    el.style.height = "0px";
    el.style.height = `${el.scrollHeight}px`;
    // `detail` too: in a narrow window the note is hidden until opened, and a hidden title
    // measures nothing — it came up zero lines tall, and the note had no title.
  }, [titleLine, openGuid, composing, detail]);

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
    <div className={`notes${detail ? " is-detail" : ""}`}>
      <aside className="notes-folders" aria-label="Note folders">
        <p className="notes-side-heading">iCloud</p>
        <button className="notes-folder" aria-current={folder === ALL} onClick={() => setFolder(ALL)}>
          <FolderOpen className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
          <span className="notes-folder-name">All iCloud</span>
          <span className="notes-folder-count">{counts.all || ""}</span>
        </button>
        {nestFolders(folders)
          .filter(({ folder: f }) => f.guid !== TRASH_FOLDER)
          .map(({ folder: f, depth }) => (
            <button
              key={f.guid}
              className="notes-folder"
              aria-current={folder === f.guid}
              onClick={() => setFolder(f.guid)}
              style={depth > 0 ? { paddingLeft: `${8 + depth * 16}px` } : undefined}
            >
              <Folder className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
              <span className="notes-folder-name">{f.title}</span>
              <span className="notes-folder-count">{counts.per.get(f.guid) || ""}</span>
            </button>
          ))}
        {folders.some((f) => f.guid === TRASH_FOLDER) && (
          <button
            className="notes-folder"
            aria-current={folder === TRASH_FOLDER}
            onClick={() => setFolder(TRASH_FOLDER)}
          >
            <Trash2 className="notes-folder-glyph" size={16} strokeWidth={2} aria-hidden="true" />
            <span className="notes-folder-name">Recently Deleted</span>
            <span className="notes-folder-count">{counts.per.get(TRASH_FOLDER) || ""}</span>
          </button>
        )}
      </aside>

      <section className="notes-list" aria-label="Notes">
        <header className="notes-list-head">
          <div className="notes-list-title">
            <h1>{heading}</h1>
            <span className="notes-list-count">
              {notes.length} {notes.length === 1 ? "note" : "notes"}
            </span>
          </div>
          <button
            className="notes-icon"
            aria-label="New note"
            title="New Note"
            onClick={() => void create()}
            disabled={busy}
          >
            <SquarePen size={18} strokeWidth={2} aria-hidden="true" />
          </button>
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
              { value: ALL, label: "All iCloud" },
              ...folders.map((f) => ({ value: f.guid, label: f.title })),
            ]}
          />
        </div>

        {error && <Notice message={error} onDismiss={() => setError(null)} />}

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
                    <span className="note-row-title">{note.title}</span>
                    <span className="note-row-meta">
                      <span className="note-row-when">{listDate(note.modifiedAt)}</span>
                      <span className="note-row-snippet">
                        {note.locked ? "Locked" : note.snippet?.slice(0, 80) || "No additional text"}
                      </span>
                    </span>
                  </button>
                </div>
              ))}
            </Fragment>
          ))}
        </div>

        {loaded && notes.length === 0 && !error && (
          <p className="cal-empty notes-empty">{searching ? "No results." : "No notes here."}</p>
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
                : saved
                  ? "Saved"
                  : editorDate(composing ? Math.floor(Date.now() / 1000) : (open?.modifiedAt ?? null))}
            </p>
            {!composing && open && !open.locked && (
              <div className="note-bar-actions">
                {!inTrash && <Picker
                  label={`Folder for ${open.title}`}
                  value={open.folderGuid ?? ALL}
                  disabled={busy}
                  onChange={(guid) => void moveTo(guid)}
                  options={[
                    // "No folder" only for an account without Apple's default "Notes" folder;
                    // with one, it was a second "Notes" that meant something else.
                    ...(folders.some((f) => f.guid === "DefaultFolder-CloudKit")
                      ? []
                      : [{ value: ALL, label: "Notes" }]),
                    ...folders
                      .filter((f) => f.guid !== TRASH_FOLDER)
                      .map((f) => ({ value: f.guid, label: f.title })),
                  ]}
                />}
                {!inTrash && (
                  <button
                    className="notes-icon"
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
                  <button className="quiet" onClick={() => void recover()} disabled={busy}>
                    Recover
                  </button>
                )}
                {inTrash ? (
                  <button className="quiet danger" onClick={() => void remove()} disabled={busy}>
                    Delete Now
                  </button>
                ) : (
                  <button
                    className="notes-icon danger"
                    aria-label="Delete"
                    title="Delete"
                    onClick={() => void remove()}
                    disabled={busy}
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
          <div className="note-editor">
            <textarea
              key="title"
              ref={editor}
              className="note-title"
              rows={1}
              aria-label={composing ? "New note" : `Title of ${open?.title ?? "note"}`}
              placeholder={composing ? "New Note" : undefined}
              value={titleLine}
              spellCheck
              onChange={(e) => apply(joinTitle(e.target.value, rest, hadBreak))}
              onKeyDown={(e) => {
                const el = e.currentTarget;
                if (e.key === "Enter") {
                  e.preventDefault();
                  const at = el.selectionStart;
                  moveWith(
                    `${titleLine.slice(0, at)}\n${titleLine.slice(at)}${hadBreak ? `\n${rest}` : ""}`,
                    bodyEditor.current,
                    0
                  );
                } else if (e.key === "ArrowDown" && el.selectionStart === titleLine.length) {
                  e.preventDefault();
                  place(bodyEditor.current, 0);
                } else if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                  e.preventDefault();
                  void (composingRef.current ? commitComposing() : flush());
                }
              }}
              onBlur={(e) => {
                if (e.relatedTarget === bodyEditor.current) return;
                void (composingRef.current ? commitComposing() : flush());
              }}
            />
            <LongTextarea
              key="body"
              ref={bodyEditor}
              className="note-text"
              aria-label={composing ? "New note text" : `Body of ${open?.title ?? "note"}`}
              value={rest}
              spellCheck
              onChange={(e) => apply(joinTitle(titleLine, e.target.value, true))}
              onKeyDown={(e) => {
                const el = e.currentTarget;
                if (e.key === "Backspace" && el.selectionStart === 0 && el.selectionEnd === 0 && hadBreak) {
                  e.preventDefault();
                  moveWith(titleLine + rest, editor.current, titleLine.length);
                } else if (e.key === "ArrowUp" && !rest.slice(0, el.selectionStart).includes("\n")) {
                  e.preventDefault();
                  place(editor.current, titleLine.length);
                } else if ((e.metaKey || e.ctrlKey) && e.key === "s") {
                  e.preventDefault();
                  void (composingRef.current ? commitComposing() : flush());
                }
              }}
              onBlur={(e) => {
                if (e.relatedTarget === editor.current) return;
                void (composingRef.current ? commitComposing() : flush());
              }}
            />
          </div>
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
          <div className="note-editor is-readonly">
            <h1 className="note-title">{splitTitle(body ?? "").title}</h1>
            <pre className="note-text">{splitTitle(body ?? "").rest}</pre>
            <p className="note-readonly">
              Quince can’t yet save changes to this note without risking what is in it — it
              is very long, or its copy on iCloud is out of step with your devices — so it is
              read-only here. Editing it on an Apple device is safe, and after that Quince can
              usually edit it again.
            </p>
          </div>
        ) : (
          loaded && <p className="cal-empty">Select a note.</p>
        )}
      </article>
    </div>
  );
}
