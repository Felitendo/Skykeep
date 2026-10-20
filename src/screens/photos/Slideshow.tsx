/**
 * A slideshow, as Photos plays one: the photos one after another on black, each easing slowly
 * larger while it shows and fading into the next. Space pauses, the arrows step, Escape ends.
 * The controls sit in a glass capsule that fades away while the pointer is still.
 *
 * Only opacity and transform move — the compositor's work — and the next picture is fetched
 * while the one before it shows, so a change never waits on a download.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Pause, Play, X } from "lucide-react";
import { pictureSrc, type Photo } from "../../lib/ipc";

const HOLD = 4000;
const FADE = 900;

export function Slideshow({ photos, start, onClose }: { photos: Photo[]; start: number; onClose: () => void }) {
  const [at, setAt] = useState(Math.max(0, Math.min(start, photos.length - 1)));
  const [playing, setPlaying] = useState(true);
  const [awake, setAwake] = useState(true);
  const still = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const go = useCallback((by: number) => setAt((i) => (i + by + photos.length) % photos.length), [photos.length]);

  // On, one after another; round again at the end, as Photos loops a slideshow.
  useEffect(() => {
    if (!playing || photos.length < 2) return undefined;
    const t = window.setTimeout(() => go(1), HOLD);
    return () => window.clearTimeout(t);
  }, [at, playing, go, photos.length]);

  // The next picture is fetched and decoded now, whole, so it is there when its turn comes.
  useEffect(() => {
    const next = photos[(at + 1) % photos.length];
    if (!next || next.kind === "video") return;
    const img = new Image();
    img.decoding = "async";
    img.src = pictureSrc(next.id, "full");
    void img.decode().catch(() => undefined);
  }, [at, photos]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === " ") setPlaying((p) => !p);
      else if (e.key === "ArrowRight") go(1);
      else if (e.key === "ArrowLeft") go(-1);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    // Before the screen's own keys, which would otherwise step the photo behind or close it.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [go, onClose]);

  // The controls show when the pointer moves and go once it rests.
  const idle = useRef<number | null>(null);
  const wake = () => {
    setAwake(true);
    if (idle.current !== null) window.clearTimeout(idle.current);
    idle.current = window.setTimeout(() => setAwake(false), 2200);
  };
  useEffect(() => {
    wake();
    return () => {
      if (idle.current !== null) window.clearTimeout(idle.current);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // The one showing and the one before it, so the old fades out over the new.
  const [before, setBefore] = useState<number | null>(null);
  const shown = useRef(at);
  useEffect(() => {
    if (shown.current === at) return undefined;
    setBefore(shown.current);
    shown.current = at;
    const t = window.setTimeout(() => setBefore(null), FADE);
    return () => window.clearTimeout(t);
  }, [at]);

  const photo = photos[at];
  if (!photo) return null;
  return (
    <div
      className={`px-show${awake ? " is-awake" : ""}${still ? " is-still" : ""}`}
      role="dialog"
      aria-label="Slideshow"
      onMouseMove={wake}
      onClick={wake}
    >
      {before !== null && photos[before] && (
        <img key={`b${photos[before].id}`} className="px-slide is-leaving" src={pictureSrc(photos[before].id, "medium")} alt="" />
      )}
      {before !== null && photos[before] && photos[before].kind !== "video" && (
        <img key={`bw${photos[before].id}`} className="px-slide is-leaving" src={pictureSrc(photos[before].id, "full")} alt="" aria-hidden="true" />
      )}
      <img
        key={`a${photo.id}-${at}`}
        className={`px-slide${playing ? "" : " is-paused"}`}
        src={pictureSrc(photo.id, "medium")}
        alt={photo.filename ?? "Photo"}
      />
      {/* The screen-sized copy is soft on a large screen; the whole picture covers it, moving
          with it on the same clock. */}
      {photo.kind !== "video" && (
        <img
          key={`w${photo.id}-${at}`}
          className={`px-slide${playing ? "" : " is-paused"}`}
          src={pictureSrc(photo.id, "full")}
          alt=""
          aria-hidden="true"
          decoding="async"
        />
      )}
      <div className="px-show-bar px-capsule" role="toolbar" aria-label="Slideshow controls">
        <button className="px-cap" aria-label="Previous" onClick={() => go(-1)}>
          <ChevronLeft size={18} />
        </button>
        <button className="px-cap" aria-label={playing ? "Pause" : "Play"} onClick={() => setPlaying((p) => !p)}>
          {playing ? <Pause size={17} fill="currentColor" strokeWidth={0} /> : <Play size={17} fill="currentColor" strokeWidth={0} />}
        </button>
        <button className="px-cap" aria-label="Next" onClick={() => go(1)}>
          <ChevronRight size={18} />
        </button>
        <span className="px-show-count" aria-live="polite">
          {at + 1} of {photos.length}
        </span>
        <button className="px-cap" aria-label="End Slideshow" title="End Slideshow (Esc)" onClick={onClose}>
          <X size={17} />
        </button>
      </div>
    </div>
  );
}
