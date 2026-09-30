import { forwardRef, useImperativeHandle, useLayoutEffect, useRef } from "react";

type Props = Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, "value" | "defaultValue"> & {
  value: string;
};

/**
 * A textarea that stays fast however long its text is.
 *
 * React keeps a controlled textarea's `defaultValue` — its child text node — equal to its
 * value, so every keystroke read and rewrote the whole text. In a 300,000-character note
 * that is a relayout of all of it per key: 35 ms each in WebKit, where a bare textarea takes
 * 4. Here the value is written to the element only when it differs from what the element
 * already holds, which while typing it never does.
 */
export const LongTextarea = forwardRef<HTMLTextAreaElement, Props>(function LongTextarea(
  { value, ...props },
  forwarded
) {
  const ref = useRef<HTMLTextAreaElement | null>(null);
  useImperativeHandle(forwarded, () => ref.current as HTMLTextAreaElement);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && el.value !== value) el.value = value;
  }, [value]);
  return <textarea ref={ref} {...props} />;
});
