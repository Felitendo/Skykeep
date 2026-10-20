import { Portal } from "./Portal";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * One line of a menu, or `null` for a separator. An entry with `submenu` leads on to a menu
 * of its own, opened beside it as macOS opens one; `keep` leaves the menu open after `run`.
 */
export type MenuEntry =
  | {
      label: string;
      run?: () => void;
      danger?: boolean;
      keep?: boolean;
      submenu?: MenuEntry[];
      /** A choice that is on or off: a check beside it when on, as a Mac menu marks one. */
      checked?: boolean;
    }
  | null;

type Anchor = { x: number; y: number; alignRight: boolean };

/**
 * A menu as macOS draws one: under the ⋯ that opened it or at the pointer for a right-click,
 * kept on screen; arrows to move, Return to choose, Escape or a click elsewhere to leave.
 * A line with a submenu shows ▸ and opens it beside itself on hover, Return or →, to the
 * right where there is room and the left where there is not; ← or Escape closes it again.
 * Choosing anything, at any depth, closes every level.
 */
export function RowMenu({ at, entries, onClose }: { at: Anchor; entries: MenuEntry[]; onClose: () => void }) {
  useEffect(() => {
    // Any press outside every level of this menu dismisses it.
    const down = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest?.(".row-menu")) return;
      onClose();
      // A press on a button that opens a menu — this one's ⋯, or another row's — only closes
      // the open menu, as on the Mac, where the click that dismisses a menu is used up by it.
      // Let through, it opened the menu again under the pointer: the menu blinked, and could
      // not be closed from the button that opened it.
      const opener = target?.closest?.('[aria-haspopup="menu"]');
      if (!opener) return;
      const swallow = (c: MouseEvent) => {
        if (opener.contains(c.target as Node)) {
          c.stopPropagation();
          c.preventDefault();
        }
      };
      document.addEventListener("click", swallow, { capture: true, once: true });
      window.setTimeout(() => document.removeEventListener("click", swallow, true), 700);
    };
    const away = () => onClose();
    // Escape closes the menu wherever the keys are: focus is not always in it — a submenu
    // that closed under the pointer took the focus it held with it.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || (e.target as HTMLElement | null)?.closest?.(".row-menu")) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", key, true);
    document.addEventListener("mousedown", down);
    window.addEventListener("blur", away);
    window.addEventListener("resize", away);
    return () => {
      window.removeEventListener("keydown", key, true);
      document.removeEventListener("mousedown", down);
      window.removeEventListener("blur", away);
      window.removeEventListener("resize", away);
    };
  }, [onClose]);
  return (
    <Portal>
      <Panel entries={entries} anchor={{ kind: "point", ...at }} onDone={onClose} onBack={onClose} focusFirst={false} />
    </Portal>
  );
}

type PanelAnchor =
  | ({ kind: "point" } & Anchor)
  /** Beside a parent menu's line: its rectangle. */
  | { kind: "beside"; rect: DOMRect };

function Panel({
  entries,
  anchor,
  onDone,
  onBack,
  focusFirst,
  onArrive,
}: {
  entries: MenuEntry[];
  anchor: PanelAnchor;
  /** Something was chosen: every level closes. */
  onDone: () => void;
  /** This level closes, back to the one that opened it. */
  onBack: () => void;
  /** Opened by the keyboard: the first line is highlighted, as on the Mac. */
  focusFirst: boolean;
  /** The pointer came into this submenu. */
  onArrive?: () => void;
}) {
  const box = useRef<HTMLDivElement | null>(null);
  const lines = useRef<(HTMLButtonElement | null)[]>([]);
  const items = entries.filter((e): e is NonNullable<MenuEntry> => e !== null);
  const checks = items.some((e) => e.checked !== undefined);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  // Nothing highlighted until the pointer or a key says so, as a Mac menu opens.
  const [active, setActive] = useState(focusFirst ? 0 : -1);
  const [sub, setSub] = useState<{ index: number; byKeyboard: boolean } | null>(null);
  /**
   * Menu aim, as macOS has it: with a submenu open, crossing another line on the way to it
   * does not close it at once. The switch waits a moment, and is called off if the pointer
   * arrives in the submenu — moving diagonally from "Add to List" to its lists used to pass
   * over "Edit Contact", close the submenu, and open it again: the flicker.
   */
  const aim = useRef<number | null>(null);
  const stopAim = () => {
    if (aim.current !== null) window.clearTimeout(aim.current);
    aim.current = null;
  };
  useEffect(() => stopAim, []);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const pad = 8;
    let left: number;
    let top: number;
    if (anchor.kind === "point") {
      left = anchor.alignRight ? anchor.x - width : anchor.x;
      top = anchor.y;
      if (top + height > window.innerHeight - pad) top = anchor.y - height - (anchor.alignRight ? 36 : 0);
    } else {
      // Beside the line, its first item level with it; to the left when the right is full.
      const r = anchor.rect;
      left = r.right + 2 + width <= window.innerWidth - pad ? r.right + 2 : r.left - 2 - width;
      top = r.top - 5;
    }
    left = Math.max(pad, Math.min(left, window.innerWidth - width - pad));
    top = Math.max(pad, Math.min(top, window.innerHeight - height - pad));
    setPos({ left, top });
    // Placed once, where it opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Focus once it is placed: drawn hidden while it is measured, and a hidden element cannot
  // take focus — the keys went to whatever had it before.
  const placed = pos !== null;
  useLayoutEffect(() => {
    if (placed) box.current?.focus({ preventScroll: true });
  }, [placed]);

  const choose = (i: number, byKeyboard: boolean) => {
    const item = items[i];
    if (!item) return;
    if (item.submenu) {
      setSub({ index: i, byKeyboard });
      return;
    }
    if (!item.keep) onDone();
    item.run?.();
  };

  let n = -1;
  const open = sub ? items[sub.index] : null;
  return (
    <>
      <div
        ref={box}
        className="row-menu"
        role="menu"
        tabIndex={-1}
        style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: "hidden" }}
        onMouseEnter={onArrive}
        onKeyDown={(e) => {
          // Keys belong to the deepest open level.
          if (sub) return;
          const move = (by: number) => {
            e.preventDefault();
            setActive((a) => (a < 0 ? (by > 0 ? 0 : items.length - 1) : (a + by + items.length) % items.length));
          };
          if (e.key === "Escape" || e.key === "ArrowLeft") {
            e.preventDefault();
            e.stopPropagation();
            onBack();
          } else if (e.key === "ArrowDown") move(1);
          else if (e.key === "ArrowUp") move(-1);
          else if ((e.key === "Enter" || e.key === " " || e.key === "ArrowRight") && active >= 0) {
            e.preventDefault();
            if (e.key === "ArrowRight" && !items[active]?.submenu) return;
            choose(active, true);
          }
        }}
      >
        {entries.map((entry, i) => {
          if (!entry) return <div key={`sep${i}`} className="row-menu-sep" role="separator" />;
          n += 1;
          const index = n;
          return (
            <button
              key={`${entry.label}-${index}`}
              ref={(el) => {
                lines.current[index] = el;
              }}
              role={entry.checked === undefined ? "menuitem" : "menuitemcheckbox"}
              aria-checked={entry.checked}
              tabIndex={-1}
              aria-haspopup={entry.submenu ? "menu" : undefined}
              aria-expanded={entry.submenu ? sub?.index === index : undefined}
              className={`row-menu-item${entry.danger ? " is-danger" : ""}${index === active ? " is-active" : ""}${entry.submenu ? " has-submenu" : ""}`}
              onMouseEnter={() => {
                const go = () => {
                  aim.current = null;
                  // The submenu going takes its focus with it: the keys come back here.
                  if (sub && !entry.submenu) box.current?.focus({ preventScroll: true });
                  setActive(index);
                  // Hovering a line with a submenu opens it; any other line closes one.
                  setSub(entry.submenu ? { index, byKeyboard: false } : null);
                };
                stopAim();
                if (sub && sub.index !== index) aim.current = window.setTimeout(go, 280);
                else go();
              }}
              onClick={() => choose(index, false)}
            >
              {/* A menu with choices in it keeps the column for the check on every line, so
                  the words line up whether a line is checked or not. */}
              {checks && (
                <span className="row-menu-check" aria-hidden="true">
                  {entry.checked ? "✓" : ""}
                </span>
              )}
              <span className="row-menu-label">{entry.label}</span>
              {entry.submenu && <span className="row-menu-chevron" aria-hidden="true">›</span>}
            </button>
          );
        })}
      </div>
      {open?.submenu && sub && lines.current[sub.index] && (
        <Panel
          key={sub.index}
          entries={open.submenu}
          anchor={{ kind: "beside", rect: lines.current[sub.index]!.getBoundingClientRect() }}
          onDone={onDone}
          onBack={() => {
            setSub(null);
            box.current?.focus({ preventScroll: true });
          }}
          onArrive={() => {
            // The pointer made it: the line that opened this stays the highlighted one.
            stopAim();
            setActive(sub.index);
          }}
          focusFirst={sub.byKeyboard}
        />
      )}
    </>
  );
}
