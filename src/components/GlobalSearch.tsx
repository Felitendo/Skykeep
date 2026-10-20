/**
 * One search across everything — notes, reminders, events, contacts, files, photos — at the
 * head of the sidebar. Neither icloud.com nor a Mac has it: there each app searches only
 * itself. Offline and at once (read from the library alone), a few of each kind with how many
 * there are, and a choice opens that very thing in its own app.
 *
 * The keys, as Spotlight's: typing searches, Up and Down move through what was found, Return
 * opens it, Escape puts the results away (and a second Escape clears the field). A section with
 * more than it shows has Show All, which shows that one whole, in place.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import {
  api,
  contactPhotoSrc,
  displayName,
  pictureSrc,
  type CalendarEvent,
  type Contact,
  type DriveItem,
  type Note,
  type Photo,
  type Reminder,
} from "../lib/ipc";
import { reveal } from "../lib/reveal";
import { AppIcon } from "./AppIcon";
import { Portal } from "./Portal";

type Everything = {
  notes: Note[];
  reminders: Reminder[];
  events: CalendarEvent[];
  contacts: Contact[];
  files: DriveItem[];
  photos: Photo[];
  counts: [number, number, number, number, number, number];
};

/** How many of a kind a section shows, and the most it shows whole. */
const SHORT = 5;
const WHOLE = 200;

type Row = {
  key: string;
  screen: string;
  id: string;
  within?: string | null;
  at?: number;
  title: string;
  detail: string;
  picture?: string | null;
};

const when = (secs: number, allDay = false) =>
  new Date(secs * 1000).toLocaleString(undefined, allDay
    ? { day: "numeric", month: "short", year: "numeric" }
    : { day: "numeric", month: "short", year: "numeric", hour: "numeric", minute: "2-digit" });

/** What was found, as the rows of each app's section, in the sidebar's own order. */
function sections(found: Everything): { screen: string; name: string; count: number; rows: Row[] }[] {
  const all = [
    {
      screen: "notes",
      name: "Notes",
      count: found.counts[0],
      rows: found.notes.map((n) => ({
        key: `n${n.guid}`, screen: "notes", id: n.guid, title: n.title || "New Note",
        detail: n.locked ? "Locked" : (n.snippet ?? "").replace(/￼/g, "").trim() || "No additional text",
      })),
    },
    {
      screen: "reminders",
      name: "Reminders",
      count: found.counts[1],
      rows: found.reminders.map((r) => ({
        key: `r${r.guid}`, screen: "reminders", id: r.guid, within: r.listGuid, title: r.title,
        detail: r.dueAt ? `Due ${when(r.dueAt)}` : (r.notes ?? "").trim(),
      })),
    },
    {
      screen: "calendar",
      name: "Calendar",
      count: found.counts[2],
      rows: found.events.map((e) => ({
        key: `e${e.guid}`, screen: "calendar", id: e.guid, within: e.calendarGuid, at: e.startsAt, title: e.title,
        detail: [when(e.startsAt, e.allDay), e.location].filter(Boolean).join(" · "),
      })),
    },
    {
      screen: "contacts",
      name: "Contacts",
      count: found.counts[3],
      rows: found.contacts.map((c) => ({
        key: `c${c.guid}`, screen: "contacts", id: c.guid, title: displayName(c),
        detail: c.methods[0]?.value ?? c.company ?? "",
        picture: c.photoUrl ? contactPhotoSrc(c.guid) : null,
      })),
    },
    {
      screen: "drivews",
      name: "iCloud Drive",
      count: found.counts[4],
      rows: found.files.map((f) => ({
        key: `f${f.drivewsid}`, screen: "drivews", id: f.drivewsid, within: f.parentId,
        title: f.extension ? `${f.name}.${f.extension}` : f.name,
        detail: f.modifiedAt ? `Modified ${when(f.modifiedAt, true)}` : "",
      })),
    },
    {
      screen: "photos",
      name: "Photos",
      count: found.counts[5],
      rows: found.photos.map((p) => ({
        key: `p${p.id}`, screen: "photos", id: p.id, title: p.title || p.filename || "Photo",
        detail: when(p.takenAt, true), picture: pictureSrc(p.id, "thumb"),
      })),
    },
  ];
  return all.filter((s) => s.rows.length > 0);
}

export function GlobalSearch({ onOpen, available }: { onOpen: (screen: string) => void; available: string[] }) {
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<Everything | null>(null);
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState(0);
  /** The one section shown whole, if any. */
  const [whole, setWhole] = useState<string | null>(null);
  const field = useRef<HTMLInputElement | null>(null);
  const box = useRef<HTMLDivElement | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  // The panel is drawn over the window (the sidebar clips what leaves it), under the field.
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);

  // Asked a moment after typing stops; the newest question's answer is the one kept.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setFound(null);
      return undefined;
    }
    let current = true;
    const t = window.setTimeout(() => {
      api.searchEverything(q, whole ? WHOLE : undefined).then(
        (f) => {
          if (!current) return;
          setFound(f as Everything);
          if (!whole) setAt(0);
        },
        () => current && setFound(null),
      );
    }, 90);
    return () => {
      current = false;
      window.clearTimeout(t);
    };
  }, [query, whole]);

  // A new search starts with every section short again.
  useEffect(() => setWhole(null), [query]);

  // Away when the window is clicked anywhere else.
  useEffect(() => {
    if (!open) return undefined;
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!box.current?.contains(t) && !panel.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("mousedown", down);
    return () => document.removeEventListener("mousedown", down);
  }, [open]);

  const groups = useMemo(
    () =>
      (found ? sections(found).filter((s) => available.includes(s.screen)) : []).map((s) =>
        s.screen === whole ? s : { ...s, rows: s.rows.slice(0, SHORT) },
      ),
    [found, available, whole],
  );
  const rows = useMemo(() => groups.flatMap((g) => g.rows), [groups]);

  const choose = (r: Row) => {
    setOpen(false);
    onOpen(r.screen);
    reveal({ screen: r.screen, id: r.id, within: r.within, at: r.at, query });
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (open && query) setOpen(false);
      else {
        setQuery("");
        field.current?.blur();
      }
    } else if (e.key === "ArrowDown" && rows.length) {
      e.preventDefault();
      setOpen(true);
      setAt((i) => Math.min(rows.length - 1, i + 1));
    } else if (e.key === "ArrowUp" && rows.length) {
      e.preventDefault();
      setAt((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter" && rows[at]) {
      e.preventDefault();
      choose(rows[at]);
    }
  };

  const showing = open && query.trim() !== "" && found !== null;
  useEffect(() => {
    if (!showing) return undefined;
    const put = () => {
      const r = box.current?.getBoundingClientRect();
      if (r) setPlace({ left: r.left, top: r.bottom + 6 });
    };
    put();
    window.addEventListener("resize", put);
    return () => window.removeEventListener("resize", put);
  }, [showing]);
  return (
    <div className="gsearch" ref={box}>
      <label className="gsearch-field">
        <Search size={14} strokeWidth={2.2} aria-hidden="true" />
        <input
          ref={field}
          type="search"
          placeholder="Search"
          aria-label="Search everything"
          aria-expanded={showing}
          aria-controls="gsearch-results"
          aria-activedescendant={showing && rows[at] ? `gs-${rows[at].key}` : undefined}
          role="combobox"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onFocus={() => query && setOpen(true)}
          onKeyDown={onKey}
        />
        {query && (
          <button className="gsearch-clear" aria-label="Clear search" onClick={() => { setQuery(""); field.current?.focus(); }}>
            <X size={12} strokeWidth={2.4} aria-hidden="true" />
          </button>
        )}
      </label>
      {showing && place && (
        <Portal>
        <div
          ref={panel}
          className="gsearch-results"
          id="gsearch-results"
          role="listbox"
          aria-label="Found"
          style={{ left: place.left, top: place.top }}
          // A click on a result never takes the focus from the field: the keys go on working.
          onMouseDown={(e) => e.preventDefault()}
        >
          {groups.length === 0 && <p className="gsearch-none">Nothing in your iCloud holds “{query.trim()}”.</p>}
          {groups.map((g) => (
            <section key={g.screen} className="gsearch-group" aria-label={g.name}>
              <h3>
                <AppIcon app={g.screen} />
                <span>{g.name}</span>
                {whole === g.screen ? (
                  <button className="gsearch-more" onClick={() => setWhole(null)}>Show Less</button>
                ) : g.count > g.rows.length ? (
                  <button
                    className="gsearch-more"
                    aria-label={`Show all ${g.count} in ${g.name}`}
                    onClick={() => setWhole(g.screen)}
                  >
                    Show All {g.count}
                  </button>
                ) : null}
              </h3>
              {g.rows.map((r) => {
                const i = rows.indexOf(r);
                return (
                  <button
                    key={r.key}
                    id={`gs-${r.key}`}
                    role="option"
                    aria-selected={i === at}
                    className={`gsearch-row${i === at ? " is-at" : ""}`}
                    onMouseMove={() => i !== at && setAt(i)}
                    onClick={() => choose(r)}
                  >
                    {r.picture && <img className="gsearch-pic" src={r.picture} alt="" decoding="async" draggable={false} />}
                    <span className="gsearch-text">
                      <span className="gsearch-title">{r.title}</span>
                      {r.detail && <span className="gsearch-detail">{r.detail}</span>}
                    </span>
                  </button>
                );
              })}
            </section>
          ))}
        </div>
        </Portal>
      )}
    </div>
  );
}
