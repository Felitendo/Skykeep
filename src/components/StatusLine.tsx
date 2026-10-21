import { Spinner } from "./Spinner";

/**
 * A line of status in place — under a sheet's button, in a sidebar — said the one way: a wait
 * with the spinner, a reason in red, a note in grey. Where there is a window's content to float
 * over, the same words go in a toast instead (components/Toast); this is for what has to stay
 * beside the control it is about.
 */
export function StatusLine({ tone, id, children }: { tone: "pending" | "error" | "note"; id?: string; children: React.ReactNode }) {
  return (
    <p id={id} className={`status-line is-${tone}`} role={tone === "error" ? "alert" : "status"}>
      {tone === "pending" && <Spinner small />}
      {children}
    </p>
  );
}
