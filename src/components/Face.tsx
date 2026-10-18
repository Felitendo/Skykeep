import { useLayoutEffect, useRef, useState } from "react";

/**
 * A person's picture over their initials, as Contacts draws one.
 *
 * The initials hold the place until the picture is decoded, and the picture then fades in
 * over them — never initials swapped for a picture in one jump, which read as a flicker.
 * A picture already to hand (the backend keeps each on disk, the window keeps it in memory)
 * is drawn at once with no fade: there is nothing to wait for, so nothing should move. One
 * that cannot be had leaves the initials, never a broken frame.
 */
export function Face({ src, initials }: { src: string | null; initials: string }) {
  const img = useRef<HTMLImageElement | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "instant" | "failed">("loading");
  // Reset for a different picture; decided before paint whether it is already there.
  useLayoutEffect(() => {
    const el = img.current;
    setState(el && el.complete && el.naturalWidth > 0 ? "instant" : "loading");
  }, [src]);
  return (
    <>
      {state !== "ready" && state !== "instant" && <span className="face-initials">{initials}</span>}
      {src && state !== "failed" && (
        <img
          ref={img}
          className={`face-photo is-${state}`}
          src={src}
          alt=""
          decoding="async"
          draggable={false}
          onLoad={() => setState((s) => (s === "instant" ? s : "ready"))}
          onError={() => setState("failed")}
        />
      )}
    </>
  );
}
