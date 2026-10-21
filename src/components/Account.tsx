import { useEffect, useState } from "react";
import { Face } from "./Face";
import { api, contactPhotoSrc, type AccountInfo } from "../lib/ipc";
import { freshness, useServiceState, useSync } from "../lib/sync";

/** "Asha Rao" → "AR"; an address → its first letter. */
export function initialsOf(name: string): string {
  const words = name.split(/[\s@._-]+/).filter(Boolean);
  if (words.length === 0) return "";
  if (name.includes("@") || words.length === 1) return words[0][0]!.toUpperCase();
  return (words[0][0]! + words[words.length - 1][0]!).toUpperCase();
}

/**
 * The account's picture: the photo on the person's own card, as Apple shows it; their
 * initials on Apple's grey disc where there is none, or until it has loaded.
 */
export function AccountAvatar({ account, large }: { account: AccountInfo | null; large?: boolean }) {
  const initials = initialsOf(account?.name ?? account?.email ?? "");
  return (
    <span className={`account-avatar${large ? " is-large" : ""}`} aria-hidden="true">
      <Face src={account?.photoContact ? contactPhotoSrc(account.photoContact) : null} initials={initials} />
    </span>
  );
}

/** The app in front, as the account's footer speaks for it: its sync service and its name. */
export type FooterApp = { service: string; name: string };

/**
 * The account at the foot of the sidebar — who is signed in and how fresh their iCloud is —
 * and the way into Settings, as System Settings puts the account at the head of its list.
 *
 * With an app in front it speaks for that app, as Mail's footer does for its mailboxes: how
 * fresh that app is, and a quiet control to update it alone. In Settings it speaks for the
 * whole account.
 */
export function AccountCard({ current, onOpen, app }: { current: boolean; onOpen: () => void; app?: FooterApp | null }) {
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const { report, syncing, error, refreshService, refresh: refreshAll } = useSync();
  const own = useServiceState(app?.service ?? "");
  // "Updated just now" has to become "2 min ago" without anything else happening.
  const [, tick] = useState(0);
  useEffect(() => {
    api.accountInfo().then(setAccount, () => {});
    const t = window.setInterval(() => tick((n) => n + 1), 30_000);
    return () => window.clearInterval(t);
  }, []);
  let failed: boolean;
  let working: boolean;
  let status: string;
  // The whole sentence, naming the app: the sidebar has room for its short form only, as
  // Mail's footer says "Updated Just Now" under the mailbox it is plainly about.
  let full: string | undefined;
  if (app) {
    failed = own.failure !== null && !own.working;
    working = own.working;
    status = working
      ? "Updating…"
      : failed
        ? own.failure!
        : own.unavailable
          ? "Not available"
          : freshness(own.lastSyncedAt);
    full = working
      ? `Updating ${app.name}…`
      : failed
        ? `${app.name} didn’t update: ${own.failure}`
        : own.unavailable
          ? `${app.name} isn’t available`
          : `${app.name} ${status.replace(/^\w/, (c) => c.toLowerCase())}`;
  } else {
    failed = !!(error || report?.services.some((s) => s.outcome === "failed"));
    working = syncing;
    status = syncing ? "Syncing…" : failed ? "Some things didn't update" : freshness(report?.asOf ?? null);
  }
  // The first name, as the menu bar greets someone: a full name did not fit the sidebar and
  // ended in "…". Settings shows it whole.
  const name = account?.name?.split(/\s+/)[0] ?? account?.email ?? "Settings";
  // Two quiet lines, each one thing, as Mail's sidebar ends: how fresh this app is — the line
  // itself the control that updates it, its dot turning while it does — and below it who is
  // signed in, the way into Settings.
  const refresh = () => void (app ? refreshService(app.service) : refreshAll());
  return (
    <div className="sidebar-foot-stack">
      <button
        className={`sidebar-sync${failed ? " is-trouble" : ""}${working ? " is-working" : ""}`}
        onClick={refresh}
        disabled={working}
        aria-busy={working}
        title={working ? full ?? status : `${full ?? status}. Click to update now`}
        aria-label={working ? full ?? status : `${full ?? status}. Update now`}
      >
        <span className={`sync-dot ${failed ? "trouble" : working ? "working" : "clean"}`} aria-hidden="true" />
        <span className="sidebar-sync-text" aria-hidden="true">{status}</span>
        <span className="sidebar-sync-now" aria-hidden="true">Update Now</span>
      </button>
      <button className="sidebar-account" aria-current={current} onClick={onOpen} title="Settings">
        <AccountAvatar account={account} />
        <span className="sidebar-account-name">{name}</span>
      </button>
    </div>
  );
}
