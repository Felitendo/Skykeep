/**
 * Photos, as on a Mac: the library edge to edge, newest first, grouped by month, with a zoom
 * for how many across; a photo opens on a dark stage with the arrows to go through them. Everything draws from the local library at once; thumbnails come
 * from the disk cache and are fetched once.
 */
import { useActiveRef } from "../lib/screen";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ChevronLeft, ChevronRight, Ellipsis, GalleryThumbnails, CopyMinus, CopyPlus, Download, Clock, Heart, ImagePlus, Images, Info, Minus, Play, Plus, RotateCcw, Scan, Trash2, EyeOff, ArrowDownUp, MonitorPlay, Square, RectangleHorizontal, Star, Layers, Search as SearchIcon, Folder as FolderIcon, Aperture, Video, X, CalendarDays, Maximize2, Minimize2, Users, type LucideIcon } from "lucide-react";
import {
  api,
  messageOf,
  pictureSrc,
  fitSrc,
  PHOTO_UPLOAD,
  type Photo,
  type PhotoAlbum,
  type PhotoView,
  type UploadEvent,
  turnStyle,
  type Turn,
} from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Notice } from "../components/Notice";
import { UploadPanel } from "./photos/UploadPanel";
import { RowMenu, type MenuEntry } from "../components/RowMenu";
import { Portal } from "../components/Portal";
import { Slideshow } from "./photos/Slideshow";
import { useAsk } from "./photos/Ask";
import { VideoPlayer } from "./photos/VideoPlayer";
import { JumpTo, type MonthCover } from "./photos/JumpTo";
import { SharedAlbums } from "./photos/SharedAlbums";
import { tintFrom } from "../lib/tint";
import { fadeIn } from "../lib/fade";
import { DownloadOptions, type Want } from "./photos/DownloadOptions";
import { AdjustDate } from "./photos/AdjustDate";

/** The sidebar, in Photos' order, each with the symbol Photos gives it. */
const VIEWS: { key: PhotoView; label: string; Icon: LucideIcon }[] = [
  { key: "library", label: "Library", Icon: Images },
  { key: "favorites", label: "Favorites", Icon: Heart },
  { key: "recents", label: "Recents", Icon: Clock },
];
/** Photos' Media Types: the library by what kind of picture each is. */
const MEDIA: typeof VIEWS = [
  { key: "videos", label: "Videos", Icon: Video },
  { key: "portrait", label: "Portrait", Icon: Aperture },
  { key: "panoramas", label: "Panoramas", Icon: RectangleHorizontal },
  { key: "screenshots", label: "Screenshots", Icon: Scan },
];
/** Photos' Utilities: what is kept out of the library. */
const UTILITIES: typeof VIEWS = [
  { key: "rated", label: "Rated", Icon: Star },
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

/** A coordinate as a map gives it: 17.7654° N. */
const degrees = (v: number, plus: string, minus: string) => `${Math.abs(v).toFixed(4)}° ${v >= 0 ? plus : minus}`;

/** How a view is sorted, as Photos offers it: by the date taken or the date added, either way. */
type Sort = { by: "taken" | "added"; oldest: boolean };
const SORTS_KEY = "quince.photos.sort";
const defaultSort = (view: PhotoView): Sort => ({ by: view === "recents" ? "added" : "taken", oldest: false });
const sortKey = (p: Photo, by: Sort["by"]) => (by === "added" ? (p.addedAt ?? p.takenAt) : p.takenAt);
const sortList = (list: Photo[], s: Sort) =>
  [...list].sort((a, b) => (sortKey(b, s.by) - sortKey(a, s.by)) * (s.oldest ? -1 : 1) || (a.id < b.id ? -1 : 1));
const readSorts = (): Partial<Record<PhotoView, Sort>> => {
  try {
    return JSON.parse(window.localStorage.getItem(SORTS_KEY) ?? "{}") ?? {};
  } catch {
    return {};
  }
};

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

/** A photo's frame when fitted: its own shape, as large as the square cell lets it be. */
const fitFrame = (p: Photo): React.CSSProperties =>
  p.width >= p.height
    ? { width: "100%", height: `${(100 * p.height) / p.width}%` }
    : { height: "100%", width: `${(100 * p.width) / p.height}%` };

/**
 * The viewer's filmstrip, as icloud.com's: the photos either side as narrow slips, the one shown
 * wider in the middle; a click goes to it. Only the sixty or so nearest are built, and the
 * strip moves at once to keep the one shown in the middle (animated, it flickered on his screen).
 */
const SLIP = 26;
const SHOWN = 54;
const GAP = 2;
const REACH = 30;
function Filmstrip({ photos, at, onPick }: { photos: Photo[]; at: number; onPick: (index: number) => void }) {
  // Enough either side to fill the window's width and more, so a slip is always built (and its
  // picture decoded) before it can be seen: built at the edge of the screen, slips came in
  // blank and filled in as each step was taken.
  const reach = Math.max(REACH, Math.ceil(window.innerWidth / (SLIP + GAP) / 2) + 12);
  const from = Math.max(0, at - reach);
  const to = Math.min(photos.length - 1, at + reach);
  // Each slip has its place by its index in the whole library, so the slips built as the
  // window moves along never jump; the strip is placed to put the shown one's centre in the middle.
  const shift = -(at * (SLIP + GAP) + SHOWN / 2);
  const slips = [];
  for (let i = from; i <= to; i++) {
    const p = photos[i];
    slips.push(
      <button
        key={p.id}
        className={`px-slip${i === at ? " is-shown" : ""}`}
        // Placed by transform, eased with its neighbours: the shown one widens as they make
        // room, all on the compositor — before, each step jumped every slip into place.
        style={{ transform: `translateX(${i * (SLIP + GAP) + (i > at ? SHOWN - SLIP : 0)}px)` }}
        aria-label={p.filename ?? "Photo"}
        aria-current={i === at}
        onClick={() => i !== at && onPick(i)}
      >
        {/* At once, never lazily: a slip coming into the strip showed empty for a moment. */}
        {/* One size for every slip, shown or not, so a step never asks for a new picture; decoded
            before it is drawn (they are small), so none appears blank first. */}
        <img src={fitSrc(p.id, SHOWN * 2 * (window.devicePixelRatio || 1))} alt="" decoding="async" draggable={false} />
      </button>,
    );
  }
  // A wheel or a two-finger swipe over the strip skims through the photos, as in Photos: one
  // photo every 40 pixels of scroll, a fling several.
  const spun = useRef(0);
  const onWheel = (e: React.WheelEvent) => {
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    spun.current += e.deltaMode === 1 ? d * 40 : d;
    const by = Math.trunc(spun.current / 40);
    if (!by) return;
    spun.current -= by * 40;
    const to = Math.max(0, Math.min(photos.length - 1, at + by));
    if (to !== at) onPick(to);
  };
  return (
    <div className="px-strip" role="toolbar" aria-label="Photos" onWheel={onWheel}>
      <div className="px-strip-run" style={{ transform: `translateX(${shift}px)` }}>
        {slips}
      </div>
    </div>
  );
}

/**
 * The pictures the viewer shows — each made the size it is shown at (see fitSrc) — already
 * fetched and decoded, by address. A photo whose picture is in here opens and is stepped to at
 * full quality at once, never a smaller copy first.
 */
const decoded = new Set<string>();
/** The rest on a tile before its picture is fetched (see Tile). */
let resting: number | undefined;
const warming = new Map<string, Promise<boolean>>();
/** The viewer's stage, in CSS pixels, as last measured (a guess from the window before). */
let stageBox = { w: 0, h: 0 };
/** The picture the viewer shows for a photo: the size its stage draws it at. */
function viewSrc(p: Photo): string {
  const box = stageBox.w ? stageBox : { w: Math.max(320, window.innerWidth - 400), h: Math.max(240, window.innerHeight - 140) };
  const r = p.width && p.height ? p.width / p.height : 4 / 3;
  const w = Math.min(box.w, box.h * r, (p.width || 4000) * 1.5);
  return fitSrc(p.id, Math.max(w, w / r) * (window.devicePixelRatio || 1));
}
function warm(p: Photo): Promise<boolean> {
  const url = viewSrc(p);
  if (decoded.has(url)) return Promise.resolve(true);
  let ready = warming.get(url);
  if (!ready) {
    const img = new Image();
    img.decoding = "async";
    img.src = url;
    ready = img
      .decode()
      .then(
        () => {
          decoded.add(url);
          return true;
        },
        () => false,
      )
      .finally(() => warming.delete(url));
    warming.set(url, ready);
  }
  return ready;
}

/** A tile's picture: the size it is drawn at — the whole cell when fitted, cropped to fill it
 *  as a square (so its longer side reaches past the cell). */
function tileSrc(p: Photo, cell: number, fit: boolean): string {
  const r = p.width && p.height ? Math.max(p.width, p.height) / Math.min(p.width, p.height) : 1;
  return fitSrc(p.id, fit ? cell : cell * r);
}

/** What a tile can ask of the screen; one object, kept the same, so tiles need not redraw. */
type Acts = {
  pick: (index: number, e: React.MouseEvent) => void;
  open: (index: number) => void;
  menu: (photo: Photo, e: React.MouseEvent) => void;
  drag: (photo: Photo, e: React.DragEvent) => void;
  dragEnd: () => void;
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
  fit,
  cell,
  act,
}: {
  photo: Photo;
  index: number;
  selected: boolean;
  tabbable: boolean;
  left: string | null;
  soon: boolean;
  fit: boolean;
  /** A cell's width in device pixels. */
  cell: number;
  act: Acts;
}) {
  const born = useRef(performance.now());
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
      // A pointer that rests on a photo starts its whole picture on the way, so opening it is
      // instant: by the double-click it is decoded. Only after a moment's rest — passing over
      // the grid fetches nothing.
      onPointerEnter={() => {
        if (photo.kind === "video") return;
        window.clearTimeout(resting);
        resting = window.setTimeout(() => void warm(photo), 140);
      }}
      onPointerLeave={() => window.clearTimeout(resting)}
      onPointerDown={() => {
        if (photo.kind !== "video") void warm(photo);
      }}
      draggable
      onDragStart={(e) => act.drag(photo, e)}
      onDragEnd={act.dragEnd}
      onContextMenu={(e) => {
        e.preventDefault();
        act.menu(photo, e);
      }}
    >
      {/* The picture's own frame: the whole tile when squares, the photo's own shape when
          fitted — what the badges and the selection sit on. */}
      <span
        className="px-pic"
        // The thumbnail underneath while the picture made for this tile's size comes.
        style={{ ...(fit && photo.width && photo.height ? fitFrame(photo) : {}), backgroundImage: `url("${pictureSrc(photo.id, "thumb")}")` }}
      >
        {/* Fetched as soon as its month is built, well ahead of the screen — not left to the
            engine's own lazy margin, which a fast scroll outran — and made the size it is
            drawn at, so the engine never shrinks it. */}
        <img
          src={tileSrc(photo, cell, fit)}
          alt=""
          decoding="async"
          draggable={false}
          // Already made (as it nearly always is, see photos_prepare): there at once. Still on
          // its way: it fades in over the thumbnail rather than snapping from soft to sharp.
          // Never made: the thumbnail stays.
          onLoad={(e) => {
            e.currentTarget.dataset.in = performance.now() - born.current > 80 ? "fade" : "now";
          }}
        />
        {photo.kind === "video" && (
          <span className="px-badge px-duration">
            <Play size={10} strokeWidth={0} fill="currentColor" aria-hidden="true" />
            {clock(photo.duration)}
          </span>
        )}
        {left && <span className={`px-badge px-left${soon ? " is-soon" : ""}`}>{left}</span>}
        {(photo.rating ?? 0) > 0 && <span className="px-stars" aria-label={`${photo.rating} stars`}>{photo.rating}★</span>}
        {photo.favorite && <Heart className="px-heart" size={13} strokeWidth={0} fill="currentColor" aria-label="Favorite" />}
      </span>
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
  fit,
  cell,
  act,
}: {
  label: string;
  items: { photo: Photo; index: number }[];
  chosen: Set<string>;
  anchor: number | null;
  deleted: boolean;
  eager: boolean;
  fit: boolean;
  cell: number;
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
    // Built well ahead and then kept: a month taken down and built again as it scrolls back
    // showed as a heading over nothing while its tiles came back.
    const watch = new IntersectionObserver(
      ([e]) => {
        if (!e.isIntersecting) return;
        setLive(true);
        watch.disconnect();
      },
      { root: el.closest(".px-scroll"), rootMargin: "4000px 0px" },
    );
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
                fit={fit}
                cell={cell}
                act={act}
              />
            );
          })}
        </div>
      ) : (
        // Not built yet (far from the screen): its tiles as plain boxes on the very grid the photos
        // will use, so a fast scroll shows photos on their way — never an empty page.
        <div className="px-grid px-later" aria-hidden="true">
          {items.map(({ photo }) => (
            <div key={photo.id} className="px-ph" />
          ))}
        </div>
      )}
    </section>
  );
});

export function PhotosView() {
  const { revision } = useSync();
  const [view, setView] = useState<PhotoView>("library");
  /** An album open instead of a view. */
  const [album, setAlbum] = useState<PhotoAlbum | null>(null);
  const [albums, setAlbums] = useState<[PhotoAlbum, number, (string | null)?][]>([]);
  /** The Albums page open: every album as a card with its cover, as icloud.com's. */
  const [albumsPage, setAlbumsPage] = useState(false);
  /** The Shared Albums page open (Sharing, in the sidebar). */
  const [sharedPage, setSharedPage] = useState(false);
  // Which of the sidebar's branches are open — Albums and Media Types — kept on this machine.
  const [branches, setBranches] = useState<{ albums: boolean; media: boolean }>(() => {
    try {
      return { albums: true, media: true, ...JSON.parse(window.localStorage.getItem("quince.photos.branches") ?? "{}") };
    } catch {
      return { albums: true, media: true };
    }
  });
  const branch = (which: "albums" | "media") =>
    setBranches((b) => {
      const next = { ...b, [which]: !b[which] };
      try {
        window.localStorage.setItem("quince.photos.branches", JSON.stringify(next));
      } catch {
        // For this run only.
      }
      return next;
    });
  /** The view's photos as read; `photos`, below, is them in the order chosen to show. */
  const [raw, setPhotos] = useState<Photo[] | null>(null);
  /**
   * A burst opened from its chosen photo, as Photos' "View all": the viewer goes through the
   * burst's photos in the order they were shot, counted "3 of 64", until Done (or the viewer
   * closes) brings the library back where it was. `from` is the chosen photo to come back to.
   */
  const [inBurst, setInBurst] = useState<{ from: string; count: number } | null>(null);
  const [counts, setCounts] = useState<Partial<Record<PhotoView, number>>>({});
  const [error, setError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1);
  // Squares that fill each cell, or each photo whole in its own shape — icloud.com's toggle,
  // kept on this machine.
  const [fit, setFit] = useState(() => {
    try {
      return window.localStorage.getItem("quince.photos.fit") === "1";
    } catch {
      return false;
    }
  });
  const toggleFit = () =>
    setFit((f) => {
      try {
        window.localStorage.setItem("quince.photos.fit", f ? "0" : "1");
      } catch {
        // For this run only.
      }
      return !f;
    });
  const [open, setOpen] = useState<number | null>(null);
  const [info, setInfo] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  /** A slideshow playing: what it shows, and where it begins. */
  const [show, setShow] = useState<{ list: Photo[]; start: number } | null>(null);
  /** Photos selected in the grid, as on a Mac: a click selects, a double-click opens. */
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const anchor = useRef<number | null>(null);
  const [uploads, setUploads] = useState<Map<string, UploadEvent>>(new Map());
  const [dropping, setDropping] = useState(false);
  const [menu, setMenu] = useState<{ at: { x: number; y: number; alignRight: boolean }; entries: MenuEntry[] } | null>(null);
  /** An album being named in the sidebar: a new one (with the photos to put in it), or a rename. */
  const [naming, setNaming] = useState<{ id: string | null; then: string[]; draft: string; folder?: boolean; parent?: string | null } | null>(null);
  /** Folders open in the sidebar, and the folder whose page is showing (none: the top). */
  const [openFolders, setOpenFolders] = useState<Set<string>>(new Set());
  const [folderPage, setFolderPage] = useState<string | null>(null);

  // Each view keeps the sort chosen for it, on this machine. Albums keep their own order, and
  // Recently Deleted its own, as in Photos.
  // The view the photos on screen belong to. A view chosen changes the title, the months and
  // the order only when its photos arrive, all with them — not a frame before, over the last
  // view's photos.
  const [drawn, setDrawn] = useState<{ view: PhotoView; album: PhotoAlbum | null; query: string }>({ view, album, query: "" });
  // What is typed in the search field, and — a moment after the typing stops — what is searched.
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState("");
  useEffect(() => {
    const t = window.setTimeout(() => setSearching(query.trim()), 140);
    return () => window.clearTimeout(t);
  }, [query]);
  const endSearch = () => {
    setQuery("");
    setSearching("");
  };
  const [sorts, setSorts] = useState(readSorts);
  const sortable = !drawn.query && !drawn.album && drawn.view !== "recentlyDeleted" && drawn.view !== "rated";
  const sort = sortable ? (sorts[drawn.view] ?? defaultSort(drawn.view)) : null;
  const chooseSort = (next: Sort) => {
    setSorts((all) => {
      const was = { ...all, [drawn.view]: next };
      try {
        window.localStorage.setItem(SORTS_KEY, JSON.stringify(was));
      } catch {
        // Kept for this run only.
      }
      return was;
    });
  };
  // Sorted where it is drawn, in the same render as the months made from it. Sorted after the
  // render instead, a view's first frame grouped one order by another's dates: months came
  // twice ("December 2025" four times over), and the repeated keys left stale months on screen.
  const photos = useMemo(
    () => (raw && sort && !inBurst ? sortList(raw, sort) : raw),
    [raw, sort?.by, sort?.oldest, inBurst !== null], // eslint-disable-line react-hooks/exhaustive-deps
  );

  // Reads can pass each other: a sync's read of one view can land after another view was
  // chosen. Only the newest read may draw, or a view shows another's photos under its title.
  const reads = useRef(0);
  /** A burst is being gone through: the library's reads wait until it is left (see viewBurst). */
  const burstOpen = useRef(false);
  const read = useCallback(async () => {
    if (burstOpen.current) return;
    const mine = ++reads.current;
    const [list, c, a] = await Promise.all([
      searching ? api.photosSearch(searching) : album ? api.photosInAlbum(album.id) : api.photosList(view),
      api.photosCounts(),
      api.photosAlbums(),
    ]);
    if (mine !== reads.current) return;
    setPhotos(list);
    setDrawn({ view, album, query: searching });
    setCounts(Object.fromEntries(c));
    setAlbums(a);
    // An album deleted elsewhere is not left open, empty.
    if (album && !a.some(([x]) => x.id === album.id)) setAlbum(null);
  }, [view, album, searching]);
  useEffect(() => {
    read().catch((e) => setError(messageOf(e)));
  }, [read, revision]);

  // Grouped by month, as the library reads; an album is one grid in its own order.
  const months = useMemo(() => {
    const out: { label: string; items: { photo: Photo; index: number }[] }[] = [];
    // An album keeps its own order, and Recently Deleted is in the order things were deleted:
    // grouped by the month they were taken, the same month came back again and again. Both are
    // one grid, as icloud.com shows them.
    if (drawn.album || drawn.view === "recentlyDeleted") return [{ label: "", items: (photos ?? []).map((photo, index) => ({ photo, index })) }];
    (photos ?? []).forEach((photo, index) => {
      // Rated is grouped by how many stars, as icloud.com's Rated page is.
      const stars = photo.rating ?? 0;
      const label = drawn.view === "rated" && !drawn.album ? `${stars} ${stars === 1 ? "Star" : "Stars"}` : monthOf(sortKey(photo, sort?.by ?? "taken"));
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push({ photo, index });
      else out.push({ label, items: [{ photo, index }] });
    });
    return out;
  }, [photos, sort?.by, drawn]);

  const shown = open !== null ? photos?.[open] ?? null : null;
  /**
   * The picture last painted on the stage, kept beneath the next one until that one is decoded:
   * a step never shows a blank, nor the soft thumbnail before the sharp picture (the flicker on
   * every switch). The next is decoded where it stands, by the engine, as it would be anyway —
   * holding the step back until a copy off the page was decoded cost a fifth of its frames.
   */
  // The viewer's date, time and place among the photos, as its bar shows them.
  const when = shown && (
    <div className="px-viewer-when">
      <strong>{new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "long" })}</strong>
      {view === "recentlyDeleted" && !album && timeLeft(shown, false) ? (
        <span className={timeLeft(shown, false)!.soon ? "is-soon" : undefined}>{timeLeft(shown, false)!.words}</span>
      ) : (
        <span>
          {new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}
          {/* Where it is among them, as icloud.com says it: "133 of 169". */}
          {photos && photos.length > 1 && open !== null && !inBurst && (
            <span className="px-viewer-place">
              {" · "}
              {(open + 1).toLocaleString()} of {photos.length.toLocaleString()}
            </span>
          )}
        </span>
      )}
    </div>
  );
  const painted = useRef<Photo | null>(null);
  const underEl = useRef<HTMLImageElement | null>(null);
  const under = shown && shown.kind !== "video" && painted.current && painted.current.id !== shown.id && painted.current.kind !== "video" && !decoded.has(viewSrc(shown)) ? painted.current : null;
  if (!shown || shown.kind === "video") painted.current = null;
  // The viewer follows the photo, not its place: when the list moves under it — its date
  // changed, a sync reordered the library — it finds the photo again rather than showing
  // whichever came to stand where it was.
  const openId = useRef<string | null>(null);
  const lastList = useRef(photos);
  useLayoutEffect(() => {
    const moved = lastList.current !== photos;
    lastList.current = photos;
    if (open === null || !photos) {
      openId.current = null;
      return;
    }
    // Only a list that changed sends it looking; a step to another photo is a new photo.
    if (moved && openId.current && photos[open]?.id !== openId.current) {
      const again = photos.findIndex((p) => p.id === openId.current);
      if (again >= 0) {
        setOpen(again);
        return;
      }
    }
    openId.current = photos[open]?.id ?? null;
  }, [open, photos]);

  /*
   * The viewer opens as Photos opens a photo: it grows out of its tile while the dark comes up
   * behind it, and closing sends it back into the tile of the photo now shown. Transform and
   * opacity only, so the compositor does it all; with reduced motion it simply appears.
   */
  const viewer = useRef<HTMLDivElement | null>(null);
  const stage = useRef<HTMLDivElement | null>(null);
  const origin = useRef<DOMRect | null>(null);
  const closing = useRef(false);
  /**
   * The grid behind a photo that has finished opening is not painted (visibility): WebKit
   * painted every tile under the opaque viewer on each step to the next photo, 80 ms a step.
   * Only while the photo rests — never while it opens or closes. Hidden with content-visibility
   * instead, the grid was dropped from the page: an open showed it one frame and then an empty
   * background under the fading backdrop, and a close brought it back over several frames,
   * dropping out again in one (his recording: 26 → 61 → 118 → 33 → 145) — the flicker on every
   * open and close. Visibility keeps its layout and its decoded pictures, so it is back whole in
   * the very frame the close begins.
   */
  const gridBehind = (on: boolean) => {
    const grid = scroller.current;
    if (!grid) return;
    if (on) grid.setAttribute("data-behind", "");
    else grid.removeAttribute("data-behind");
  };
  useEffect(() => {
    if (open === null) gridBehind(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  // A close still running: its animations and its fallback, so a photo opened in the middle
  // of it is not shut by the close finishing, nor left faded out by it.
  const closingRun = useRef<{ anims: Animation[]; timer: number } | null>(null);
  /** Every tile lifted for a photo's flight put back. */
  const land = () => document.querySelectorAll(".px-tile[data-lifted]").forEach((t) => t.removeAttribute("data-lifted"));
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
    // What the close wrote as the elements' own style (see glide) goes with it: the viewer stays,
    // whole, as it was before the close began.
    const el = viewer.current;
    if (el) {
      el.style.opacity = "";
      el.style.visibility = "";
      for (const part of el.querySelectorAll<HTMLElement>(".px-backdrop, .px-viewer-bar, .px-nav, .px-info, .px-strip")) part.style.opacity = "";
      const img = el.querySelector<HTMLElement>(".px-pan, .px-video");
      if (img) img.style.transform = "";
    }
  };
  // The photo's own box, laid out by the engine itself against the stage (a size container):
  // as large as fits, never past 1.5× its own pixels, its shape exact. Known before it loads,
  // so its thumbnail fills that box at once and the sharper picture lands without anything
  // moving; and a window being resized redraws it every frame with no script in the way.
  const fitted = (p: Photo): React.CSSProperties => {
    // The grid's thumbnail, already decoded, painted under the picture: never a black stage.
    const under = { backgroundImage: `url("${pictureSrc(p.id, "thumb")}")` };
    if (!p.width || !p.height) return under;
    // Both sides set, each the smaller of the room across and the room down in the photo's own
    // proportion: WebKit sized a replaced image's height from its width alone, ignoring
    // aspect-ratio, and drew every photo square.
    const r = (p.width / p.height).toFixed(6);
    return {
      ...under,
      width: `min(100cqw, ${r} * 100cqh, ${Math.round(p.width * 1.5)}px)`,
      height: `min(100cqw / ${r}, 100cqh, ${Math.round(p.height * 1.5)}px)`,
      maxWidth: "none",
      maxHeight: "none",
    };
  };
  const still = () => typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // A photo opens and closes at once, with no animation: every way of animating it — the photo
  // growing out of its tile, the backdrop fading, the bars fading, the grid behind it hidden or
  // shown during the move — still showed him a flicker in WebKitGTK on Hyprland. Opened, the
  // grid behind it is left unpainted (gridBehind); it is painted again before the viewer goes.
  useLayoutEffect(() => {
    if (open === null) return;
    origin.current = null;
    const el = viewer.current;
    if (el) el.style.visibility = "";
    gridBehind(true);
  }, [open !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  // The window takes the viewer's near-black while it is open, and the app's colour again
  // after: a resize then never shows a lighter band beside the photo.
  useLayoutEffect(() => {
    if (open === null) return undefined;
    tintFrom(viewer.current?.querySelector(".px-backdrop") ?? null);
    return () => tintFrom(document.body);
  }, [open !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  const close = useCallback(() => {
    gridBehind(false);
    land();
    setOpen(null);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  const step = useCallback(
    (by: number) =>
      setOpen((at) => {
        if (at === null || !photos) return at;
        const to = Math.max(0, Math.min(photos.length - 1, at + by));
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
  // The lens lives outside React: a gesture writes the frame's transform straight to it, once a
  // frame, so a pan or a pinch never redraws the screen (which made it crawl). Only crossing
  // into or out of zoom is state — the cursor and the double-click's meaning change with it.
  const lensRef = useRef({ k: 1, x: 0, y: 0 });
  const [zoomed, setZoomed] = useState(false);
  const pan = useRef<HTMLDivElement | null>(null);
  const frame = useRef<number | null>(null);
  // What is on the screen now, and a glide on its way to the lens.
  const drawn_ = useRef({ k: 1, x: 0, y: 0 });
  const gliding = useRef<number | null>(null);
  /**
   * The lens drawn by the page itself, once a frame: never a CSS transition, never will-change,
   * never a 3D transform. Each of those hands the picture to WebKitGTK's compositor (a layer
   * made as a zoom began and dropped as it ended), and on his screen that flickered — 88
   * frames in one recording of zooming. A double-click's or a key's zoom glides over a moment,
   * stepped from here; a pinch or a drag follows the hand directly.
   */
  const draw = (l: { k: number; x: number; y: number }) => {
    drawn_.current = l;
    const el = pan.current;
    if (el) el.style.transform = l.k > 1 || l.x || l.y ? `translate(${l.x}px, ${l.y}px) scale(${l.k})` : "";
    if (zoomSlider.current && zoomSlider.current !== document.activeElement) zoomSlider.current.value = String(l.k);
  };
  const paint = (next: { k: number; x: number; y: number }, glide = false) => {
    lensRef.current = next;
    setZoomed(next.k > 1);
    if (gliding.current !== null) {
      cancelAnimationFrame(gliding.current);
      gliding.current = null;
    }
    const still = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (glide && !still) {
      const from = drawn_.current;
      const start = performance.now();
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / 220);
        const e = 1 - (1 - t) ** 3;
        draw({ k: from.k + (next.k - from.k) * e, x: from.x + (next.x - from.x) * e, y: from.y + (next.y - from.y) * e });
        gliding.current = t < 1 ? requestAnimationFrame(step) : null;
      };
      gliding.current = requestAnimationFrame(step);
      return;
    }
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      draw(lensRef.current);
    });
  };
  const viewBurst = async (p: Photo) => {
    if (!p.burst) return;
    try {
      const list = await api.photosBurst(p.burst);
      reads.current++; // a sync's read landing now must not put the library back over it
      setPhotos(list);
      setInBurst({ from: p.id, count: list.length });
      // Opened by the photo, not a place: the list is drawn as it is shown.
      setReopen(p.id);
      burstOpen.current = true;
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const leaveBurst = async (keepOpen: boolean) => {
    const back = inBurst?.from;
    setInBurst(null);
    burstOpen.current = false;
    await read().catch(() => {});
    if (keepOpen && back) setReopen(back);
  };
  /** A photo to open again once the library is back (after a burst). */
  const [reopen, setReopen] = useState<string | null>(null);
  useEffect(() => {
    if (!reopen || !photos) return;
    const at = photos.findIndex((x) => x.id === reopen);
    setReopen(null);
    if (at >= 0) setOpen(at);
  }, [reopen, photos]);
  // No picture anywhere in Photos — tile, slip, cover — ever shows the engine's broken box: one
  // listener (errors do not bubble, so it listens on the way down) marks any that fails, and the
  // style draws nothing for it; what is under it shows. The viewer tries the thumbnail first.
  useEffect(() => {
    const failed = (e: Event) => {
      const img = e.target;
      if (img instanceof HTMLImageElement && img.closest(".px, .px-viewer, .px-jump, .px-show") && !img.classList.contains("px-shown")) {
        img.dataset.failed = "";
      }
    };
    document.addEventListener("error", failed, true);
    return () => document.removeEventListener("error", failed, true);
  }, []);
  // However the viewer went (closed, its last photo deleted, another view chosen), no tile is
  // left lifted out after it.
  useEffect(() => {
    if (open !== null) return undefined;
    const id = requestAnimationFrame(land);
    return () => cancelAnimationFrame(id);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  // Closing the viewer inside a burst brings the library back.
  useEffect(() => {
    if (open === null && inBurst) void leaveBurst(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  /**
   * The window full screen for a photo, as icloud.com's viewer offers: the button, and Escape
   * leaving full screen before it closes the photo. Closing the photo leaves it too.
   */
  const [fullscreen, setFullscreen] = useState(false);
  const toggleFullscreen = async (on = !fullscreen) => {
    try {
      setFullscreen(await api.windowFullscreen(on));
    } catch (e) {
      setError(messageOf(e));
    }
  };
  useEffect(() => {
    if (open === null && fullscreen) void toggleFullscreen(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  /**
   * Full screen is the photo alone, as icloud.com's: black to every edge, its bar out of sight
   * until the pointer goes to the top (or a key's focus lands in it), the thumbnails along the
   * bottom only when asked for — remembered for next time.
   */
  const [thumbs, setThumbs] = useState(() => {
    try {
      return localStorage.getItem("quince.photos.fullThumbs") === "1";
    } catch {
      return false;
    }
  });
  const toggleThumbs = () =>
    setThumbs((v) => {
      try {
        localStorage.setItem("quince.photos.fullThumbs", v ? "0" : "1");
      } catch {
        /* a convenience only */
      }
      return !v;
    });
  const [barUp, setBarUp] = useState(true);
  const barTimer = useRef<number | undefined>(undefined);
  const fullBar = useRef<HTMLElement | null>(null);
  // Shown as full screen begins, then away after a moment unless the pointer is on it.
  const barAway = (after = 1600) => {
    window.clearTimeout(barTimer.current);
    barTimer.current = window.setTimeout(() => {
      if (fullBar.current?.matches(":hover") || fullBar.current?.querySelector(":focus-visible")) return;
      setBarUp(false);
    }, after);
  };
  useEffect(() => {
    if (!fullscreen) return window.clearTimeout(barTimer.current);
    setBarUp(true);
    barAway();
    return () => window.clearTimeout(barTimer.current);
  }, [fullscreen]); // eslint-disable-line react-hooks/exhaustive-deps
  // It comes back with a page-drawn fade, never the compositor's (see lib/fade).
  useLayoutEffect(() => {
    if (fullscreen && barUp) fadeIn(fullBar.current, { duration: 120 });
  }, [barUp, fullscreen]);
  const zoomSlider = useRef<HTMLInputElement | null>(null);
  /** Zoom about the stage's middle, as the bar's slider and its − and + do. */
  const zoomMiddle = (k: number, glide = true) => {
    const box = stage.current?.getBoundingClientRect();
    if (box) zoomAt(k, box.left + box.width / 2, box.top + box.height / 2, glide);
  };
  /** A Live Photo's moving part playing over it. */
  const [moving, setMoving] = useState<string | null>(null);
  useEffect(() => setMoving(null), [shown?.id]);
  /** How the film being played is turned, as its file records it (see turnStyle). */
  const [turn, setTurn] = useState<{ key: string; value: Turn } | null>(null);
  const turnFor = shown?.kind === "video" ? `${shown.id}/original` : moving && moving === shown?.id ? `${moving}/video` : null;
  useEffect(() => {
    if (!turnFor || turn?.key === turnFor) return undefined;
    let live = true;
    const [id, which] = turnFor.split("/") as [string, "original" | "video"];
    api.photoVideoTurn(id, which).then(
      (value) => live && setTurn({ key: turnFor, value }),
      // Not known: drawn as served, as it was before.
      () => live && setTurn({ key: turnFor, value: [1, 0, 0, 1] }),
    );
    return () => {
      live = false;
    };
  }, [turnFor]); // eslint-disable-line react-hooks/exhaustive-deps
  /** Videos this computer's engine could not play; each opens in the system's player instead. */
  const [unplayable, setUnplayable] = useState<Set<string>>(() => new Set());
  // The window's player reads only http(s): videos come from the app's own loopback server.
  const [videoBase, setVideoBase] = useState<string | null>(null);
  useEffect(() => {
    api.videoBase().then(setVideoBase, () => setVideoBase(null));
  }, []);
  const unzoom = (glide = true) => paint({ k: 1, x: 0, y: 0 }, glide);
  // Kept so the image's edge never comes in past the middle of the stage.
  const bound = (k: number, x: number, y: number) => {
    const el = pan.current;
    if (!el || k <= 1) return { k: 1, x: 0, y: 0 };
    const mx = ((k - 1) * el.offsetWidth) / 2;
    const my = ((k - 1) * el.offsetHeight) / 2;
    return { k, x: Math.max(-mx, Math.min(mx, x)), y: Math.max(-my, Math.min(my, y)) };
  };
  // Zoom to k, keeping the point under (sx, sy) — screen coordinates — where it is.
  const zoomAt = (k: number, sx: number, sy: number, glide = true) => {
    const el = pan.current;
    if (!el) return;
    // The frame's centre as laid out, before any transform: the transform on screen may be a
    // frame behind the lens while a pinch runs.
    const host = (el.offsetParent as HTMLElement | null)?.getBoundingClientRect();
    if (!host) return;
    const was = lensRef.current;
    const cx = host.left + el.offsetLeft + el.offsetWidth / 2;
    const cy = host.top + el.offsetTop + el.offsetHeight / 2;
    const [dx, dy] = [sx - cx, sy - cy];
    const px = (dx - was.x) / was.k;
    const py = (dy - was.y) / was.k;
    const next = Math.max(1, Math.min(5, k));
    paint(bound(next, dx - next * px, dy - next * py), glide);
  };
  useEffect(() => {
    unzoom(false);
  }, [shown?.id]);
  // The stage, measured as it opens and as the window is resized: the picture the viewer shows
  // is made for its size (viewSrc), and asked for again only when that size crosses a step.
  const [, setStep] = useState("");
  useLayoutEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const measure = () => {
      // Out of sight (another screen in front) it has no box: nothing to measure.
      if (!el.clientWidth) return;
      const cs = getComputedStyle(el);
      stageBox = {
        w: el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight),
        h: el.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom),
      };
      if (shown && shown.kind !== "video") setStep(viewSrc(shown));
      // Where the photo itself sits on the stage, for the LIVE mark laid on its corner: read a
      // frame later, once the picture has its box. Only for a Live Photo, and written only when
      // it moved — read and set on every step, it restyled the whole stage each time and cost
      // the stepping a fifth of its frames.
      if (shown?.kind !== "live") return;
      requestAnimationFrame(() => {
        const p = el.querySelector<HTMLElement>(".px-pan");
        if (!p) return;
        const x = `${p.offsetLeft}px`;
        const y = `${p.offsetTop}px`;
        if (el.style.getPropertyValue("--photo-x") !== x) el.style.setProperty("--photo-x", x);
        if (el.style.getPropertyValue("--photo-y") !== y) el.style.setProperty("--photo-y", y);
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [open !== null, shown?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  // A pinch or Ctrl + wheel: the page must not zoom too, so this listener is not passive.
  useEffect(() => {
    const el = stage.current;
    if (!el) return undefined;
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomAt(lensRef.current.k * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY, false);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });
  const dragging = useRef<{ x: number; y: number; lx: number; ly: number } | null>(null);
  // The photos either side are fetched and decoded while this one shows, so a step lands on
  // a whole picture at once. (No slide between them: moving a picture as it changes is what
  // tore in WebKitGTK.)
  useEffect(() => {
    if (open === null || !photos) return;
    for (const by of [1, -1, 2, -2]) {
      const p = photos[open + by];
      if (p && p.kind !== "video") void warm(p);
    }
  }, [open, photos]);

  // The viewer's keys: arrows to go through, Escape to close, I for the details.
  const live = useActiveRef();
  useEffect(() => {
    if (open === null) return undefined;
    const current = photos?.[open] ?? null;
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      // Keys typed into a field — the caption — are words, not commands.
      if ((e.target as HTMLElement | null)?.closest?.("input:not([type=range]), textarea, [contenteditable]")) return;
      // The zoom slider's own arrows move it; every other key still works with it focused.
      if ((e.target as HTMLElement | null)?.matches?.("input[type=range]") && e.key.startsWith("Arrow")) return;
      if (e.key === "ArrowRight") step(1);
      else if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "Escape" && lensRef.current.k > 1) unzoom();
      else if (e.key === "Escape" && fullscreen) void toggleFullscreen(false);
      else if (e.key === "Escape") close();
      // Cmd/Ctrl + "." closes too, as icloud.com's one-up view does.
      else if ((e.ctrlKey || e.metaKey) && e.key === ".") close();
      // Z: fit, or the photo at full size about its middle.
      else if (e.key.toLowerCase() === "z" && !e.ctrlKey && !e.metaKey && current?.kind !== "video") {
        const box = pan.current?.getBoundingClientRect();
        if (box) zoomAt(lensRef.current.k > 1 ? 1 : 2.5, box.left + box.width / 2, box.top + box.height / 2);
      }
      else if ((e.ctrlKey || e.metaKey) && (e.key === "=" || e.key === "+" || e.key === "-" || e.key === "0") && current?.kind !== "video") {
        const box = pan.current?.getBoundingClientRect();
        if (!box) return;
        const k = e.key === "0" ? 1 : lensRef.current.k * (e.key === "-" ? 1 / 1.5 : 1.5);
        zoomAt(k, box.left + box.width / 2, box.top + box.height / 2);
      }
      else if (e.key === " " && current?.kind === "video") void play(current);
      else if (e.key.toLowerCase() === "i" && !e.metaKey && !e.ctrlKey) setInfo((v) => !v);
      // As in Photos: "." favorites, Delete sends it to Recently Deleted.
      // F as well as ".", and H as well as Ctrl+L, as icloud.com takes them.
      else if ((e.key === "." || (e.key.toLowerCase() === "f" && !e.ctrlKey && !e.metaKey)) && current && view !== "recentlyDeleted") void change(current, "favorite", !current.favorite);
      else if (((e.ctrlKey || e.metaKey) ? e.key.toLowerCase() === "l" : e.key.toLowerCase() === "h") && current && view !== "recentlyDeleted") void change(current, "hidden", view !== "hidden");
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
  const [ask, asking] = useAsk();
  const change = (p: Photo, flag: Flag, on: boolean) => changeAll([p], flag, on);
  const changeAll = async (list: Photo[], flag: Flag, on: boolean) => {
    if (list.length === 0) return;
    const n = list.length;
    // Asked first, in icloud.com's words: a delete reaches every device.
    if (
      flag === "deleted" && on &&
      !(await ask({
        title: n === 1 ? "Delete this photo?" : `Delete ${n} photos?`,
        body: `${n === 1 ? "It" : "They"} will be deleted from iCloud Photos on all your devices. ${n === 1 ? "It stays" : "They stay"} in Recently Deleted for 30 days.`,
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    // For good, in Photos' words: it cannot be undone.
    if (
      flag === "expunged" &&
      !(await ask({
        title: n === 1 ? "Are you sure you want to delete this item permanently?" : `Are you sure you want to delete these ${n} items permanently?`,
        body: "This action cannot be undone.",
        action: "Delete",
        danger: true,
        symbol: "trash",
      }))
    ) {
      return;
    }
    // Hiding asks too, in Photos' words: where the photos go.
    if (
      flag === "hidden" && on &&
      !(await ask({
        title: n === 1 ? "Hide this photo?" : `Hide ${n} photos?`,
        body: `${n === 1 ? "It" : "They"} will be hidden from your library, and can be found in the Hidden album.`,
        action: "Hide",
        symbol: "hide",
      }))
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
      // The whole selection in one call: the library changes at once, Apple hears it batched.
      await api.photosSetFlag(list.map((p) => p.id), flag, on);
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
  const createAlbum = async (name: string, then: string[], folder = false, parent: string | null = null) => {
    const before = albums;
    const temp: PhotoAlbum = { id: `new:${Date.now()}`, name, position: -1, changeTag: null, folder, parent };
    setAlbums((list) => [[temp, then.length], ...list]);
    try {
      const made = await api.photosAlbumCreate(name, parent ?? undefined, folder);
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
    // A folder, in icloud.com's words: it goes with every album in it.
    const sure = await ask({
      title: a.folder ? `Delete the folder “${a.name}”?` : `Delete the album “${a.name}”?`,
      body: a.folder
        ? "This will also delete all albums inside this folder. The photos in them stay in your library."
        : "The photos in it stay in your library.",
      action: "Delete",
      danger: true,
      symbol: "trash",
    });
    if (!sure) return;
    const before = albums;
    // Everything under a folder goes with it, here as on iCloud.
    const gone = new Set([a.id]);
    for (let grew = true; grew; ) {
      grew = false;
      for (const [x] of albums) {
        if (x.parent && gone.has(x.parent) && !gone.has(x.id)) {
          gone.add(x.id);
          grew = true;
        }
      }
    }
    setAlbums((list) => list.filter((x) => !gone.has(x[0].id)));
    if (folderPage && gone.has(folderPage)) setFolderPage(a.parent ?? null);
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
    const sure = await ask({
      title: `Remove ${n === 1 ? "this photo" : `${n} photos`} from the album?`,
      body: `${n === 1 ? "It stays" : "They stay"} in your library.`,
      action: "Remove",
    });
    if (!sure) return;
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

  const openAt = async (index: number) => {
    const p = photos?.[index];
    if (!p) return;
    // Open at full quality: the whole picture decoded first — from disk, a moment — but never
    // waited on for longer than a blink (a click must answer at once); past that it opens on
    // its thumbnail and screen-sized picture, and the whole one follows.
    if (p.kind !== "video") await Promise.race([warm(p), new Promise((r) => window.setTimeout(r, 150))]);
    stopClosing();
    const tile = document.querySelector<HTMLElement>(`.px-tile[data-id="${CSS.escape(p.id)}"]`);
    origin.current = tile?.getBoundingClientRect() ?? null;
    land();
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
      if (!live.current) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input, textarea, [contenteditable], .row-menu, [role=dialog]") || naming) return;
      const list = photos ?? [];
      const ctrl = e.ctrlKey || e.metaKey;
      // Ctrl+A is the photos' wherever the focus rests on this screen — the app list keeps it
      // after Photos is chosen there — never the page's text.
      const selectAll = ctrl && e.key.toLowerCase() === "a";
      if (!selectAll && !t?.closest?.(".px") && t !== document.body) return;
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
      } else if (!ctrl && (e.key === "=" || e.key === "+" || e.key === "-")) {
        // = and − size the thumbnails, as in icloud.com's grid.
        setZoom((z) => (e.key === "-" ? Math.max(0, z - 1) : Math.min(ACROSS.length - 1, z + 1)));
      } else if (!ctrl && e.key.toLowerCase() === "i" && selected.length) {
        // I: the selected photo, opened with its details.
        setInfo(true);
        openAt(list.indexOf(selected[0]));
      } else if ((ctrl ? e.key.toLowerCase() === "l" : e.key.toLowerCase() === "h") && selected.length && view !== "recentlyDeleted") {
        // ⌘L in Photos: hide, or in Hidden, unhide.
        void changeAll(selected, "hidden", view !== "hidden");
      } else if ((e.key === "." || (!ctrl && e.key.toLowerCase() === "f")) && selected.length && view !== "recentlyDeleted") {
        // As Photos: if any is not a favorite, all become favorites; else all stop being.
        void changeAll(selected, "favorite", selected.some((p) => !p.favorite));
      } else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // A slideshow of what is selected when that is more than one photo, else of the whole view
  // from this photo on, as Photos starts one.
  const startShow = (from: Photo) => {
    const list = selected.length > 1 && chosen.has(from.id) ? selected : (photos ?? []);
    setMenu(null);
    setShow({ list, start: Math.max(0, list.findIndex((p) => p.id === from.id)) });
  };

  const albumEntries = (ids: string[]): MenuEntry[] => [
    ...albums.filter(([a]) => !a.folder && a.id !== album?.id && !a.id.startsWith("new:")).map(([a]) => ({ label: a.name, run: () => void addTo(a, ids) })),
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
                { label: `Download ${n} Items`, run: () => void downloadAs(group, "originals") },
                { label: "More Download Options…", run: () => setOptions(group) },
                null,
                { label: "Start Slideshow", run: () => startShow(p) },
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
            { label: "More Download Options…", run: () => setOptions([p]) },
            { label: "Start Slideshow", run: () => startShow(p) },
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
  /*
   * Dragging photos onto an album in the sidebar puts them in it, as on a Mac: what is dragged
   * is the photo, or the whole selection when the photo is part of it.
   */
  const dragging2 = useRef<string[] | null>(null);
  const [dropOn, setDropOn] = useState<string | null>(null);
  const dragFrom = (p: Photo, e: React.DragEvent) => {
    const ids = chosen.has(p.id) ? selected.map((x) => x.id) : [p.id];
    if (!chosen.has(p.id)) {
      setChosen(new Set([p.id]));
      anchor.current = photos?.findIndex((x) => x.id === p.id) ?? null;
    }
    dragging2.current = ids;
    e.dataTransfer.effectAllowed = "copy";
    e.dataTransfer.setData("application/x-quince-photos", ids.join(","));
    // A stack of photos with how many, as Photos drags them.
    const img = e.currentTarget.querySelector("img");
    if (img && typeof e.dataTransfer.setDragImage === "function") {
      const ghost = document.createElement("div");
      ghost.className = "px-ghost";
      const pic = img.cloneNode() as HTMLImageElement;
      ghost.appendChild(pic);
      if (ids.length > 1) {
        const n = document.createElement("span");
        n.textContent = String(ids.length);
        ghost.appendChild(n);
      }
      document.body.appendChild(ghost);
      e.dataTransfer.setDragImage(ghost, 36, 36);
      window.setTimeout(() => ghost.remove(), 0);
    }
  };
  const dropTarget = (a: PhotoAlbum) => (a.folder ? {} : {
    onDragOver: (e: React.DragEvent) => {
      if (!dragging2.current || a.id.startsWith("new:") || album?.id === a.id) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      if (dropOn !== a.id) setDropOn(a.id);
    },
    onDragLeave: (e: React.DragEvent) => {
      // Passing over the row's own symbol or name is not leaving it. Where the engine does not
      // say where the pointer went, the row's own bounds do.
      if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
      const box = e.currentTarget.getBoundingClientRect();
      if (e.clientX > box.left && e.clientX < box.right && e.clientY > box.top && e.clientY < box.bottom) return;
      setDropOn((d) => (d === a.id ? null : d));
    },
    onDrop: (e: React.DragEvent) => {
      const ids = dragging2.current;
      dragging2.current = null;
      setDropOn(null);
      if (!ids || a.id.startsWith("new:")) return;
      e.preventDefault();
      void addTo(a, ids);
    },
  });

  const acts = useRef({ pick, openAt, photoMenu, dragFrom });
  acts.current = { pick, openAt, photoMenu, dragFrom };
  const act = useMemo<Acts>(
    () => ({
      pick: (i, e) => acts.current.pick(i, e),
      open: (i) => acts.current.openAt(i),
      menu: (p, e) => acts.current.photoMenu(p, { x: e.clientX, y: e.clientY, alignRight: false }),
      drag: (p, e) => acts.current.dragFrom(p, e),
      dragEnd: () => {
        dragging2.current = null;
        setDropOn(null);
      },
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
    if (naming.id === null) void createAlbum(name, naming.then, naming.folder ?? false, naming.parent ?? null);
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

  /*
   * Title and caption, as Photos' Info has them: typed in place, Return (in the title) or
   * leaving them saves both at once — moving from one to the other is not leaving — and Escape
   * puts back what was there. Shown at once; put back if iCloud refuses.
   */
  const titleField = useRef<HTMLInputElement | null>(null);
  const captionField = useRef<HTMLTextAreaElement | null>(null);
  const grow = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  };
  const wordKeys = (e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>, was: string | null | undefined) => {
    if (e.key === "Enter" && (e.currentTarget.tagName === "INPUT" || !e.shiftKey)) {
      e.preventDefault();
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      e.currentTarget.value = was ?? "";
      e.currentTarget.blur();
      e.stopPropagation();
    }
  };
  const wordsLeft = (p: Photo, e: React.FocusEvent) => {
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    void words(p, titleField.current?.value ?? p.title ?? "", captionField.current?.value ?? p.caption ?? "");
  };
  const keywords = async (p: Photo, words: string[]) => {
    const before = photos;
    setPhotos((all) => (all ?? []).map((x) => (x.id === p.id ? { ...x, keywords: words } : x)));
    try {
      await api.photoSetKeywords(p.id, words);
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

  const rate = async (p: Photo, rating: number) => {
    const before = photos;
    const leaves = view === "rated" && rating === 0;
    setPhotos((all) =>
      leaves ? (all ?? []).filter((x) => x.id !== p.id) : (all ?? []).map((x) => (x.id === p.id ? { ...x, rating } : x)),
    );
    try {
      await api.photoSetRating(p.id, rating);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

  const words = async (p: Photo, title: string, caption: string) => {
    const [t, c] = [title.trim(), caption.trim()];
    if (t === (p.title ?? "") && c === (p.caption ?? "")) return;
    const before = photos;
    setPhotos((all) => (all ?? []).map((x) => (x.id === p.id ? { ...x, title: t || null, caption: c || null } : x)));
    try {
      await api.photoSetText(p.id, t, c);
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

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

  /** Adjust Date and Time open for this photo. */
  const [adjusting, setAdjusting] = useState<Photo | null>(null);
  const redate = async (p: Photo, takenAt: number) => {
    setAdjusting(null);
    const before = photos;
    setPhotos((all) => (all ?? []).map((x) => (x.id === p.id ? { ...x, takenAt } : x)));
    try {
      await api.photoSetDate(p.id, takenAt);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
    }
  };

  /** Download Options open for these photos. */
  const [options, setOptions] = useState<Photo[] | null>(null);
  const downloadAs = async (list: Photo[], want: Want) => {
    setOptions(null);
    let last = "";
    try {
      for (const p of list) last = await api.photoDownload(p.id, want);
      setSaved(list.length === 1 ? `Saved to ${last}` : `${list.length} saved to Downloads`);
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
      // "planned" says a batch is all listed; it rides on its first file's id, and kept, it
      // took that file's place — the first photo of each batch went missing from the panel.
      if (u.state === "planned") return;
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
      // With an album open, what is added goes into it too, as Photos files an import.
      await api.photosUpload(paths, album?.id);
    } catch (e) {
      setError(messageOf(e));
    }
  }, [album]);

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
          // Photos of the library's own being dragged to an album are not files arriving.
          if (dragging2.current) return;
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
  /** The month picker, open under its button. */
  const [jumping, setJumping] = useState<DOMRect | null>(null);
  // Each month the grid shows, with a photo from it, for the picker: the grid's own months, so
  // the two always agree.
  const monthCovers = useMemo<MonthCover[]>(() => {
    if (drawn.album || drawn.view === "rated") return [];
    return months
      .filter((m) => m.label && m.items.length)
      .map((m) => {
        const cover = m.items[0].photo;
        const d = new Date(sortKey(cover, sort?.by ?? "taken") * 1000);
        return { label: m.label, year: d.getFullYear(), month: d.getMonth(), cover };
      });
  }, [months, drawn, sort?.by]);
  const goToMonth = (label: string) => {
    const el = scroller.current;
    const at = el?.querySelector<HTMLElement>(`.px-month[aria-label="${CSS.escape(label)}"]`);
    // Measured against the scroller itself: offsetTop counts from whatever positioned box
    // happens to hold the month, which is not always the scroller.
    if (el && at) {
      const top = at.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop - 4;
      el.scrollTo({ top, behavior: still() ? "auto" : "smooth" });
    }
  };
  const scroller = useRef<HTMLDivElement | null>(null);
  const looking = useRef(false);
  // Read a few times a second, not every frame, and by halving rather than going down every
  // month: at the foot of three hundred months, measuring each one on every frame was work
  // the scroll itself was waiting for.
  const onScroll = useCallback(() => {
    if (looking.current) return;
    looking.current = true;
    window.setTimeout(() => {
      looking.current = false;
      const el = scroller.current;
      if (!el || album) return setHere(null);
      if (el.scrollTop < 24) return setHere(null);
      const edge = el.getBoundingClientRect().top + 96;
      const months = el.querySelectorAll<HTMLElement>(".px-month");
      let [lo, hi] = [0, months.length - 1];
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (months[mid].getBoundingClientRect().bottom > edge) hi = mid;
        else lo = mid + 1;
      }
      setHere(months[lo]?.getAttribute("aria-label") ?? null);
    }, 120);
  }, [album]);
  useEffect(() => setHere(null), [view, album]);
  // Info closes with the photo: the next photo opens plain, as he expects it to.
  useEffect(() => {
    if (open === null) setInfo(false);
  }, [open === null]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    setChosen(new Set());
    anchor.current = null;
  }, [view, album]);

  // Zoom from the keys as in Photos: Ctrl and + or −.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!live.current) return;
      if (!(e.ctrlKey || e.metaKey) || e.altKey || viewer.current) return;
      if (e.key === "=" || e.key === "+") setZoom((z) => Math.min(ACROSS.length - 1, z + 1));
      else if (e.key === "-") setZoom((z) => Math.max(0, z - 1));
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // A view chosen whose photos have not come yet. Nearly always they come within a frame or
  // two and nothing in between is shown; when the app is busy and they take longer, the new
  // view's title is up at once with its tiles as plain boxes — never the last view lingering
  // under the new choice, which read as the app not having heard the click.
  const switching = drawn.view !== view || (drawn.album?.id ?? null) !== (album?.id ?? null) || drawn.query !== searching;
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    setSlow(false);
    if (!switching) return undefined;
    const t = window.setTimeout(() => setSlow(true), 120);
    return () => window.clearTimeout(t);
  }, [switching, view, album?.id, searching]);
  const waiting = switching && slow;
  const shownAs = waiting ? { view, album, query: searching } : drawn;
  const title = shownAs.query
    ? `“${shownAs.query}”`
    : shownAs.album
    ? (album?.id === shownAs.album.id ? album.name : shownAs.album.name)
    : [...VIEWS, ...MEDIA, ...UTILITIES].find((v) => v.key === shownAs.view)?.label ?? "Photos";
  const waitingFor = waiting ? Math.min(84, album ? (albums.find(([a]) => a.id === album.id)?.[1] ?? 21) : (counts[view] ?? 21)) : 0;
  // A view arrives at its top, not wherever the last one was scrolled to — and settles in with a
  // short fade (drawn frame by frame by the page, never the compositor's, which glitched on his
  // screen), from part-shown so there is never an empty moment between the two.
  const firstDraw = useRef(true);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
    if (firstDraw.current) {
      firstDraw.current = false;
      return;
    }
    fadeIn(scroller.current, { from: 0.35, duration: 160 });
  }, [drawn]);
  const across = ACROSS[zoom];
  // A cell's width in device pixels, for the size each tile's picture is made at; in steps of
  // 32, so a window being resized does not redraw every tile on every frame.
  const [gridWidth, setGridWidth] = useState(0);
  // The header's height, for the room the grid leaves under it: one row in a wide window, two
  // when the controls go under the title.
  const bar = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    const el = bar.current;
    const main = el?.parentElement;
    if (!el || !main) return undefined;
    // Written only when it changes, and a frame later: the room it makes changes the grid's own
    // box, which its own watcher sees — in the same frame, a loop the engine reports.
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (!el.clientWidth) return;
        const h = `${Math.ceil(el.getBoundingClientRect().height)}px`;
        if (main.style.getPropertyValue("--bar") !== h) main.style.setProperty("--bar", h);
      });
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => {
      cancelAnimationFrame(frame);
      watch.disconnect();
    };
  }, []);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return undefined;
    // Never measured while out of sight: with no width, the grid laid itself out empty and lost
    // its place.
    const measure = () => el.clientWidth && setGridWidth(Math.ceil(el.clientWidth / 32) * 32);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);
  const cell = Math.round(((gridWidth || 1200) / across) * (window.devicePixelRatio || 1));
  // The app makes, ahead, every picture the grid and the viewer are about to show at these
  // sizes (photos_prepare): the first look at a photo is as sharp as the hundredth. Told again
  // when they change, once the window has settled.
  useEffect(() => {
    const t = window.setTimeout(() => {
      const box = stageBox.w ? stageBox : { w: Math.max(320, window.innerWidth - 400), h: Math.max(240, window.innerHeight - 140) };
      api.photosPrepare({ cell, fit, stageW: box.w, stageH: box.h, dpr: window.devicePixelRatio || 1 }).catch(() => {});
    }, 400);
    return () => window.clearTimeout(t);
  }, [cell, fit, open !== null]); // eslint-disable-line react-hooks/exhaustive-deps

  // A new album or folder, at the top or in a folder: named in place in the sidebar.
  const startNew = (parent: string | null, folder: boolean) => {
    if (!branches.albums) branch("albums");
    if (parent) setOpenFolders((open) => new Set(open).add(parent));
    setNaming({ id: null, then: [], draft: "", folder, parent });
  };
  const newIn = (parent: string | null, at: { x: number; y: number; alignRight: boolean }) =>
    setMenu({
      at,
      entries: [
        { label: "New Album", run: () => startNew(parent, false) },
        { label: "New Folder", run: () => startNew(parent, true) },
      ],
    });
  const openFolder = (id: string | null) => {
    setAlbumsPage(true);
    setFolderPage(id);
    endSearch();
    setOpen(null);
  };

  /** The albums and folders in one folder (or at the top), folders open to theirs, nested. */
  const albumRows = (parent: string | null, depth: number): React.ReactNode[] => {
    const rows: React.ReactNode[] = [];
    const indent = { "--depth": depth } as React.CSSProperties;
    if (naming?.id === null && (naming.parent ?? null) === parent) {
      rows.push(<span key="new" className="px-depth" style={indent}>{nameField}</span>);
    }
    for (const [a, n, cover] of albums) {
      if ((a.parent ?? null) !== parent) continue;
      if (naming?.id === a.id) {
        rows.push(<span key={a.id} className="px-depth" style={indent}>{nameField}</span>);
        continue;
      }
      if (a.folder) {
        const isOpen = openFolders.has(a.id);
        rows.push(
          <div key={a.id} className="px-parent-row px-depth" style={indent}>
            <button
              className="px-twisty"
              aria-label={isOpen ? `Hide ${a.name}` : `Show ${a.name}`}
              aria-expanded={isOpen}
              onClick={() =>
                setOpenFolders((open) => {
                  const next = new Set(open);
                  if (next.has(a.id)) next.delete(a.id);
                  else next.add(a.id);
                  return next;
                })
              }
            >
              <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
            </button>
            <button
              className="px-source px-parent"
              aria-current={albumsPage && folderPage === a.id}
              onClick={() => openFolder(a.id)}
              onDoubleClick={() => setNaming({ id: a.id, then: [], draft: a.name })}
              onContextMenu={(e) => {
                e.preventDefault();
                if (a.id.startsWith("new:")) return;
                setMenu({
                  at: { x: e.clientX, y: e.clientY, alignRight: false },
                  entries: [
                    { label: "New Album", run: () => startNew(a.id, false) },
                    { label: "New Folder", run: () => startNew(a.id, true) },
                    null,
                    { label: "Rename Folder", run: () => setNaming({ id: a.id, then: [], draft: a.name }) },
                    null,
                    { label: "Delete Folder", danger: true, run: () => void deleteAlbum(a) },
                  ],
                });
              }}
            >
              <FolderIcon size={16} strokeWidth={1.8} className="px-glyph" aria-hidden="true" />
              <span className="px-label">{a.name}</span>
            </button>
          </div>,
        );
        if (isOpen) rows.push(...albumRows(a.id, depth + 1));
        continue;
      }
      rows.push(
        <button
          key={a.id}
          className="px-source px-depth"
          style={indent}
          aria-current={!albumsPage && album?.id === a.id}
          onClick={() => {
            setAlbum(a);
            setAlbumsPage(false);
            setSharedPage(false);
            endSearch();
            setOpen(null);
          }}
          onDoubleClick={() => setNaming({ id: a.id, then: [], draft: a.name })}
          {...dropTarget(a)}
          data-drop={dropOn === a.id || undefined}
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
          {cover ? (
            <img className="px-cover" src={pictureSrc(cover, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
          ) : (
            <AlbumGlyph />
          )}
          <span className="px-label">{a.name}</span>
          {n ? <span className="px-count">{n}</span> : null}
        </button>,
      );
    }
    return rows;
  };

  const source = (v: (typeof VIEWS)[number]) => (
    <button
      key={v.key}
      className="px-source"
      aria-current={!album && !albumsPage && !sharedPage && view === v.key}
      onClick={() => {
        setView(v.key);
        setAlbum(null);
        setAlbumsPage(false);
        setSharedPage(false);
        endSearch();
        setOpen(null);
      }}
    >
      <v.Icon className="px-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
      <span className="px-label">{v.label}</span>
      {counts[v.key] ? <span className="px-count">{counts[v.key]}</span> : null}
    </button>
  );

  return (
    <div className={`px${dropping ? " is-dropping" : ""}`} data-drop-label={album ? `Add to ${album.name}` : "Add to Library"}>
      <aside className="px-side" aria-label="Photos">
        <p className="px-side-heading">Photos</p>
        {VIEWS.map(source)}
        {/* Collections, as icloud.com lists them: Albums and Media Types open to what they
            hold, then Rated, Hidden and Recently Deleted. */}
        <p className="px-side-heading px-side-gap">Collections</p>
        <div className="px-parent-row">
          {/* The chevron opens the list here; the row itself opens the Albums page. */}
          <button className="px-twisty" aria-label={branches.albums ? "Hide Albums" : "Show Albums"} aria-expanded={branches.albums} onClick={() => branch("albums")}>
            <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
          </button>
          <button
            className="px-source px-parent"
            aria-current={albumsPage && folderPage === null}
            onClick={() => {
              setAlbumsPage(true);
              setSharedPage(false);
              setFolderPage(null);
              endSearch();
              setOpen(null);
            }}
          >
            <AlbumGlyph />
            <span className="px-label">Albums</span>
          </button>
          <button className="px-side-add" aria-label="New Album" title="New Album or Folder" aria-haspopup="menu" onClick={(e) => {
            const b = e.currentTarget.getBoundingClientRect();
            newIn(null, { x: b.left, y: b.bottom + 4, alignRight: false });
          }}>
            <Plus size={14} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
        {branches.albums && (
          <div className="px-albums px-children" role="group" aria-label="Albums">
            {albumRows(null, 0)}
          </div>
        )}
        <button className="px-source px-parent" aria-expanded={branches.media} onClick={() => branch("media")}>
          <ChevronRight className="px-chevron" size={12} strokeWidth={2.4} aria-hidden="true" />
          <Layers size={16} strokeWidth={1.8} className="px-glyph" aria-hidden="true" />
          <span className="px-label">Media Types</span>
        </button>
        {branches.media && (
          <div className="px-children" role="group" aria-label="Media Types">
            {/* As icloud.com lists them: only the kinds the library has. */}
            {MEDIA.filter((m) => (counts[m.key] ?? 0) > 0 || (!album && !albumsPage && view === m.key)).map(source)}
          </div>
        )}
        {UTILITIES.map(source)}
        {/* Sharing, as icloud.com lists it: the shared albums this account has made or joined. */}
        <p className="px-side-heading px-side-gap">Sharing</p>
        <button
          className="px-source"
          aria-current={sharedPage}
          onClick={() => {
            setSharedPage(true);
            setAlbumsPage(false);
            endSearch();
            setOpen(null);
          }}
        >
          <Users className="px-glyph" size={16} strokeWidth={1.8} aria-hidden="true" />
          <span className="px-label">Shared Albums</span>
        </button>
      </aside>

      <section className="px-main" aria-label={sharedPage ? "Shared Albums" : albumsPage ? "Albums" : title} data-page={albumsPage || sharedPage ? "albums" : undefined}>
        {sharedPage && <SharedAlbums ask={ask} onError={(m) => setError(m)} />}
        {albumsPage && !sharedPage && (
          <div className="px-albums-page">
            {(() => {
              // This page's level: the top, or a folder's own albums and folders.
              const here = albums.filter(([a]) => (a.parent ?? null) === folderPage);
              const folder = folderPage ? albums.find(([a]) => a.id === folderPage)?.[0] : undefined;
              const held = (id: string) => albums.filter(([a]) => a.parent === id).length;
              return (
                <>
                  <header className="px-albums-head">
                    {folder && (
                      <div className="px-capsule">
                        <button className="px-cap" aria-label="Back" title="Back" onClick={() => openFolder(folder.parent ?? null)}>
                          <ChevronLeft size={17} strokeWidth={2} aria-hidden="true" />
                        </button>
                      </div>
                    )}
                    <div className="px-heading">
                      <h1>{folder ? folder.name : "Albums"}</h1>
                      <p className="px-span">{here.length === 1 ? "1 Item" : `${here.length} Items`}</p>
                    </div>
                    <div className="px-capsule">
                      <button
                        className="px-cap"
                        aria-label="New Album or Folder"
                        title="New Album or Folder"
                        aria-haspopup="menu"
                        onClick={(e) => {
                          const b = e.currentTarget.getBoundingClientRect();
                          newIn(folderPage, { x: b.right, y: b.bottom + 6, alignRight: true });
                        }}
                      >
                        <Plus size={16} strokeWidth={2} aria-hidden="true" />
                      </button>
                    </div>
                  </header>
                  {here.length === 0 ? (
                    <p className="px-empty">
                      {folder ? "This folder is empty. Make an album in it with +." : "No albums yet. Make one with +, then add photos to it from their menu or by dragging them onto it."}
                    </p>
                  ) : (
                    <div className="px-cards" role="list">
                      {here.map(([a, n, cover]) => (
                        <button
                          key={a.id}
                          role="listitem"
                          className={`px-card${a.folder ? " is-folder" : ""}`}
                          onClick={() => {
                            if (a.folder) return openFolder(a.id);
                            setAlbum(a);
                            setAlbumsPage(false);
                            setSharedPage(false);
                          }}
                          {...dropTarget(a)}
                          data-drop={dropOn === a.id || undefined}
                          onContextMenu={(e) => {
                            e.preventDefault();
                            if (a.id.startsWith("new:")) return;
                            const kind = a.folder ? "Folder" : "Album";
                            setMenu({
                              at: { x: e.clientX, y: e.clientY, alignRight: false },
                              entries: [
                                { label: `Rename ${kind}`, run: () => {
                                  if (!branches.albums) branch("albums");
                                  if (a.parent) setOpenFolders((open) => new Set(open).add(a.parent!));
                                  setNaming({ id: a.id, then: [], draft: a.name });
                                } },
                                null,
                                { label: `Delete ${kind}`, danger: true, run: () => void deleteAlbum(a) },
                              ],
                            });
                          }}
                        >
                          <span className="px-card-pic">
                            {a.folder ? (
                              <FolderIcon className="px-glyph" size={44} strokeWidth={1.4} aria-hidden="true" />
                            ) : cover ? (
                              <img src={pictureSrc(cover, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
                            ) : (
                              <AlbumGlyph />
                            )}
                          </span>
                          <span className="px-card-name">{a.name}</span>
                          <span className="px-card-count">
                            {a.folder ? (held(a.id) === 1 ? "1 album" : `${held(a.id)} albums`) : n === 1 ? "1 item" : `${n} items`}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
                </>
              );
            })()}
          </div>
        )}
        {/* As in Photos on macOS 26: the title and the controls float over the library, which
            scrolls beneath them and fades out at the top edge. */}
        <header className="px-bar" ref={bar}>
          <div className="px-heading">
            <h1>{title}</h1>
            {selected.length > 0 ? (
              <p className="px-span px-picked">{pickedWords(selected)}</p>
            ) : (
              drawn.query ? (
                <p className="px-span">{photos?.length === 1 ? "1 Result" : `${(photos?.length ?? 0).toLocaleString()} Results`}</p>
              ) : (
                (here ?? span) && <p className="px-span">{here ?? span}</p>
              )
            )}
          </div>
          <div className="px-tools">
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
          <div className="px-capsule">
            <button
              className="px-cap"
              aria-label={fit ? "Show Square Photos" : "Show Whole Photos"}
              aria-pressed={fit}
              title={fit ? "Square photos" : "Whole photos"}
              onClick={toggleFit}
            >
              {fit ? <Square size={15} strokeWidth={1.9} aria-hidden="true" /> : <RectangleHorizontal size={16} strokeWidth={1.9} aria-hidden="true" />}
            </button>
          </div>
          {sort && (
            <div className="px-capsule">
              <button
                className="px-cap"
                aria-label="Sort"
                aria-haspopup="menu"
                title="Sort"
                onClick={(e) => {
                  const box = e.currentTarget.getBoundingClientRect();
                  setMenu({
                    at: { x: box.right, y: box.bottom + 6, alignRight: true },
                    entries: [
                      { label: "Date Captured", checked: sort.by === "taken", run: () => chooseSort({ ...sort, by: "taken" }) },
                      { label: "Date Added", checked: sort.by === "added", run: () => chooseSort({ ...sort, by: "added" }) },
                      null,
                      { label: "Newest First", checked: !sort.oldest, run: () => chooseSort({ ...sort, oldest: false }) },
                      { label: "Oldest First", checked: sort.oldest, run: () => chooseSort({ ...sort, oldest: true }) },
                    ],
                  });
                }}
              >
                <ArrowDownUp size={16} strokeWidth={1.9} aria-hidden="true" />
              </button>
            </div>
          )}
          {monthCovers.length > 1 && (
            <div className="px-capsule">
              <button
                className="px-cap"
                aria-label="Go to a Month"
                aria-haspopup="dialog"
                aria-expanded={jumping !== null}
                title="Go to a month"
                onClick={(e) => setJumping(jumping ? null : e.currentTarget.getBoundingClientRect())}
              >
                <CalendarDays size={16} strokeWidth={1.9} aria-hidden="true" />
              </button>
            </div>
          )}
          {jumping && <JumpTo months={monthCovers} at={jumping} current={here ?? monthCovers[0]?.label ?? null} onPick={goToMonth} onClose={() => setJumping(null)} />}
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
          {/* Search, as icloud.com's toolbar has it: names, titles, captions, keywords, years and
              months, from the library on this machine — nothing typed leaves it. */}
          <label className="px-capsule px-search">
            <SearchIcon size={15} strokeWidth={2} aria-hidden="true" />
            <input
              type="search"
              aria-label="Search Photos"
              placeholder="Search Photos"
              value={query}
              spellCheck={false}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  endSearch();
                  e.currentTarget.blur();
                }
              }}
            />
            {query && (
              <button className="px-search-clear" aria-label="Clear Search" onClick={endSearch}>
                <X size={11} strokeWidth={3} aria-hidden="true" />
              </button>
            )}
          </label>
          </div>
        </header>

        {error && (
          <div className="px-notice">
            <Notice message={error} onDismiss={() => setError(null)} />
          </div>
        )}

        {photos && photos.length === 0 && (
          <p className="px-empty">
            {drawn.query
              ? `No photos match “${drawn.query}”. Search finds names, titles, captions, keywords, years and months.`
              : album
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
          }} data-fit={fit || undefined} style={{ "--across": across } as React.CSSProperties}>
          {waiting && (
            <div className="px-month" aria-busy="true">
              <h2 className="px-month-title px-month-title-ph" aria-hidden="true">&nbsp;</h2>
              <div className="px-grid px-later" aria-hidden="true">
                {Array.from({ length: waitingFor }, (_, i) => (
                  <div key={i} className="px-ph" />
                ))}
              </div>
            </div>
          )}
          {!waiting && months.map((m, k) => (
            <Month
              key={`${k}/${m.label}`}
              label={m.label}
              items={m.items}
              chosen={chosen}
              anchor={anchor.current}
              deleted={drawn.view === "recentlyDeleted" && !drawn.album}
              eager={k < 3}
              fit={fit}
              cell={cell}
              act={act}
            />
          ))}
          {tally && <p className="px-tally">{tally}</p>}
        </div>

        {shown && (
          <div
            className={`px-viewer${fullscreen ? " is-full" : ""}${fullscreen && thumbs ? " has-thumbs" : ""}`}
            ref={viewer}
            role="dialog"
            aria-label={shown.filename ?? "Photo"}
            onPointerMove={
              fullscreen
                ? (e) => {
                    // The bar comes up as the pointer reaches the top, as icloud.com's does.
                    if (e.clientY < 56) {
                      if (!barUp) setBarUp(true);
                      window.clearTimeout(barTimer.current);
                    } else if (barUp) barAway(700);
                  }
                : undefined
            }
          >
            <div className="px-backdrop" aria-hidden="true" />
            {fullscreen ? (
            <header
              key="full"
              className={`px-full-bar${barUp ? "" : " is-away"}`}
              ref={fullBar}
              // Kept up while a key's focus is in it; a click's focus does not hold it.
              onFocus={(e) => {
                if (!(e.target as HTMLElement).matches(":focus-visible")) return;
                setBarUp(true);
                window.clearTimeout(barTimer.current);
              }}
              onBlur={() => barAway()}
            >
              <div className="px-full-side">
                <button className="px-full-btn" aria-label="Exit Full Screen" title="Exit Full Screen (Esc)" onClick={() => void toggleFullscreen(false)}>
                  <Minimize2 size={16} />
                </button>
                {photos && photos.length > 1 && (
                  <button
                    className="px-full-btn"
                    aria-label={thumbs ? "Hide Thumbnails" : "Show Thumbnails"}
                    aria-pressed={thumbs}
                    title={thumbs ? "Hide Thumbnails" : "Show Thumbnails"}
                    onClick={toggleThumbs}
                  >
                    <GalleryThumbnails size={17} />
                  </button>
                )}
                {shown.kind !== "video" && (
                  <div className="px-zoom">
                    <button className="px-full-btn is-small" aria-label="Zoom Out" title="Zoom Out" onClick={() => zoomMiddle(lensRef.current.k / 1.5)}>
                      <Minus size={13} />
                    </button>
                    <input
                      ref={zoomSlider}
                      type="range"
                      min={1}
                      max={5}
                      step={0.01}
                      defaultValue={lensRef.current.k}
                      aria-label="Zoom"
                      onInput={(e) => zoomMiddle(Number(e.currentTarget.value), false)}
                    />
                    <button className="px-full-btn is-small" aria-label="Zoom In" title="Zoom In" onClick={() => zoomMiddle(lensRef.current.k * 1.5)}>
                      <Plus size={13} />
                    </button>
                  </div>
                )}
              </div>
              {when}
              <div className="px-full-side is-end">
                <button className="px-full-btn" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                  <Info size={17} />
                </button>
                {view === "recentlyDeleted" && !album ? (
                  <>
                    <button className="px-full-btn" aria-label="Recover" title="Recover" onClick={() => void change(shown, "deleted", false)}>
                      <RotateCcw size={17} />
                    </button>
                    <button className="px-full-btn" aria-label="Delete Permanently" title="Delete permanently (Delete)" onClick={() => void change(shown, "expunged", true)}>
                      <Trash2 size={17} />
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      className="px-full-btn"
                      aria-label={shown.favorite ? "Unfavorite" : "Favorite"}
                      aria-pressed={shown.favorite}
                      title={shown.favorite ? "Unfavorite (.)" : "Favorite (.)"}
                      onClick={() => void change(shown, "favorite", !shown.favorite)}
                    >
                      <Heart size={17} fill={shown.favorite ? "currentColor" : "none"} />
                    </button>
                    <button className="px-full-btn" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                      <Download size={17} />
                    </button>
                    {album ? (
                      <button className="px-full-btn" aria-label="Remove from Album" title="Remove from Album (Delete)" onClick={() => void removeFrom([shown])}>
                        <CopyMinus size={17} />
                      </button>
                    ) : (
                      <button className="px-full-btn" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                        <Trash2 size={17} />
                      </button>
                    )}
                  </>
                )}
                <button
                  className="px-full-btn"
                  aria-label="More"
                  aria-haspopup="menu"
                  title="More"
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    photoMenu(shown, { x: r.right, y: r.bottom + 4, alignRight: true });
                  }}
                >
                  <Ellipsis size={17} />
                </button>
              </div>
            </header>
            ) : (
            <header key="bar" className="px-viewer-bar">
              <div className="px-capsule">
                <button className="px-cap" aria-label="Close" title="Close (Esc)" onClick={close}>
                  <X size={17} strokeWidth={2} />
                </button>
                <button
                  className="px-cap"
                  aria-label={fullscreen ? "Exit Full Screen" : "Enter Full Screen"}
                  aria-pressed={fullscreen}
                  title={fullscreen ? "Exit Full Screen" : "Enter Full Screen"}
                  onClick={() => void toggleFullscreen()}
                >
                  {fullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
                </button>
              </div>
              {when}
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
                <button className="px-cap" aria-label="Start Slideshow" title="Start Slideshow" onClick={() => startShow(shown)}>
                  <MonitorPlay size={17} />
                </button>
                <button className="px-cap" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                  <Download size={17} />
                </button>
                <button className="px-cap" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                  <Info size={17} />
                </button>
              </div>
            </header>
            )}
            <div
              className="px-stage"
              ref={stage}
              onContextMenu={(e) => {
                // The photo's own menu on the photo itself, as on a Mac.
                e.preventDefault();
                photoMenu(shown, { x: e.clientX, y: e.clientY, alignRight: false });
              }}
            >
              {shown.kind === "video" && videoBase && !unplayable.has(shown.id) ? (
                // Played here, as Photos plays one: from the disk cache, in the pieces the
                // player asks for, so it starts at once and seeks anywhere.
                <div className="px-video">
                  <VideoPlayer
                    key={shown.id}
                    turn={turn?.key === `${shown.id}/original` ? turn.value : undefined}
                    shape={{ w: shown.width, h: shown.height }}
                    src={`${videoBase}/${shown.id}/original`}
                    poster={viewSrc(shown)}
                    label={shown.filename ?? "Video"}
                    style={fitted(shown)}
                    onError={() => setUnplayable((was) => new Set(was).add(shown.id))}
                  />
                </div>
              ) : shown.kind === "video" ? (
                // This computer's engine could not play it (no codec for it): a still, and Play
                // opens it in the system's own player.
                <button className="px-video" aria-label="Play in your video player" onClick={() => void play(shown)}>
                  <img key={shown.id} className="px-shown" src={pictureSrc(shown.id, "medium")} alt={shown.filename ?? ""} decoding="async" style={fitted(shown)} />
                  <span className="px-play" aria-hidden="true">
                    <Play size={28} strokeWidth={0} fill="currentColor" />
                  </span>
                </button>
              ) : (
                <>
                {under && (
                  // The last picture painted, held until the next one over it is decoded.
                  <img
                    key={`${under.id}-under`}
                    ref={underEl}
                    className="px-under"
                    src={viewSrc(under)}
                    alt=""
                    aria-hidden="true"
                    decoding="async"
                    draggable={false}
                    style={fitted(under)}
                  />
                )}
                <div
                  ref={pan}
                  className={`px-pan${zoomed ? " is-zoomed" : ""}`}
                  onDoubleClick={(e) => (lensRef.current.k > 1 ? unzoom() : zoomAt(2.5, e.clientX, e.clientY))}
                  onPointerDown={(e) => {
                    if (lensRef.current.k <= 1 || e.button !== 0) return;
                    e.currentTarget.setPointerCapture(e.pointerId);
                    dragging.current = { x: e.clientX, y: e.clientY, lx: lensRef.current.x, ly: lensRef.current.y };
                  }}
                  onPointerMove={(e) => {
                    const d = dragging.current;
                    if (!d) return;
                    paint(bound(lensRef.current.k, d.lx + e.clientX - d.x, d.ly + e.clientY - d.y));
                  }}
                  onPointerUp={() => (dragging.current = null)}
                  onPointerCancel={() => (dragging.current = null)}
                >
                  {/* The picture made the size the stage draws it at — every pixel used, none
                      thrown away by the engine — over the last one painted, or on first opening
                      over the grid's thumbnail, until it is decoded. */}
                  <img
                    key={shown.id}
                    className="px-shown"
                    src={viewSrc(shown)}
                    // What was asked for, kept whatever the fallback below does to `src`.
                    data-picture={viewSrc(shown)}
                    alt={shown.filename ?? ""}
                    decoding="async"
                    draggable={false}
                    style={under ? { ...fitted(shown), backgroundImage: "none" } : fitted(shown)}
                    onLoad={(e) => {
                      const img = e.currentTarget;
                      const url = viewSrc(shown);
                      const done = () => {
                        decoded.add(url);
                        painted.current = shown;
                        if (underEl.current) underEl.current.style.visibility = "hidden";
                      };
                      img.decode().then(done, done);
                    }}
                    // Never the engine's broken-picture box (a white frame, a question mark, the
                    // file's name): a picture that will not come falls back to the thumbnail,
                    // and past that shows nothing over the thumbnail already painted under it.
                    onError={(e) => {
                      const img = e.currentTarget;
                      const thumb = pictureSrc(shown.id, "thumb");
                      if (!img.src.includes("thumb")) img.src = thumb;
                      else img.dataset.failed = "";
                    }}
                  />
                  {moving === shown.id && videoBase && (
                    // The moving part, laid exactly over the still, played once and gone —
                    // turned as it was recorded (a selfie's is a quarter turn and a mirror),
                    // and not shown until that turn is known, so it never shows sideways.
                    <video
                      key={`${shown.id}-live`}
                      className="px-live"
                      src={`${videoBase}/${shown.id}/video`}
                      style={
                        turn?.key === `${shown.id}/video`
                          ? turnStyle(turn.value, shown.width, shown.height)
                          : { visibility: "hidden" }
                      }
                      autoPlay
                      playsInline
                      onEnded={() => setMoving(null)}
                      onError={() => setMoving(null)}
                    />
                  )}
                  {/* Zoomed in, the whole picture: only then are its own pixels the ones seen. */}
                  {zoomed && (
                    <img
                      key={`${shown.id}-full`}
                      className="px-sharp"
                      src={pictureSrc(shown.id, "full")}
                      alt=""
                      aria-hidden="true"
                      decoding="async"
                      draggable={false}
                      onLoad={(e) => {
                        e.currentTarget.classList.add("is-in");
                        fadeIn(e.currentTarget, { duration: 160 });
                      }}
                    />
                  )}
                </div>
                </>
              )}
              {/* What kind of photo it is, as Photos marks it at the stage's corner: a Live
                  Photo plays its moving part while the pointer rests on LIVE; a burst's BURST
                  goes through all of it. */}
              {shown.kind === "live" && videoBase && !zoomed && (
                <button
                  className={`px-mark${moving === shown.id ? " is-on" : ""}`}
                  aria-label="Play Live Photo"
                  onPointerEnter={() => setMoving(shown.id)}
                  onFocus={() => setMoving(shown.id)}
                  onClick={() => setMoving(shown.id)}
                >
                  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                    <circle cx="8" cy="8" r="2.2" fill="currentColor" />
                    <circle cx="8" cy="8" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.3" />
                    <circle cx="8" cy="8" r="7" fill="none" stroke="currentColor" strokeWidth="1.1" strokeDasharray="1.2 1.6" />
                  </svg>
                  LIVE
                </button>
              )}
              {shown.burst && (shown.burstCount ?? 0) > 1 && !inBurst && !zoomed && (
                <button className="px-mark" aria-label={`View all ${shown.burstCount} photos in this burst`} onClick={() => void viewBurst(shown)}>
                  <Layers size={13} aria-hidden="true" />
                  BURST
                  <span className="px-mark-count">{shown.burstCount}</span>
                </button>
              )}
              {inBurst && (
                <div className="px-mark is-burst" role="status">
                  <Layers size={13} aria-hidden="true" />
                  Burst · {(open ?? 0) + 1} of {inBurst.count}
                  <button className="px-mark-done" onClick={() => void leaveBurst(true)}>
                    Done
                  </button>
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
            {open !== null && photos && photos.length > 1 && (!fullscreen || thumbs) && (
              <Filmstrip photos={photos} at={open} onPick={(i) => step(i - open)} />
            )}
            {info && (
              <dl className="px-info">
                {/* As in Photos' Info: the caption first, typed in place, and kept on every
                    device; Return or leaving the field saves it, Escape puts it back. */}
                <dd className="px-caption-row" onBlur={(e) => wordsLeft(shown, e)}>
                  <input
                    key={`t${shown.id}`}
                    ref={titleField}
                    className="px-title-field"
                    aria-label="Title"
                    placeholder="Add a Title"
                    defaultValue={shown.title ?? ""}
                    maxLength={255}
                    readOnly={view === "recentlyDeleted"}
                    onKeyDown={(e) => wordKeys(e, shown.title)}
                  />
                  <textarea
                    key={`c${shown.id}`}
                    ref={captionField}
                    className="px-caption"
                    aria-label="Caption"
                    placeholder="Add a Caption"
                    defaultValue={shown.caption ?? ""}
                    maxLength={2000}
                    rows={1}
                    readOnly={view === "recentlyDeleted"}
                    onInput={(e) => grow(e.currentTarget)}
                    onFocus={(e) => grow(e.currentTarget)}
                    onKeyDown={(e) => wordKeys(e, shown.caption)}
                  />
                </dd>
                {/* Stars, as Info sets them: a click gives that many; the same again takes them away. */}
                <dt>Rating</dt>
                <dd className="px-rating" role="radiogroup" aria-label="Rating">
                  {[1, 2, 3, 4, 5].map((n) => (
                    <button
                      key={n}
                      role="radio"
                      aria-checked={(shown.rating ?? 0) === n}
                      aria-label={`${n} ${n === 1 ? "star" : "stars"}`}
                      className={`px-star${(shown.rating ?? 0) >= n ? " is-on" : ""}`}
                      disabled={view === "recentlyDeleted"}
                      onClick={() => void rate(shown, (shown.rating ?? 0) === n ? 0 : n)}
                    >
                      <Star size={15} fill={(shown.rating ?? 0) >= n ? "currentColor" : "none"} strokeWidth={1.8} />
                    </button>
                  ))}
                </dd>
                <dt>Name</dt>
                <dd>{shown.filename ?? "—"}</dd>
                <dt>Taken</dt>
                <dd className="px-taken">
                  {view !== "recentlyDeleted" && (
                    <button className="px-link px-adjust" onClick={() => setAdjusting(shown)}>
                      Adjust
                    </button>
                  )}
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
                {shown.latitude != null && shown.longitude != null && (
                  <>
                    <dt>Place</dt>
                    <dd className="px-place">
                      <span>{degrees(shown.latitude, "N", "S")}, {degrees(shown.longitude, "E", "W")}</span>
                      <button className="px-link" onClick={() => void api.photoOpenMap(shown.id).catch((e) => setError(messageOf(e)))}>
                        Open in Map
                      </button>
                    </dd>
                  </>
                )}
                {/* Keywords, as Info keeps them: each its own chip, taken away with ×; a word
                    typed and Return adds one. */}
                <dt>Keywords</dt>
                <dd className="px-words-field">
                  {(shown.keywords ?? []).map((k) => (
                    <span key={k} className="px-chip">
                      {k}
                      {view !== "recentlyDeleted" && (
                        <button
                          aria-label={`Remove ${k}`}
                          onClick={() => void keywords(shown, (shown.keywords ?? []).filter((x) => x !== k))}
                        >
                          <X size={10} strokeWidth={3} aria-hidden="true" />
                        </button>
                      )}
                    </span>
                  ))}
                  {view !== "recentlyDeleted" && (
                    <input
                      key={`k${shown.id}`}
                      className="px-keyword-field"
                      aria-label="Add a Keyword"
                      placeholder="Add a Keyword"
                      maxLength={100}
                      onKeyDown={(e) => {
                        const word = e.currentTarget.value.trim();
                        if ((e.key === "Enter" || e.key === ",") && word) {
                          e.preventDefault();
                          e.currentTarget.value = "";
                          if (!(shown.keywords ?? []).some((x) => x.toLowerCase() === word.toLowerCase())) {
                            void keywords(shown, [...(shown.keywords ?? []), word]);
                          }
                        } else if (e.key === "Escape") {
                          e.currentTarget.value = "";
                          e.currentTarget.blur();
                          e.stopPropagation();
                        }
                      }}
                    />
                  )}
                </dd>
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
      {asking}
      {/* Drawn at the top of the page: inside the screen, whose content moves by a transform,
          "fixed" was fixed to the screen and a slideshow played inside a box, not the window. */}
      {show && (
        <Portal>
          <Slideshow photos={show.list} start={show.start} onClose={() => setShow(null)} />
        </Portal>
      )}
      {adjusting && (
        <Portal>
          <AdjustDate photo={adjusting} onSave={(t) => void redate(adjusting, t)} onClose={() => setAdjusting(null)} />
        </Portal>
      )}
      {options && (
        <Portal>
          <DownloadOptions photos={options} onPick={(w) => void downloadAs(options, w)} onClose={() => setOptions(null)} />
        </Portal>
      )}
      <UploadPanel uploads={[...uploads.values()]} onDismiss={dismissUploads} cancel={api.photosUploadCancel} />
    </div>
  );
}
