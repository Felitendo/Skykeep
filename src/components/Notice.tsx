import { TriangleAlert, X } from "lucide-react";

/**
 * Something went wrong, said once and plainly, beside a way to put it away.
 *
 * Every screen showed failures as a bare red line — on Reminders, CloudKit's own JSON in
 * full — that stayed until something else replaced it.
 */
export function Notice({ message, onDismiss }: { message: string; onDismiss: () => void }) {
  return (
    <div className="notice" role="alert">
      <TriangleAlert size={15} strokeWidth={2} aria-hidden="true" />
      <p>{message}</p>
      <button className="notice-close" aria-label="Dismiss" title="Dismiss" onClick={onDismiss}>
        <X size={14} strokeWidth={2.25} aria-hidden="true" />
      </button>
    </div>
  );
}
