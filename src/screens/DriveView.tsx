import { useCallback, useEffect, useRef, useState } from "react";
import { Notice } from "../components/Notice";
import { Portal } from "../components/Portal";
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
  type RecentItem,
  type TrashItem,
  type UploadEvent,
} from "../lib/ipc";
import { ActionGlyph, FileGlyph } from "../components/FileGlyph";
import { UploadTray } from "../components/UploadTray";
import { listen } from "@tauri-apps/api/event";
import { Picker } from "../components/Picker";
import { RowMenu } from "../components/RowMenu";
import { dateText, timeText } from "../lib/when";
import { Clock } from "lucide-react";

/** iCloud Drive's own four orders, as its "by Name" menu offers them. */
const SORTS = [
  { key: "name", label: "Name" },
  { key: "date", label: "Date" },
  { key: "kind", label: "Kind" },
  { key: "size", label: "Size" },
] as const;
type SortKey = (typeof SORTS)[number]["key"];
const SORT_KEY = "quince.drive.sort";

/**
 * Names compare as Finder compares them — "File 2" before "File 10", case ignored; dates and
 * sizes newest and largest first. Folders keep together at the top under every order but
 * Kind, which is itself the grouping.
 */
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
function sortItems(rows: DriveItem[], by: SortKey): DriveItem[] {
  const name = (a: DriveItem, b: DriveItem) => collator.compare(a.name, b.name);
  const folderFirst = (a: DriveItem, b: DriveItem) => Number(isContainer(b)) - Number(isContainer(a));
  const kindOf = (i: DriveItem) => (isContainer(i) ? " " : (i.extension ?? "").toLowerCase());
  const order: Record<SortKey, (a: DriveItem, b: DriveItem) => number> = {
    name: (a, b) => folderFirst(a, b) || name(a, b),
    date: (a, b) => folderFirst(a, b) || (b.modifiedAt ?? b.createdAt ?? 0) - (a.modifiedAt ?? a.createdAt ?? 0) || name(a, b),
    kind: (a, b) => collator.compare(kindOf(a), kindOf(b)) || name(a, b),
    size: (a, b) => folderFirst(a, b) || (b.size ?? 0) - (a.size ?? 0) || name(a, b),
  };
  return [...rows].sort(order[by]);
}

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
    : dateText(new Date(seconds * 1000), {
        day: "numeric",
        month: "short",
        year: "numeric",
      });

/**
 * Recently Deleted, as icloud.com shows it: what was deleted, when, and two ways out —
 * Recover, back to where it was, and Delete Now, for good. Each leaves the list the moment it
 * is asked for; a refusal puts the row back and says why. Delete Now asks first, as Apple's
 * does, because nothing brings it back.
 */
function DriveTrash({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<TrashItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const read = useCallback(async () => {
    try {
      setRows(await api.driveTrash());
    } catch (e) {
      setError(messageOf(e));
      setRows((r) => r ?? []);
    }
  }, []);
  useEffect(() => {
    void read();
  }, [read]);

  const out = async (gone: TrashItem[], run: (i: DriveItem) => Promise<void>) => {
    const ids = new Set(gone.map((i) => i.drivewsid));
    setRows((r) => (r ?? []).filter((i) => !ids.has(i.drivewsid)));
    setError(null);
    const failed = (await Promise.allSettled(gone.map(run))).find((r) => r.status === "rejected");
    if (failed) {
      setError(messageOf((failed as PromiseRejectedResult).reason));
      await read();
    }
  };
  const recover = (items: TrashItem[]) => out(items, (i) => api.drivePutBack(i));
  const destroy = (items: TrashItem[]) => {
    const what = items.length === 1 ? `\u201c${driveName(items[0])}\u201d` : `these ${items.length} items`;
    if (!window.confirm(`Delete ${what} now? You can\u2019t undo this.`)) return;
    void out(items, (i) => api.driveDeleteForever(i));
  };

  return (
    <div className="drive drive-trash">
      <header className="cal-bar">
        <nav className="crumbs" aria-label="Folder path">
          <button className="crumb" onClick={onClose}>iCloud Drive</button>
          <span className="crumb-sep" aria-hidden="true">›</span>
          <button className="crumb" aria-current>Recently Deleted</button>
        </nav>
        {rows && rows.length > 0 && (
          <div className="cal-actions">
            <button className="quiet" onClick={() => void recover(rows)}>Recover All</button>
            <button className="quiet danger" onClick={() => destroy(rows)}>Delete All</button>
          </div>
        )}
      </header>
      {error && <Notice message={error} onDismiss={() => setError(null)} />}
      {rows === null && <p className="cal-empty">Loading…</p>}
      {rows && rows.length === 0 && !error && (
        <p className="cal-empty">Nothing in Recently Deleted.</p>
      )}
      {rows && rows.length > 0 && (
        <ul className="drive-rows">
          {rows.map((item) => (
            <li key={item.drivewsid}>
              <div className="drive-row">
                <span className="drive-row-hit">
                  <span className="drive-badge" aria-hidden="true">
                    <FileGlyph kind={item.kind} extension={item.extension} />
                  </span>
                  <span className="drive-name" title={item.restorePath ? `From ${item.restorePath}` : undefined}>
                    {driveName(item)}
                  </span>
                  <span className="drive-meta drive-kind">
                    {item.restorePath ? `From ${item.restorePath.split("/").slice(0, -1).join("/") || "iCloud Drive"}` : kindName(item)}
                  </span>
                  <span className="drive-meta">{isContainer(item) ? "" : fileSize(item.size)}</span>
                  <span className={`drive-meta${daysLeft(item.expiresAt) !== null && (daysLeft(item.expiresAt) ?? 99) <= 3 ? " is-soon" : ""}`}>
                    {timeLeft(item.expiresAt)}
                  </span>
                </span>
                <span className="drive-row-actions is-shown">
                  <button className="quiet" onClick={() => void recover([item])}>Recover</button>
                  <button className="quiet danger" onClick={() => destroy([item])}>Delete Now</button>
                </span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Recents, as icloud.com and Finder have it: the files opened most recently, under Today,
 * Yesterday, Previous 7 Days, Previous 30 Days and then each month. Read from Apple each
 * time it is opened, since what was opened on the phone a minute ago belongs here too.
 */
function DriveRecents({ onClose }: { onClose: () => void }) {
  const [rows, setRows] = useState<RecentItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedTo, setSavedTo] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ item: RecentItem; x: number; y: number; alignRight: boolean } | null>(null);
  useEffect(() => {
    api.driveRecents().then(
      (found) => setRows(Array.isArray(found) ? found : []),
      (e) => {
        setError(messageOf(e));
        setRows([]);
      },
    );
  }, []);
  useEffect(() => {
    if (!savedTo) return undefined;
    const timer = window.setTimeout(() => setSavedTo(null), 4000);
    return () => window.clearTimeout(timer);
  }, [savedTo]);
  const download = async (item: RecentItem) => {
    setBusy(item.drivewsid);
    setError(null);
    try {
      setSavedTo(await api.driveDownload(item.drivewsid, item));
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  };
  const groups = groupRecents(rows ?? []);
  return (
    <div className="drive drive-recents">
      <header className="cal-bar">
        <nav className="crumbs" aria-label="Folder path">
          <button className="crumb" onClick={onClose}>iCloud Drive</button>
          <span className="crumb-sep" aria-hidden="true">›</span>
          <button className="crumb" aria-current>Recents</button>
        </nav>
      </header>
      {error && <Notice message={error} onDismiss={() => setError(null)} />}
      {savedTo && <p className="drive-saved" role="status">Saved to {savedTo}</p>}
      {rows === null && <p className="cal-empty">Loading…</p>}
      {rows && rows.length === 0 && !error && (
        <p className="cal-empty">Files which you’ve opened recently will appear here.</p>
      )}
      {groups.map((g) => (
        <section key={g.heading} aria-label={g.heading}>
          <h2 className="drive-group">{g.heading}</h2>
          <ul className="drive-rows">
            {g.items.map((item) => (
              <li key={item.drivewsid}>
                <div
                  className={`drive-row${selected === item.drivewsid ? " is-selected" : ""}${busy === item.drivewsid ? " is-busy" : ""}`}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setSelected(item.drivewsid);
                    setMenu({ item, x: e.clientX, y: e.clientY, alignRight: false });
                  }}
                >
                  <button
                    className="drive-row-hit"
                    aria-pressed={selected === item.drivewsid}
                    onClick={() => setSelected(item.drivewsid)}
                    onDoubleClick={() => void download(item)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.preventDefault();
                        void download(item);
                      }
                    }}
                  >
                    <span className="drive-badge" aria-hidden="true">
                      {busy === item.drivewsid ? <span className="drive-spinner" /> : <FileGlyph kind={item.kind} extension={item.extension} />}
                    </span>
                    <span className="drive-name">{driveName(item)}</span>
                    <span className="drive-meta drive-kind">{kindName(item)}</span>
                    <span className="drive-meta">{fileSize(item.size)}</span>
                    <span className="drive-meta">{openedText(item.openedAt)}</span>
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {menu && (
        <RowMenu
          at={menu}
          onClose={() => setMenu(null)}
          entries={[{ label: "Download a Copy", run: () => void download(menu.item) }]}
        />
      )}
    </div>
  );
}

/** Recents' headings, newest first: Today, Yesterday, Previous 7 and 30 Days, then months. */
export function groupRecents(rows: RecentItem[], now = new Date()): { heading: string; items: RecentItem[] }[] {
  const midnight = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const today = midnight(now);
  const out: { heading: string; items: RecentItem[] }[] = [];
  for (const item of rows) {
    const when = new Date((item.openedAt ?? 0) * 1000);
    const ago = Math.round((today - midnight(when)) / 864e5);
    const heading =
      ago <= 0 ? "Today"
      : ago === 1 ? "Yesterday"
      : ago <= 7 ? "Previous 7 Days"
      : ago <= 30 ? "Previous 30 Days"
      : dateText(when, when.getFullYear() === now.getFullYear() ? { month: "long" } : { month: "long", year: "numeric" });
    const last = out[out.length - 1];
    if (last?.heading === heading) last.items.push(item);
    else out.push({ heading, items: [item] });
  }
  return out;
}

const openedText = (s: number | null) => {
  if (s === null) return "";
  const d = new Date(s * 1000);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? timeText(d, { hour: "numeric", minute: "2-digit" })
    : dateText(d, { day: "numeric", month: "short", year: "numeric" });
};

/** What a file's kind is called, as icloud.com's File Info names it. */
const KINDS: Record<string, string> = {
  pdf: "PDF document", png: "PNG image", jpg: "JPEG image", jpeg: "JPEG image", heic: "HEIC image",
  gif: "GIF image", webp: "WebP image", mp4: "MPEG-4 movie", mov: "QuickTime movie", mp3: "MP3 audio",
  m4a: "Apple MPEG-4 audio", wav: "WAVE audio", txt: "Plain text document", md: "Markdown document",
  csv: "Comma-separated values", json: "JSON document", zip: "ZIP archive", html: "HTML document",
  docx: "Office Open XML word processing document", doc: "Microsoft Word document",
  xlsx: "Office Open XML spreadsheet", pptx: "Office Open XML presentation",
  pages: "Pages document", numbers: "Numbers spreadsheet", key: "Keynote presentation",
};
function kindName(item: DriveItem) {
  if (item.kind === "FOLDER") return "Folder";
  if (item.kind === "APP_LIBRARY") return "Application Library";
  const ext = (item.extension ?? "").toLowerCase();
  return KINDS[ext] ?? (ext ? `${ext.toUpperCase()} file` : "Document");
}

/** File Info, as icloud.com shows it: the mark, the name, then what it is and where. */
function InfoSheet({ item, where, thumb, onClose }: { item: DriveItem; where: string; thumb?: string; onClose: () => void }) {
  const done = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    const id = window.setTimeout(() => done.current?.focus(), 0);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => { window.clearTimeout(id); window.removeEventListener("keydown", onKey); };
  }, [onClose]);
  const when = (s: number | null) => (s === null ? "—" : new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }));
  const rows: [string, string][] = [
    ["Kind", kindName(item)],
    ...(isContainer(item)
      ? item.childCount === null ? [] : [["Contents", `${item.childCount} item${item.childCount === 1 ? "" : "s"}`] as [string, string]]
      : [["Size", fileSize(item.size)] as [string, string]]),
    ["Where", where],
    ["Created", when(item.createdAt)],
    ["Modified", when(item.modifiedAt ?? item.createdAt)],
  ];
  return (
    <Portal>
    <div className="modal-veil" onClick={onClose}>
      <div className="modal-sheet info-sheet" role="dialog" aria-label="File Info" onClick={(e) => e.stopPropagation()}>
        <header className="info-head">
          <h2>{isContainer(item) ? "Folder Info" : "File Info"}</h2>
          <button ref={done} className="quiet info-done" onClick={onClose}>Done</button>
        </header>
        <div className={`info-mark${thumb ? " has-thumb" : ""}`} aria-hidden="true">
          {thumb ? <img src={thumb} alt="" draggable={false} /> : <FileGlyph kind={item.kind} extension={item.extension} />}
        </div>
        <p className="info-name">{driveName(item)}</p>
        <dl className="info-rows">
          {rows.map(([k, v]) => (
            <div key={k}><dt>{k}</dt><dd>{v}</dd></div>
          ))}
        </dl>
      </div>
    </div>
    </Portal>
  );
}

/**
 * Sizes as icloud.com writes storage: in thousands, not 1024s — so a 5 GB plan reads "5 GB"
 * — kB with no decimals, MB with one, GB and up with two.
 */
function decimalSize(bytes: number): string {
  const units = ["bytes", "kB", "MB", "GB", "TB", "PB"];
  const places = [0, 0, 1, 2, 2, 2];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const p = 10 ** places[unit];
  return `${(Math.round(value * p) / p).toLocaleString()} ${units[unit]}`;
}

/** Days until Apple deletes it for good, rounded up; `null` when Apple did not say. */
function daysLeft(expiresAt: number | null): number | null {
  if (expiresAt === null) return null;
  return Math.max(0, Math.ceil((expiresAt * 1000 - Date.now()) / 86_400_000));
}
/** "21 days", "1 day", "Today" — icloud.com's Time Left. */
function timeLeft(expiresAt: number | null): string {
  const d = daysLeft(expiresAt);
  if (d === null) return "";
  return d === 0 ? "Today" : `${d} day${d === 1 ? "" : "s"}`;
}

type Crumb = { id: string; name: string };

export function DriveView() {
  const [trail, setTrail] = useState<Crumb[]>([{ id: DRIVE_ROOT, name: "iCloud Drive" }]);
  const [items, setItems] = useState<DriveItem[]>([]);
  const [inTrash, setInTrash] = useState(false);
  const [inRecents, setInRecents] = useState(false);
  /** Apple's own pictures of this folder's files, where it has them; the rest keep icons. */
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  /** What is free, as icloud.com says it under a folder's name: "5.18 GB available". */
  const [available, setAvailable] = useState<string | null>(null);
  useEffect(() => {
    // Shown only when Apple says: an empty or odd answer leaves the count on its own.
    api.storageInfo().then(
      (s) => setAvailable(
        s && Number.isFinite(s.totalBytes) && Number.isFinite(s.usedBytes)
          ? decimalSize(Math.max(0, s.totalBytes - s.usedBytes))
          : null),
      () => setAvailable(null),
    );
  }, []);
  /** icloud.com's "View as": Grid or List, remembered. */
  const [grid, setGrid] = useState(() => {
    try {
      return window.localStorage.getItem("quince.drive.view") === "grid";
    } catch {
      return false;
    }
  });
  const chooseView = (asGrid: boolean) => {
    setGrid(asGrid);
    try {
      window.localStorage.setItem("quince.drive.view", asGrid ? "grid" : "list");
    } catch {
      // Shown so for now; forgotten on restart.
    }
  };
  const [sortBy, setSortBy] = useState<SortKey>(() => {
    try {
      const saved = window.localStorage.getItem(SORT_KEY);
      return SORTS.some((o) => o.key === saved) ? (saved as SortKey) : "name";
    } catch {
      return "name";
    }
  });
  const chooseSort = (key: SortKey) => {
    setSortBy(key);
    try {
      window.localStorage.setItem(SORT_KEY, key);
    } catch {
      // Sorted for now; forgotten on restart.
    }
  };
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
  const [info, setInfo] = useState<DriveItem | null>(null);
  /** The one item selected, as a click in Finder selects it. */
  const [selected, setSelected] = useState<string | null>(null);
  /** The ⋯ menu, or the same menu opened by a right-click, for one item. */
  const [menu, setMenu] = useState<{ item: DriveItem; x: number; y: number; alignRight: boolean } | null>(null);
  /** An item of Drive's own being dragged onto a folder, and the folder it is over. */
  const dragItem = useRef<DriveItem | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  /** True while files are held over the window, so the drop target can say so. */
  const [dropping, setDropping] = useState(false);

  const here = trail[trail.length - 1];
  // The folder on screen now, for work that finishes later. A refresh bound to the folder
  // open when an upload or a new folder began re-read that folder when it ended — after the
  // person had gone elsewhere — and its rows replaced the folder they were in: Drive "went
  // back to the previous location".
  const hereRef = useRef(here.id);
  hereRef.current = here.id;
  /** Items deleted here that Apple's listing may still carry for a moment after. */
  const [gone, setGone] = useState<Set<string>>(new Set());

  // A folder load is asynchronous and the id it was for is not always the one still on
  // screen by the time it resolves — a fast double click used to let a second, third,
  // fourth open() stack navigation onto a folder that had not loaded yet, each one seeing
  // the same stale row and appending an identical crumb for it. This is what a real trail
  // fifteen "Downloads" deep, all pointing at the one actual folder, turned out to be.
  const openToken = useRef(0);

  // Why the last folder was left, kept on screen in the one stepped back to.
  const leftBecause = useRef<string | null>(null);

  const open = useCallback(async (folderId: string) => {
    const token = ++openToken.current;
    setLoading(true);
    setError(leftBecause.current);
    leftBecause.current = null;
    try {
      // Show what is stored first, then fetch — a folder visited before opens instantly.
      const cached = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      // Always this folder's own rows, even none: keeping the last folder's rows on screen
      // while a slow fetch ran meant a click landed on a row from the level just left, and
      // Drive seemed to jump into folders nobody opened.
      setItems(Array.isArray(cached) ? cached : []);
      setThumbs({});
      await api.driveSyncFolder(folderId);
      if (token !== openToken.current) return;
      const fresh = await api.driveChildren(folderId);
      if (token !== openToken.current) return;
      setItems(Array.isArray(fresh) ? fresh : []);
      // Pictures last, and never in the way: a folder is usable before they arrive.
      api.driveThumbnails(folderId).then(
        (t) => { if (token === openToken.current && t) setThumbs(t); },
        () => {},
      );
    } catch (e) {
      if (token !== openToken.current) return;
      setError(messageOf(e));
      // Deleted or moved on another device: the backend has forgotten it, so step back out
      // to the nearest folder still here rather than sit in one that is not.
      if (folderId !== DRIVE_ROOT && (await api.driveItem(folderId).catch(() => undefined)) === null) {
        if (token !== openToken.current) return;
        leftBecause.current = messageOf(e);
        setTrail((t) => {
          const at = t.findIndex((c) => c.id === folderId);
          return at > 0 ? t.slice(0, at) : t;
        });
      }
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

  const refresh = useCallback(() => open(hereRef.current), [open]);

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
      // The new name shows the moment Return is pressed; Apple's answer confirms or undoes it.
      const ext = item.extension ? `.${item.extension}` : "";
      const shown = ext && trimmed.endsWith(ext)
        ? { name: trimmed.slice(0, -ext.length), extension: item.extension }
        : { name: trimmed, extension: null };
      setItems((rows) => rows.map((r) => (r.drivewsid === item.drivewsid ? { ...r, ...shown } : r)));
      try {
        await api.driveRename(item.drivewsid, trimmed);
        await refresh();
      } catch (e) {
        setItems((rows) => rows.map((r) => (r.drivewsid === item.drivewsid ? item : r)));
        setError(messageOf(e));
      } finally {
        setBusyGuid(null);
      }
    },
    [refresh]
  );

  /**
   * Delete, as Finder does it: no question, since it goes to Recently Deleted and comes back
   * from there. The row leaves at once and stays gone — Apple's listing can carry it for a
   * moment, and it used to flicker back — unless Apple refuses, when it returns and says why.
   */
  const remove = useCallback(
    async (item: DriveItem) => {
      setError(null);
      setGone((g) => new Set(g).add(item.drivewsid));
      try {
        await api.driveDelete(item.drivewsid);
        await refresh();
      } catch (e) {
        setGone((g) => {
          const next = new Set(g);
          next.delete(item.drivewsid);
          return next;
        });
        setError(messageOf(e));
      }
    },
    [refresh]
  );

  /**
   * Duplicate, as Finder: the copy lands beside the original as "Name 2", and shows as being
   * made at once — Apple decides its name, so the row is a stand-in until it answers.
   */
  const duplicate = useCallback(
    async (item: DriveItem) => {
      setError(null);
      const key = `copy-${Date.now()}`;
      const ext = item.extension ? `.${item.extension}` : "";
      setPending((p) => [...p, { key, folder: here.id, name: `${item.name} copy${ext}`, doing: "Duplicating…" }]);
      try {
        await api.driveDuplicate(item.drivewsid);
        await refresh();
      } catch (e) {
        setError(messageOf(e));
      } finally {
        setPending((p) => p.filter((x) => x.key !== key));
      }
    },
    [refresh, here.id]
  );

  const move = useCallback(
    async (item: DriveItem, destinationId: string) => {
      setMoving(null);
      setError(null);
      // Out of this folder at once; back, with the reason, if Apple refuses.
      setGone((g) => new Set(g).add(item.drivewsid));
      try {
        await api.driveMove(item.drivewsid, destinationId);
        await refresh();
      } catch (e) {
        setGone((g) => {
          const next = new Set(g);
          next.delete(item.drivewsid);
          return next;
        });
        setError(messageOf(e));
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

  // At most one re-read of the open folder every couple of seconds while files land.
  const landedTimer = useRef<number | null>(null);
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const landed = () => {
    if (landedTimer.current !== null) return;
    landedTimer.current = window.setTimeout(() => {
      landedTimer.current = null;
      void refreshRef.current();
    }, 2000);
  };
  useEffect(() => () => {
    if (landedTimer.current !== null) window.clearTimeout(landedTimer.current);
  }, []);

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
      // Each file that lands shows as it lands, not all at once when the batch ends.
      if (u.state === "done") landed();
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
        if (into === hereRef.current) await refresh();
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
          // A row of Drive's own being moved is not a file arriving from this machine.
          if (dragItem.current) return;
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

  if (inTrash) return <DriveTrash onClose={() => setInTrash(false)} />;
  if (inRecents) return <DriveRecents onClose={() => setInRecents(false)} />;

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
                className={`crumb${dropOn === crumb.id ? " is-drop" : ""}`}
                aria-current={i === trail.length - 1}
                onClick={() => setTrail((t) => t.slice(0, i + 1))}
                // A folder further up the path takes a dragged item too, as Finder's path bar does.
                onDragOver={(e) => {
                  if (!dragItem.current || i === trail.length - 1) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  if (dropOn !== crumb.id) setDropOn(crumb.id);
                }}
                onDragLeave={() => setDropOn((d) => (d === crumb.id ? null : d))}
                onDrop={(e) => {
                  e.preventDefault();
                  const moving = dragItem.current;
                  dragItem.current = null;
                  setDropOn(null);
                  if (moving && i < trail.length - 1) void move(moving, crumb.id);
                }}
              >
                {crumb.name}
              </button>
            </span>
          ))}
        </nav>
        {loaded && (
          // As icloud.com writes it under a folder's name: how much is in it.
          <p className="drive-count" aria-live="polite">
            {(() => {
              const n = items.filter((x) => !gone.has(x.drivewsid)).length;
              return `${n} item${n === 1 ? "" : "s"}${available ? `, ${available} available` : ""}`;
            })()}
          </p>
        )}
        <div className="cal-actions">
          <button className="quiet with-glyph" title="Recents" onClick={() => setInRecents(true)}>
            <Clock size={15} strokeWidth={1.6} aria-hidden="true" />
            Recents
          </button>
          <button className="quiet with-glyph" title="Recently Deleted" onClick={() => setInTrash(true)}>
            <ActionGlyph name="delete" />
            Recently Deleted
          </button>
          <Picker
            className="drive-view"
            label="View as"
            value={grid ? "grid" : "list"}
            onChange={(v) => chooseView(v === "grid")}
            options={[{ value: "grid", label: "Grid" }, { value: "list", label: "List" }]}
          />
          <Picker
            className="drive-sort"
            label="Sort by"
            value={sortBy}
            onChange={(v) => chooseSort(v as SortKey)}
            options={SORTS.map((o) => ({ value: o.key, label: `by ${o.label}` }))}
          />
          <button
            className="quiet with-glyph"
            title="New Folder"
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

      {!grid && (
        // Finder's and icloud.com's column headings: what each column is, and a click to
        // sort by it — the menu above and these are the same choice.
        <div className="drive-row drive-head" role="presentation">
          <span className="drive-row-hit">
            <span />
            {(["name", "kind", "size", "date"] as const).map((key) => (
              <button
                key={key}
                className={`drive-col${sortBy === key ? " is-on" : ""}${key === "name" ? " is-name" : ""}`}
                aria-pressed={sortBy === key}
                onClick={() => chooseSort(key)}
              >
                {SORTS.find((o) => o.key === key)?.label}
                {sortBy === key && <span aria-hidden="true"> ⌄</span>}
              </button>
            ))}
          </span>
          <span className="drive-row-actions" aria-hidden="true" />
        </div>
      )}
      <ul className={`drive-rows${grid ? " is-grid" : ""}`}>
        {pending
          .filter((p) => p.folder === here.id && !items.some((i) => driveName(i) === p.name))
          .map((p) => (
            <li key={p.key} className="drive-row is-busy" aria-busy="true">
              <span className="drive-row-hit">
                <span className="drive-badge" aria-hidden="true">
                  <span className="drive-spinner" />
                </span>
                <span className="drive-name">{p.name}</span>
                <span className="drive-meta drive-kind" />
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

        {sortItems(items.filter((item) => !gone.has(item.drivewsid)), sortBy).map((item) => {
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
                <div
                  className={`drive-row${container ? " is-folder" : ""}${busy ? " is-busy" : ""}${menu?.item.drivewsid === item.drivewsid ? " is-menu" : ""}${selected === item.drivewsid ? " is-selected" : ""}${dropOn === item.drivewsid ? " is-drop" : ""}`}
                  // Dragged onto a folder, an item moves into it — as in Finder and icloud.com.
                  draggable={!busy && item.kind !== "APP_LIBRARY"}
                  onDragStart={(e) => {
                    dragItem.current = item;
                    e.dataTransfer.effectAllowed = "move";
                    e.dataTransfer.setData("application/x-quince-drive", item.drivewsid);
                  }}
                  onDragEnd={() => { dragItem.current = null; setDropOn(null); }}
                  onDragOver={(e) => {
                    const moving = dragItem.current;
                    if (!moving || item.kind !== "FOLDER" || moving.drivewsid === item.drivewsid) return;
                    e.preventDefault();
                    e.dataTransfer.dropEffect = "move";
                    if (dropOn !== item.drivewsid) setDropOn(item.drivewsid);
                  }}
                  onDragLeave={() => setDropOn((d) => (d === item.drivewsid ? null : d))}
                  onDrop={(e) => {
                    e.preventDefault();
                    const moving = dragItem.current;
                    dragItem.current = null;
                    setDropOn(null);
                    if (moving && item.kind === "FOLDER" && moving.drivewsid !== item.drivewsid) void move(moving, item.drivewsid);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    if (!busy) setMenu({ item, x: e.clientX, y: e.clientY, alignRight: false });
                  }}
                >
                  <button
                    className="drive-row-hit"
                    // As Finder and icloud.com: a click selects, a double-click opens — a folder
                    // goes in, a file is saved to Downloads and shown there. Return opens the
                    // selected item too; a single click that opened made every misclick a
                    // download.
                    aria-pressed={selected === item.drivewsid}
                    onClick={() => setSelected(item.drivewsid)}
                    onDoubleClick={() => (container ? enter(item) : void download(item))}
                    onKeyDown={(e) => {
                      const mod = e.ctrlKey || e.metaKey;
                      if (e.key === "Enter" || (mod && e.key === "ArrowDown")) {
                        e.preventDefault();
                        if (container) enter(item);
                        else void download(item);
                      } else if (mod && e.key === "ArrowUp") {
                        // Finder's Command-Up: the folder this one is in.
                        e.preventDefault();
                        if (trail.length > 1) setTrail((t) => t.slice(0, -1));
                      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                        // The arrows walk the selection, as in Finder and icloud.com.
                        e.preventDefault();
                        const rows = [...(e.currentTarget.closest("ul")?.querySelectorAll<HTMLButtonElement>(".drive-row-hit") ?? [])];
                        const next = rows[rows.indexOf(e.currentTarget) + (e.key === "ArrowDown" ? 1 : -1)];
                        next?.focus();
                        next?.click();
                      } else if (e.key === "Delete" || (e.key === "Backspace" && (e.metaKey || e.ctrlKey))) {
                        e.preventDefault();
                        void remove(item);
                      }
                    }}
                    disabled={busy}
                  >
                    <span className="drive-badge" aria-hidden="true">
                      {busy ? (
                        <span className="drive-spinner" />
                      ) : (
                        thumbs[item.drivewsid] ? <img className="drive-thumb" src={thumbs[item.drivewsid]} alt="" draggable={false} /> : <FileGlyph kind={item.kind} extension={item.extension} />
                      )}
                    </span>
                    <span className="drive-name">{driveName(item)}</span>
                    <span className="drive-meta drive-kind">{kindName(item)}</span>
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
                      className="drive-action"
                      aria-label={`Actions for ${driveName(item)}`}
                      aria-haspopup="menu"
                      title="More"
                      disabled={busy}
                      onClick={(e) => {
                        const r = e.currentTarget.getBoundingClientRect();
                        setMenu({ item, x: r.right, y: r.bottom + 4, alignRight: true });
                      }}
                    >
                      <span aria-hidden="true" className="drive-more">•••</span>
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

      {menu && (
        <RowMenu
          at={menu}
          onClose={() => setMenu(null)}
          entries={[
            { label: isContainer(menu.item) ? "Open" : "Download a Copy", run: () => (isContainer(menu.item) ? enter(menu.item) : void download(menu.item)) },
            { label: "Get Info", run: () => setInfo(menu.item) },
            null,
            ...(isContainer(menu.item) ? [] : [{ label: "Duplicate", run: () => void duplicate(menu.item) }]),
            { label: "Rename", run: () => setRenaming(menu.item.drivewsid) },
            { label: "Move to Folder…", run: () => setMoving(menu.item) },
            null,
            { label: "Delete", danger: true, run: () => void remove(menu.item) },
          ]}
        />
      )}

      {info && (
        <InfoSheet item={info} where={trail.map((c) => c.name).join(" › ")} thumb={thumbs[info.drivewsid]} onClose={() => setInfo(null)} />
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
    <Portal>
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
    </Portal>
  );
}
