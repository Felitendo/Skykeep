import { useState } from "react";
import { freshness, useSync } from "../lib/sync";

/**
 * The one place the whole account's freshness is stated, and the one control that refreshes
 * it.
 *
 * Every screen used to carry its own "Updated 3 min ago" and its own Refresh button, which
 * meant four different answers to one question and four ways to be out of date. This sits
 * at the foot of the sidebar — where a mail client puts it — and speaks for the library as
 * a whole: the age it reports is the age of its stalest part, never its freshest.
 *
 * Trouble is disclosed, not hidden and not shouted: a service that failed is named with the
 * reason, and everything else stays current around it.
 */
export function SyncStatus() {
  const { report, syncing, refresh, error } = useSync();
  const [showTrouble, setShowTrouble] = useState(false);

  const failed = report?.services.filter((s) => s.outcome === "failed") ?? [];
  const state = error || failed.length > 0 ? "trouble" : syncing ? "working" : report?.asOf ? "clean" : "idle";
  const label = syncing ? "Updating…" : freshness(report?.asOf ?? null);

  return (
    <div className="sync">
      <button
        className="sync-row"
        onClick={() => void refresh()}
        disabled={syncing}
        aria-label={syncing ? "Updating your iCloud" : `${label}. Update now`}
      >
        <span className={`sync-dot ${state}`} aria-hidden="true" />
        <span className="sync-label">{label}</span>
        <span className={`sync-glyph${syncing ? " spinning" : ""}`} aria-hidden="true">
          ↻
        </span>
      </button>

      {error && (
        <p className="sync-trouble" role="alert">
          {error}
        </p>
      )}

      {failed.length > 0 && (
        <>
          <button
            className="sync-disclose"
            aria-expanded={showTrouble}
            onClick={() => setShowTrouble((v) => !v)}
          >
            {failed.length === 1
              ? `${failed[0].label} didn't update`
              : `${failed.length} services didn't update`}
          </button>
          {showTrouble && (
            <ul className="sync-trouble-list">
              {failed.map((s) => (
                <li key={s.service}>
                  <b>{s.label}</b> {s.detail}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
