import { useCallback, useEffect, useMemo, useState } from "react";
import { api, messageOf, type Note, type NoteFolder } from "../lib/ipc";
import { useSync } from "../lib/sync";

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
function bodyRepeatsTitle(note: Note): boolean {
  const body = note.body?.trimStart();
  if (!body) return false;
  const title = note.title.trim();
  return title.length > 0 && body.startsWith(title);
}

export function NotesView() {
  const [folders, setFolders] = useState<NoteFolder[]>([]);
  const [folder, setFolder] = useState<string>(ALL);
  const [notes, setNotes] = useState<Note[]>([]);
  const [openGuid, setOpenGuid] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { revision } = useSync();

  const read = useCallback(async () => {
    const [found, list] = await Promise.all([
      api.noteFolders(),
      api.notesIn(folder === ALL ? null : folder),
    ]);
    setFolders(Array.isArray(found) ? found : []);
    setNotes(Array.isArray(list) ? list : []);
    setLoaded(true);
  }, [folder]);

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
        {folders.map((f) => (
          <button
            key={f.guid}
            className="rem-list"
            aria-current={folder === f.guid}
            onClick={() => setFolder(f.guid)}
          >
            <span className="rem-list-name">{f.title}</span>
          </button>
        ))}
      </aside>

      <section className="notes-list" aria-label="Notes">
        <header className="cal-bar">
          <div className="cal-nav">
            <h1>{folder === ALL ? "All Notes" : folders.find((f) => f.guid === folder)?.title}</h1>
            <span className="badge">
              {notes.length} {notes.length === 1 ? "note" : "notes"}
            </span>
          </div>
        </header>

        {/* Shown only when the folder column has folded away. */}
        <div className="notes-folder-picker">
          <select
            value={folder}
            onChange={(e) => setFolder(e.target.value)}
            aria-label="Folder"
          >
            <option value={ALL}>All Notes</option>
            {folders.map((f) => (
              <option key={f.guid} value={f.guid}>
                {f.title}
              </option>
            ))}
          </select>
        </div>

        {error && <p className="error" role="alert">{error}</p>}

        <ul className="note-rows">
          {notes.map((note) => (
            <li key={note.guid}>
              <button
                className="note-row"
                aria-current={openGuid === note.guid}
                onClick={() => setOpenGuid(note.guid)}
              >
                <span className="note-row-title">
                  {note.pinned && <span className="pin" aria-label="Pinned">●</span>}
                  {note.title}
                </span>
                <span className="note-row-meta">
                  <span className="note-row-when">{when(note.modifiedAt)}</span>
                  <span className="note-row-snippet">
                    {note.locked ? "Locked" : note.snippet || note.body?.slice(0, 60) || "No text"}
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
        {open ? (
          open.locked ? (
            <p className="cal-empty">
              This note is locked. Apple keeps it encrypted, so there is nothing here to
              show — open it on a device where you can enter its password.
            </p>
          ) : (
            <>
              {/* Apple derives a note's title from its first line, so the body already
                  contains it. Rendering both shows the same sentence twice. */}
              {!bodyRepeatsTitle(open) && <h2>{open.title}</h2>}
              <p className="note-when">{when(open.modifiedAt)}</p>
              <pre className="note-text">{open.body ?? ""}</pre>
            </>
          )
        ) : (
          loaded && <p className="cal-empty">Select a note.</p>
        )}
      </article>
    </div>
  );
}
