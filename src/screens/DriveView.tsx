import { useCallback, useEffect, useRef, useState } from "react";
import { Notice } from "../components/Notice";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  acceptsUploads,
  api,
  driveName,
  isContainer,
  messageOf,
  DRIVE_ROOT,
  UPLOAD,
  type DriveItem,
  type UploadEvent,
} from "../lib/ipc";
import { ActionGlyph, FileGlyph } from "../components/FileGlyph";
import { UploadTray } from "../components/UploadTray";
import { listen } from "@tauri-apps/api/event";

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
  /** True while files are held over the window, so the drop target can say so. */
  const [dropping, setDropping] = useState(false);

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
      // Always this folder's own rows, even none: keeping the last folder's rows on screen
      // while a slow fetch ran meant a click landed on a row from the level just left, and
      // Drive seemed to jump into folders nobody opened.
      setItems(Array.isArray(cached) ? cached : []);
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

  /**
   * Send files from this machine to the folder currently open.
   *
   * Only the paths cross into the backend; the bytes are read and streamed there, which is
   * what lets a large file go up without the interface holding it in memory.
   *
   * One file Apple refuses must not look like all of them failing, so each is reported by
   * name and the rest still land.
   */
  /** Every file sent from this window, until the tray is closed. */
  const [uploads, setUploads] = useState<Map<string, UploadEvent>>(new Map());
  const [uploadMenu, setUploadMenu] = useState(false);
  /**
   * What has been asked for here and is not back from Apple yet, drawn at once as rows of
   * its own: a folder being created, files going up. Nothing showed until Apple answered —
   * seconds later — so a press seemed to do nothing, and then a folder appeared from nowhere.
   */
  const [pending, setPending] = useState<{ key: string; folder: string; name: string; doing: string }[]>([]);
  const settle = (key: string) => setPending((p) => p.filter((x) => !x.key.startsWith(key)));

  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    listen<UploadEvent>(UPLOAD, (event) => {
      const u = event.payload;
      setUploads((was) => {
        const next = new Map(was);
        const before = next.get(u.id);
        // Events for one file can pass each other; a finished file never goes back.
        if (before && (before.state === "done" || before.state === "failed")) return was;
        next.set(u.id, u);
        return next;
      });
    })
      .then((unlisten) => {
        // Stopping may throw where there is no event system behind the page — a plain
        // browser running the interface — and a screen closing must not throw for that.
        const quietly = () => {
          try {
            // It answers with a promise, whose failure would otherwise escape this try.
            void Promise.resolve(unlisten() as unknown).catch(() => {});
          } catch {
            // Nothing was listening.
          }
        };
        if (cancelled) quietly();
        else stop = quietly;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);

  const upload = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      setError(null);
      // Never blocks the window: browse, open another folder, start another upload.
      const batch = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
      const into = here.id;
      setPending((p) => [
        ...p,
        ...paths.map((path, i) => ({
          key: `${batch}:${i}`,
          folder: into,
          name: path.split(/[\\/]/).filter(Boolean).pop() ?? path,
          doing: "Uploading…",
        })),
      ]);
      try {
        await api.driveUpload(batch, into, paths);
        // What failed is named in the tray, file by file; the folder is re-read either way.
        if (into === here.id) await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        settle(batch);
      }
    },
    [here.id, refresh]
  );

  const chooseAndUpload = useCallback(
    async (folders: boolean) => {
      setUploadMenu(false);
      try {
        const chosen = await openFileDialog({
          multiple: true,
          directory: folders,
          title: folders ? "Upload folders to iCloud Drive" : "Upload to iCloud Drive",
        });
        if (!chosen) return;
        await upload(Array.isArray(chosen) ? chosen : [chosen]);
      } catch (e) {
        setError(messageOf(e));
      }
    },
    [upload]
  );

  // Dropping files on the window uploads them to the folder that is open. Tauri hands over
  // paths rather than contents, so a drop costs nothing until the file is actually read.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    // Outside the app shell — a browser running the interface on its own, as the checks do —
    // there is no webview to ask, and asking throws where it stands. Uploading by button
    // still works, so this is a capability that may be missing, not a failure.
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === "over") setDropping(true);
          else if (event.payload.type === "leave") setDropping(false);
          else if (event.payload.type === "drop") {
            setDropping(false);
            void upload(event.payload.paths);
          }
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else stop = unlisten;
        })
        .catch(() => {});
    } catch {
      // No drag and drop here.
    }
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [upload, here.id]);

  /**
   * One folder per name typed. Enter submits, which closes the field, and closing it blurs
   * the field, which submitted again with the same name still in hand — Apple made the folder
   * the first time and declined the second, and the refusal was what showed.
   */
  const folderPending = useRef(false);
  const createFolder = useCallback(async () => {
    if (folderPending.current) return;
    const name = newFolderName.trim();
    setCreatingFolder(false);
    setNewFolderName("");
    if (!name) return;
    // Finder will not put two things of one name in a folder; neither will iCloud.
    if (items.some((i) => i.name.toLocaleLowerCase() === name.toLocaleLowerCase())) {
      setError(`There's already something called \u201c${name}\u201d here.`);
      return;
    }
    folderPending.current = true;
    setError(null);
    const key = `folder-${Date.now()}`;
    setPending((p) => [...p, { key, folder: here.id, name, doing: "Creating…" }]);
    try {
      await api.driveCreateFolder(here.id, name);
      await refresh();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      folderPending.current = false;
      settle(key);
    }
  }, [newFolderName, here.id, refresh, items]);

  return (
    <div className={`drive${dropping ? " is-dropping" : ""}`}>
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
            className="quiet with-glyph"
            onClick={() => setCreatingFolder(true)}
            disabled={loading || creatingFolder}
          >
            <ActionGlyph name="newFolder" />
            New Folder
          </button>
          {/* An app's own library takes files from the app, not from here. */}
          <span className="upload-menu-wrap">
            <button
              className="primary with-glyph"
              aria-haspopup="menu"
              aria-expanded={uploadMenu}
              onClick={() => setUploadMenu((open) => !open)}
              disabled={loading || !acceptsUploads(here.id)}
              title={
                acceptsUploads(here.id)
                  ? undefined
                  : "This folder belongs to an app, which is the only thing that can add to it"
              }
            >
              <ActionGlyph name="upload" />
              Upload
            </button>
            {uploadMenu && (
              <span
                className="upload-menu"
                role="menu"
                onKeyDown={(e) => e.key === "Escape" && setUploadMenu(false)}
              >
                <button role="menuitem" autoFocus onClick={() => void chooseAndUpload(false)}>
                  Files…
                </button>
                <button role="menuitem" onClick={() => void chooseAndUpload(true)}>
                  Folder…
                </button>
              </span>
            )}
          </span>
        </div>
      </header>

      {error && (
        <Notice message={error} onDismiss={() => setError(null)} />
      )}
      {savedTo && (
        <p className="drive-saved" role="status">
          Saved to {savedTo}
        </p>
      )}

      <ul className="drive-rows">
        {pending
          .filter((p) => p.folder === here.id && !items.some((i) => driveName(i) === p.name))
          .map((p) => (
            <li key={p.key} className="drive-row is-busy" aria-busy="true">
              <span className="drive-row-hit">
                <span className="drive-badge" aria-hidden="true">
                  <span className="drive-spinner" />
                </span>
                <span className="drive-name">{p.name}</span>
                <span className="drive-meta">{p.doing}</span>
                <span className="drive-meta" />
              </span>
            </li>
          ))}
        {creatingFolder && (
          <li>
            <form
              className="drive-row drive-row-new"
              onSubmit={(e) => {
                e.preventDefault();
                void createFolder();
              }}
            >
              <span className="drive-badge" aria-hidden="true">
                <FileGlyph kind="FOLDER" extension={null} />
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
                  <span className="drive-badge" aria-hidden="true">
                    <FileGlyph kind={item.kind} extension={item.extension} />
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
                    <span className="drive-badge" aria-hidden="true">
                      {busy ? (
                        <span className="drive-spinner" />
                      ) : (
                        <FileGlyph kind={item.kind} extension={item.extension} />
                      )}
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
                    {!container && (
                      <button
                        className="drive-action"
                        aria-label={`Download ${driveName(item)}`}
                        title="Download"
                        disabled={busy}
                        onClick={() => void download(item)}
                      >
                        <ActionGlyph name="download" />
                      </button>
                    )}
                    <button
                      className="drive-action"
                      aria-label={`Move ${driveName(item)}`}
                      title="Move to…"
                      disabled={busy}
                      onClick={() => setMoving(item)}
                    >
                      <ActionGlyph name="move" />
                    </button>
                    <button
                      className="drive-action"
                      aria-label={`Rename ${driveName(item)}`}
                      title="Rename"
                      disabled={busy}
                      onClick={() => setRenaming(item.drivewsid)}
                    >
                      <ActionGlyph name="rename" />
                    </button>
                    <button
                      className="drive-action danger"
                      aria-label={`Delete ${driveName(item)}`}
                      title="Delete"
                      disabled={busy}
                      onClick={() => void remove(item)}
                    >
                      <ActionGlyph name="delete" />
                    </button>
                  </span>
                </div>
              )}
            </li>
          );
        })}
      </ul>

      {items.length === 0 && !creatingFolder && !error && (loading || !loaded) && (
        <p className="cal-empty" aria-live="polite">Opening {here.name}…</p>
      )}
      {loaded && !loading && items.length === 0 && !creatingFolder && !error && (
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

      <UploadTray uploads={[...uploads.values()]} onDismiss={() => setUploads(new Map())} />
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
                  <span className="drive-badge" aria-hidden="true">
                    <FileGlyph kind="FOLDER" extension={null} />
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
