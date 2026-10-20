/**
 * A slideshow, as Photos plays one: the photos one after another on black, each easing slowly
 * larger while it shows and dissolving into the next. Space pauses, the arrows step, Escape ends.
 * The controls sit in a glass capsule that fades away while the pointer is still.
 *
 * Each photo is the picture made the size of the screen (fitSrc), so it is sharp from its first
 * frame — never a soft copy with a sharper one popping in over it. The next is fetched and
 * decoded before its turn, and the change waits for it: a photo is never shown half there. The
 * one leaving fades out as the next fades in — before, it stood whole behind the next for the
 * length of the fade, its edges showing round a narrower photo, and then vanished at once.
 */
import { useActiveRef } from "../../lib/screen";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Pause, Play, X } from "lucide-react";
import { fitSrc, type Photo } from "../../lib/ipc";
import { fadeIn } from "../../lib/fade";

const HOLD = 4000;
const FADE = 900;

/** The picture a photo is shown at, whole on this screen. */
const screenSrc = (p: Photo) => {
  const r = p.width && p.height ? p.width / p.height : 4 / 3;
  const w = Math.min(window.innerWidth, window.innerHeight * r);
  return fitSrc(p.id, Math.max(w, w / r) * (window.devicePixelRatio || 1));
};

/** Fetched and decoded, by address. */
const ready = new Map<string, Promise<void>>();
const decode = (url: string) => {
  let p = ready.get(url);
  if (!p) {
    const img = new Image();
    img.decoding = "async";
    img.src = url;
    p = img.decode().catch(() => {});
    ready.set(url, p);
  }
  return p;
};

export function Slideshow({ photos, start, onClose }: { photos: Photo[]; start: number; onClose: () => void }) {
  // Every photo, a video shown by its still, as Photos includes them.
  const list = photos;
  const first = Math.max(0, Math.min(start, photos.length - 1));
  // What shows, and the one fading out under it.
  const [at, setAt] = useState(first);
  const [leaving, setLeaving] = useState<number | null>(null);
  const [playing, setPlaying] = useState(true);
  const [awake, setAwake] = useState(true);
  const shown = useRef(first);

  // To another photo, once its picture is decoded.
  const asked = useRef(0);
  const go = useCallback(
    (by: number) => {
      const to = (shown.current + by + list.length) % list.length;
      const mine = ++asked.current;
      void decode(screenSrc(list[to])).then(() => {
        if (mine !== asked.current || to === shown.current) return;
        setLeaving(shown.current);
        shown.current = to;
        setAt(to);
      });
    },
    [list],
  );
  useEffect(() => {
    if (leaving === null) return undefined;
    const t = window.setTimeout(() => setLeaving(null), FADE);
    return () => window.clearTimeout(t);
  }, [leaving, at]);

  // On, one after another; round again at the end, as Photos loops a slideshow.
  useEffect(() => {
    if (!playing || list.length < 2) return undefined;
    const t = window.setTimeout(() => go(1), HOLD);
    return () => window.clearTimeout(t);
  }, [at, playing, go, list.length]);

  // The next is on its way while this one shows.
  useEffect(() => {
    const next = list[(at + 1) % list.length];
    if (next) void decode(screenSrc(next));
  }, [at, list]);

  const live = useActiveRef();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
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

  // The arriving photo dissolves in over the one leaving, frame by frame from the page, set before
  // its first frame is painted.
  const arriving = useRef<HTMLImageElement | null>(null);
  useLayoutEffect(() => {
    if (leaving !== null) fadeIn(arriving.current, { duration: FADE });
  }, [at]); // eslint-disable-line react-hooks/exhaustive-deps

  const photo = list[at];
  if (!photo) return null;
  const old = leaving !== null ? list[leaving] : null;
  return (
    <div
      className={`px-show${awake ? " is-awake" : ""}`}
      role="dialog"
      aria-label="Slideshow"
      onMouseMove={wake}
      onClick={wake}
    >
      {old && <img key={`l${old.id}-${leaving}`} className="px-slide is-leaving" src={screenSrc(old)} alt="" aria-hidden="true" />}
      <img
        key={`a${photo.id}-${at}`}
        ref={arriving}
        className="px-slide"
        src={screenSrc(photo)}
        alt={photo.filename ?? "Photo"}
      />
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
          {at + 1} of {list.length}
        </span>
        <button className="px-cap" aria-label="End Slideshow" title="End Slideshow (Esc)" onClick={onClose}>
          <X size={17} />
        </button>
      </div>
    </div>
  );
}
