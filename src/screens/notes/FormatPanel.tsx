import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useActiveRef } from "../../lib/screen";
import { Check } from "lucide-react";
import { inMenu, Portal } from "../../components/Portal";
import type { EditorStatus } from "./NoteEditor";
import { STYLE } from "./rich";

/** The styles, in icloud.com's order, each drawn the way it sets a line. */
const STYLES: { style: number | null; label: string; className: string; mark?: string }[] = [
  { style: STYLE.title, label: "Title", className: "fp-title" },
  { style: STYLE.heading, label: "Heading", className: "fp-heading" },
  { style: STYLE.subheading, label: "Subheading", className: "fp-subheading" },
  { style: null, label: "Body", className: "fp-body" },
  { style: STYLE.mono, label: "Monospaced", className: "fp-mono" },
  { style: STYLE.bulleted, label: "Bulleted List", className: "fp-list", mark: "•" },
  { style: STYLE.dashed, label: "Dashed List", className: "fp-list", mark: "-" },
  { style: STYLE.numbered, label: "Numbered List", className: "fp-list", mark: "1." },
];

const MARKS = [
  { mark: "bold", label: "Bold", glyph: <b>B</b> },
  { mark: "italic", label: "Italic", glyph: <i>I</i> },
  { mark: "underline", label: "Underline", glyph: <u>U</u> },
  { mark: "strike", label: "Strikethrough", glyph: <s>S</s> },
] as const;

/**
 * Notes' Aa panel: bold, italic, underline and strikethrough across the top, the paragraph
 * styles beneath with the current one ticked — as icloud.com and the Mac lay it out. It acts
 * on the note without taking the cursor out of it, so a style is chosen and typing goes on.
 */
export function FormatPanel({
  at,
  status,
  onClose,
  onStyle,
  onMark,
  onQuote,
}: {
  at: { x: number; y: number };
  status: EditorStatus | null;
  onClose: () => void;
  onStyle: (style: number | null) => void;
  onMark: (mark: "bold" | "italic" | "underline" | "strike") => void;
  onQuote: () => void;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: at.x, top: at.y });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const left = Math.max(8, Math.min(at.x - width / 2, window.innerWidth - width - 8));
    const top = at.y + height > window.innerHeight - 8 ? Math.max(8, window.innerHeight - height - 8) : at.y;
    setPos({ left, top });
  }, [at]);
  const live = useActiveRef();
  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!box.current?.contains(t) && !t.closest(".notes-aa") && !inMenu(t)) onClose();
    };
    const key = (e: KeyboardEvent) => live.current && e.key === "Escape" && onClose();
    document.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const current = status?.style ?? null;
  const isCurrent = (style: number | null) =>
    style === null ? current === null || current === STYLE.body : current === style;

  return (
    <Portal>
      <div
        ref={box}
        className="format-panel"
        role="dialog"
        aria-label="Format"
        style={{ left: pos.left, top: pos.top }}
        // Clicks here never take focus from the note.
        onMouseDown={(e) => e.preventDefault()}
      >
        <div className="fp-marks" role="group" aria-label="Text style">
          {MARKS.map(({ mark, label, glyph }) => (
            <button
              key={mark}
              type="button"
              className="fp-mark"
              aria-label={label}
              aria-pressed={Boolean(status?.[mark])}
              title={label}
              onClick={() => onMark(mark)}
            >
              {glyph}
            </button>
          ))}
        </div>
        <div className="fp-styles" role="menu" aria-label="Paragraph style">
          {STYLES.map(({ style, label, className, mark }) => (
            <button
              key={label}
              type="button"
              role="menuitemradio"
              aria-checked={isCurrent(style)}
              className={`fp-style ${className}`}
              onClick={() => {
                onStyle(style);
                onClose();
              }}
            >
              <span className="fp-check" aria-hidden="true">{isCurrent(style) && <Check size={15} strokeWidth={2} />}</span>
              {mark && <span className="fp-list-mark" aria-hidden="true">{mark}</span>}
              <span>{label}</span>
            </button>
          ))}
        </div>
        {/* Not a style but a setting on top of one, as Notes keeps it: a quoted heading or
            list stays a heading or a list. */}
        <div className="fp-styles fp-extra" role="group" aria-label="Block quote">
          <button
            type="button"
            role="menuitemcheckbox"
            aria-checked={Boolean(status?.quote)}
            className="fp-style fp-quote"
            onClick={() => {
              onQuote();
              onClose();
            }}
          >
            <span className="fp-check" aria-hidden="true">{status?.quote && <Check size={15} strokeWidth={2} />}</span>
            <span className="fp-quote-bar" aria-hidden="true" />
            <span>Block Quote</span>
          </button>
        </div>
      </div>
    </Portal>
  );
}
