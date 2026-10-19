/**
 * Photos, as on a Mac: the library edge to edge, newest first, grouped by month, with a zoom
 * for how many across; a photo opens on a dark stage with the arrows to go through them. Everything draws from the local library at once; thumbnails come
 * from the disk cache and are fetched once.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, Download, Heart, Info, Play, RotateCcw, Trash2, X } from "lucide-react";
import { api, messageOf, pictureSrc, type Photo, type PhotoAlbum, type PhotoView } from "../lib/ipc";
import { useSync } from "../lib/sync";
import { Notice } from "../components/Notice";

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

  const title = album?.name ?? VIEWS.find((v) => v.key === view)?.label ?? "Photos";
  const across = ACROSS[zoom];

  return (
    <div className="px">
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
        {albums.length > 0 && (
          <>
            <p className="px-side-heading">Albums</p>
            {albums.map(([a, n]) => (
              <button
                key={a.id}
                className="px-source"
                aria-current={album?.id === a.id}
                onClick={() => {
                  setAlbum(a);
                  setOpen(null);
                }}
              >
                <span>{a.name}</span>
                {n ? <span className="px-count">{n}</span> : null}
              </button>
            ))}
          </>
        )}
      </aside>

      <section className="px-main" aria-label={title}>
        <header className="px-bar">
          <h1>{title}</h1>
          <span className="px-total">
            {photos ? `${photos.length} ${photos.length === 1 ? "item" : "items"}` : ""}
          </span>
          <label className="px-zoom">
            <span className="visually-hidden">Zoom</span>
            <input
              type="range"
              min={0}
              max={ACROSS.length - 1}
              step={1}
              value={zoom}
              aria-label="Zoom"
              onChange={(e) => setZoom(Number(e.target.value))}
            />
          </label>
        </header>

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

        <div className="px-scroll" style={{ "--across": across } as React.CSSProperties}>
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
                  <button className="px-icon" aria-label="Delete" title="Delete (Delete)" onClick={() => void change(shown, "deleted", true)}>
                    <Trash2 size={17} />
                  </button>
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
      {error && <Notice message={error} onDismiss={() => setError(null)} />}
      {saved && (
        <p className="drive-saved px-saved" role="status" onClick={() => setSaved(null)}>
          {saved}
        </p>
      )}
    </div>
  );
}
