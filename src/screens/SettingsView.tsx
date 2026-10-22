import { ConfirmSheet } from "../components/Confirm";
import { toast, toastSaved, useErrorToast } from "../components/Toast";
import { Progress } from "../components/Spinner";
import { useEffect, useState, useSyncExternalStore } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { pickOpen } from "../lib/pick";
import {
  api,
  EXPORT_CANCELLED,
  EXPORT_PROGRESS,
  exportApi,
  type AccountInfo,
  type DriveFolder,
  type ExportProgress,
  type ExportService,
  type LocalStorage,
  type LoginItem,
  messageOf,
  settingsApi,
} from "../lib/ipc";
import { freshness, useSync } from "../lib/sync";
import { setTheme, storedTheme, type Theme } from "../lib/theme";
import { AppIcon } from "../components/AppIcon";
import { AccountAvatar } from "../components/Account";

/** The app each service's freshness belongs to, for its icon. */
const ICON: Record<string, string> = {
  calendar: "calendar",
  reminders: "reminders",
  notes: "notes",
  contacts: "contacts",
  drive: "drivews",
};

/** The Drive folder's line when it is up: up to date, uploading, or why something is waiting. */
function folderState(f: DriveFolder): string {
  const w = f.waiting;
  const files = (n: number) => `${n} ${n === 1 ? "file" : "files"}`;
  if (!w || w.files === 0) return `Up to date · ${f.path}`;
  if (w.failure) return `${files(w.files)} waiting — ${w.failure}`;
  if (w.sending > 0) return `Uploading ${files(w.files)}…`;
  return `${files(w.files)} waiting to upload`;
}

function size(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  if (gb >= 1) return `${gb >= 10 ? gb.toFixed(0) : gb.toFixed(1)} GB`;
  if (bytes < 1024 ** 2) return bytes < 1024 ? "Zero KB" : `${Math.round(bytes / 1024)} KB`;
  return `${Math.round(bytes / 1024 ** 2)} MB`;
}

/** A row's mark where no app's icon fits: a plain tile, the same size as the apps' icons. */
function Glyph({ kind }: { kind: "library" | "other" }) {
  return (
    <span className="settings-glyph" aria-hidden="true">
      <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.4">
        {kind === "library" ? (
          <>
            <ellipse cx="8" cy="4" rx="5" ry="2" />
            <path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
          </>
        ) : (
          <g fill="currentColor" stroke="none">
            <circle cx="3.5" cy="8" r="1.4" />
            <circle cx="8" cy="8" r="1.4" />
            <circle cx="12.5" cy="8" r="1.4" />
          </g>
        )}
      </svg>
    </span>
  );
}

/**
 * Settings, as macOS System Settings lays it out: the account at the head, then grouped
 * sections of rows, a label on the left and its control on the right. Only what really
 * changes something is here — every switch is one the backend obeys, every figure is read
 * from Apple or from this computer's library.
 */
type Unsent = { changes: number; files: number; photos: number };

/** What has not reached iCloud yet, as a sentence's subject: "2 changes and 1 file". */
function unsentPhrase(u: Unsent): string | null {
  const n = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
  const parts = [
    u.changes > 0 && n(u.changes, "change", "changes"),
    u.files > 0 && n(u.files, "file", "files"),
    u.photos > 0 && n(u.photos, "photo", "photos"),
  ].filter((p): p is string => Boolean(p));
  if (parts.length === 0) return null;
  return parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/**
 * Signing out asks what macOS asks: keep a copy here, or remove it. Kept, everything is here
 * again on signing back in — and another Apple ID signing in meanwhile finds it set aside, never
 * mixed with its own. What has not reached iCloud yet is said plainly, since removing loses it.
 */
function SignOutSheet({
  unsent,
  onCancel,
  onChoose,
}: {
  unsent: Unsent;
  onCancel: () => void;
  onChoose: (keep: boolean) => void;
}) {
  const waiting = unsentPhrase(unsent);
  const total = unsent.changes + unsent.files + unsent.photos;
  return (
    <ConfirmSheet<boolean>
      title="Keep a copy of your iCloud data on this computer?"
      body="A copy keeps your notes, photos, files and everything else here, ready when you sign back in. Nothing is deleted from iCloud either way."
      choices={[
        { label: "Keep a Copy", value: true, primary: true, focus: true },
        { label: "Remove from This Computer", value: false, danger: true },
      ]}
      onAnswer={onChoose}
      onCancel={onCancel}
    >
      {waiting && (
        <p className="signout-unsent">
          {waiting} {total === 1 ? "hasn't" : "haven't"} reached iCloud yet. Keep a copy and{" "}
          {total === 1 ? "it goes" : "they go"} up when you sign back in; remove it and{" "}
          {total === 1 ? "it's" : "they're"} lost.
        </p>
      )}
    </ConfirmSheet>
  );
}


/** A folder of iCloud on this computer, as a row: where it is or why it is not, Open, and its switch. */
function FolderRow({
  app,
  label,
  off,
  folder,
  busy,
  line,
  onOpen,
  onSet,
}: {
  app: string;
  label: string;
  off: string;
  folder: DriveFolder | null;
  busy: boolean;
  line: (f: DriveFolder) => string;
  onOpen: () => Promise<unknown>;
  onSet: (on: boolean) => void;
}) {
  // Why the folder could not be opened, said on its own row until it is tried again.
  const [openFailed, setOpenFailed] = useState<string | null>(null);
  const problem = openFailed ?? folder?.problem;
  const trouble = Boolean(problem || folder?.waiting?.failure);
  return (
    <div className="settings-row">
      <AppIcon app={app} />
      <span className="settings-label settings-stack">
        {label}
        <span className={`settings-hint${trouble ? " is-trouble" : ""}`} title={folder?.on ? folder.path : undefined}>
          {problem ?? (folder?.on ? line(folder) : off)}
        </span>
      </span>
      {folder?.on && (
        <button
          className="quiet"
          onClick={() => {
            setOpenFailed(null);
            onOpen().catch((e) => setOpenFailed(messageOf(e)));
          }}
        >
          Open
        </button>
      )}
      <button
        className="switch"
        role="switch"
        aria-checked={folder?.on ?? false}
        aria-label={label}
        disabled={!folder || busy}
        onClick={() => {
          setOpenFailed(null);
          if (folder) onSet(!folder.on);
        }}
      >
        <span className="switch-knob" />
      </button>
    </div>
  );
}

/**
 * Export Everything, kept outside the screen: an export runs on while Settings is closed, and
 * coming back finds it where it is, not idle.
 */
type ExportState =
  | { phase: "idle" }
  | { phase: "running"; progress: ExportProgress | null; cancelling: boolean }
  | { phase: "done"; path: string; photosMissing: number };
let exportState: ExportState = { phase: "idle" };
const exportListeners = new Set<() => void>();
const setExportState = (s: ExportState) => {
  exportState = s;
  exportListeners.forEach((l) => l());
};
const subscribeExport = (l: () => void) => {
  exportListeners.add(l);
  return () => exportListeners.delete(l);
};

const SERVICE_LABEL: Record<ExportService, string> = {
  notes: "Notes",
  reminders: "Reminders",
  contacts: "Contacts",
  calendar: "Calendar",
  drive: "Drive",
  photos: "Photos",
};

/** How far the whole export is, 0 to 1: services done, and the share of the one under way. */
function exportShare(p: ExportProgress | null): number | null {
  if (!p || p.steps === 0) return null;
  const within = p.total > 0 ? p.done / p.total : p.done === p.total ? 1 : 0;
  return (p.step - 1 + within) / p.steps;
}

async function runExport(photos: boolean, onError: (m: string) => void) {
  const dest = (await pickOpen({ directory: true, multiple: false, title: "Choose where to export" }))?.[0];
  if (!dest) return;
  setExportState({ phase: "running", progress: null, cancelling: false });
  const stop = listen<ExportProgress>(EXPORT_PROGRESS, (e) => {
    if (exportState.phase === "running") setExportState({ ...exportState, progress: e.payload });
  });
  try {
    const summary = await exportApi.start(dest, photos);
    setExportState({ phase: "done", path: summary.path, photosMissing: summary.photosMissing });
    toast({
      message: `Exported to ${folderName(summary.path)}`,
      tone: "done",
      action: { label: "Show in Folder", run: () => void exportApi.reveal().catch((e) => onError(messageOf(e))) },
    });
  } catch (e) {
    const message = messageOf(e);
    setExportState({ phase: "idle" });
    if (message === EXPORT_CANCELLED) toast("Export cancelled. Nothing was left behind.");
    else onError(message);
  } finally {
    void stop.then((unlisten) => unlisten()).catch(() => {});
  }
}

const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/** The row: what goes out and where, Export, then the bar and Cancel, then Show in Folder. */
function ExportRow({ onError }: { onError: (m: string) => void }) {
  const state = useSyncExternalStore(subscribeExport, () => exportState);
  const [photos, setPhotos] = useState(false);
  const running = state.phase === "running";
  const p = running ? state.progress : null;
  const hint = running
    ? state.cancelling
      ? "Cancelling…"
      : p
        ? `${SERVICE_LABEL[p.service]} · ${p.total > 0 ? `${Math.min(p.done, p.total).toLocaleString()} of ${p.total.toLocaleString()}` : "nothing to export"}`
        : "Getting ready…"
    : state.phase === "done"
      ? `Exported to ${folderName(state.path)}${state.photosMissing > 0 ? ` · ${state.photosMissing.toLocaleString()} ${state.photosMissing === 1 ? "photo wasn't" : "photos weren't"} on this computer` : ""}`
      : "Notes, reminders, contacts, calendars and Drive, in open formats";
  return (
    <div className="settings-row is-stacked" aria-label="Export Everything" role="group">
      <div className="settings-row-line" style={{ alignItems: "center" }}>
        <span className="settings-label settings-stack">
          Export Everything
          <span className="settings-hint" aria-live="polite" title={state.phase === "done" ? state.path : undefined}>
            {hint}
          </span>
        </span>
        {state.phase === "done" && !running && (
          <button className="quiet" onClick={() => void exportApi.reveal().catch((e) => onError(messageOf(e)))}>
            Show in Folder
          </button>
        )}
        {running ? (
          <button
            className="quiet"
            disabled={state.cancelling}
            onClick={() => {
              setExportState({ ...state, cancelling: true });
              void exportApi.cancel().catch(() => {});
            }}
          >
            Cancel
          </button>
        ) : (
          <button className="quiet" onClick={() => void runExport(photos, onError)}>
            Export
          </button>
        )}
      </div>
      {running ? (
        <Progress value={exportShare(p)} label="Export progress" />
      ) : (
        <div className="settings-row-line" style={{ alignItems: "center" }}>
          <span className="settings-label settings-stack">
            <span className="settings-hint">Include Photos · only those on this computer, and it can be large</span>
          </span>
          <button
            className="switch"
            role="switch"
            aria-checked={photos}
            aria-label="Include Photos"
            onClick={() => setPhotos(!photos)}
          >
            <span className="switch-knob" />
          </button>
        </div>
      )}
    </div>
  );
}

export function SettingsView({ onSignOut }: { onSignOut: (keep: boolean) => Promise<void> }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [storage, setStorage] = useState<{ usedBytes: number; totalBytes: number } | null>(null);
  const [local, setLocal] = useState<LocalStorage | null>(null);
  const [clearing, setClearing] = useState(false);
  const [freed, setFreed] = useState<number | null>(null);
  const [alerts, setAlerts] = useState<{ reminderAlerts: boolean; eventAlerts: boolean } | null>(null);
  const [alertsShown, setAlertsShown] = useState<boolean | null>(null);
  const [theme, chooseTheme] = useState<Theme>(storedTheme);
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useErrorToast(error, () => setError(null));
  const [signingOut, setSigningOut] = useState<Unsent | null>(null);
  const [folder, setFolder] = useState<DriveFolder | null>(null);
  const [folderBusy, setFolderBusy] = useState(false);
  const [photosFolder, setPhotosFolder] = useState<DriveFolder | null>(null);
  const [photosBusy, setPhotosBusy] = useState(false);
  const [login, setLogin] = useState<LoginItem | null>(null);
  const [loginBusy, setLoginBusy] = useState(false);
  const { report, syncing, refresh } = useSync();

  useEffect(() => {
    api.accountInfo().then(setAccount, () => setAccount({ name: null, email: null }));
    api.storageInfo().then(setStorage, () => {});
    api.settingsGet().then(setAlerts, (e) => setError(messageOf(e)));
    settingsApi.alertsShown().then(setAlertsShown, () => {});
    settingsApi.localStorage().then(setLocal, () => {});
    getVersion().then(setVersion, () => {});
    api.driveFolderStatus().then(setFolder, () => {});
    api.photosFolderStatus().then(setPhotosFolder, () => {});
    api.loginItem().then(setLogin, () => {});
  }, []);
  // While the Drive folder is up, what it has still to send is read every two seconds (a
  // local question; nothing asked of Apple), so nothing ever waits unseen.
  const folderOn = folder?.on ?? false;
  useEffect(() => {
    if (!folderOn) return undefined;
    const t = window.setInterval(() => {
      api.driveFolderStatus().then(setFolder, () => {});
    }, 2000);
    return () => window.clearInterval(t);
  }, [folderOn]);

  // A switch moves the moment it is pressed, and goes back with the reason if it could not
  // be kept.
  const toggle = (key: "reminderAlerts" | "eventAlerts") => {
    if (!alerts) return;
    const on = !alerts[key];
    setAlerts({ ...alerts, [key]: on });
    api.settingsSet(key, on).catch((e) => {
      setAlerts((now) => (now ? { ...now, [key]: !on } : now));
      setError(messageOf(e));
    });
  };

  // A folder's switch moves at once; Apple's answer only matters when it could not come up,
  // and then the reason is said on the folder's own row, where the eye already is.
  const setFolderOn = (
    current: DriveFolder,
    on: boolean,
    put: (f: DriveFolder) => void,
    busy: (b: boolean) => void,
    ask: (on: boolean) => Promise<DriveFolder>,
  ) => {
    put({ ...current, on, problem: null });
    busy(true);
    ask(on)
      .then(put, (e) => put({ ...current, problem: messageOf(e) }))
      .finally(() => busy(false));
  };

  const clearPictures = () => {
    if (!local) return;
    const before = local.total;
    setClearing(true);
    setFreed(null);
    settingsApi
      .localStorageClear()
      .then(
        (after) => {
          setLocal(after);
          setFreed(Math.max(0, before - after.total));
        },
        (e) => setError(messageOf(e)),
      )
      .finally(() => setClearing(false));
  };

  const used = storage && storage.totalBytes > 0 ? Math.min(1, storage.usedBytes / storage.totalBytes) : 0;
  const services = (report?.services ?? []).filter((s) => s.outcome !== "unavailable");
  const showDrive = folder?.available !== false;
  const showPhotos = photosFolder?.available !== false;
  const MB = 1024 ** 2;

  return (
    <div className="settings">
      <h1 className="settings-title">Settings</h1>
      {/* Held at the top of the window as it scrolls: a reason is never left out of sight. */}

      <section className="settings-group settings-account" aria-label="Account">
        <AccountAvatar account={account} large />
        <div className="settings-account-text">
          <strong>{account?.name ?? "Your Apple Account"}</strong>
          <span>{account?.email ?? ""}</span>
        </div>
        <button
          className="quiet"
          onClick={() =>
            api.unsent().then(setSigningOut, () => setSigningOut({ changes: 0, files: 0, photos: 0 }))
          }
        >
          Sign Out
        </button>
      </section>
      {signingOut && (
        <SignOutSheet
          unsent={signingOut}
          onCancel={() => setSigningOut(null)}
          onChoose={(keep) => {
            setSigningOut(null);
            onSignOut(keep).catch((e) => setError(messageOf(e)));
          }}
        />
      )}

      <h2 className="settings-heading">iCloud</h2>
      <section className="settings-group">
        <div className="settings-row is-stacked">
          <div className="settings-row-line">
            <span>{storage ? `${size(storage.usedBytes)} of ${size(storage.totalBytes)} used` : "Reading storage…"}</span>
            {storage && (
              <span className="settings-value">{size(Math.max(0, storage.totalBytes - storage.usedBytes))} available</span>
            )}
          </div>
          <div
            className="storage-bar"
            role="meter"
            aria-label="iCloud storage used"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(used * 100)}
          >
            <span style={{ width: `${used * 100}%` }} />
          </div>
        </div>
      </section>

      <h2 className="settings-heading">Sync</h2>
      <section className="settings-group">
        {services.map((s) => (
          <div className="settings-row" key={s.service}>
            <AppIcon app={ICON[s.service] ?? s.service} />
            <span className="settings-label">{s.label}</span>
            <span className={`settings-value${s.outcome === "failed" ? " is-trouble" : ""}`}>
              {s.outcome === "failed"
                ? (s.detail ?? "Didn't update")
                : syncing && !s.lastSyncedAt
                  ? "Downloading…"
                  : freshness(s.lastSyncedAt ?? null)}
            </span>
          </div>
        ))}
        <div className="settings-row">
          <span className="settings-label settings-hint">Changes from your other devices arrive within seconds.</span>
          <button className="quiet" onClick={() => void refresh()} disabled={syncing}>
            {syncing ? "Syncing…" : "Sync Now"}
          </button>
        </div>
      </section>

      {(showDrive || showPhotos) && (
        <>
          <h2 className="settings-heading">Folders</h2>
          <section className="settings-group">
            {showDrive && (
              <FolderRow
                app="drivews"
                label="Drive folder"
                off="Your iCloud Drive in Files and every other app"
                folder={folder}
                busy={folderBusy}
                line={folderState}
                onOpen={api.driveFolderOpen}
                onSet={(on) => folder && setFolderOn(folder, on, setFolder, setFolderBusy, api.driveFolderSet)}
              />
            )}
            {showPhotos && (
              <FolderRow
                app="photos"
                label="Photos folder"
                off="Your photos by date and album, for every other app"
                folder={photosFolder}
                busy={photosBusy}
                line={(f) => f.path}
                onOpen={api.photosFolderOpen}
                onSet={(on) =>
                  photosFolder && setFolderOn(photosFolder, on, setPhotosFolder, setPhotosBusy, api.photosFolderSet)
                }
              />
            )}
          </section>
        </>
      )}

      <h2 className="settings-heading settings-heading-line">
        <span>Storage on This Computer</span>
        {local && <span className="settings-heading-value">{size(local.total)}</span>}
      </h2>
      <section className="settings-group" aria-busy={!local}>
        {(
          [
            ["photos", "Photos", local?.photos],
            ["notes", "Notes", local?.notes],
            ["drivews", "Drive", local?.drive],
            ["library", "Library", local?.library],
            ["other", "Other", local?.other],
          ] as const
        )
          .filter(([key, , bytes]) => key !== "other" || (bytes ?? 0) >= MB)
          .map(([key, label, bytes]) => (
            <div className="settings-row" key={key}>
              {key === "library" || key === "other" ? <Glyph kind={key} /> : <AppIcon app={key} />}
              <span className="settings-label">{label}</span>
              <span className="settings-value">{bytes == null ? "…" : size(bytes)}</span>
            </div>
          ))}
        <div className="settings-row">
          <span className="settings-label settings-stack">
            Cached pictures
            <span className="settings-hint" aria-live="polite">
              {freed != null ? `${size(freed)} cleared. They download again when needed.` : "They download again when needed."}
            </span>
          </span>
          {local && local.clearable >= MB && <span className="settings-value">{size(local.clearable)}</span>}
          <button className="quiet" onClick={clearPictures} disabled={!local || clearing || local.clearable < MB}>
            {clearing ? "Clearing…" : "Clear"}
          </button>
        </div>
        <ExportRow onError={setError} />
      </section>

      <h2 className="settings-heading">General</h2>
      <section className="settings-group">
        <div className="settings-row">
          <span className="settings-label">Appearance</span>
          <div className="segmented" role="group" aria-label="Appearance">
            {(["auto", "light", "dark"] as const).map((t) => (
              <button
                key={t}
                className="segment"
                aria-pressed={theme === t}
                onClick={() => {
                  chooseTheme(t);
                  setTheme(t);
                }}
              >
                {t === "auto" ? "Automatic" : t === "light" ? "Light" : "Dark"}
              </button>
            ))}
          </div>
        </div>
        <div className="settings-row">
          <span className="settings-label settings-stack">
            Open at Login
            <span className="settings-hint">
              {login?.on && !login.honoured
                ? "This desktop doesn't open apps at login by itself. Add this to its startup:"
                : "Starts in the background, so alerts ring and changes sync"}
            </span>
            {login?.on && !login.honoured && <code className="settings-command">{login.command}</code>}
          </span>
          <button
            className="switch"
            role="switch"
            aria-checked={login?.on ?? false}
            aria-label="Open at Login"
            disabled={!login || loginBusy}
            onClick={() => {
              if (!login) return;
              const was = login;
              setLogin({ ...login, on: !login.on });
              setLoginBusy(true);
              api
                .loginItemSet(!was.on)
                .then(setLogin, (e) => {
                  setLogin(was);
                  setError(messageOf(e));
                })
                .finally(() => setLoginBusy(false));
            }}
          >
            <span className="switch-knob" />
          </button>
        </div>
      </section>

      <h2 className="settings-heading">Notifications</h2>
      <section className="settings-group">
        {(
          [
            ["reminderAlerts", "Reminder alerts", "reminders"],
            ["eventAlerts", "Calendar alerts", "calendar"],
          ] as const
        ).map(([key, label, icon]) => (
          <div className="settings-row" key={key}>
            <AppIcon app={icon} />
            <span className="settings-label">{label}</span>
            <button
              className="switch"
              role="switch"
              aria-checked={alerts?.[key] ?? false}
              aria-label={label}
              disabled={!alerts}
              onClick={() => toggle(key)}
            >
              <span className="switch-knob" />
            </button>
          </div>
        ))}
        {/* Asked of the desktop itself: without a notification service every alert is dropped. */}
        {alertsShown === false && (
          <div className="settings-row">
            <span className="settings-label settings-hint is-trouble">
              Alerts can't appear: this desktop has no notification service running.
            </span>
          </div>
        )}
      </section>

      <h2 className="settings-heading">About</h2>
      <section className="settings-group">
        <div className="settings-row">
          <span className="settings-label">Version</span>
          <span className="settings-value">{version ?? "—"}</span>
        </div>
        <div className="settings-row">
          <span className="settings-label settings-stack">
            Diagnostic Report
            <span className="settings-hint settings-hint-wrap">Versions and sync states only. Nothing is sent.</span>
          </span>
          <button
            className="quiet"
            aria-label="Save Diagnostic Report"
            onClick={() =>
              api.diagnosticsSave().then(
                (path) => void toastSaved(path),
                (e) => setError(messageOf(e)),
              )
            }
          >
            Save
          </button>
        </div>
      </section>
      <p className="settings-note">Skykeep talks only to Apple, and to calendars you subscribe to. It has no servers of its own.</p>
    </div>
  );
}
