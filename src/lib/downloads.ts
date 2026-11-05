/**
 * Downloads into Downloads that are under way, just finished, or left unfinished — for the
 * whole window: kept here rather than in Drive's screen, so a download goes on being shown
 * after the screen is left and come back to, and its thousands of events never redraw the
 * file list.
 */
import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, DOWNLOAD, messageOf, type DownloadEvent, type DriveItem } from "./ipc";
import { apply, finish, newLine, pausedLine, type DownloadLine } from "./downloadModel";

const lines = new Map<string, DownloadLine>();
const listeners = new Set<() => void>();
let version = 0;
let flushing: number | null = null;

/**
 * Told to whoever draws it at most every 80 ms: a folder of small files sends hundreds of
 * events a second, and drawing the tray for each one was all the window did.
 */
function changed() {
  if (flushing !== null) return;
  flushing = window.setTimeout(() => {
    flushing = null;
    version += 1;
    listeners.forEach((l) => l());
  }, 80);
}

let listening: Promise<void> | null = null;
/** Heard for as long as the window is open, from before the first download begins. */
function hear(): Promise<void> {
  listening ??= listen<DownloadEvent>(DOWNLOAD, (event) => {
    const line = lines.get(event.payload.batch);
    if (!line) return;
    apply(line, event.payload);
    changed();
  }).then(
    () => undefined,
    // No event system behind the page (a plain browser): nothing will be heard.
    () => undefined,
  );
  return listening;
}

/**
 * Save `items` into Downloads as they are in Drive — or, when the same was downloaded before
 * and not finished, go on with it where it was. Never blocks the window: it is shown in the
 * tray as it goes, and what it came to stays there until the tray is closed.
 */
export async function startDownload(items: DriveItem[], title: string): Promise<void> {
  const batch = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  const line = newLine(batch, title, items);
  lines.set(batch, line);
  changed();
  await hear();
  try {
    finish(line, await api.driveDownloadItems(batch, title, items), null);
  } catch (e) {
    finish(line, null, messageOf(e));
  }
  changed();
}

/** Go on with a download that stopped short — or start it again — in place of its line. */
export function downloadAgain(batch: string): Promise<void> {
  const line = lines.get(batch);
  if (!line) return Promise.resolve();
  lines.delete(batch);
  return startDownload(line.items, line.title);
}

/** Stop a download: what has arrived stays, the rest does not begin. */
export function cancelDownload(batch: string): Promise<void> {
  return api.driveDownloadCancel([], batch).catch(() => {});
}

/** No longer offer to go on with this download; what it saved stays where it is. */
export function forgetDownload(batch: string, key: string): Promise<void> {
  lines.delete(batch);
  changed();
  return api.driveDownloadForget(key).catch(() => {});
}

/** Close the finished lines; any still running stay. */
export function dismissFinished() {
  for (const [batch, line] of lines) if (line.finished) lines.delete(batch);
  changed();
}

let looked = false;
/**
 * Downloads left unfinished when Skykeep was last closed, offered once a session so they can
 * be gone on with.
 */
export async function offerUnfinished(): Promise<void> {
  if (looked) return;
  looked = true;
  try {
    for (const u of await api.driveDownloadsUnfinished()) {
      const batch = `unfinished:${u.key}`;
      if (!lines.has(batch)) lines.set(batch, pausedLine(u.key, u.title, u.items, u.saved));
    }
    changed();
  } catch {
    // Nothing to offer.
  }
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};
const current = () => version;

/** Every download in the tray, oldest first; drawn again as they move. */
export function useDownloads(): DownloadLine[] {
  useSyncExternalStore(subscribe, current, current);
  return [...lines.values()];
}
