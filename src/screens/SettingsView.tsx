import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { api, type DriveFolder, type LoginItem, messageOf, type AccountInfo } from "../lib/ipc";
import { freshness, useSync } from "../lib/sync";
import { setTheme, storedTheme, type Theme } from "../lib/theme";
import { AppIcon } from "../components/AppIcon";
import { AccountAvatar } from "../components/Account";
import { Portal } from "../components/Portal";

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
  return `${Math.max(1, Math.round(bytes / 1024 ** 2))} MB`;
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
    <Portal>
      <div className="modal-veil" onClick={onCancel}>
        <div
          className="modal-sheet px-ask"
          role="alertdialog"
          aria-modal="true"
          aria-label="Keep a copy of your iCloud data on this computer?"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              onCancel();
            }
          }}
        >
          <h2>Keep a copy of your iCloud data on this computer?</h2>
          <p>
            A copy keeps your notes, photos, files and everything else here, ready when you sign back in. Nothing is
            deleted from iCloud either way.
          </p>
          {waiting && (
            <p className="signout-unsent">
              {waiting} {total === 1 ? "hasn't" : "haven't"} reached iCloud yet. Keep a copy and{" "}
              {total === 1 ? "it goes" : "they go"} up when you sign back in; remove it and{" "}
              {total === 1 ? "it's" : "they're"} lost.
            </p>
          )}
          <div className="px-ask-actions is-stacked">
            <button className="is-act" autoFocus onClick={() => onChoose(true)}>
              Keep a Copy
            </button>
            <button className="is-act danger" onClick={() => onChoose(false)}>
              Remove from This Computer
            </button>
            <button onClick={onCancel}>Cancel</button>
          </div>
        </div>
      </div>
    </Portal>
  );
}

export function SettingsView({ onSignOut }: { onSignOut: (keep: boolean) => Promise<void> }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [storage, setStorage] = useState<{ usedBytes: number; totalBytes: number } | null>(null);
  const [alerts, setAlerts] = useState<{ reminderAlerts: boolean; eventAlerts: boolean } | null>(null);
  const [theme, chooseTheme] = useState<Theme>(storedTheme);
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [signingOut, setSigningOut] = useState<Unsent | null>(null);
  const [savedReport, setSavedReport] = useState<string | null>(null);
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

  const used = storage && storage.totalBytes > 0 ? Math.min(1, storage.usedBytes / storage.totalBytes) : 0;

  return (
    <div className="settings">
      <h1 className="settings-title">Settings</h1>
      {error && <p className="settings-error" role="alert">{error}</p>}

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

      {account?.dataProtection != null && (
        <section className="settings-group settings-gap">
          <div className="settings-row">
            <span className="settings-label">Advanced Data Protection</span>
            <span className="settings-value">{account.dataProtection ? "On" : "Off"}</span>
          </div>
        </section>
      )}

      <h2 className="settings-heading">iCloud Storage</h2>
      <section className="settings-group">
        <div className="settings-row is-stacked">
          <div className="settings-row-line">
            <span>{storage ? `${size(storage.usedBytes)} of ${size(storage.totalBytes)} used` : "Reading storage…"}</span>
            {storage && <span className="settings-value">{size(Math.max(0, storage.totalBytes - storage.usedBytes))} available</span>}
          </div>
          <div className="storage-bar" role="meter" aria-label="iCloud storage used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(used * 100)}>
            <span style={{ width: `${used * 100}%` }} />
          </div>
        </div>
      </section>

      <h2 className="settings-heading">Sync</h2>
      <section className="settings-group">
        {(report?.services ?? []).filter((s) => s.outcome !== "unavailable").map((s) => (
          <div className="settings-row" key={s.service}>
            <AppIcon app={ICON[s.service] ?? s.service} />
            <span className="settings-label">{s.label}</span>
            <span className={`settings-value${s.outcome === "failed" ? " is-trouble" : ""}`}>
              {s.outcome === "failed" ? (s.detail ?? "Didn't update") : freshness(s.lastSyncedAt ?? null)}
            </span>
          </div>
        ))}
        <div className="settings-row">
          <span className="settings-label settings-hint">Changes made on your other devices arrive here within seconds.</span>
          <button className="quiet" onClick={() => void refresh()} disabled={syncing}>
            {syncing ? "Syncing…" : "Sync Now"}
          </button>
        </div>
      </section>

      <h2 className="settings-heading">On This Computer</h2>
      <section className="settings-group">
        <div className="settings-row">
          <AppIcon app="drivews" />
          <span className="settings-label settings-stack">
            iCloud Drive folder
            {/* Where it is, and — when it could not come up — why, in a sentence. */}
            <span className={`settings-hint${folder?.problem || folder?.waiting?.failure ? " is-trouble" : ""}`}>
              {folder?.problem ?? (folder?.on ? folderState(folder) : "Your iCloud Drive in Files and every other app")}
            </span>
          </span>
          {folder?.on && (
            <button className="quiet" onClick={() => api.driveFolderOpen().catch((e) => setError(messageOf(e)))}>
              Open
            </button>
          )}
          <button
            className="switch"
            role="switch"
            aria-checked={folder?.on ?? false}
            aria-label="iCloud Drive folder"
            disabled={!folder || folderBusy}
            onClick={() => {
              if (!folder) return;
              const on = !folder.on;
              // Moves at once; Apple's answer only matters when the folder could not come up.
              setFolder({ ...folder, on, problem: null });
              setFolderBusy(true);
              api
                .driveFolderSet(on)
                .then(setFolder, (e) => {
                  setFolder({ ...folder, problem: messageOf(e) });
                })
                .finally(() => setFolderBusy(false));
            }}
          >
            <span className="switch-knob" />
          </button>
        </div>
        <div className="settings-row">
          <AppIcon app="photos" />
          <span className="settings-label settings-stack">
            iCloud Photos folder
            <span className={`settings-hint${photosFolder?.problem ? " is-trouble" : ""}`}>
              {photosFolder?.problem ??
                (photosFolder?.on ? photosFolder.path : "Your photos by date and album, for every other app")}
            </span>
          </span>
          {photosFolder?.on && (
            <button className="quiet" onClick={() => api.photosFolderOpen().catch((e) => setError(messageOf(e)))}>
              Open
            </button>
          )}
          <button
            className="switch"
            role="switch"
            aria-checked={photosFolder?.on ?? false}
            aria-label="iCloud Photos folder"
            disabled={!photosFolder || photosBusy}
            onClick={() => {
              if (!photosFolder) return;
              const on = !photosFolder.on;
              setPhotosFolder({ ...photosFolder, on, problem: null });
              setPhotosBusy(true);
              api
                .photosFolderSet(on)
                .then(setPhotosFolder, (e) => setPhotosFolder({ ...photosFolder, problem: messageOf(e) }))
                .finally(() => setPhotosBusy(false));
            }}
          >
            <span className="switch-knob" />
          </button>
        </div>
        <div className="settings-row">
          <span className="settings-label settings-stack">
            Open at Login
            <span className="settings-hint">
              {login?.on && !login.honoured
                ? "This desktop doesn't open apps at login by itself. Add this to its startup:"
                : "Starts in the background, so reminders ring and changes sync without the window"}
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

      <h2 className="settings-heading">Appearance</h2>
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
      </section>

      <h2 className="settings-heading">Notifications</h2>
      <section className="settings-group">
        {([
          ["reminderAlerts", "Reminder alerts", "reminders"],
          ["eventAlerts", "Calendar alerts", "calendar"],
        ] as const).map(([key, label, icon]) => (
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
      </section>

      <h2 className="settings-heading">About</h2>
      <section className="settings-group">
        <div className="settings-row">
          <span className="settings-label">Version</span>
          <span className="settings-value">{version ?? "—"}</span>
        </div>
        <div className="settings-row">
          <span className="settings-label settings-hint">
            Your iCloud is kept on this computer and talks only to Apple. Quince has no servers.
          </span>
        </div>
        <div className="settings-row">
          <span className="settings-label settings-hint">
            {savedReport
              ? `Saved to ${savedReport.split("/").slice(-2).join("/")} — versions and sync states only, no names or contents. Attach it to a bug report.`
              : "A plain report of how Quince is doing, for a bug report. Nothing is sent anywhere."}
          </span>
          <button
            className="quiet"
            onClick={() => api.diagnosticsSave().then(setSavedReport, (e) => setError(messageOf(e)))}
          >
            Save Diagnostic Report
          </button>
        </div>
      </section>
    </div>
  );
}
