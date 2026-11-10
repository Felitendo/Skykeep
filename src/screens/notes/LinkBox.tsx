import { useEffect, useLayoutEffect, useRef, useState } from "react";
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
 * Add Link (Command-K on a Mac): the address, and with nothing selected the text to show.
 * Return adds it, Escape or a click away leaves the note as it was; on a link already there,
 * Remove Link takes it off and leaves the words.
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
        <label className="link-box-row">
          <span>Link to</span>
          <input
            autoFocus
            value={address}
            inputMode="url"
            spellCheck={false}
            autoCapitalize="off"
            autoComplete="off"
            placeholder="example.com"
            aria-invalid={wrong}
            aria-describedby={wrong ? "link-box-wrong" : undefined}
            onChange={(e) => {
              setAddress(e.target.value);
              setWrong(false);
            }}
          />
        </label>
        {ask.needsText && (
          <label className="link-box-row">
            <span>Text</span>
            <input value={text} placeholder="Shown as the address" onChange={(e) => setText(e.target.value)} />
          </label>
        )}
        {wrong && (
          <p id="link-box-wrong" className="link-box-wrong" role="alert">
            Enter a web address, an email address, or a tel: or geo: link.
          </p>
        )}
        <div className="link-box-actions">
          {ask.href && (
            <button type="button" className="quiet link-box-remove" onClick={onRemove}>
              Remove Link
            </button>
          )}
          <button type="button" onClick={() => onCancel(true)}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!address.trim()}>
            {ask.href ? "Done" : "Add Link"}
          </button>
        </div>
      </form>
    </Portal>
  );
}
