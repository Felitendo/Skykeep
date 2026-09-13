import { useCallback, useEffect, useId, useRef, useState } from "react";

export type PickerOption = { value: string; label: string };

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
}: {
  value: string;
  options: PickerOption[];
  onChange: (value: string) => void;
  /** Names the control for anyone who cannot see it. */
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const items = useRef<(HTMLLIElement | null)[]>([]);
  const listId = useId();

  const current = options.find((o) => o.value === value);

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
      if (option) onChange(option.value);
      close(true);
    },
    [options, onChange, close]
  );

  // Opening lands the highlight on what is already chosen, not at the top — the current
  // value is where someone is navigating away from.
  const openMenu = () => {
    if (disabled) return;
    setActive(Math.max(0, options.findIndex((o) => o.value === value)));
    setOpen(true);
  };

  useEffect(() => {
    if (open) items.current[active]?.focus();
  }, [open, active]);

  // Clicking anywhere else dismisses the menu, which is what every menu does.
  useEffect(() => {
    if (!open) return undefined;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const onListKey = (e: React.KeyboardEvent) => {
    const move = (to: number) => {
      e.preventDefault();
      setActive(Math.max(0, Math.min(to, options.length - 1)));
    };
    if (e.key === "ArrowDown") move(active + 1);
    else if (e.key === "ArrowUp") move(active - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(options.length - 1);
    else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      choose(active);
    } else if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
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
        onClick={() => (open ? close(false) : openMenu())}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            openMenu();
          }
        }}
      >
        <span className="picker-value">{current?.label ?? ""}</span>
        <span className="picker-chevron" aria-hidden="true">
          ⌄
        </span>
      </button>

      {open && (
        <ul className="picker-menu" role="listbox" id={listId} aria-label={label}>
          {options.map((option, index) => (
            <li
              key={option.value}
              ref={(el) => {
                items.current[index] = el;
              }}
              role="option"
              aria-selected={option.value === value}
              tabIndex={index === active ? 0 : -1}
              className={index === active ? "is-active" : undefined}
              onKeyDown={onListKey}
              onClick={() => choose(index)}
              onMouseEnter={() => setActive(index)}
            >
              <span className="picker-check" aria-hidden="true">
                {option.value === value ? "✓" : ""}
              </span>
              <span className="picker-label">{option.label}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
