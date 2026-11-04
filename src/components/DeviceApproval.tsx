import { useLayoutEffect, useRef, useState } from "react";
import { Smartphone } from "lucide-react";
import { fadeIn } from "../lib/fade";
import { api, messageOf, type DataAccess } from "../lib/ipc";
import { Spinner } from "./Spinner";

/** What each service is called on screen. */
const NAMES: Record<string, string> = {
  notes: "Notes",
  reminders: "Reminders",
  photos: "Photos",
  drive: "iCloud Drive",
};

/** The states in which a service waits on an approval on the person's devices. */
export function awaitsApproval(access: DataAccess | null, service: string): boolean {
  return Boolean(
    access && (access.state === "waiting" || access.state === "needed" || access.state === "notSent") && access.services.includes(service),
  );
}

/** Ask Apple again, saying what came back or why it could not be asked. */
function useAsk(onAccess: (a: DataAccess) => void) {
  const [asking, setAsking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ask = () => {
    setAsking(true);
    setError(null);
    api
      .dataAccessAsk()
      .then(onAccess, (e) => setError(messageOf(e)))
      .finally(() => setAsking(false));
  };
  return { ask, asking, error };
}

/** The line under the heading: where the approval stands. */
function standing(access: DataAccess): string {
  if (access.state === "notSent") {
    return "Apple couldn’t reach your devices. Check that your iPhone, iPad or Mac is on and connected, then try again.";
  }
  if (access.state === "waiting") return "Waiting for your iPhone, iPad or Mac…";
  return "Click Try Again, then approve on your iPhone, iPad or Mac.";
}

/**
 * Advanced Data Protection keeps this service end-to-end encrypted, and Apple opens it to
 * Skykeep once the person approves on one of their devices. Drawn in place of a screen that has
 * nothing on this computer yet, in the same calm layout as the web-access screen beside it.
 */
export function ApprovalScreen({
  access,
  service,
  onAccess,
}: {
  access: DataAccess;
  service: string;
  onAccess: (a: DataAccess) => void;
}) {
  const { ask, asking, error } = useAsk(onAccess);
  const name = NAMES[service] ?? "this";
  const waiting = access.state === "waiting";
  const problem = error ?? access.problem;
  return (
    <div className="web-access-off device-approval">
      <Smartphone className="device-approval-glyph" size={30} strokeWidth={1.6} aria-hidden="true" />
      <h1>Approve on your iPhone, iPad or Mac</h1>
      <p>
        Your Apple Account uses Advanced Data Protection, which keeps {name} end-to-end encrypted. Once you
        approve Skykeep on one of your devices, it opens here.
      </p>
      <p className="device-approval-status" role="status" aria-live="polite">
        {waiting && <Spinner small />}
        <span>{problem ?? standing(access)}</span>
      </p>
      <button className="primary" onClick={ask} disabled={asking}>
        Try Again
      </button>
    </div>
  );
}

/**
 * The same, for a screen that already has its copy on this computer: the screen stays, and
 * one card at the foot of the content says what would bring it up to date.
 */
export function ApprovalNotice({
  access,
  service,
  onAccess,
}: {
  access: DataAccess;
  service: string;
  onAccess: (a: DataAccess) => void;
}) {
  const { ask, asking, error } = useAsk(onAccess);
  const card = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => fadeIn(card.current, { duration: 140 }), []);
  const name = NAMES[service] ?? "this";
  const waiting = access.state === "waiting";
  const problem = error ?? access.problem;
  return (
    <div ref={card} className="toast update-notice device-approval-notice" role="group" aria-label="Approve on your devices">
      {waiting ? <Spinner small /> : <Smartphone className="toast-glyph" size={15} strokeWidth={2} aria-hidden="true" />}
      <div className="update-body">
        <p className="update-line" role="status" aria-live="polite">
          {problem ??
            (waiting
              ? `Approve Skykeep on your iPhone, iPad or Mac to bring ${name} up to date.`
              : `To bring ${name} up to date, approve Skykeep on your iPhone, iPad or Mac.`)}
        </p>
        {!waiting && (
          <div className="update-actions">
            <button className="toast-action" onClick={ask} disabled={asking}>
              Try Again
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
