import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { api, messageOf, type AccountInfo } from "../lib/ipc";
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
};

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
export function SettingsView({ onSignOut }: { onSignOut: () => void }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [storage, setStorage] = useState<{ usedBytes: number; totalBytes: number } | null>(null);
  const [alerts, setAlerts] = useState<{ reminderAlerts: boolean; eventAlerts: boolean } | null>(null);
  const [theme, chooseTheme] = useState<Theme>(storedTheme);
  const [version, setVersion] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const { report, syncing, refresh } = useSync();

  useEffect(() => {
    api.accountInfo().then(setAccount, () => setAccount({ name: null, email: null }));
    api.storageInfo().then(setStorage, () => {});
    api.settingsGet().then(setAlerts, (e) => setError(messageOf(e)));
    getVersion().then(setVersion, () => {});
  }, []);

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
        {confirming ? (
          <span className="settings-confirm">
            <button className="quiet" onClick={() => setConfirming(false)}>Cancel</button>
            <button className="quiet danger" onClick={onSignOut}>Sign Out</button>
          </span>
        ) : (
          <button className="quiet" onClick={() => setConfirming(true)}>Sign Out</button>
        )}
      </section>
      {confirming && (
        <p className="settings-note">
          Signing out removes your iCloud data and sign-in from this computer. Nothing is deleted from iCloud.
        </p>
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
      </section>
    </div>
  );
}
