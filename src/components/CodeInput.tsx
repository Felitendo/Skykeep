import { Fragment, useEffect, useRef } from "react";

/**
 * Six-digit verification code.
 *
 * Behaves the way people actually use these: typing advances, backspace on an empty box steps
 * back, arrows move, and pasting the whole code from anywhere fills every box at once.
 */
export function CodeInput({
  value,
  onChange,
  onComplete,
  disabled,
}: {
  value: string;
  onChange: (next: string) => void;
  onComplete: (code: string) => void;
  disabled?: boolean;
}) {
  const boxes = useRef<(HTMLInputElement | null)[]>([]);

  useEffect(() => {
    boxes.current[0]?.focus();
  }, []);

  const digits = value.padEnd(6, " ").slice(0, 6).split("");

  function setAt(index: number, digit: string) {
    const next = digits.map((d, i) => (i === index ? digit : d)).join("").trimEnd();
    onChange(next);
    return next;
  }

  function handleChange(index: number, raw: string) {
    const typed = raw.replace(/\D/g, "");
    if (!typed) return;
    // Typing or pasting several digits at once fills forward from here.
    if (typed.length > 1) {
      const merged = (value.slice(0, index) + typed).replace(/\D/g, "").slice(0, 6);
      onChange(merged);
      const landed = Math.min(merged.length, 5);
      boxes.current[landed]?.focus();
      if (merged.length === 6) onComplete(merged);
      return;
    }
    const next = setAt(index, typed);
    if (index < 5) boxes.current[index + 1]?.focus();
    if (next.replace(/\s/g, "").length === 6) onComplete(next);
  }

  function handleKeyDown(index: number, e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Backspace") {
      e.preventDefault();
      if (digits[index].trim()) {
        setAt(index, " ");
      } else if (index > 0) {
        setAt(index - 1, " ");
        boxes.current[index - 1]?.focus();
      }
    } else if (e.key === "ArrowLeft" && index > 0) {
      boxes.current[index - 1]?.focus();
    } else if (e.key === "ArrowRight" && index < 5) {
      boxes.current[index + 1]?.focus();
    }
  }

  return (
    <div className="code" role="group" aria-label="Six-digit verification code">
      {digits.map((digit, i) => (
        <Fragment key={i}>
          {i === 3 && <span className="gap" aria-hidden="true" />}
          <input
            ref={(el) => {
              boxes.current[i] = el;
            }}
            value={digit.trim()}
            onChange={(e) => handleChange(i, e.target.value)}
            onKeyDown={(e) => handleKeyDown(i, e)}
            onFocus={(e) => e.target.select()}
            inputMode="numeric"
            autoComplete={i === 0 ? "one-time-code" : "off"}
            maxLength={6}
            disabled={disabled}
            aria-label={`Digit ${i + 1}`}
          />
        </Fragment>
      ))}
    </div>
  );
}
