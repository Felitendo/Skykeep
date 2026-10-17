import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuEntry = { label: string; run: () => void; danger?: boolean } | null;

/**
 * A menu as macOS draws one: under the ⋯ that opened it or at the pointer for a right-click,
 * kept on screen, arrows to move, Return to choose, Escape or a click elsewhere to leave.
 */
export function RowMenu({ at, entries, onClose }: {
  at: { x: number; y: number; alignRight: boolean };
  entries: MenuEntry[];
  onClose: () => void;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: at.x, top: at.y });
  const items = entries.filter((e): e is NonNullable<MenuEntry> => e !== null);
  const [active, setActive] = useState(0);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    let left = at.alignRight ? at.x - width : at.x;
    let top = at.y;
    left = Math.max(8, Math.min(left, window.innerWidth - width - 8));
    if (top + height > window.innerHeight - 8) top = Math.max(8, at.y - height - (at.alignRight ? 36 : 0));
    setPos({ left, top });
    el.focus();
  }, [at]);
  useEffect(() => {
    const down = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) onClose(); };
    const away = () => onClose();
    document.addEventListener("mousedown", down);
    window.addEventListener("blur", away);
    window.addEventListener("resize", away);
    return () => {
      document.removeEventListener("mousedown", down);
      window.removeEventListener("blur", away);
      window.removeEventListener("resize", away);
    };
  }, [onClose]);
  const pick = (i: number) => { onClose(); items[i]?.run(); };
  let n = -1;
  return (
    <div
      ref={box}
      className="row-menu"
      role="menu"
      tabIndex={-1}
      style={{ left: pos.left, top: pos.top }}
      onKeyDown={(e) => {
        if (e.key === "Escape") { e.preventDefault(); onClose(); }
        else if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => (a + 1) % items.length); }
        else if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => (a - 1 + items.length) % items.length); }
        else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); pick(active); }
      }}
    >
      {entries.map((entry, i) => {
        if (!entry) return <div key={`sep${i}`} className="row-menu-sep" role="separator" />;
        n += 1;
        const index = n;
        return (
          <button
            key={entry.label}
            role="menuitem"
            tabIndex={-1}
            className={`row-menu-item${entry.danger ? " is-danger" : ""}${index === active ? " is-active" : ""}`}
            onMouseEnter={() => setActive(index)}
            onClick={() => pick(index)}
          >
            {entry.label}
          </button>
        );
      })}
    </div>
  );
}

