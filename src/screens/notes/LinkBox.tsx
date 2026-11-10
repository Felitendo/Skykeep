import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Link, Type, Unlink } from "lucide-react";
import { Portal } from "../../components/Portal";
import { useActiveRef } from "../../lib/screen";
import { linkAddress } from "./linking";

/** What the box was opened on: where, the link already there, and whether text is asked for. */
export type LinkAsk = {
  /** The selection's foot on screen; the box opens below it, or above when there is no room. */
  at: { x: number; top: number; bottom: number };
  /** The address of the link the cursor stands in, to change or take off. */
  href: string | null;
  /** Nothing selected and no link there: the words to show are asked for too. */
  needsText: boolean;
};

/**
 * Add Link (Command-K on a Mac), drawn as Notes' own link popover and as this app's other
 * glass surfaces are: one field for the address, a second for the words to show only when
 * nothing is selected, no buttons. Return adds it, Escape or a click away leaves the note as it
 * was; on a link already there, the unlink symbol in the field takes it off.
 */
export function LinkBox({
  ask,
  onDone,
  onRemove,
  onCancel,
}: {
  ask: LinkAsk;
  onDone: (href: string, text: string) => void;
  onRemove: () => void;
  /** Closed with nothing changed: `back` into the note (Escape, Cancel), or clicked away. */
  onCancel: (back: boolean) => void;
}) {
  const box = useRef<HTMLFormElement | null>(null);
  const [address, setAddress] = useState(ask.href ?? "");
  const [text, setText] = useState("");
  const [wrong, setWrong] = useState(false);
  const [pos, setPos] = useState({ left: ask.at.x, top: ask.at.bottom + 6 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const width = el.offsetWidth;
    const height = el.offsetHeight;
    const left = Math.max(8, Math.min(ask.at.x - width / 2, window.innerWidth - width - 8));
    const below = ask.at.bottom + 6;
    const top = below + height <= window.innerHeight - 8 ? below : Math.max(8, ask.at.top - height - 6);
    setPos({ left, top });
  }, [ask, wrong]);
  const live = useActiveRef();
  useEffect(() => {
    const down = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) onCancel(false);
    };
    const key = (e: KeyboardEvent) => {
      if (live.current && e.key === "Escape") {
        e.preventDefault();
        onCancel(true);
      }
    };
    const resize = () => onCancel(true);
    document.addEventListener("mousedown", down);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", resize);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", resize);
    };
  }, [onCancel, live]);

  const title = ask.href ? "Edit Link" : "Add Link";
  return (
    <Portal>
      <form
        ref={box}
        className="link-box popover"
        role="dialog"
        aria-label={title}
        style={{ left: pos.left, top: pos.top }}
        onSubmit={(e) => {
          e.preventDefault();
          const href = linkAddress(address);
          if (!href) {
            setWrong(true);
            return;
          }
          onDone(href, text);
        }}
      >
        <div className="search-field link-box-field">
          <Link size={14} strokeWidth={2} aria-hidden="true" />
          <input
            autoFocus
            aria-label="Link to"
            value={address}
            inputMode="url"
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            placeholder="Type or paste a link"
            aria-invalid={wrong}
            aria-describedby={wrong ? "link-box-wrong" : undefined}
            onChange={(e) => {
              setAddress(e.target.value);
              setWrong(false);
            }}
          />
          {ask.href && (
            <button type="button" className="icon-button small" aria-label="Remove Link" title="Remove Link" onClick={onRemove}>
              <Unlink size={14} strokeWidth={2} aria-hidden="true" />
            </button>
          )}
        </div>
        {ask.needsText && (
          <div className="search-field link-box-field">
            <Type size={14} strokeWidth={2} aria-hidden="true" />
            <input aria-label="Text" value={text} placeholder="Text to show (optional)" onChange={(e) => setText(e.target.value)} />
          </div>
        )}
        {wrong && (
          <p id="link-box-wrong" className="link-box-wrong" role="alert">
            That isn’t a link Notes can open. Try a web or email address.
          </p>
        )}
        {/* Return in either field adds it, as in Notes; Escape leaves. */}
        <button type="submit" hidden aria-hidden="true" tabIndex={-1} />
      </form>
    </Portal>
  );
}
