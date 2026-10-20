/**
 * Photos, as on a Mac: the library edge to edge, newest first, grouped by month, with a zoom
 * for how many across; a photo opens on a dark stage with the arrows to go through them. Everything draws from the local library at once; thumbnails come
 * from the disk cache and are fetched once.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ChevronLeft, ChevronRight, CopyMinus, CopyPlus, Download, Heart, ImagePlus, Info, Minus, Play, Plus, RotateCcw, Trash2, X } from "lucide-react";
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

const VIEWS: { key: PhotoView; label: string }[] = [
  { key: "library", label: "Library" },
  { key: "favorites", label: "Favorites" },
  { key: "recents", label: "Recents" },
  { key: "videos", label: "Videos" },
  { key: "screenshots", label: "Screenshots" },
  { key: "recentlyDeleted", label: "Recently Deleted" },
];

/** How many across at each step of the zoom, as Photos steps it. */
const ACROSS = [10, 7, 5, 3];

const monthOf = (seconds: number) =>
  new Date(seconds * 1000).toLocaleDateString(undefined, { month: "long", year: "numeric" });

/** What Photos takes, as icloud.com's picker offers it. */
const KINDS = ["jpg", "jpeg", "heic", "heif", "png", "gif", "tif", "tiff", "webp", "dng", "mov", "mp4", "m4v", "avi"];

const clock = (s: number | null) => {
  if (!s) return "";
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.round(s % 60)).padStart(2, "0")}`;
};

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
  const step = useCallback(
    (by: number) => setOpen((at) => (at === null || !photos ? at : Math.max(0, Math.min(photos.length - 1, at + by)))),
    [photos],
  );

  // The viewer's keys: arrows to go through, Escape to close, I for the details.
  useEffect(() => {
    if (open === null) return undefined;
    const current = photos?.[open] ?? null;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight") step(1);
      else if (e.key === "ArrowLeft") step(-1);
      else if (e.key === "Escape") setOpen(null);
      else if (e.key === " " && current?.kind === "video") void play(current);
      else if (e.key.toLowerCase() === "i" && !e.metaKey && !e.ctrlKey) setInfo((v) => !v);
      // As in Photos: "." favorites, Delete sends it to Recently Deleted.
      else if (e.key === "." && current && view !== "recentlyDeleted") void change(current, "favorite", !current.favorite);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && album) void removeFrom([current]);
      else if ((e.key === "Delete" || e.key === "Backspace") && current && view !== "recentlyDeleted") void change(current, "deleted", true);
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
  const change = async (p: Photo, flag: "favorite" | "deleted", on: boolean) => {
    // Asked first, in icloud.com's words: a delete reaches every device.
    if (
      flag === "deleted" && on &&
      !window.confirm("Delete this photo?\n\nIt will be deleted from iCloud Photos on all your devices. It stays in Recently Deleted for 30 days.")
    ) {
      return;
    }
    const before = photos;
    const leaves = flag === "deleted" && (on ? view !== "recentlyDeleted" : view === "recentlyDeleted");
    setPhotos((list) =>
      leaves
        ? (list ?? []).filter((x) => x.id !== p.id)
        : (list ?? []).map((x) => (x.id === p.id ? { ...x, [flag]: on } : x)),
    );
    if (leaves) {
      // The viewer goes on to the next photo, or the one before at the end, or closes.
      setOpen((at) => {
        if (at === null || !photos) return at;
        const left = photos.length - 1;
        return left === 0 ? null : Math.min(at, left - 1);
      });
    }
    try {
      await api.photoSetFlag(p.id, flag, on);
      read().catch(() => {});
    } catch (e) {
      setPhotos(before);
      setError(messageOf(e));
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

  const albumEntries = (ids: string[]): MenuEntry[] => [
    ...albums.filter(([a]) => a.id !== album?.id && !a.id.startsWith("new:")).map(([a]) => ({ label: a.name, run: () => void addTo(a, ids) })),
    ...(albums.length ? [null] : []),
    { label: "New Album…", run: () => setNaming({ id: null, then: ids, draft: "" }) },
  ];

  const photoMenu = (p: Photo, at: { x: number; y: number; alignRight: boolean }) => {
    const entries: MenuEntry[] =
      view === "recentlyDeleted" && !album
        ? [{ label: "Recover", run: () => void change(p, "deleted", false) }]
        : [
            { label: p.favorite ? "Unfavorite" : "Favorite", run: () => void change(p, "favorite", !p.favorite) },
            { label: "Download", run: () => void download(p) },
            null,
            { label: "Add to Album", submenu: albumEntries([p.id]) },
            null,
            album
              ? { label: "Remove from Album", danger: true, run: () => void removeFrom([p]) }
              : { label: "Delete", danger: true, run: () => void change(p, "deleted", true) },
          ];
    setMenu({ at, entries });
  };

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

  // Zoom from the keys as in Photos: Ctrl and + or −.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
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

  return (
    <div className={`px${dropping ? " is-dropping" : ""}`}>
      <aside className="px-side" aria-label="Photos">
        <p className="px-side-heading">Photos</p>
        {VIEWS.map((v) => (
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
            <span>{v.label}</span>
            {counts[v.key] ? <span className="px-count">{counts[v.key]}</span> : null}
          </button>
        ))}
        <div className="px-side-row">
          <p className="px-side-heading">Albums</p>
          <button className="px-side-add" aria-label="New Album" title="New Album" onClick={() => setNaming({ id: null, then: [], draft: "" })}>
            <Plus size={14} strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
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
              <span>{a.name}</span>
              {n ? <span className="px-count">{n}</span> : null}
            </button>
          ),
        )}
      </aside>

      <section className="px-main" aria-label={title}>
        {/* As in Photos on macOS 26: the title and the controls float over the library, which
            scrolls beneath them and fades out at the top edge. */}
        <header className="px-bar">
          <div className="px-heading">
            <h1>{title}</h1>
            {(here ?? span) && <p className="px-span">{here ?? span}</p>}
          </div>
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

        <div className="px-scroll" ref={scroller} onScroll={onScroll} style={{ "--across": across } as React.CSSProperties}>
          {months.map((m) => (
            <section key={m.label} className="px-month" aria-label={m.label}>
              {m.label && <h2 className="px-month-title">{m.label}</h2>}
              <div className="px-grid" role="list">
                {m.items.map(({ photo, index }) => (
                  <button
                    key={photo.id}
                    role="listitem"
                    className="px-tile"
                    aria-label={`${photo.filename ?? "Photo"}, ${new Date(photo.takenAt * 1000).toLocaleString()}`}
                    onClick={() => setOpen(index)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      photoMenu(photo, { x: e.clientX, y: e.clientY, alignRight: false });
                    }}
                  >
                    <img src={pictureSrc(photo.id, "thumb")} alt="" loading="lazy" decoding="async" draggable={false} />
                    {photo.kind === "video" && (
                      <span className="px-badge px-duration">
                        <Play size={10} strokeWidth={0} fill="currentColor" aria-hidden="true" />
                        {clock(photo.duration)}
                      </span>
                    )}
                    {photo.favorite && (
                      <Heart className="px-heart" size={13} strokeWidth={0} fill="currentColor" aria-label="Favorite" />
                    )}
                  </button>
                ))}
              </div>
            </section>
          ))}
          {tally && <p className="px-tally">{tally}</p>}
        </div>

        {shown && (
          <div className="px-viewer" role="dialog" aria-label={shown.filename ?? "Photo"}>
            <header className="px-viewer-bar">
              <button className="px-icon" aria-label="Close" title="Close (Esc)" onClick={() => setOpen(null)}>
                <X size={18} />
              </button>
              <div className="px-viewer-when">
                <strong>{new Date(shown.takenAt * 1000).toLocaleDateString(undefined, { dateStyle: "long" })}</strong>
                <span>{new Date(shown.takenAt * 1000).toLocaleTimeString(undefined, { timeStyle: "short" })}</span>
              </div>
              {view === "recentlyDeleted" && !album ? (
                <button className="px-icon" aria-label="Recover" title="Recover" onClick={() => void change(shown, "deleted", false)}>
                  <RotateCcw size={17} />
                </button>
              ) : (
                <>
                  <button
                    className={`px-icon${shown.favorite ? " is-favorite" : ""}`}
                    aria-label={shown.favorite ? "Unfavorite" : "Favorite"}
                    aria-pressed={shown.favorite}
                    title={shown.favorite ? "Unfavorite (.)" : "Favorite (.)"}
                    onClick={() => void change(shown, "favorite", !shown.favorite)}
                  >
                    <Heart size={17} fill={shown.favorite ? "currentColor" : "none"} />
                  </button>
                  <button
                    className="px-icon"
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
                    <button className="px-icon" aria-label="Remove from Album" title="Remove from Album (Delete)" onClick={() => void removeFrom([shown])}>
                      <CopyMinus size={17} />
                    </button>
                  ) : (
                    <button className="px-icon" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                      <Trash2 size={17} />
                    </button>
                  )}
                </>
              )}
              <button className="px-icon" aria-label="Download" title="Download the original" onClick={() => void download(shown)}>
                <Download size={17} />
              </button>
              <button className="px-icon" aria-label="Info" aria-pressed={info} title="Info (I)" onClick={() => setInfo((v) => !v)}>
                <Info size={17} />
              </button>
            </header>
            <div className="px-stage">
              {shown.kind === "video" ? (
                // A still of the video, and Play opens it in this computer's own player: the
                // window's engine plays nothing without codec plugins many systems lack.
                <button className="px-video" aria-label="Play in your video player" onClick={() => void play(shown)}>
                  <img key={shown.id} src={pictureSrc(shown.id, "medium")} alt={shown.filename ?? ""} decoding="async" />
                  <span className="px-play" aria-hidden="true">
                    <Play size={28} strokeWidth={0} fill="currentColor" />
                  </span>
                </button>
              ) : (
                <img key={shown.id} src={pictureSrc(shown.id, "medium")} alt={shown.filename ?? ""} decoding="async" />
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
                <dd>{new Date(shown.takenAt * 1000).toLocaleString()}</dd>
                <dt>Size</dt>
                <dd>
                  {shown.width} × {shown.height}
                  {shown.size ? ` · ${(shown.size / 1024 / 1024).toFixed(1)} MB` : ""}
                </dd>
                <dt>Kind</dt>
                <dd>{shown.kind === "video" ? "Video" : shown.kind === "live" ? "Live Photo" : "Photo"}</dd>
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
