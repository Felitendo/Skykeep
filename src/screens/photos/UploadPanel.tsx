/**
 * Photos' uploads, as icloud.com shows them: a panel that slides in at the right with the files
 * on their way — each with its own picture, how much has gone, Pending with a way to stop it —
 * and, at the end, what did not go and why ("Duplicate item."), until Done slides it away. A
 * clean finish goes by itself after a moment.
 */
import { Progress, Spinner } from "../../components/Spinner";
import { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Check, CloudAlert, CloudUpload, Info, X } from "lucide-react";
import type { UploadEvent } from "../../lib/ipc";

const ended = (u: UploadEvent) => u.state === "done" || u.state === "failed" || u.state === "cancelled";

const bytes = (n: number) => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
};

const items = (n: number) => `${n.toLocaleString()} ${n === 1 ? "Item" : "Items"}`;

/** The file's own picture, made small by the app; a plain glyph for what cannot be drawn. */
function Thumb({ id }: { id: string }) {
  const [broken, setBroken] = useState(false);
  if (broken) return <span className="px-up-thumb is-blank" aria-hidden="true" />;
  return (
    <img
      className="px-up-thumb"
      src={convertFileSrc(`up/${id}`, "quince")}
      alt=""
      loading="lazy"
      decoding="async"
      draggable={false}
      onError={() => setBroken(true)}
    />
  );
}

export function UploadPanel({
  uploads,
  cancel,
  onDismiss,
}: {
  uploads: UploadEvent[];
  cancel: (ids: string[], batch?: string) => Promise<void>;
  onDismiss: () => void;
}) {
  const files = uploads.filter((u) => !u.folder && u.state !== "planned");
  const active = files.some((u) => !ended(u));
  const failed = files.filter((u) => u.state === "failed");
  const cancelled = files.filter((u) => u.state === "cancelled").length;
  const total = files.reduce((n, u) => n + u.total, 0);
  const sent = files.reduce((n, u) => n + (u.state === "done" ? u.total : u.sent), 0);
  // The bar only ever goes forward: events arrive out of order, and a bar that steps back reads
  // as something gone wrong.
  const most = useRef(0);
  if (files.length === 0) most.current = 0;
  const percent = (most.current = Math.max(most.current, total ? sent / total : 0));
  const processing = active && files.every((u) => ended(u) || u.state === "verifying");

  // Gone at once, as it came: a panel the window's height sliding was a large layer moving,
  // the kind WebKitGTK on Hyprland flickers.
  const close = onDismiss;

  // A clean finish goes by itself; anything that did not go waits to be read.
  useEffect(() => {
    if (files.length === 0 || active || failed.length) return undefined;
    const t = window.setTimeout(close, 4000);
    return () => window.clearTimeout(t);
  }, [files.length, active, failed.length, close]);

  if (files.length === 0) return null;
  const done = files.length - failed.length - cancelled;
  const title = active
    ? processing
      ? "Finalizing Uploads"
      : `Uploading ${items(files.length)} to iCloud`
    : failed.length
      ? failed.length === files.length
        ? `${items(failed.length)} Not Uploaded`
        : `${failed.length} of ${items(files.length)} Not Uploaded`
      : cancelled && !done
        ? "Upload Stopped"
        : `${items(done)} Uploaded`;
  const batches = [...new Set(files.filter((u) => !ended(u)).map((u) => u.batch))];

  return (
    <aside className="px-up" aria-label="Uploads" aria-live="polite">
      <header className="px-up-head">
        {active ? (
          <button className="link px-up-link" onClick={() => batches.forEach((b) => void cancel([], b))}>
            Stop Upload
          </button>
        ) : (
          <button className="link px-up-link" onClick={close}>
            Done
          </button>
        )}
      </header>
      <div className="px-up-hero">
        {active ? (
          <CloudUpload className="px-up-icon" size={34} strokeWidth={1.6} aria-hidden="true" />
        ) : failed.length ? (
          <CloudAlert className="px-up-icon" size={34} strokeWidth={1.6} aria-hidden="true" />
        ) : (
          <Check className="px-up-icon" size={34} strokeWidth={1.8} aria-hidden="true" />
        )}
        <h2 className="px-up-title">{title}</h2>
        {active && (
          <Progress value={percent} label="Uploaded so far" className="px-up-bar" />
        )}
      </div>
      <ul className="px-up-rows">
        {(active ? files : [...failed, ...files.filter((u) => u.state !== "failed")]).map((u) => (
          <li key={u.id} className={`px-up-row is-${u.state}`}>
            <Thumb id={u.id} />
            <span className="px-up-text">
              <span className="px-up-name">{u.name}</span>
              <span className="px-up-sub">
                {u.state === "failed"
                  ? u.error ?? "Not uploaded."
                  : u.state === "cancelled"
                    ? "Stopped"
                    : u.state === "done"
                      ? bytes(u.total)
                      : u.state === "verifying"
                        ? "Processing…"
                        : `${bytes(u.sent)} of ${bytes(u.total)}`}
              </span>
            </span>
            <span className="px-up-end">
              {u.state === "waiting" && (
                <>
                  <span className="px-up-pending">Pending</span>
                  <button className="icon-button small round px-up-stop" aria-label={`Stop ${u.name}`} title="Stop" onClick={() => void cancel([u.id])}>
                    <X size={11} strokeWidth={3} aria-hidden="true" />
                  </button>
                </>
              )}
              {(u.state === "sending" || u.state === "verifying") && <Spinner label="Uploading" />}
              {u.state === "done" && <Check className="px-up-ok" size={15} strokeWidth={2.4} aria-label="Uploaded" />}
              {u.state === "failed" && <Info className="px-up-why" size={15} strokeWidth={2} aria-label={u.error ?? "Not uploaded"} />}
            </span>
          </li>
        ))}
      </ul>
    </aside>
  );
}
