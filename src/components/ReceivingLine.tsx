import { Progress, Spinner } from "./Spinner";

/**
 * Many things arriving from iCloud at once — "Receiving 48 photos" — said in place, calmly: the
 * words, and under them how far along once that is known (a thin bar), or a small spinner beside
 * them until it is. One line that keeps its place while the count changes (figures are tabular),
 * announced politely, never a sheet or a toast. The styles are kit.css's `.receiving-line`.
 */
export function ReceivingLine({ label, done, total }: { label: string; done?: number; total?: number }) {
  const known = typeof done === "number" && typeof total === "number" && total > 0;
  return (
    <p className="receiving-line" role="status" aria-live="polite">
      <span className="receiving-line-text">
        {!known && <Spinner small />}
        {label}
      </span>
      {known && <Progress value={done / total} label={label} />}
    </p>
  );
}
