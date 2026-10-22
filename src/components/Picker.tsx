import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Portal } from "./Portal";
import { useTypeSelect } from "./RowMenu";

/** `divider` draws a rule above the choice, setting it apart from the ones before it;
 *  `disabled` shows a choice greyed, as a Mac menu shows one that cannot be taken here. */
export type PickerOption = { value: string; label: string; divider?: boolean; disabled?: boolean };

/**
 * A pop-up button, in the shape macOS actually uses.
 *
 * A native `<select>` renders its menu from the operating system, so on Linux it arrives as
 * a square grey list with a blue band through it — the one piece of the window that looks
 * like it belongs to a different decade. Everything visible here is ours: the closed
 * control, the menu, the checkmark against the current choice.
 *
 * It behaves the way a listbox is supposed to: one tab stop, arrows move the highlight,
 * Enter takes it, Escape leaves without changing anything, and focus comes back to the
 * button either way.
 */
export function Picker({
  value,
  options,
  onChange,
  label,
  disabled = false,
  className = "",
  pullDown = true,
}: {
  value: string;
  options: PickerOption[];
  onChange: (value: string) => void;
  /** Names the control for anyone who cannot see it. */
  label: string;
  disabled?: boolean;
  className?: string;
  /** Dropped below the button (the default), as a pull-down is, rather than laid over it
   *  with the choice on the button as a Mac pop-up is. Laid over, a menu in a form covered
   *  the rows around it, and one in a toolbar climbed over the window's top edge. */
  pullDown?: boolean;
}) {
  const [open, setOpenState] = useState(false);
  /** Whether the menu is open as of the last key, ahead of the render that shows it: two
   *  keys can arrive before React has drawn the first one's answer. */
  const isOpen = useRef(false);
  const setOpen = useCallback((to: boolean) => {
    isOpen.current = to;
    setOpenState(to);
  }, []);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const items = useRef<(HTMLLIElement | null)[]>([]);
  const listId = useId();

  const current = options.find((o) => o.value === value);

  /**
   * Where the menu sits, as a Mac pop-up button places it: over the button, the current
   * choice lying exactly on it, so the eye does not have to move — and then kept inside the
   * window, scrolling if it is taller than the window.
   *
   * Drawn in a layer of its own over everything (a portal), never inside the control's own
   * box. Inside a popover, a scrolling list or a clipped pane the menu was cut off, flipped
   * after being drawn, or sat at the wrong place under a transformed ancestor — the menu
   * problems that kept coming back. Placed before it is painted, so it never jumps.
   */
  const menu = useRef<HTMLUListElement | null>(null);
  const [place, setPlace] = useState<{ left: number; top: number; width: number; maxHeight: number } | null>(null);
  useLayoutEffect(() => {
    if (!open || !menu.current || !button.current) {
      setPlace(null);
      return;
    }
    const b = button.current.getBoundingClientRect();
    const list = menu.current;
    const chosen = items.current[Math.max(0, options.findIndex((o) => o.value === value))];
    const pad = 8;
    const maxHeight = window.innerHeight - pad * 2;
    const height = Math.min(list.scrollHeight, maxHeight);
    const width = Math.max(b.width, list.scrollWidth);
    if (pullDown) {
      // Below the button, its left edge with the button's; above it only where there is not
      // room below.
      const below = window.innerHeight - pad - (b.bottom + 4);
      const fitsBelow = below >= Math.min(height, 160);
      const room = fitsBelow ? below : b.top - 4 - pad;
      const h = Math.min(height, room);
      const top = fitsBelow ? b.bottom + 4 : b.top - 4 - h;
      const left = Math.max(pad, Math.min(b.left, window.innerWidth - pad - width));
      setPlace({ left, top, width, maxHeight: room });
      return;
    }
    // The chosen row's label over the button's label.
    const rowTop = chosen ? chosen.offsetTop : 0;
    let top = b.top - rowTop + (b.height - (chosen?.offsetHeight ?? b.height)) / 2;
    top = Math.max(pad, Math.min(top, window.innerHeight - pad - height));
    const left = Math.max(pad, Math.min(b.left - 4, window.innerWidth - pad - width));
    setPlace({ left, top, width, maxHeight });
    // Taller than the window: scrolled so the chosen row is in view.
    if (chosen && list.scrollHeight > maxHeight) list.scrollTop = Math.max(0, rowTop - (b.top - top));
    // On opening only: what is chosen does not move the menu while it is open.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = useCallback(
    (restoreFocus: boolean) => {
      setOpen(false);
      if (restoreFocus) button.current?.focus();
    },
    []
  );

  const choose = useCallback(
    (index: number) => {
      const option = options[index];
      if (option?.disabled) return;
      if (option) onChange(option.value);
      close(true);
    },
    [options, onChange, close]
  );

  // Opening lands the highlight on what is already chosen, not at the top — the current
  // value is where someone is navigating away from.
  /** Where the pointer was when it was last seen over the menu; none since it opened. */
  const pointer = useRef<{ x: number; y: number } | null>(null);
  /** Whether a row is lit. On opening only the ✓ shows what is chosen, as a Mac menu opens:
   *  lit as well, the current row read as two marks for one thing. The pointer moving or a
   *  key lights a row. */
  const [lit, setLitState] = useState(false);
  const litNow = useRef(false);
  const setLit = (to: boolean) => {
    litNow.current = to;
    setLitState(to);
  };
  const openMenu = (byKey = false) => {
    if (disabled) return;
    pointer.current = null;
    // Opened from the keyboard, the current row is lit straight away, as on the Mac.
    setLit(byKey);
    setActive(Math.max(0, options.findIndex((o) => o.value === value)));
    setOpen(true);
  };

  // Focus follows the keyboard, not the pointer. Moving focus on every hover made the
  // browser re-scroll and repaint the menu under the mouse, which read as flicker.
  const byKeyboard = useRef(true);
  // Only once the menu is placed and showing: it is drawn hidden while it is measured, and
  // a hidden element cannot take focus — the first key went nowhere.
  const shown = open && place !== null;
  useEffect(() => {
    if (shown && byKeyboard.current) items.current[active]?.focus({ preventScroll: true });
  }, [shown, active]);
  useEffect(() => {
    if (shown) {
      byKeyboard.current = true;
      items.current[active]?.focus({ preventScroll: true });
    }
    // Only on opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown]);

  // Clicking anywhere else dismisses the menu, which is what every menu does.
  useEffect(() => {
    if (!open) return undefined;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!root.current?.contains(t) && !menu.current?.contains(t)) setOpen(false);
    };
    // A menu fixed over a page that scrolls would be left floating away from its button; as
    // on the Mac, it goes — but only when its button has really moved. Any scroll at all
    // closed it, and a page still settling as the menu opened (a panel easing into view)
    // closed it the moment it appeared: open, shut, a flicker. Its own scrolling is not that.
    const at = button.current?.getBoundingClientRect();
    const onMove = (e: Event) => {
      if (menu.current?.contains(e.target as Node)) return;
      const now = button.current?.getBoundingClientRect();
      if (!at || !now || Math.abs(now.top - at.top) > 2 || Math.abs(now.left - at.left) > 2) setOpen(false);
    };
    const onLeave = () => setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("scroll", onMove, true);
    window.addEventListener("resize", onLeave);
    window.addEventListener("blur", onLeave);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("scroll", onMove, true);
      window.removeEventListener("resize", onLeave);
      window.removeEventListener("blur", onLeave);
    };
  }, [open]);

  const typeSelect = useTypeSelect(() => options.map((o) => o.label));
  const onListKey = (e: React.KeyboardEvent) => {
    // From the highlight as it stands after any key not yet drawn, not as last rendered.
    const move = (to: (at: number) => number) => {
      e.preventDefault();
      byKeyboard.current = true;
      // The first key lights the row it starts from; the next ones move.
      if (!litNow.current) {
        setLit(true);
        return;
      }
      setActive((at) => {
        let next = Math.max(0, Math.min(to(at), options.length - 1));
        // A greyed choice is passed over, in the direction the key went.
        const step = next >= at ? 1 : -1;
        while (options[next]?.disabled && next + step >= 0 && next + step < options.length) next += step;
        return options[next]?.disabled ? at : next;
      });
    };
    // Typing goes to the choice that begins with it, as a Mac pop-up's menu does.
    const typedTo = typeSelect(e, litNow.current ? active : -1);
    if (typedTo !== null) {
      e.preventDefault();
      byKeyboard.current = true;
      setLit(true);
      setActive(typedTo);
      return;
    }
    if (e.key === "ArrowDown") move((at) => at + 1);
    else if (e.key === "ArrowUp") move((at) => at - 1);
    else if (e.key === "Home") move(() => 0);
    else if (e.key === "End") move(() => options.length - 1);
    else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      choose(active);
    } else if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      // Escape closes the menu and nothing more — not the popover or sheet it is in.
      if (e.key === "Escape") e.stopPropagation();
      close(true);
    }
  };

  return (
    <div className={`picker ${className}`.trim()} ref={root}>
      <button
        type="button"
        ref={button}
        className="picker-button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        disabled={disabled}
        onClick={() => (open ? close(false) : openMenu(false))}
        onKeyDown={(e) => {
          // Open already, with focus not yet moved in: a key pressed that quickly is the
          // menu's. Opening it again put the highlight back and the key was lost.
          if (isOpen.current) return onListKey(e);
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            openMenu(true);
          }
        }}
      >
        <span className="picker-value">{current?.label ?? ""}</span>
        <span className="picker-chevron" aria-hidden="true">
          ⌄
        </span>
      </button>

      {open && (
        <Portal>
        <ul
          ref={menu}
          className="picker-menu"
          role="listbox"
          id={listId}
          aria-label={label}
          style={
            place
              ? { left: place.left, top: place.top, minWidth: place.width, maxHeight: place.maxHeight }
              : { left: 0, top: 0, visibility: "hidden" }
          }
        >
          {options.map((option, index) => (
            <li
              key={option.value}
              ref={(el) => {
                items.current[index] = el;
              }}
              role="option"
              aria-selected={option.value === value}
              aria-disabled={option.disabled || undefined}
              tabIndex={index === active ? 0 : -1}
              className={[lit && index === active ? "is-active" : "", option.divider ? "has-divider" : "", option.disabled ? "is-disabled" : ""].join(" ").trim() || undefined}
              onKeyDown={onListKey}
              onClick={() => choose(index)}
              // The pointer highlights a row only once it moves. The menu opens over the
              // button, under a pointer that is still: the row it lands on is not chosen by
              // it, and taking it as chosen threw the keyboard's highlight back.
              onMouseMove={(e) => {
                const at = pointer.current;
                if (at && at.x === e.clientX && at.y === e.clientY) return;
                const first = at === null;
                pointer.current = { x: e.clientX, y: e.clientY };
                if (first || option.disabled) return;
                byKeyboard.current = false;
                setLit(true);
                setActive(index);
              }}
            >
              <span className="picker-check" aria-hidden="true">
                {option.value === value ? "✓" : ""}
              </span>
              <span className="picker-label">{option.label}</span>
            </li>
          ))}
        </ul>
        </Portal>
      )}
    </div>
  );
}
