import { TriangleAlert } from "lucide-react";
import type { RefusedChange } from "../lib/ipc";

/** The small grey words beside something saved here that Apple does not have yet. */
export const WAITING = "Waiting to sync";

/** Inline, so it sits beside any row or title without a stylesheet of its own. */
export function Waiting({ shown }: { shown: boolean }) {
  if (!shown) return null;
  return (
    <span className="sync-waiting" role="status">
      {WAITING}
    </span>
  );
}

/**
 * A change Apple turned down for good — edited in the same place on another device, deleted
 * elsewhere — said with what it held, never dropped: the words can be copied out, sent again
 * or let go. One at a time, oldest first, as the queue holds them.
 */
export function Unsent({
  refused,
  what,
  onResend,
  onDiscard,
}: {
  refused: RefusedChange[];
  /** What a change is, in this app's words: "an edit to a note". */
  what: string;
  onResend: (r: RefusedChange) => void;
  onDiscard: (r: RefusedChange) => void;
}) {
  const r = refused[0];
  if (!r) return null;
  const more = refused.length - 1;
  const preview = r.content.length > 120 ? `${r.content.slice(0, 120)}…` : r.content;
  const copy = () => void navigator.clipboard?.writeText(r.content).catch(() => {});
  return (
    // In the flow, not floating like a toast: it stays until decided, and over the list it
    // would hide the rows under it for as long.
    <div className="unsent" role="alert">
      <TriangleAlert className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />
      <div className="unsent-body">
        {/* Apple's reason already says what was not saved; `what` names it for a reader. */}
        <p aria-label={`iCloud didn’t take ${what}: ${r.reason}`}>
          {r.reason}
          {more > 0 ? ` (and ${more} more)` : ""}
        </p>
        {preview && <p className="unsent-content">“{preview}”</p>}
        <div className="unsent-actions">
          {r.content && (
            <button className="quiet" onClick={copy}>
              Copy Text
            </button>
          )}
          <button className="quiet" onClick={() => onResend(r)}>
            Try Again
          </button>
          <button className="quiet danger" onClick={() => onDiscard(r)}>
            Discard
          </button>
        </div>
      </div>
    </div>
  );
}
