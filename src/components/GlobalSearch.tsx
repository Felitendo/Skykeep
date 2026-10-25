/**
 * One search across everything — notes, reminders, events, contacts, files, photos — at the
 * head of the sidebar. Neither icloud.com nor a Mac has it: there each app searches only
 * itself. Offline and at once (read from the library's own search index, search.rs), the best
 * few of each app with how many there are, and a choice opens that very thing in its own app.
 *
 * Every key asks at once — the index answers in a millisecond or two — and each question is
 * numbered: an answer overtaken by a newer question is dropped (the library does not even run
 * one still waiting), and what was found stays on screen until the next answer replaces it,
 * never flashing empty in between. Show All shows one app whole, read a page at a time as it is
 * scrolled and drawn only where it is in view, so a thousand results scroll like ten.
 *
 * The keys, as Spotlight's: Ctrl+K (⌘K) comes here from anywhere; typing searches; Up and Down
 * move through what was found and Return opens it; Escape steps back — from Show All to every
 * app, then puts the results away, then clears the field.
 *
 * For assistive tech it is a combobox over one listbox: every row, and each app's Show All (or
 * Show Less), is an option the arrows reach — a button Tab could never get to, drawn out in a
 * layer of its own, had left Show All to the mouse. The field keeps the focus throughout and
 * names the selected option; a polite status says how many were found.
 *
 * Folded to icons, the sidebar keeps a magnifier: it, or Ctrl+K, brings the field out over the
 * window beside the rail. In a narrow window Ctrl+K brings the sidebar out first.
 */
import { Search } from "lucide-react";
import { SearchField } from "./SearchField";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, contactPhotoSrc, pictureSrc, type SearchGroup, type SearchHit } from "../lib/ipc";
import { reveal } from "../lib/reveal";
import { AppIcon } from "./AppIcon";
import { Portal } from "./Portal";
import { modalOpen } from "../lib/screen";

/** The height of a result: fixed, so a long list is drawn only where it is in view. */
const ROW = 44;
/** Rows drawn past each edge of the view, so a fast scroll never shows a gap. */
const OVERSCAN = 16;
/** The rows drawn change only as the scroll crosses a band this many rows deep: most frames of
 * a scroll draw nothing new and are the browser's alone. */
const BAND = 8;
/** The most rows the panel shows at once (560 px high). */
const VIEW = 13;
/** How many Show All reads at a time. */
const PAGE = 200;

const NAMES: Record<string, string> = {
  notes: "Notes", reminders: "Reminders", calendar: "Calendar", contacts: "Contacts", drivews: "Drive", photos: "Photos",
};

const thisYear = new Date().getFullYear();
const day = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
const dayYear = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const time = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
/** A date as a result's trailing line reads it: the year only when it is not this one. */
function dated(secs: number, withTime: boolean): string {
  const d = new Date(secs * 1000);
  const date = (d.getFullYear() === thisYear ? day : dayYear).format(d);
  return withTime ? `${date}, ${time.format(d)}` : date;
}

/** `text` with where the typed words fall in bold. */
function Marked({ text, marks }: { text: string; marks: [number, number][] }) {
  if (!marks.length) return <>{text}</>;
  const out: React.ReactNode[] = [];
  let from = 0;
  marks.forEach(([a, b], i) => {
    if (a < from) return;
    if (a > from) out.push(text.slice(from, a));
    out.push(<mark key={i}>{text.slice(a, b)}</mark>);
    from = b;
  });
  if (from < text.length) out.push(text.slice(from));
  return <>{out}</>;
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter((w) => /\p{L}/u.test(w[0] ?? ""));
  return ((parts[0]?.[0] ?? "") + (parts.length > 1 ? parts[parts.length - 1][0] : "")).toUpperCase();
}

type RowProps = {
  app: string;
  hit: SearchHit;
  index: number;
  selected: boolean;
  onPick: (index: number) => void;
  onPoint: (index: number) => void;
};

/** One result. Drawn again only when it, or whether it is selected, changes. */
const Row = memo(function Row({ app, hit, index, selected, onPick, onPoint }: RowProps) {
  const when =
    hit.when == null
      ? null
      : app === "reminders" || app === "calendar"
        ? dated(hit.when, !hit.allDay)
        : dated(hit.when, false);
  const detail =
    app === "notes"
      ? hit.locked ? "Locked" : hit.detail ?? "No additional text"
      : hit.detail;
  return (
    <button
      id={`gs-${index}`}
      role="option"
      tabIndex={-1}
      aria-selected={selected}
      className={`gsearch-row${selected ? " is-at" : ""}`}
      onMouseMove={selected ? undefined : () => onPoint(index)}
      onClick={() => onPick(index)}
    >
      {app === "photos" ? (
        <img className="gsearch-pic" src={pictureSrc(hit.id, "thumb")} alt="" decoding="async" loading="lazy" draggable={false} />
      ) : app === "contacts" ? (
        hit.picture ? (
          <img className="gsearch-pic is-round" src={contactPhotoSrc(hit.id)} alt="" decoding="async" loading="lazy" draggable={false} />
        ) : (
          <span className="gsearch-pic is-round gsearch-initials" aria-hidden="true">{initials(hit.title)}</span>
        )
      ) : null}
      <span className="gsearch-text">
        <span className="gsearch-title"><Marked text={hit.title} marks={hit.titleMarks} /></span>
        {detail && (
          <span className="gsearch-detail">
            <Marked text={detail} marks={detail === hit.detail ? hit.detailMarks : []} />
          </span>
        )}
      </span>
      {when && <span className="gsearch-when">{app === "reminders" ? `Due ${when}` : when}</span>}
    </button>
  );
});

type Whole = { app: string; q: string; count: number; hits: SearchHit[]; reading: boolean };

/** What the arrows move through: the results, and each app's Show All or Show Less. */
type Item = { kind: "hit"; app: string; hit: SearchHit } | { kind: "more"; group: SearchGroup } | { kind: "less" };

/** The selection starts on the first result, never on a Show All above it. */
const FIRST = -1;

export function GlobalSearch({
  onOpen,
  available,
  folded = false,
  onSummon,
}: {
  onOpen: (screen: string) => void;
  available: string[];
  /** The sidebar is folded to icons: a magnifier stands in for the field until it is wanted. */
  folded?: boolean;
  /** Ctrl+K was pressed: whatever hides the field (a narrow window's sidebar) brings it out. */
  onSummon?: () => void;
}) {
  const [query, setQuery] = useState("");
  /** The newest answer drawn, and the question it answered. */
  const [found, setFound] = useState<{ q: string; groups: SearchGroup[] } | null>(null);
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState(FIRST);
  /** Folded: the field is out over the window, beside the rail. */
  const [out, setOut] = useState(false);
  const magnifier = useRef<HTMLButtonElement | null>(null);
  /** The one app shown whole, if any, and as much of it as has been read. */
  const [whole, setWhole] = useState<Whole | null>(null);
  /** Which band of rows the Show All list is scrolled to. */
  const [band, setBand] = useState(0);
  const field = useRef<HTMLInputElement | null>(null);
  const box = useRef<HTMLDivElement | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  // The panel is drawn over the window (the sidebar clips what leaves it), under the field.
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);
  /** The field's own capsule: in the sidebar, or out over the window beside the rail. */
  const wrap = useRef<HTMLDivElement | null>(null);
  const [outAt, setOutAt] = useState<{ left: number; top: number } | null>(null);
  /** Questions are numbered; the newest answer drawn is the only one that may be replaced. */
  const asked = useRef(0);
  const drawn = useRef(0);

  // Every key asks at once; an older answer that lands late is dropped.
  useEffect(() => {
    const q = query.trim();
    setWhole(null);
    if (!q) {
      asked.current += 1;
      drawn.current = asked.current;
      setFound(null);
      return;
    }
    const seq = ++asked.current;
    api.searchEverything(q, { seq }).then(
      (f) => {
        if (!f || seq < drawn.current) return;
        drawn.current = seq;
        setFound({ q, groups: f.groups });
        setAt(FIRST);
      },
      () => {
        // A search that failed leaves what was found; the next key asks again.
      },
    );
  }, [query]);

  // Ctrl+K (⌘K) from anywhere in the window comes to the field, what is in it selected.
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (modalOpen()) return;
        summon.current();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, []);
  /** To the field, what is in it selected — out of the rail, or out of a narrow window's
   * hidden sidebar, first. The field is focused once it has been drawn where it can be. */
  const summon = useRef(() => {});
  summon.current = () => {
    onSummon?.();
    if (folded) setOut(true);
    setOpen(true);
    const to = () => {
      field.current?.focus();
      field.current?.select();
    };
    // At once where the field is drawn; once it has been, where it was folded away or hidden.
    to();
    if (document.activeElement !== field.current) requestAnimationFrame(() => document.activeElement !== field.current && to());
  };
  // Unfolded again, the field is back in the sidebar.
  useEffect(() => {
    if (!folded) setOut(false);
  }, [folded]);
  /** Put away out of the rail: the focus goes back to the magnifier when it was the keys. */
  const putAway = (refocus: boolean) => {
    setOut(false);
    setOpen(false);
    if (refocus) requestAnimationFrame(() => magnifier.current?.focus());
  };

  // Away when the window is clicked anywhere else.
  useEffect(() => {
    if (!open) return undefined;
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!box.current?.contains(t) && !panel.current?.contains(t) && !wrap.current?.contains(t)) {
        setOpen(false);
        setOut(false);
      }
    };
    document.addEventListener("mousedown", down);
    return () => document.removeEventListener("mousedown", down);
  }, [open]);

  const groups = useMemo(
    () => (found ? found.groups.filter((g) => available.includes(g.app)) : []),
    [found, available],
  );
  /** Every option the keys move through, in the order drawn: each app's Show All above its
   * rows, as its heading is; Show Less above the whole app. */
  const rows = useMemo<Item[]>(
    () =>
      whole
        ? [{ kind: "less" } as Item, ...whole.hits.map((hit): Item => ({ kind: "hit", app: whole.app, hit }))]
        : groups.flatMap((g): Item[] => [
            ...(g.count > g.hits.length ? [{ kind: "more", group: g } as Item] : []),
            ...g.hits.map((hit): Item => ({ kind: "hit", app: g.app, hit })),
          ]),
    [groups, whole],
  );
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  /** The selected option: the first result until the keys or the pointer choose another. */
  const cur = at === FIRST ? Math.max(0, rows.findIndex((r) => r.kind === "hit")) : Math.min(at, Math.max(0, rows.length - 1));

  const choose = useCallback(
    (i: number) => {
      const r = rowsRef.current[i];
      if (!r) return;
      if (r.kind === "more") return showAllRef.current(r.group);
      if (r.kind === "less") return showLessRef.current();
      setOpen(false);
      setOut(false);
      onOpen(r.app);
      reveal({
        screen: r.app,
        id: r.hit.id,
        within: r.hit.within,
        at: r.app === "calendar" ? r.hit.when ?? undefined : undefined,
        query: query.trim(),
      });
    },
    [onOpen, query],
  );
  const point = useCallback((i: number) => setAt(i), []);

  /** Show one app whole: what is already here at once, the rest read a page at a time. */
  const showAll = (g: SearchGroup) => {
    const q = found?.q ?? query.trim();
    setWhole({ app: g.app, q, count: g.count, hits: g.hits, reading: false });
    setBand(0);
    setAt(FIRST);
    if (panel.current) panel.current.scrollTop = 0;
  };
  /** Back to every app, the selection on the app's Show All it came from. */
  const showLess = () => {
    const app = whole?.app;
    setWhole(null);
    const back = groups.flatMap((g): Item[] => [
      ...(g.count > g.hits.length ? [{ kind: "more", group: g } as Item] : []),
      ...g.hits.map((hit): Item => ({ kind: "hit", app: g.app, hit })),
    ]).findIndex((r) => r.kind === "more" && r.group.app === app);
    setAt(back >= 0 ? back : FIRST);
  };
  const showAllRef = useRef(showAll);
  showAllRef.current = showAll;
  const showLessRef = useRef(showLess);
  showLessRef.current = showLess;
  // The next page, once the end of what was read comes near.
  const last = Math.min(whole?.hits.length ?? 0, band * BAND + VIEW + BAND + OVERSCAN);
  useEffect(() => {
    if (!whole || whole.reading || whole.hits.length >= whole.count || last < whole.hits.length - PAGE / 4) return;
    const { app, q } = whole;
    const skip = whole.hits.length;
    setWhole({ ...whole, reading: true });
    api.searchEverything(q, { seq: asked.current, only: app, skip, each: PAGE }).then(
      (f) => {
        const more = f?.groups.find((g) => g.app === app)?.hits ?? [];
        setWhole((w) => (w && w.app === app && w.q === q && w.hits.length === skip
          ? { ...w, hits: w.hits.concat(more), reading: false, count: more.length ? w.count : w.hits.length }
          : w));
      },
      () => setWhole((w) => (w && w.app === app ? { ...w, reading: false } : w)),
    );
  }, [whole, last]);

  // The selected row kept in view as the keys move it.
  useLayoutEffect(() => {
    const p = panel.current;
    if (!p || !open) return;
    if (whole) {
      // Option 0 is Show Less, in the heading that stays at the top; the rows follow it.
      const head = 34;
      const top = head + Math.max(0, cur - 1) * ROW;
      if (cur === 0 || top - head < p.scrollTop) p.scrollTop = top - head;
      else if (top + ROW > p.scrollTop + p.clientHeight) p.scrollTop = top + ROW - p.clientHeight;
    } else {
      document.getElementById(`gs-${cur}`)?.scrollIntoView({ block: "nearest" });
    }
  }, [cur, whole?.app, open]); // eslint-disable-line react-hooks/exhaustive-deps

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      if (whole) showLess();
      else if (open && query) setOpen(false);
      else if (out) {
        setQuery("");
        putAway(true);
      } else if (window.matchMedia("(max-width: 900px)").matches) {
        // In a narrow window the sidebar is over the content: this Escape is the sidebar's,
        // which goes, and gives the focus to the button that brings it back.
        setQuery("");
        return;
      } else {
        setQuery("");
        field.current?.blur();
      }
      e.preventDefault();
    } else if (e.key === "ArrowDown" && rows.length) {
      e.preventDefault();
      if (!open) return setOpen(true);
      setAt(Math.min(rows.length - 1, cur + 1));
    } else if (e.key === "ArrowUp" && rows.length) {
      e.preventDefault();
      setAt(Math.max(0, cur - 1));
    } else if (e.key === "Enter" && rows[cur]) {
      e.preventDefault();
      choose(cur);
    }
  };

  const showing = open && query.trim() !== "" && found !== null;
  // Out of the rail: beside the magnifier, over the window (the sidebar clips what leaves it).
  useLayoutEffect(() => {
    if (!out) return setOutAt(null);
    const r = magnifier.current?.getBoundingClientRect();
    if (r) setOutAt({ left: r.right + 14, top: r.top - 2 });
  }, [out]);
  // Drawn out of the rail: the field takes the keys before the next one is pressed — a frame
  // later, what was typed straight after Ctrl+K was lost.
  useLayoutEffect(() => {
    if (out && outAt && document.activeElement !== field.current) {
      field.current?.focus();
      field.current?.select();
    }
  }, [out, outAt]);
  useEffect(() => {
    if (!showing) return undefined;
    const put = () => {
      const r = (wrap.current ?? box.current)?.getBoundingClientRect();
      if (r) setPlace({ left: r.left, top: r.bottom + 6 });
    };
    put();
    window.addEventListener("resize", put);
    return () => window.removeEventListener("resize", put);
  }, [showing, outAt]);

  // The heading is drawn as one, but inside a listbox only options may be read: the group
  // carries the app's name and count instead, and Show All is one more option.
  const header = (g: { app: string; count: number }, isWhole: boolean, shown: number, index: number) => (
    <h3 className={isWhole ? "is-stuck" : undefined} role="none">
      <AppIcon app={g.app} size={16} />
      <span aria-hidden="true">{NAMES[g.app] ?? g.app}</span>
      <span className="gsearch-count" aria-hidden="true">{g.count}</span>
      {isWhole || g.count > shown ? (
        <button
          id={`gs-${index}`}
          role="option"
          tabIndex={-1}
          aria-selected={cur === index}
          className={`gsearch-more${cur === index ? " is-at" : ""}`}
          aria-label={isWhole ? `Show less of ${NAMES[g.app] ?? g.app}` : `Show all ${g.count} in ${NAMES[g.app] ?? g.app}`}
          onMouseMove={cur === index ? undefined : () => point(index)}
          onClick={() => choose(index)}
        >
          {isWhole ? "Show Less" : "Show All"}
        </button>
      ) : null}
    </h3>
  );
  const groupName = (g: { app: string; count: number }) =>
    `${NAMES[g.app] ?? g.app}, ${g.count} ${g.count === 1 ? "result" : "results"}`;

  let body: React.ReactNode;
  if (whole) {
    // Only the rows in view are drawn; the space of the rest is held by two spacers.
    const total = whole.count;
    const first = Math.max(0, band * BAND - OVERSCAN);
    const end = Math.min(total, band * BAND + VIEW + BAND + OVERSCAN);
    const drawnRows = [];
    for (let i = first; i < end; i++) {
      const hit = whole.hits[i];
      drawnRows.push(
        hit ? (
          <Row key={`${hit.id}`} app={whole.app} hit={hit} index={i + 1} selected={i + 1 === cur} onPick={choose} onPoint={point} />
        ) : (
          <div key={`wait-${i}`} className="gsearch-row is-waiting" aria-hidden="true" />
        ),
      );
    }
    body = (
      <section className="gsearch-group" role="group" aria-label={groupName(whole)}>
        {header(whole, true, whole.hits.length, 0)}
        <div style={{ height: first * ROW }} />
        {drawnRows}
        <div style={{ height: (total - end) * ROW }} />
      </section>
    );
  } else {
    let i = 0;
    body = groups.map((g) => {
      const more = g.count > g.hits.length ? i++ : -1;
      return (
        <section key={g.app} className="gsearch-group" role="group" aria-label={groupName(g)}>
          {header(g, false, g.hits.length, more)}
          {g.hits.map((hit) => {
            const index = i++;
            return <Row key={hit.id} app={g.app} hit={hit} index={index} selected={index === cur} onPick={choose} onPoint={point} />;
          })}
        </section>
      );
    });
  }

  const empty = groups.length === 0 && !whole;
  const total = whole ? whole.count : groups.reduce((n, g) => n + g.count, 0);

  return (
    <div className={`gsearch${folded ? " is-folded" : ""}${out ? " is-out" : ""}`} ref={box}>
      {folded && (
        <button
          ref={magnifier}
          type="button"
          className="icon-button gsearch-magnifier"
          aria-label="Search everything"
          aria-keyshortcuts="Control+K"
          aria-expanded={out}
          title="Search everything"
          onClick={() => (out ? putAway(false) : summon.current())}
        >
          <Search size={16} strokeWidth={2} aria-hidden="true" />
        </button>
      )}
      {(() => {
        const capsule = (
          <div ref={wrap} className={out ? "gsearch-out" : "gsearch-field"} style={out && outAt ? { left: outAt.left, top: outAt.top } : undefined}>
      <SearchField
        ref={field}
        placeholder="Search"
        aria-label="Search everything"
        aria-expanded={showing}
        aria-controls="gsearch-results"
        aria-activedescendant={showing && rows[cur] ? `gs-${cur}` : undefined}
        aria-autocomplete="list"
        aria-keyshortcuts="Control+K"
        role="combobox"
        value={query}
        onChange={(v) => {
          setQuery(v);
          setOpen(true);
        }}
        onClear={() => {
          setQuery("");
          field.current?.focus();
        }}
        onFocus={() => query && setOpen(true)}
        onBlur={(e) => {
          // Out of the rail, it goes back once the focus has left it and its results.
          const to = e.relatedTarget as Node | null;
          if (out && to && !wrap.current?.contains(to) && !panel.current?.contains(to)) putAway(false);
        }}
        onKeyDown={onKey}
      />
          </div>
        );
        if (!folded) return capsule;
        return out && outAt ? <Portal>{capsule}</Portal> : null;
      })()}
      {/* How many were found, said once each answer lands; drawn nowhere. */}
      <span className="visually-hidden" role="status">
        {showing ? (empty ? `No results for “${found?.q}”` : `${total} ${total === 1 ? "result" : "results"}`) : ""}
      </span>
      {showing && place && (
        <Portal>
          {/* The shadow on a surface that never scrolls; the list scrolls inside it. */}
          <div className="gsearch-panel" style={{ left: place.left, top: place.top }}>
            <div
              ref={panel}
              className={`gsearch-results${whole ? " is-whole" : ""}`}
              id="gsearch-results"
              role={empty ? undefined : "listbox"}
              aria-label={empty ? undefined : "Found"}
              data-scroller=""
              onScroll={whole ? (e) => setBand(Math.floor(e.currentTarget.scrollTop / (ROW * BAND))) : undefined}
              // A click on a result never takes the focus from the field: the keys go on working.
              onMouseDown={(e) => e.preventDefault()}
            >
              {empty ? <p className="gsearch-none">No results for “{found?.q}”</p> : body}
            </div>
          </div>
        </Portal>
      )}
    </div>
  );
}
