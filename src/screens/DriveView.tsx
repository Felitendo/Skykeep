import { useCallback, useEffect, useState } from "react";
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

  const here = trail[trail.length - 1];

  const open = useCallback(async (folderId: string) => {
    setLoading(true);
    setError(null);
    try {
      // Show what is stored first, then fetch — a folder visited before opens instantly.
      const cached = await api.driveChildren(folderId);
      if (Array.isArray(cached) && cached.length > 0) setItems(cached);
      await api.driveSyncFolder(folderId);
      const fresh = await api.driveChildren(folderId);
      setItems(Array.isArray(fresh) ? fresh : []);
    } catch (e) {
      // Whatever was stored stays on screen; a failed fetch should not empty the folder.
      setError(messageOf(e));
    } finally {
      setLoading(false);
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void open(here.id);
  }, [open, here.id]);

  function enter(item: DriveItem) {
    if (!isContainer(item)) return;
    setTrail((t) => [...t, { id: item.drivewsid, name: driveName(item) }]);
  }

  return (
    <div className="drive">
      <header className="cal-bar">
        <nav className="crumbs" aria-label="Folder path">
          {trail.map((crumb, i) => (
            <span key={crumb.id}>
              {i > 0 && <span className="crumb-sep" aria-hidden="true">›</span>}
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
          <button className="quiet" onClick={() => void open(here.id)} disabled={loading}>
            {loading ? "Refreshing…" : "Refresh"}
          </button>
        </div>
      </header>

      {error && <p className="error" role="alert">{error}</p>}

      <ul className="drive-rows">
        {items.map((item) => {
          const container = isContainer(item);
          return (
            <li key={item.drivewsid}>
              <button
                className={`drive-row${container ? " is-folder" : ""}`}
                onClick={() => enter(item)}
                // A file cannot be opened yet, so it must not look as though it can.
                disabled={!container}
              >
                <span className={`drive-badge${container ? " folder" : ""}`} aria-hidden="true">
                  {badge(item)}
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
            </li>
          );
        })}
      </ul>

      {loaded && items.length === 0 && !error && (
        <p className="cal-empty">This folder is empty.</p>
      )}
    </div>
  );
}
