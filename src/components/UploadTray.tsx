import type { UploadEvent } from "../lib/ipc";
import { FileGlyph } from "./FileGlyph";

/** What a person reads about one file, in the fewest words that are still true. */
function status(u: UploadEvent): string {
  switch (u.state) {
    case "waiting":
      return "Waiting";
    case "sending":
      return `${Math.floor((u.sent / Math.max(u.total, 1)) * 100)}%`;
    case "verifying":
      return "Checking";
    case "done":
      return "Uploaded";
    case "failed":
      return "Didn’t upload";
  }
}

const extensionOf = (name: string) => {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1) : null;
};

/**
 * Every file going up, each with its own progress, in the corner like a download list.
 *
 * "Uploading…" on a button said nothing about which of forty photos had gone and which had
 * not. This names each one, shows how far it has got, and only calls it uploaded once iCloud
 * has been shown to hold exactly its bytes — so "Uploaded" is a statement, not a hope.
 */
export function UploadTray({
  uploads,
  onDismiss,
}: {
  uploads: UploadEvent[];
  onDismiss: () => void;
}) {
  if (uploads.length === 0) return null;
  const finished = uploads.filter((u) => u.state === "done" || u.state === "failed");
  const failed = uploads.filter((u) => u.state === "failed").length;
  const active = finished.length < uploads.length;
  const total = uploads.reduce((n, u) => n + u.total, 0);
  const sent = uploads.reduce((n, u) => n + (u.state === "done" ? u.total : u.sent), 0);
  const percent = total > 0 ? Math.floor((sent / total) * 100) : 100;

  const heading = active
    ? `Uploading ${finished.length + 1 > uploads.length ? uploads.length : finished.length + 1} of ${uploads.length}`
    : failed > 0
      ? `${failed} of ${uploads.length} didn’t upload`
      : uploads.length === 1
        ? "Uploaded"
        : `${uploads.length} uploaded`;

  return (
    <section className="upload-tray" aria-label="Uploads" aria-live="polite">
      <header className="upload-tray-head">
        <span className="upload-tray-title">{heading}</span>
        {active ? (
          <span className="upload-tray-percent">{percent}%</span>
        ) : (
          <button className="upload-tray-close" onClick={onDismiss} aria-label="Close uploads">
            Done
          </button>
        )}
      </header>
      {active && (
        <div className="upload-bar" aria-hidden="true">
          <span style={{ width: `${percent}%` }} />
        </div>
      )}
      <ul className="upload-rows">
        {uploads.map((u) => (
          <li key={u.id} className={`upload-row is-${u.state}`}>
            <span className="upload-glyph" aria-hidden="true">
              <FileGlyph kind="FILE" extension={extensionOf(u.name)} />
            </span>
            <span className="upload-text">
              <span className="upload-name">{u.name}</span>
              <span className="upload-where">
                {u.state === "failed" && u.error ? u.error : u.within || " "}
              </span>
              {(u.state === "sending" || u.state === "verifying") && (
                <span className="upload-bar small" aria-hidden="true">
                  <span style={{ width: `${Math.floor((u.sent / Math.max(u.total, 1)) * 100)}%` }} />
                </span>
              )}
            </span>
            <span className="upload-state">
              {u.state === "done" ? <span aria-label="Uploaded">✓</span> : status(u)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
