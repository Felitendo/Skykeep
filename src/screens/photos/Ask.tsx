/**
 * A question before a change that reaches every device, as icloud.com's Photos asks it: a
 * sheet over the dimmed window — a symbol, the question in bold, a line on what follows, and
 * Cancel beside the action, red when it destroys. Escape or a click outside cancels; Cancel
 * has the focus, so Return never deletes by accident.
 */
import { useCallback, useRef, useState } from "react";
import { EyeOff, Trash2 } from "lucide-react";
import { Portal } from "../../components/Portal";

export type Question = {
  title: string;
  body: string;
  action: string;
  /** Red, and the trash symbol: a delete. */
  danger?: boolean;
  symbol?: "trash" | "hide";
};

export function useAsk(): [(q: Question) => Promise<boolean>, React.ReactNode] {
  const [asking, setAsking] = useState<Question | null>(null);
  const answer = useRef<((yes: boolean) => void) | null>(null);
  const ask = useCallback(
    (q: Question) =>
      new Promise<boolean>((resolve) => {
        answer.current?.(false);
        answer.current = resolve;
        setAsking(q);
      }),
    [],
  );
  const done = (yes: boolean) => {
    answer.current?.(yes);
    answer.current = null;
    setAsking(null);
  };
  const sheet = asking && (
    <Portal>
      <div className="modal-veil" onClick={() => done(false)}>
        <div
          className="modal-sheet px-ask"
          role="alertdialog"
          aria-modal="true"
          aria-label={asking.title}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              done(false);
            }
          }}
        >
          {asking.symbol && (
            <span className={`px-ask-symbol${asking.danger ? " is-danger" : ""}`} aria-hidden="true">
              {asking.symbol === "trash" ? <Trash2 size={30} strokeWidth={1.6} /> : <EyeOff size={30} strokeWidth={1.6} />}
            </span>
          )}
          <h2>{asking.title}</h2>
          <p>{asking.body}</p>
          <div className="px-ask-actions">
            <button autoFocus onClick={() => done(false)}>
              Cancel
            </button>
            <button className={`is-act${asking.danger ? " danger" : ""}`} onClick={() => done(true)}>
              {asking.action}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
  return [ask, sheet];
}
