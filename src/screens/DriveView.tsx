import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  driveName,
  isContainer,
  messageOf,
  DRIVE_ROOT,
  type DriveItem,
} from "../lib/ipc";

const KB = 1024;

function fileSize(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < KB) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / KB;
  let unit = 0;
  while (value >= KB && unit < units.length - 1) {
    value /= KB;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

const modified = (seconds: number | null) =>
  seconds === null
    ? ""
    : new Date(seconds * 1000).toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
        year: "numeric",
      });

/** A folder glyph for containers, and the extension for files. */
function badge(item: DriveItem): string {
  if (item.kind === "APP_LIBRARY") return "APP";
  if (item.kind === "FOLDER") return "▸";
  return (item.extension || "file").toUpperCase().slice(0, 4);
}

type Crumb = { id: string; name: string };

export function DriveView() {
  const [trail, setTrail] = useState<Crumb[]>([{ id: DRIVE_ROOT, name: "iCloud Drive" }]);
  const [items, setItems] = useState<DriveItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyGuid, setBusyGuid] = useState<string | null>(null);
  /** A file just saved to Downloads, and where — shown for a moment, then cleared. */
  const [savedTo, setSavedTo] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  /** The item a Move dialog is open for, if any. */
  const [moving, setMoving] = useState<DriveItem | null>(null);

  const here = trail[trail.length - 1];

  // A folder load is asynchronous and the id it was for is not always the one still on
  // screen by the time it resolves — a fast double click used to let a second, third,
  // fourth open() stack navigation onto a folder that had not loaded yet, each one seeing
  // the same stale row and appending an identical crumb for it. This is what a real trail
  // fifteen "Downloads" deep, all pointing at the one actual folder, turned out to be.
  const openToken = useRef(0);

  const open = useCallback(async (folderId: string) => {
    const token = ++openToken.current;
    setLoading(true);
    setError(null);
    try {
      // Show what is stored first, then fetch — a folder visited before opens instantly.
      const cached = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      if (Array.isArray(cached) && cached.length > 0) setItems(cached);
      await api.driveSyncFolder(folderId);
      if (token !== openToken.current) return;
      const fresh = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      setItems(Array.isArray(fresh) ? fresh : []);
    } catch (e) {
      if (token === openToken.current) setError(messageOf(e));
    } finally {
      if (token === openToken.current) {
        setLoading(false);
        setLoaded(true);
      }
    }
  }, []);

  useEffect(() => {
    void open(here.id);
  }, [open, here.id]);

  function enter(item: DriveItem) {
    if (!isContainer(item)) return;
    // Never append a crumb for the folder already open — the second half of the fix, so
    // even a click that does land twice cannot duplicate the trail.
    if (item.drivewsid === here.id) return;
    setCreatingFolder(false);
    setTrail((t) => [...t, { id: item.drivewsid, name: driveName(item) }]);
  }

  const refresh = useCallback(() => open(here.id), [open, here.id]);

  const download = useCallback(async (item: DriveItem) => {
    setBusyGuid(item.drivewsid);
    setError(null);
    try {
      const path = await api.driveDownload(item.drivewsid);
      setSavedTo(path);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusyGuid(null);
    }
  }, []);

  // The confirmation is a moment, not a state that lingers over the next thing you do.
  useEffect(() => {
    if (!savedTo) return undefined;
    const timer = window.setTimeout(() => setSavedTo(null), 4000);
    return () => window.clearTimeout(timer);
  }, [savedTo]);

  const rename = useCallback(
    async (item: DriveItem, name: string) => {
      setRenaming(null);
      const trimmed = name.trim();
      if (!trimmed || trimmed === driveName(item)) return;
      setBusyGuid(item.drivewsid);
      setError(null);
      try {
        await api.driveRename(item.drivewsid, trimmed);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setBusyGuid(null);
      }
    },
    [refresh]
  );

  const remove = useCallback(
    async (item: DriveItem) => {
      if (
        !window.confirm(
          `Move "${driveName(item)}" to Recently Deleted?${
            isContainer(item) ? " Everything inside it goes with it." : ""
          }`
        )
      ) {
        return;
      }
      setBusyGuid(item.drivewsid);
      setError(null);
      try {
        await api.driveDelete(item.drivewsid);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setBusyGuid(null);
      }
    },
    [refresh]
  );

  const move = useCallback(
    async (item: DriveItem, destinationId: string) => {
      setMoving(null);
      setBusyGuid(item.drivewsid);
      setError(null);
      try {
        await api.driveMove(item.drivewsid, destinationId);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setBusyGuid(null);
      }
    },
    [refresh]
  );

  const createFolder = useCallback(async () => {
    const name = newFolderName.trim();
    setCreatingFolder(false);
    setNewFolderName("");
    if (!name) return;
    setError(null);
    try {
      await api.driveCreateFolder(here.id, name);
      await refresh();
    } catch (e) {
      setError(messageOf(e));
    }
  }, [newFolderName, here.id, refresh]);

  return (
    <div className="drive">
      <header className="cal-bar">
        <nav className="crumbs" aria-label="Folder path">
          {trail.map((crumb, i) => (
            <span key={crumb.id}>
              {i > 0 && (
                <span className="crumb-sep" aria-hidden="true">
                  ›
                </span>
              )}
              <button
                className="crumb"
                aria-current={i === trail.length - 1}
                onClick={() => setTrail((t) => t.slice(0, i + 1))}
              >
                {crumb.name}
              </button>
            </span>
          ))}
        </nav>
        <div className="cal-actions">
          <button
            className="quiet"
            onClick={() => setCreatingFolder(true)}
            disabled={loading || creatingFolder}
          >
            New folder
          </button>
          <button className="quiet" onClick={() => void refresh()} disabled={loading}>
            {loading ? "Refreshing…" : "Refresh"}
          </button>
        </div>
      </header>

      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      {savedTo && (
        <p className="drive-saved" role="status">
          Saved to {savedTo}
        </p>
      )}

      <ul className="drive-rows">
        {creatingFolder && (
          <li>
            <form
              className="drive-row drive-row-new"
              onSubmit={(e) => {
                e.preventDefault();
                void createFolder();
              }}
            >
              <span className="drive-badge folder" aria-hidden="true">
                ▸
              </span>
              <input
                autoFocus
                value={newFolderName}
                onChange={(e) => setNewFolderName(e.target.value)}
                onBlur={() => void createFolder()}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.currentTarget.value = "";
                    setNewFolderName("");
                    setCreatingFolder(false);
                  }
                }}
                placeholder="Untitled Folder"
                aria-label="New folder name"
              />
            </form>
          </li>
        )}

        {items.map((item) => {
          const container = isContainer(item);
          const busy = busyGuid === item.drivewsid;
          return (
            <li key={item.drivewsid}>
              {renaming === item.drivewsid ? (
                <div className="drive-row">
                  <span className={`drive-badge${container ? " folder" : ""}`} aria-hidden="true">
                    {badge(item)}
                  </span>
                  <input
                    className="drive-rename-input"
                    autoFocus
                    defaultValue={driveName(item)}
                    aria-label={`Rename ${driveName(item)}`}
                    onBlur={(e) => void rename(item, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") e.currentTarget.blur();
                      if (e.key === "Escape") {
                        e.currentTarget.value = driveName(item);
                        setRenaming(null);
                      }
                    }}
                  />
                </div>
              ) : (
                <div className={`drive-row${container ? " is-folder" : ""}${busy ? " is-busy" : ""}`}>
                  <button
                    className="drive-row-hit"
                    // A folder opens; a file downloads to Downloads and reveals itself
                    // there — the same gesture either way, matching what a click on a row
                    // does in every real file browser.
                    onClick={() => (container ? enter(item) : void download(item))}
                    disabled={busy}
                  >
                    <span className={`drive-badge${container ? " folder" : ""}`} aria-hidden="true">
                      {busy ? "…" : badge(item)}
                    </span>
                    <span className="drive-name">{driveName(item)}</span>
                    <span className="drive-meta">
                      {container
                        ? item.childCount === null
                          ? ""
                          : `${item.childCount} item${item.childCount === 1 ? "" : "s"}`
                        : fileSize(item.size)}
                    </span>
                    <span className="drive-meta">{modified(item.modifiedAt ?? item.createdAt)}</span>
                  </button>
                  <span className="drive-row-actions">
                    <button
                      className="rem-list-action"
                      aria-label={`Move ${driveName(item)}`}
                      disabled={busy}
                      onClick={() => setMoving(item)}
                    >
                      ⇥
                    </button>
                    <button
                      className="rem-list-action"
                      aria-label={`Rename ${driveName(item)}`}
                      disabled={busy}
                      onClick={() => setRenaming(item.drivewsid)}
                    >
                      ✎
                    </button>
                    <button
                      className="rem-list-action danger"
                      aria-label={`Delete ${driveName(item)}`}
                      disabled={busy}
                      onClick={() => void remove(item)}
                    >
                      ✕
                    </button>
                  </span>
                </div>
              )}
            </li>
          );
        })}
      </ul>

      {loaded && items.length === 0 && !creatingFolder && !error && (
        <p className="cal-empty">This folder is empty.</p>
      )}

      {moving && (
        <MoveDialog
          item={moving}
          startingIn={here.id}
          onCancel={() => setMoving(null)}
          onMove={(destinationId) => void move(moving, destinationId)}
        />
      )}
    </div>
  );
}

/**
 * Where to move something — a small folder browser of its own, in the same shape Files'
 * own Move sheet takes: breadcrumbs, folders only, a Move button that names where it will
 * land. You cannot navigate into the item being moved, which is what stops the one obvious
 * way to move something inside itself.
 */
function MoveDialog({
  item,
  startingIn,
  onCancel,
  onMove,
}: {
  item: DriveItem;
  startingIn: string;
  onCancel: () => void;
  onMove: (destinationId: string) => void;
}) {
  const [trail, setTrail] = useState<Crumb[]>([{ id: DRIVE_ROOT, name: "iCloud Drive" }]);
  const [folders, setFolders] = useState<DriveItem[]>([]);
  const [loading, setLoading] = useState(false);

  const here = trail[trail.length - 1];

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    (async () => {
      try {
        await api.driveSyncFolder(here.id);
        const fresh = await api.driveChildren(here.id);
        if (!cancelled) {
          setFolders(
            (Array.isArray(fresh) ? fresh : []).filter(
              (i) => isContainer(i) && i.drivewsid !== item.drivewsid
            )
          );
        }
      } catch {
        if (!cancelled) setFolders([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [here.id, item.drivewsid]);

  const alreadyHere = here.id === startingIn;

  // A sheet is modal: Escape leaves it exactly as Cancel would, and it must not be possible
  // to tab out into the page still open underneath.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  // JSX autoFocus loses a real race: the click that opened this dialog also gives the
  // trigger button native focus, and that lands after React's own autoFocus handling —
  // so a keyboard user who opened this with Enter stayed on the row behind it, in a dialog
  // that was visually in front but not where their next Tab or Escape would act. Moving
  // focus explicitly, one tick later, wins the race properly.
  const cancelRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    const id = window.setTimeout(() => cancelRef.current?.focus(), 0);
    return () => window.clearTimeout(id);
  }, []);

  return (
    <div className="modal-veil" onClick={onCancel}>
      <div
        className="modal-sheet move-sheet"
        role="dialog"
        aria-label={`Move ${driveName(item)}`}
        onClick={(e) => e.stopPropagation()}
      >
        <h2>Move “{driveName(item)}”</h2>

        <nav className="crumbs" aria-label="Destination folder">
          {trail.map((crumb, i) => (
            <span key={crumb.id}>
              {i > 0 && (
                <span className="crumb-sep" aria-hidden="true">
                  ›
                </span>
              )}
              <button
                className="crumb"
                aria-current={i === trail.length - 1}
                onClick={() => setTrail((t) => t.slice(0, i + 1))}
              >
                {crumb.name}
              </button>
            </span>
          ))}
        </nav>

        <ul className="move-sheet-list">
          {loading && <li className="cal-empty">Loading…</li>}
          {!loading && folders.length === 0 && (
            <li className="cal-empty">No folders here.</li>
          )}
          {!loading &&
            folders.map((folder) => (
              <li key={folder.drivewsid}>
                <button
                  className="drive-row-hit move-sheet-row"
                  onClick={() =>
                    setTrail((t) => [...t, { id: folder.drivewsid, name: driveName(folder) }])
                  }
                >
                  <span className="drive-badge folder" aria-hidden="true">
                    ▸
                  </span>
                  <span className="drive-name">{driveName(folder)}</span>
                </button>
              </li>
            ))}
        </ul>

        <div className="cal-form-actions">
          <button
            className="primary"
            disabled={alreadyHere}
            onClick={() => onMove(here.id)}
          >
            Move here
          </button>
          <button className="quiet" ref={cancelRef} onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
