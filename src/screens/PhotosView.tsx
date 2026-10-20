/**
 * Photos, as on a Mac: the library edge to edge, newest first, grouped by month, with a zoom
 * for how many across; a photo opens on a dark stage with the arrows to go through them. Everything draws from the local library at once; thumbnails come
 * from the disk cache and are fetched once.
 */
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ChevronLeft, ChevronRight, CopyMinus, CopyPlus, Download, Clock, Heart, ImagePlus, Images, Info, Minus, Play, Plus, RotateCcw, Scan, Trash2, EyeOff, Video, X, type LucideIcon } from "lucide-react";
import {
  api,
  messageOf,
  pictureSrc,
  PHOTO_UPLOAD,
  type Photo,
  type PhotoAlbum,
  type PhotoView,
  type UploadEvent,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Notice } from "../components/Notice";
import { UploadTray } from "../components/UploadTray";
import { RowMenu, type MenuEntry } from "../components/RowMenu";

/** The sidebar, in Photos' order, each with the symbol Photos gives it. */
const VIEWS: { key: PhotoView; label: string; Icon: LucideIcon }[] = [
  { key: "library", label: "Library", Icon: Images },
  { key: "favorites", label: "Favorites", Icon: Heart },
  { key: "recents", label: "Recents", Icon: Clock },
  { key: "videos", label: "Videos", Icon: Video },
  { key: "screenshots", label: "Screenshots", Icon: Scan },
];
/** Photos' Utilities: what is kept out of the library. */
const UTILITIES: typeof VIEWS = [
  { key: "hidden", label: "Hidden", Icon: EyeOff },
  { key: "recentlyDeleted", label: "Recently Deleted", Icon: Trash2 },
];

/** How many across at each step of the zoom, as Photos steps it. */
const ACROSS = [10, 7, 5, 3];

const monthOf = (seconds: number) =>
  new Date(seconds * 1000).toLocaleDateString(undefined, { month: "long", year: "numeric" });

/** What Photos takes, as icloud.com's picker offers it. */
const KINDS = ["jpg", "jpeg", "heic", "heif", "png", "gif", "tif", "tiff", "webp", "dng", "mov", "mp4", "m4v", "avi"];

/** Photos' album symbol: a card with the cards behind it showing above, as a stack. */
const AlbumGlyph = () => (
  <svg className="px-glyph" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
    <rect x="3" y="9" width="18" height="12" rx="2.5" />
    <path d="M5.5 5.5h13M8 2.5h8" />
  </svg>
);

/** "3 Photos Selected", "1 Video Selected", "5 Items Selected" — Photos' words. */
const pickedWords = (list: Photo[]) => {
  const videos = list.filter((p) => p.kind === "video").length;
  const n = list.length;
  const noun = videos === 0 ? "Photo" : videos === n ? "Video" : "Item";
  return `${n.toLocaleString()} ${n === 1 ? noun : `${noun}s`} Selected`;
};

type Flag = "favorite" | "deleted" | "hidden" | "expunged";

/**
 * How long a photo in Recently Deleted has left, in Photos' words: "29 days left" (or, on a
 * tile, "29 days"), then hours, then minutes; red under four days, as Photos turns it.
 */
const timeLeft = (p: Photo, compact: boolean) => {
  if (!p.expungeAt) return null;
  const ms = Math.max(0, p.expungeAt * 1000 - Date.now());
  const [unit, size] = ms >= 86_400_000 ? ["day", 86_400_000] : ms >= 3_600_000 ? ["hour", 3_600_000] : ["minute", 60_000];
  const n = Math.max(1, Math.floor(ms / size));
  return { words: `${n} ${unit}${n === 1 ? "" : "s"}${compact ? "" : " left"}`, soon: ms < 4 * 86_400_000 };
};

const clock = (s: number | null) => {
  if (!s) return "";
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.round(s % 60)).padStart(2, "0")}`;
};

/** What a tile can ask of the screen; one object, kept the same, so tiles need not redraw. */
type Acts = {
  pick: (index: number, e: React.MouseEvent) => void;
  open: (index: number) => void;
  menu: (photo: Photo, e: React.MouseEvent) => void;
};

/**
 * One photo in the grid. It redraws only when its own photo, its selection or its place
 * changes: a click that selects one of twenty thousand redraws two tiles, not all of them.
 */
const Tile = memo(function Tile({
  photo,
  index,
  selected,
  tabbable,
  left,
  soon,
  act,
}: {
  photo: Photo;
  index: number;
  selected: boolean;
  tabbable: boolean;
  left: string | null;
  soon: boolean;
  act: Acts;
}) {
  return (
    <button
      role="listitem"
      className="px-tile"
      aria-label={`${photo.filename ?? "Photo"}, ${new Date(photo.takenAt * 1000).toLocaleString()}`}
      data-id={photo.id}
      data-index={index}
      aria-selected={selected}
      tabIndex={tabbable ? 0 : -1}
      onClick={(e) => {
        // The keys follow the click, as in Photos: WebKit does not focus a button it clicks,
        // and the arrows went on to whatever had the focus before — the app list, which they
        // moved through instead of the photos.
        e.currentTarget.focus({ preventScroll: true });
        act.pick(index, e);
      }}
      onDoubleClick={() => act.open(index)}
      onContextMenu={(e) => {
        e.preventDefault();
        act.menu(photo, e);
      }}
    >
      <img src={pictureSrc(photo.id, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
      {photo.kind === "video" && (
        <span className="px-badge px-duration">
          <Play size={10} strokeWidth={0} fill="currentColor" aria-hidden="true" />
          {clock(photo.duration)}
        </span>
      )}
      {left && <span className={`px-badge px-left${soon ? " is-soon" : ""}`}>{left}</span>}
      {photo.favorite && <Heart className="px-heart" size={13} strokeWidth={0} fill="currentColor" aria-label="Favorite" />}
    </button>
  );
});

/**
 * A month of the grid. Only months near the view are built: one further off is an empty
 * block exactly as tall as its rows will be (worked out in CSS from the grid's own width),
 * so a library of tens of thousands opens at once and scrolls without the bar jumping.
 */
const Month = memo(function Month({
  label,
  items,
  chosen,
  anchor,
  deleted,
  eager,
  across,
  act,
}: {
  label: string;
  items: { photo: Photo; index: number }[];
  chosen: Set<string>;
  anchor: number | null;
  deleted: boolean;
  eager: boolean;
  across: number;
  act: Acts;
}) {
  const ref = useRef<HTMLElement | null>(null);
  const [live, setLive] = useState(eager);
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setLive(true);
      return undefined;
    }
    const watch = new IntersectionObserver(([e]) => setLive(e.isIntersecting), {
      root: el.closest(".px-scroll"),
      rootMargin: "1600px 0px",
    });
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  return (
    <section ref={ref} className="px-month" aria-label={label || undefined}>
      {label && <h2 className="px-month-title">{label}</h2>}
      {live ? (
        <div className="px-grid" role="list">
          {items.map(({ photo, index }) => {
            const t = deleted ? timeLeft(photo, true) : null;
            return (
              <Tile
                key={photo.id}
                photo={photo}
                index={index}
                selected={chosen.has(photo.id)}
                tabbable={anchor === index || (anchor === null && index === 0)}
                left={t?.words ?? null}
                soon={t?.soon ?? false}
                act={act}
              />
            );
          })}
        </div>
      ) : (
        <div className="px-later" style={{ "--rows": Math.ceil(items.length / across) } as React.CSSProperties} aria-hidden="true" />
      )}
    </section>
  );
});

export function PhotosView() {
  const { revision } = useSync();
  const [view, setView] = useState<PhotoView>("library");
  /** An album open instead of a view. */
  const [album, setAlbum] = useState<PhotoAlbum | null>(null);
  const [albums, setAlbums] = useState<[PhotoAlbum, number][]>([]);
  const [photos, setPhotos] = useState<Photo[] | null>(null);
  const [counts, setCounts] = useState<Partial<Record<PhotoView, number>>>({});
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  const [open, setOpen] = useState<number | null>(null);
  const [info, setInfo] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  /** Photos selected in the grid, as on a Mac: a click selects, a double-click opens. */
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const anchor = useRef<number | null>(null);
  const [uploads, setUploads] = useState<Map<string, UploadEvent>>(new Map());
  const [dropping, setDropping] = useState(false);
  const [menu, setMenu] = useState<{ at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] } | null>(null);
  /** An album being named in the sidebar: a new one (with the photos to put in it), or a rename. */
  const [naming, setNaming] = useState<{ id: string | null; then: string[]; draft: string } | null>(null);

  const read = useCallback(async () => {
    const [list, c, a] = await Promise.all([
      album ? api.photosInAlbum(album.id) : api.photosList(view),
      api.photosCounts(),
      api.photosAlbums(),
    ]);
    setPhotos(list);
    setCounts(Object.fromEntries(c));
    setAlbums(a);
    // An album deleted elsewhere is not left open, empty.
    if (album && !a.some(([x]) => x.id === album.id)) setAlbum(null);
  }, [view, album]);
  useEffect(() => {
    read().catch((e) => setError(messageOf(e)));
  }, [read, revision]);

  // Grouped by month, as the library reads; an album is one grid in its own order.
  const months = useMemo(() => {
    const out: { label: string; items: { photo: Photo; index: number }[] }[] = [];
    if (album) return [{ label: "", items: (photos ?? []).map((photo, index) => ({ photo, index })) }];
    (photos ?? []).forEach((photo, index) => {
      const label = monthOf(view === "recents" ? (photo.addedAt ?? photo.takenAt) : photo.takenAt);
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push({ photo, index });
      else out.push({ label, items: [{ photo, index }] });
    });
    return out;
  }, [photos, view, album]);

  const shown = open !== null ? photos?.[open] ?? null : null;

  /*
   * The viewer opens as Photos opens a photo: it grows out of its tile while the dark comes up
   * behind it, and closing sends it back into the tile of the photo now shown. Transform and
   * opacity only, so the compositor does it all; with reduced motion it simply appears.
   */
  const viewer = useRef<HTMLDivElement | null>(null);
  const stage = useRef<HTMLDivElement | null>(null);
  const origin = useRef<DOMRect | null>(null);
  const closing = useRef(false);
  // A close still running: its animations and its fallback, so a photo opened in the middle
  // of it is not shut by the close finishing, nor left faded out by it.
  const closingRun = useRef<{ anims: Animation[]; timer: number } | null>(null);
  const stopClosing = () => {
    const run = closingRun.current;
    if (!run) return;
    closingRun.current = null;
    closing.current = false;
    window.clearTimeout(run.timer);
    run.anims.forEach((a) => {
      a.onfinish = null;
      a.cancel();
    });
  };
  const [room, setRoom] = useState<{ w: number; h: number } | null>(null);
  useLayoutEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const measure = () => {
      const cs = getComputedStyle(el);
      setRoom({
        w: el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight),
        h: el.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom),
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [open !== null]); // eslint-disable-line react-hooks/exhaustive-deps
  // The photo's own box, worked out before it loads, so its thumbnail fills that box at once
  // and the sharper picture lands on top of it without anything moving.
  const fitted = (p: Photo): React.CSSProperties | undefined => {
    if (!room || !p.width || !p.height) return undefined;
    const k = Math.min(room.w / p.width, room.h / p.height, 1.5);
    return {
      width: Math.round(p.width * k),
      height: Math.round(p.height * k),
      maxWidth: "none",
      maxHeight: "none",
      backgroundImage: `url("${pictureSrc(p.id, "thumb")}")`,
    };
  };
  const still = () => typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const spring = () => getComputedStyle(document.documentElement).getPropertyValue("--spring-smooth").trim() || "ease-out";
  // From the shown picture's box to a tile's: the transform that lays one on the other.
  const toTile = (img: HTMLElement, tile: DOMRect) => {
    const r = img.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    const k = Math.max(tile.width / r.width, tile.height / r.height);
    const dx = tile.left + tile.width / 2 - (r.left + r.width / 2);
    const dy = tile.top + tile.height / 2 - (r.top + r.height / 2);
    return `translate(${dx}px, ${dy}px) scale(${k})`;
  };
  useLayoutEffect(() => {
    if (open === null || !room) return;
    const from = origin.current;
    origin.current = null;
    const el = viewer.current;
    if (!from || !el || still() || typeof el.animate !== "function") return;
    const img = el.querySelector<HTMLElement>(".px-shown");
    const t = img && toTile(img, from);
    if (img && t) img.animate([{ transform: t }, { transform: "none" }], { duration: 460, easing: spring() });
    for (const part of el.querySelectorAll<HTMLElement>(".px-backdrop, .px-viewer-bar, .px-nav, .px-info")) {
      part.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 260, easing: "ease-out" });
    }
  }, [open !== null && room !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const close = useCallback(() => {
    const el = viewer.current;
    const current = open !== null ? photos?.[open] : null;
    const tile = current ? document.querySelector<HTMLElement>(`.px-tile[data-id="${CSS.escape(current.id)}"]`) : null;
    const img = el?.querySelector<HTMLElement>(".px-shown");
    const box = tile?.getBoundingClientRect();
    const scroller = tile?.closest(".px-scroll")?.getBoundingClientRect();
    // Only back into a tile that is on screen; otherwise the viewer just fades.
    const seen = box && scroller && box.bottom > scroller.top && box.top < scroller.bottom;
    if (!el || closing.current || still() || typeof el.animate !== "function") {
      setOpen(null);
      return;
    }
    closing.current = true;
    const done = () => {
      if (!closingRun.current) return;
      closingRun.current = null;
      closing.current = false;
      setOpen(null);
    };
    const t = seen && img ? toTile(img, box) : null;
    const parts = [...el.querySelectorAll<HTMLElement>(".px-backdrop, .px-viewer-bar, .px-nav, .px-info")];
    const anims = parts.map((part) => part.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 220, easing: "ease-in", fill: "forwards" }));
    const last = t && img
      ? img.animate([{ transform: "none" }, { transform: t }], { duration: 340, easing: spring(), fill: "forwards" })
      : el.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 200, easing: "ease-in", fill: "forwards" });
    anims.push(last);
    last.onfinish = done;
    // A finish that never comes (a hidden window stops the clock) must not leave it open.
    closingRun.current = { anims, timer: window.setTimeout(done, 600) };
  }, [open, photos]);
  // Which way the last step went, so the next photo slides in from that side.
  const stepped = useRef(0);
  const step = useCallback(
    (by: number) =>
      setOpen((at) => {
        if (at === null || !photos) return at;
        const to = Math.max(0, Math.min(photos.length - 1, at + by));
        if (to !== at) stepped.current = by;
        return to;
      }),
    [photos],
  );

  /*
   * Zooming into a photo, as Photos does: double-click zooms to where was clicked and back;
   * Ctrl + scroll (a trackpad pinch arrives as that) zooms about the pointer; dragging pans.
   * One transform on the photo's frame, so it is the compositor's work. Past the fit, the
   * full picture is laid over the screen-sized one once it has loaded.
   */
  const [lens, setLens] = useState({ k: 1, x: 0, y: 0 });
  const lensRef = useRef(lens);
  lensRef.current = lens;
  const pan = useRef<HTMLDivElement | null>(null);
  const [sharp, setSharp] = useState<string | null>(null);
  const unzoom = () => setLens({ k: 1, x: 0, y: 0 });
  // Kept so the image's edge never comes in past the middle of the stage.
  const bound = (k: number, x: number, y: number) => {
    const el = pan.current;
    if (!el || k <= 1) return { k: 1, x: 0, y: 0 };
    const mx = ((k - 1) * el.offsetWidth) / 2;
    const my = ((k - 1) * el.offsetHeight) / 2;
    return { k, x: Math.max(-mx, Math.min(mx, x)), y: Math.max(-my, Math.min(my, y)) };
  };
  // Zoom to k, keeping the point under (sx, sy) — screen coordinates — where it is.
  const zoomAt = (k: number, sx: number, sy: number) => {
    const el = pan.current;
    if (!el) return;
    const box = el.getBoundingClientRect();
    const was = lensRef.current;
    const cx = box.left + box.width / 2 - was.x;
    const cy = box.top + box.height / 2 - was.y;
    const [dx, dy] = [sx - cx, sy - cy];
    const px = (dx - was.x) / was.k;
    const py = (dy - was.y) / was.k;
    const next = Math.max(1, Math.min(5, k));
    setLens(bound(next, dx - next * px, dy - next * py));
  };
  useEffect(() => {
    unzoom();
    setSharp(null);
  }, [shown?.id]);
  useEffect(() => {
    if (lens.k > 1.15 && shown && shown.kind !== "video") setSharp(shown.id);
  }, [lens.k > 1.15, shown?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // A pinch or Ctrl + wheel: the page must not zoom too, so this listener is not passive.
  useEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomAt(lensRef.current.k * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });
  const dragging = useRef<{ x: number; y: number; lx: number; ly: number } | null>(null);
  // The next photo slides in from the side the step went to.
  useLayoutEffect(() => {
    const by = stepped.current;
    stepped.current = 0;
    const el = stage.current?.querySelector<HTMLElement>(".px-pan, .px-video");
    if (!by || !el || still() || typeof el.animate !== "function") return;
    el.animate(
      [{ transform: `translateX(${by * 56}px)`, opacity: 0.2 }, { transform: "none", opacity: 1 }],
      { duration: 300, easing: spring() },
    );
  }, [shown?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // The viewer's keys: arrows to go through, Escape to close, I for the details.
  useEffect(() => {
    if (open === null) return undefined;
    const current = photos?.[open] ?? null;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight") step(1);
      else if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "Escape" && lensRef.current.k > 1) unzoom();
      else if (e.key === "Escape") close();
      else if ((e.ctrlKey || e.metaKey) && (e.key === "=" || e.key === "+" || e.key === "-" || e.key === "0") && current?.kind !== "video") {
        const box = pan.current?.getBoundingClientRect();
        if (!box) return;
        const k = e.key === "0" ? 1 : lensRef.current.k * (e.key === "-" ? 1 / 1.5 : 1.5);
        zoomAt(k, box.left + box.width / 2, box.top + box.height / 2);
      }
      else if (e.key === " " && current?.kind === "video") void play(current);
      else if (e.key.toLowerCase() === "i" && !e.metaKey && !e.ctrlKey) setInfo((v) => !v);
      // As in Photos: "." favorites, Delete sends it to Recently Deleted.
      else if (e.key === "." && current && view !== "recentlyDeleted") void change(current, "favorite", !current.favorite);
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l" && current && view !== "recentlyDeleted") void change(current, "hidden", view !== "hidden");
      else if ((e.key === "Delete" || e.key === "Backspace") && current && album) void removeFrom([current]);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && view !== "recentlyDeleted") void change(current, "deleted", true);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && view === "recentlyDeleted" && !album) void change(current, "expunged", true);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  /**
   * Favorite, Delete and Recover show at once, as on the phone; Apple's answer only matters if
   * it refuses, which puts the photo back as it was and says why.
   */
  const change = (p: Photo, flag: Flag, on: boolean) => changeAll([p], flag, on);
  const changeAll = async (list: Photo[], flag: Flag, on: boolean) => {
    if (list.length === 0) return;
    const n = list.length;
    // Asked first, in icloud.com's words: a delete reaches every device.
    if (
      flag === "deleted" && on &&
      !window.confirm(
        `${n === 1 ? "Delete this photo?" : `Delete ${n} photos?`}\n\n${n === 1 ? "It" : "They"} will be deleted from iCloud Photos on all your devices. ${n === 1 ? "It stays" : "They stay"} in Recently Deleted for 30 days.`,
      )
    ) {
      return;
    }
    // For good, in Photos' words: it cannot be undone.
    if (
      flag === "expunged" &&
      !window.confirm(
        `${n === 1 ? "Are you sure you want to delete this item permanently?" : `Are you sure you want to delete these ${n} items permanently?`}\n\nThis action cannot be undone.`,
      )
    ) {
      return;
    }
    // Hiding asks too, in Photos' words: where the photos go.
    if (
      flag === "hidden" && on &&
      !window.confirm(`${n === 1 ? "Hide this photo?" : `Hide ${n} photos?`}\n\n${n === 1 ? "It" : "They"} will be hidden from your library, and can be found in the Hidden album.`)
    ) {
      return;
    }
    const before = photos;
    const ids = new Set(list.map((p) => p.id));
    // Which changes take a photo out of the view it is in.
    const leaves =
      flag === "expunged" ? true
      : album ? flag !== "favorite" && on
      : flag === "deleted" ? (on ? view !== "recentlyDeleted" : view === "recentlyDeleted")
      : flag === "hidden" ? (on ? view !== "hidden" : view === "hidden")
      : !on && view === "favorites";
    setPhotos((all) =>
      leaves
        ? (all ?? []).filter((x) => !ids.has(x.id))
        : (all ?? []).map((x) => (ids.has(x.id) ? { ...x, [flag]: on } : x)),
    );
    if (leaves) {
      setChosen((c) => new Set([...c].filter((id) => !ids.has(id))));
      // The viewer goes on to the next photo, or the one before at the end, or closes.
      setOpen((at) => {
        if (at === null || !photos) return at;
        const left = photos.length - n;
        return left <= 0 ? null : Math.min(at, left - 1);
      });
    }
    try {
      for (const p of list) await api.photoSetFlag(p.id, flag, on);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
      read().catch(() => {});
    }
  };

  /*
   * Albums, as Photos keeps them: every change shows at once and goes to iCloud behind it; a
   * refusal puts the list back and says why. A new album sorts first, as Photos puts it.
   */
  const createAlbum = async (name: string, then: string[]) => {
    const before = albums;
    const temp: PhotoAlbum = { id: `new:${Date.now()}`, name, position: -1, changeTag: null };
    setAlbums((list) => [[temp, then.length], ...list]);
    try {
      const made = await api.photosAlbumCreate(name);
      setAlbums((list) => list.map((x) => (x[0].id === temp.id ? [made, x[1]] : x)));
      if (then.length) await api.photosAlbumAdd(made.id, then);
      read().catch(() => {});
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  const renameAlbum = async (a: PhotoAlbum, name: string) => {
    if (name === a.name) return;
    const before = albums;
    setAlbums((list) => list.map((x) => (x[0].id === a.id ? [{ ...x[0], name }, x[1]] : x)));
    if (album?.id === a.id) setAlbum({ ...a, name });
    try {
      await api.photosAlbumRename(a.id, name);
    } catch (e) {
      setAlbums(before);
      if (album?.id === a.id) setAlbum(a);
      setError(messageOf(e));
    }
  };

  const deleteAlbum = async (a: PhotoAlbum) => {
    if (!window.confirm(`Delete the album “${a.name}”?\n\nThe photos in it stay in your library.`)) return;
    const before = albums;
    setAlbums((list) => list.filter((x) => x[0].id !== a.id));
    if (album?.id === a.id) setAlbum(null);
    try {
      await api.photosAlbumDelete(a.id);
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  const addTo = async (a: PhotoAlbum, ids: string[]) => {
    const before = albums;
    setAlbums((list) => list.map((x) => (x[0].id === a.id ? [x[0], x[1] + ids.length] : x)));
    try {
      await api.photosAlbumAdd(a.id, ids);
      setSaved(ids.length === 1 ? `Added to ${a.name}` : `${ids.length} added to ${a.name}`);
      read().catch(() => {});
    } catch (e) {
      setAlbums(before);
      setError(messageOf(e));
    }
  };

  /** Out of the album open; the photos stay in the library. */
  const removeFrom = async (gone: Photo[]) => {
    if (!album) return;
    const n = gone.length;
    if (!window.confirm(`Remove ${n === 1 ? "this photo" : `${n} photos`} from the album?\n\n${n === 1 ? "It stays" : "They stay"} in your library.`)) return;
    const before = photos;
    const ids = new Set(gone.map((p) => p.id));
    setPhotos((list) => (list ?? []).filter((x) => !ids.has(x.id)));
    setAlbums((list) => list.map((x) => (x[0].id === album.id ? [x[0], Math.max(0, x[1] - n)] : x)));
    setOpen((at) => {
      if (at === null || !photos) return at;
      const left = photos.length - n;
      return left <= 0 ? null : Math.min(at, left - 1);
    });
    try {
      await api.photosAlbumRemove(album.id, [...ids]);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

  const selected = useMemo(() => (photos ?? []).filter((p) => chosen.has(p.id)), [photos, chosen]);

  const openAt = (index: number) => {
    const p = photos?.[index];
    if (!p) return;
    stopClosing();
    const tile = document.querySelector<HTMLElement>(`.px-tile[data-id="${CSS.escape(p.id)}"]`);
    origin.current = tile?.getBoundingClientRect() ?? null;
    setOpen(index);
  };

  // A click as Photos takes it: alone it selects that one, with Ctrl (⌘) it adds or takes
  // away, with Shift it selects everything from the last one chosen.
  const pick = (index: number, e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) => {
    const list = photos ?? [];
    const id = list[index]?.id;
    if (!id) return;
    if (e.shiftKey && anchor.current !== null) {
      const [a, b] = [Math.min(anchor.current, index), Math.max(anchor.current, index)];
      const span = list.slice(a, b + 1).map((p) => p.id);
      setChosen((c) => new Set([...(e.ctrlKey || e.metaKey ? c : []), ...span]));
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      setChosen((c) => {
        const next = new Set(c);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
    } else {
      setChosen(new Set([id]));
    }
    anchor.current = index;
  };

  // The tile to land on from another by an arrow: the nearest in that direction as laid out,
  // so moving down from the last row of a month goes into the next month's first row.
  const neighbour = (from: number, key: string): number | null => {
    const count = photos?.length ?? 0;
    if (key === "ArrowLeft") return from > 0 ? from - 1 : null;
    if (key === "ArrowRight") return from < count - 1 ? from + 1 : null;
    // Up and down follow the grid as laid out: the same column a row away, and across a
    // month's edge into the next month's first row or the last month's last. Worked out
    // from the months themselves, so it reaches months that are not built yet.
    const k = months.findIndex((m) => m.items.length && from >= m.items[0].index && from <= m.items[m.items.length - 1].index);
    if (k < 0) return null;
    const m = months[k];
    const at = from - m.items[0].index;
    const col = at % across;
    if (key === "ArrowDown") {
      if (at + across < m.items.length) return from + across;
      // Still a row below in this month, but shorter: its last photo.
      if (Math.floor(at / across) < Math.floor((m.items.length - 1) / across)) return m.items[m.items.length - 1].index;
      const next = months[k + 1];
      return next ? next.items[Math.min(col, next.items.length - 1)].index : null;
    }
    if (at - across >= 0) return from - across;
    const last = months[k - 1];
    if (!last) return null;
    const lastRow = Math.floor((last.items.length - 1) / across) * across;
    return last.items[Math.min(lastRow + col, last.items.length - 1)].index;
  };

  // The tile at an index, focused and in view; built first if its month is not yet.
  const reach = (to: number) => {
    const sel = `.px-scroll .px-tile[data-index="${to}"]`;
    let tries = 0;
    const go = () => {
      const tile = document.querySelector<HTMLElement>(sel);
      if (tile) {
        tile.focus({ preventScroll: true });
        tile.scrollIntoView({ block: "nearest" });
        return;
      }
      const k = months.findIndex((m) => m.items.some((x) => x.index === to));
      document.querySelectorAll<HTMLElement>(".px-scroll .px-month")[k]?.scrollIntoView({ block: "nearest" });
      if (++tries < 12) requestAnimationFrame(go);
    };
    go();
  };

  // The grid's keys, while no photo is open and nothing is being typed.
  useEffect(() => {
    if (open !== null) return undefined;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input, textarea, [contenteditable], .row-menu, [role=dialog]") || naming) return;
      if (!t?.closest?.(".px") && t !== document.body) return;
      const list = photos ?? [];
      const ctrl = e.ctrlKey || e.metaKey;
      if (ctrl && e.key.toLowerCase() === "a") {
        setChosen(new Set(list.map((p) => p.id)));
      } else if (e.key === "Escape" && chosen.size) {
        setChosen(new Set());
      } else if ((e.key === "Enter" || e.key === " ") && selected.length) {
        openAt(list.indexOf(selected[0]));
      } else if (e.key.startsWith("Arrow") && list.length) {
        const from = anchor.current ?? (e.key === "ArrowLeft" || e.key === "ArrowUp" ? list.length : -1);
        const to = from < 0 ? 0 : from >= list.length ? list.length - 1 : neighbour(from, e.key);
        if (to === null) return e.preventDefault();
        if (e.shiftKey && anchor.current !== null) setChosen((c) => new Set([...c, list[to].id]));
        else setChosen(new Set([list[to].id]));
        anchor.current = to;
        reach(to);
      } else if ((e.key === "Delete" || e.key === "Backspace") && selected.length) {
        if (album) void removeFrom(selected);
        else if (view !== "recentlyDeleted") void changeAll(selected, "deleted", true);
        else void changeAll(selected, "expunged", true);
      } else if (ctrl && e.key.toLowerCase() === "l" && selected.length && view !== "recentlyDeleted") {
        // ⌘L in Photos: hide, or in Hidden, unhide.
        void changeAll(selected, "hidden", view !== "hidden");
      } else if (e.key === "." && selected.length && view !== "recentlyDeleted") {
        // As Photos: if any is not a favorite, all become favorites; else all stop being.
        void changeAll(selected, "favorite", selected.some((p) => !p.favorite));
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const albumEntries = (ids: string[]): MenuEntry[] => [
    ...albums.filter(([a]) => a.id !== album?.id && !a.id.startsWith("new:")).map(([a]) => ({ label: a.name, run: () => void addTo(a, ids) })),
    ...(albums.length ? [null] : []),
    { label: "New Album…", run: () => setNaming({ id: null, then: ids, draft: "" }) },
  ];

  const photoMenu = (p: Photo, at: { x: number; y: number; alignRight: boolean }) => {
    // A right-click on a selected photo acts on the whole selection; on any other, it
    // selects that one first, as Photos does.
    const group = chosen.has(p.id) && selected.length > 1 ? selected : [p];
    if (!chosen.has(p.id)) {
      setChosen(new Set([p.id]));
      anchor.current = photos?.findIndex((x) => x.id === p.id) ?? null;
    }
    if (group.length > 1) {
      const n = group.length;
      const anyPlain = group.some((x) => !x.favorite);
      setMenu({
        at,
        entries:
          view === "recentlyDeleted" && !album
            ? [
                { label: `Recover ${n} Items`, run: () => void changeAll(group, "deleted", false) },
                null,
                { label: `Delete ${n} Items`, danger: true, run: () => void changeAll(group, "expunged", true) },
              ]
            : [
                { label: anyPlain ? "Favorite" : "Unfavorite", run: () => void changeAll(group, "favorite", anyPlain) },
                null,
                { label: "Add to Album", submenu: albumEntries(group.map((x) => x.id)) },
                null,
                view === "hidden" && !album
                  ? { label: `Unhide ${n} Photos`, run: () => void changeAll(group, "hidden", false) }
                  : { label: `Hide ${n} Photos`, run: () => void changeAll(group, "hidden", true) },
                album
                  ? { label: `Remove ${n} Photos from Album`, danger: true, run: () => void removeFrom(group) }
                  : { label: `Delete ${n} Photos`, danger: true, run: () => void changeAll(group, "deleted", true) },
              ],
      });
      return;
    }
    const entries: MenuEntry[] =
      view === "recentlyDeleted" && !album
        ? [
            { label: "Recover", run: () => void change(p, "deleted", false) },
            null,
            { label: "Delete", danger: true, run: () => void change(p, "expunged", true) },
          ]
        : [
            { label: p.favorite ? "Unfavorite" : "Favorite", run: () => void change(p, "favorite", !p.favorite) },
            { label: "Download", run: () => void download(p) },
            null,
            { label: "Add to Album", submenu: albumEntries([p.id]) },
            null,
            view === "hidden" && !album
              ? { label: "Unhide Photo", run: () => void change(p, "hidden", false) }
              : { label: "Hide Photo", run: () => void change(p, "hidden", true) },
            album
              ? { label: "Remove from Album", danger: true, run: () => void removeFrom([p]) }
              : { label: "Delete", danger: true, run: () => void change(p, "deleted", true) },
          ];
    setMenu({ at, entries });
  };

  // The tiles' handlers, one object for the life of the screen; each call reaches the newest.
  const acts = useRef({ pick, openAt, photoMenu });
  acts.current = { pick, openAt, photoMenu };
  const act = useMemo<Acts>(
    () => ({
      pick: (i, e) => acts.current.pick(i, e),
      open: (i) => acts.current.openAt(i),
      menu: (p, e) => acts.current.photoMenu(p, { x: e.clientX, y: e.clientY, alignRight: false }),
    }),
    [],
  );

  // Enter submits and closes the field, and closing it blurs it, which would submit again
  // with the same name still in hand: one album per name typed.
  const namedOnce = useRef(false);
  useEffect(() => {
    if (naming) namedOnce.current = false;
  }, [naming === null]); // eslint-disable-line react-hooks/exhaustive-deps
  const finishNaming = () => {
    if (!naming || namedOnce.current) return;
    namedOnce.current = true;
    const name = naming.draft.trim();
    setNaming(null);
    if (!name) return;
    if (naming.id === null) void createAlbum(name, naming.then);
    else {
      const a = albums.find(([x]) => x.id === naming.id)?.[0];
      if (a) void renameAlbum(a, name);
    }
  };

  const nameField = (
    <input
      className="px-source px-naming"
      aria-label="Album name"
      autoFocus
      value={naming?.draft ?? ""}
      placeholder="Untitled Album"
      onChange={(e) => setNaming((n) => (n ? { ...n, draft: e.target.value } : n))}
      onKeyDown={(e) => {
        if (e.key === "Enter") finishNaming();
        else if (e.key === "Escape") {
          namedOnce.current = true;
          setNaming(null);
        }
        e.stopPropagation();
      }}
      onBlur={finishNaming}
      onFocus={(e) => e.currentTarget.select()}
    />
  );

  const play = async (p: Photo) => {
    try {
      await api.photoPlay(p.id);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const download = async (p: Photo) => {
    try {
      const where = await api.photoDownload(p.id);
      setSaved(`Saved to ${where}`);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  // Uploads report as they go; a finished one never goes back, as events can pass each other.
  // The library itself is redrawn by the sync each landed photo starts.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    listen<UploadEvent>(PHOTO_UPLOAD, (event) => {
      const u = event.payload;
      setUploads((was) => {
        const before = was.get(u.id);
        if (before && (before.state === "done" || before.state === "failed" || before.state === "cancelled")) return was;
        return new Map(was).set(u.id, u);
      });
    })
      .then((unlisten) => {
        const quietly = () => {
          try {
            void Promise.resolve(unlisten() as unknown).catch(() => {});
          } catch {
            // Nothing was listening.
          }
        };
        if (cancelled) quietly();
        else stop = quietly;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);
  const dismissUploads = useCallback(() => setUploads(new Map()), []);

  const add = useCallback(async (paths: string[]) => {
    if (paths.length === 0) return;
    try {
      await api.photosUpload(paths);
    } catch (e) {
      setError(messageOf(e));
    }
  }, []);

  const choose = async () => {
    try {
      const chosen = await openFileDialog({
        multiple: true,
        title: "Add to iCloud Photos",
        filters: [{ name: "Photos and Videos", extensions: KINDS }],
      });
      if (chosen) await add(Array.isArray(chosen) ? chosen : [chosen]);
    } catch (e) {
      setError(messageOf(e));
    }
  };

  // Photos and videos dropped on the window go into the library. Outside the app shell there
  // is no webview to ask; the button still works.
  useEffect(() => {
    let stop: (() => void) | undefined;
    let cancelled = false;
    try {
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === "over") setDropping(true);
          else if (event.payload.type === "leave") setDropping(false);
          else if (event.payload.type === "drop") {
            setDropping(false);
            void add(event.payload.paths);
          }
        })
        .then((unlisten) => {
          if (cancelled) unlisten();
          else stop = unlisten;
        })
        .catch(() => {});
    } catch {
      // No drag and drop here.
    }
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [add]);

  // Under the title, the stretch of time shown — "September – October 2026" — as Photos has it.
  const span = useMemo(() => {
    if (!photos || photos.length === 0) return null;
    const times = photos.map((p) => p.takenAt);
    const [a, b] = [new Date(Math.min(...times) * 1000), new Date(Math.max(...times) * 1000)];
    const month = (d: Date, year: boolean) => d.toLocaleDateString(undefined, year ? { month: "long", year: "numeric" } : { month: "long" });
    if (a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth()) return month(b, true);
    return `${month(a, a.getFullYear() !== b.getFullYear())} – ${month(b, true)}`;
  }, [photos]);
  // At the foot of the library, what it holds: "132 Photos, 2 Videos".
  const tally = useMemo(() => {
    if (!photos || photos.length === 0) return null;
    const videos = photos.filter((p) => p.kind === "video").length;
    const stills = photos.length - videos;
    const part = (n: number, one: string) => `${n.toLocaleString()} ${n === 1 ? one : `${one}s`}`;
    return [stills && part(stills, "Photo"), videos && part(videos, "Video")].filter(Boolean).join(", ");
  }, [photos]);

  // Scrolled, the line under the title names the month at the top of the view, as Photos
  // does; read once a frame at most, and set only when it changes.
  const [here, setHere] = useState<string | null>(null);
  const scroller = useRef<HTMLDivElement | null>(null);
  const looking = useRef(false);
  const onScroll = useCallback(() => {
    if (looking.current) return;
    looking.current = true;
    requestAnimationFrame(() => {
      looking.current = false;
      const el = scroller.current;
      if (!el || album) return setHere(null);
      if (el.scrollTop < 24) return setHere(null);
      const edge = el.getBoundingClientRect().top + 96;
      for (const m of el.querySelectorAll<HTMLElement>(".px-month")) {
        if (m.getBoundingClientRect().bottom > edge) return setHere(m.getAttribute("aria-label"));
      }
    });
  }, [album]);
  useEffect(() => setHere(null), [view, album]);
  useEffect(() => {
    setChosen(new Set());
    anchor.current = null;
  }, [view, album]);

  // Zoom from the keys as in Photos: Ctrl and + or −.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || viewer.current) return;
      if (e.key === "=" || e.key === "+") setZoom((z) => Math.min(ACROSS.length - 1, z + 1));
      else if (e.key === "-") setZoom((z) => Math.max(0, z - 1));
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const title = album?.name ?? VIEWS.find((v) => v.key === view)?.label ?? "Photos";
  const across = ACROSS[zoom];

  const source = (v: (typeof VIEWS)[number]) => (
    <button
      key={v.key}
      className="px-source"
      aria-current={!album && view === v.key}
      onClick={() => {
        setView(v.key);
        setAlbum(null);
        setOpen(null);
      }}
    >
      <v.Icon className="px-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
      <span className="px-label">{v.label}</span>
      {counts[v.key] ? <span className="px-count">{counts[v.key]}</span> : null}
    </button>
  );

  return (
    <div className={`px${dropping ? " is-dropping" : ""}`}>
      <aside className="px-side" aria-label="Photos">
        <p className="px-side-heading">Photos</p>
        {VIEWS.map(source)}
        <div className="px-side-row">
          <p className="px-side-heading">Albums</p>
          <button className="px-side-add" aria-label="New Album" title="New Album" onClick={() => setNaming({ id: null, then: [], draft: "" })}>
            <Plus size={14} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
        <div className="px-albums" role="group" aria-label="Albums">
        {naming?.id === null && nameField}
        {albums.map(([a, n]) =>
          naming?.id === a.id ? (
            <span key={a.id}>{nameField}</span>
          ) : (
            <button
              key={a.id}
              className="px-source"
              aria-current={album?.id === a.id}
              onClick={() => {
                setAlbum(a);
                setOpen(null);
              }}
              onDoubleClick={() => setNaming({ id: a.id, then: [], draft: a.name })}
              onContextMenu={(e) => {
                e.preventDefault();
                if (a.id.startsWith("new:")) return;
                setMenu({
                  at: { x: e.clientX, y: e.clientY, alignRight: false },
                  entries: [
                    { label: "Rename Album", run: () => setNaming({ id: a.id, then: [], draft: a.name }) },
                    null,
                    { label: "Delete Album", danger: true, run: () => void deleteAlbum(a) },
                  ],
                });
              }}
            >
              <AlbumGlyph />
              <span className="px-label">{a.name}</span>
              {n ? <span className="px-count">{n}</span> : null}
            </button>
          ),
        )}
        </div>
        <p className="px-side-heading px-side-gap">Utilities</p>
        {UTILITIES.map(source)}
      </aside>

      <section className="px-main" aria-label={title}>
        {/* As in Photos on macOS 26: the title and the controls float over the library, which
            scrolls beneath them and fades out at the top edge. */}
        <header className="px-bar">
          <div className="px-heading">
            <h1>{title}</h1>
            {selected.length > 0 ? (
              <p className="px-span px-picked">{pickedWords(selected)}</p>
            ) : (
              (here ?? span) && <p className="px-span">{here ?? span}</p>
            )}
          </div>
          {view === "recentlyDeleted" && !album && photos && photos.length > 0 && (
            // Recently Deleted's own toolbar, in Photos' words: Recover and Delete for what is
            // selected, Delete All when nothing is.
            <div className="px-capsule px-words">
              {selected.length > 0 ? (
                <>
                  <button className="px-word" onClick={() => void changeAll(selected, "deleted", false)}>
                    Recover
                  </button>
                  <button className="px-word is-danger" onClick={() => void changeAll(selected, "expunged", true)}>
                    Delete
                  </button>
                </>
              ) : (
                <button className="px-word is-danger" onClick={() => void changeAll(photos, "expunged", true)}>
                  Delete All
                </button>
              )}
            </div>
          )}
          <div className="px-capsule" role="group" aria-label="Zoom">
            <button className="px-cap" aria-label="Zoom Out" title="Zoom Out (Ctrl −)" disabled={zoom === 0} onClick={() => setZoom((z) => Math.max(0, z - 1))}>
              <Minus size={15} strokeWidth={2.2} aria-hidden="true" />
            </button>
            <button className="px-cap" aria-label="Zoom In" title="Zoom In (Ctrl +)" disabled={zoom === ACROSS.length - 1} onClick={() => setZoom((z) => Math.min(ACROSS.length - 1, z + 1))}>
              <Plus size={15} strokeWidth={2.2} aria-hidden="true" />
            </button>
          </div>
          <div className="px-capsule">
            <button className="px-cap px-add" onClick={() => void choose()} title="Add photos and videos" aria-label="Add Photos">
              <ImagePlus size={16} strokeWidth={1.9} aria-hidden="true" />
            </button>
          </div>
        </header>

        {error && (
          <div className="px-notice">
            <Notice message={error} onDismiss={() => setError(null)} />
          </div>
        )}

        {photos && photos.length === 0 && (
          <p className="px-empty">
            {album
              ? "This album is empty. Photos added to it on any device show here."
              : view === "recentlyDeleted"
              ? "Nothing deleted. Photos you delete stay here for 30 days."
              : view === "favorites"
                ? "Mark a photo with the heart and it shows here."
                : "No photos here yet."}
          </p>
        )}

        <div
          className="px-scroll"
          ref={scroller}
          onScroll={onScroll}
          onClick={(e) => {
            // A click on the space between photos lets the selection go, as in Photos.
            if (!(e.target as HTMLElement).closest(".px-tile")) setChosen(new Set());
          }} style={{ "--across": across } as React.CSSProperties}>
          {months.map((m, k) => (
            <Month
              key={`${view}/${album?.id ?? ""}/${m.label}`}
              label={m.label}
              items={m.items}
              chosen={chosen}
              anchor={anchor.current}
              deleted={view === "recentlyDeleted" && !album}
              eager={k < 3}
              across={across}
              act={act}
            />
          ))}
          {tally && <p className="px-tally">{tally}</p>}
        </div>

        {shown && (
          <div className="px-viewer" ref={viewer} role="dialog" aria-label={shown.filename ?? "Photo"}>
            <div className="px-backdrop" aria-hidden="true" />
            <header className="px-viewer-bar">
              <div className="px-capsule">
                <button className="px-cap" aria-label="Close" title="Close (Esc)" onClick={close}>
                  <X size={17} strokeWidth={2} />
                </button>
              </div>
              <div className="px-viewer-when">
                <strong>{new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "long" })}</strong>
                {view === "recentlyDeleted" && !album && timeLeft(shown, false) ? (
                  <span className={timeLeft(shown, false)!.soon ? "is-soon" : undefined}>{timeLeft(shown, false)!.words}</span>
                ) : (
                  <span>{new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}</span>
                )}
              </div>
              <div className="px-capsule">
              {view === "recentlyDeleted" && !album ? (
                <>
                  <button className="px-cap" aria-label="Recover" title="Recover" onClick={() => void change(shown, "deleted", false)}>
                    <RotateCcw size={17} />
                  </button>
                  <button className="px-cap" aria-label="Delete Permanently" title="Delete permanently (Delete)" onClick={() => void change(shown, "expunged", true)}>
                    <Trash2 size={17} />
                  </button>
                </>
              ) : (
                <>
                  <button
                    className="px-cap"
                    aria-label={shown.favorite ? "Unfavorite" : "Favorite"}
                    aria-pressed={shown.favorite}
                    title={shown.favorite ? "Unfavorite (.)" : "Favorite (.)"}
                    onClick={() => void change(shown, "favorite", !shown.favorite)}
                  >
                    <Heart size={17} fill={shown.favorite ? "currentColor" : "none"} />
                  </button>
                  <button
                    className="px-cap"
                    aria-label="Add to Album"
                    aria-haspopup="menu"
                    title="Add to Album"
                    onClick={(e) => {
                      const r = e.currentTarget.getBoundingClientRect();
                      setMenu({ at: { x: r.right, y: r.bottom + 4, alignRight: true }, entries: albumEntries([shown.id]) });
                    }}
                  >
                    <CopyPlus size={17} />
                  </button>
                  {album ? (
                    <button className="px-cap" aria-label="Remove from Album" title="Remove from Album (Delete)" onClick={() => void removeFrom([shown])}>
                      <CopyMinus size={17} />
                    </button>
                  ) : (
                    <button className="px-cap" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                      <Trash2 size={17} />
                    </button>
                  )}
                </>
              )}
              </div>
              <div className="px-capsule">
                <button className="px-cap" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                  <Download size={17} />
                </button>
                <button className="px-cap" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                  <Info size={17} />
                </button>
              </div>
            </header>
            <div className="px-stage" ref={stage}>
              {shown.kind === "video" ? (
                // A still of the video, and Play opens it in this computer's own player: the
                // window's engine plays nothing without codec plugins many systems lack.
                <button className="px-video" aria-label="Play in your video player" onClick={() => void play(shown)}>
                  <img key={shown.id} className="px-shown" src={pictureSrc(shown.id, "medium")} alt={shown.filename ?? ""} decoding="async" style={fitted(shown)} />
                  <span className="px-play" aria-hidden="true">
                    <Play size={28} strokeWidth={0} fill="currentColor" />
                  </span>
                </button>
              ) : (
                <div
                  ref={pan}
                  className={`px-pan${lens.k > 1 ? " is-zoomed" : ""}`}
                  style={{ transform: lens.k > 1 ? `translate(${lens.x}px, ${lens.y}px) scale(${lens.k})` : undefined }}
                  onDoubleClick={(e) => (lens.k > 1 ? unzoom() : zoomAt(2.5, e.clientX, e.clientY))}
                  onPointerDown={(e) => {
                    if (lensRef.current.k <= 1 || e.button !== 0) return;
                    e.currentTarget.setPointerCapture(e.pointerId);
                    dragging.current = { x: e.clientX, y: e.clientY, lx: lensRef.current.x, ly: lensRef.current.y };
                  }}
                  onPointerMove={(e) => {
                    const d = dragging.current;
                    if (!d) return;
                    setLens(bound(lensRef.current.k, d.lx + e.clientX - d.x, d.ly + e.clientY - d.y));
                  }}
                  onPointerUp={() => (dragging.current = null)}
                  onPointerCancel={() => (dragging.current = null)}
                >
                  <img key={shown.id} className="px-shown" src={pictureSrc(shown.id, "medium")} alt={shown.filename ?? ""} decoding="async" draggable={false} style={fitted(shown)} />
                  {sharp === shown.id && (
                    <img
                      key={`${shown.id}-full`}
                      className="px-sharp"
                      src={pictureSrc(shown.id, "full")}
                      alt=""
                      aria-hidden="true"
                      decoding="async"
                      draggable={false}
                      onLoad={(e) => e.currentTarget.classList.add("is-in")}
                    />
                  )}
                </div>
              )}
              {open !== null && open > 0 && (
                <button className="px-nav is-prev" aria-label="Previous" onClick={() => step(-1)}>
                  <ChevronLeft size={22} />
                </button>
              )}
              {open !== null && photos && open < photos.length - 1 && (
                <button className="px-nav is-next" aria-label="Next" onClick={() => step(1)}>
                  <ChevronRight size={22} />
                </button>
              )}
            </div>
            {info && (
              <dl className="px-info">
                <dt>Name</dt>
                <dd>{shown.filename ?? "—"}</dd>
                <dt>Taken</dt>
                <dd>
                  {new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "full" })} at{" "}
                  {new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}
                </dd>
                <dt>Kind</dt>
                <dd>
                  {shown.kind === "video" ? `Video${shown.duration ? `, ${clock(shown.duration)}` : ""}` : shown.kind === "live" ? "Live Photo" : "Photo"}
                </dd>
                <dt>Dimensions</dt>
                <dd>{shown.width.toLocaleString()} × {shown.height.toLocaleString()}</dd>
                {shown.size ? (
                  <>
                    <dt>File size</dt>
                    <dd>{shown.size >= 1024 * 1024 ? `${(shown.size / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(shown.size / 1024))} KB`}</dd>
                  </>
                ) : null}
              </dl>
            )}
          </div>
        )}
      </section>
      {saved && (
        <p className="drive-saved px-saved" role="status" onClick={() => setSaved(null)}>
          {saved}
        </p>
      )}
      {menu && <RowMenu at={menu.at} entries={menu.entries} onClose={() => setMenu(null)} />}
      <UploadTray uploads={[...uploads.values()]} onDismiss={dismissUploads} cancel={api.photosUploadCancel} />
    </div>
  );
}
