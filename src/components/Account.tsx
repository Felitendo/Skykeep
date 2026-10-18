import { useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { freshness, useSync } from "../lib/sync";

/** "Asha Rao" → "AR"; an address → its first letter. */
export function initialsOf(name: string): string {
  const words = name.split(/[\s@._-]+/).filter(Boolean);
  if (words.length === 0) return "";
  if (name.includes("@") || words.length === 1) return words[0][0]!.toUpperCase();
  return (words[0][0]! + words[words.length - 1][0]!).toUpperCase();
}

/**
 * The account at the foot of the sidebar — who is signed in and how fresh their iCloud is —
 * and the way into Settings, as System Settings puts the account at the head of its list.
 */
export function AccountCard({ current, onOpen }: { current: boolean; onOpen: () => void }) {
  const [account, setAccount] = useState<{ name: string | null; email: string | null } | null>(null);
  const { report, syncing, error } = useSync();
  useEffect(() => {
    api.accountInfo().then(setAccount, () => {});
  }, []);
  const failed = error || report?.services.some((s) => s.outcome === "failed");
  const status = syncing ? "Syncing…" : failed ? "Some things didn't update" : freshness(report?.asOf ?? null);
  const name = account?.name ?? account?.email ?? "Settings";
  return (
    <button className="sidebar-account" aria-current={current} onClick={onOpen} title="Settings">
      <span className="account-avatar" aria-hidden="true">{initialsOf(name)}</span>
      <span className="sidebar-account-text">
        <span className="sidebar-account-name">{name}</span>
        <span className={`sidebar-account-status${failed ? " is-trouble" : ""}`}>
          <span className={`sync-dot ${failed ? "trouble" : syncing ? "working" : "clean"}`} aria-hidden="true" />
          {status}
        </span>
      </span>
    </button>
  );
}
