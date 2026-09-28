/**
 * The small icons the Reminders sidebar marks its built-in views with.
 *
 * Drawn rather than typed: the glyphs here used to be a mix of emoji and dingbat
 * characters — a literal 📅 next to a ☀ next to a ≡ — which every font renders at a
 * different weight, size and baseline, so the row of them never lined up and never looked
 * like it came from the same hand. These are one stroke weight on one grid.
 */
export type GlyphName =
  | "today"
  | "scheduled"
  | "all"
  | "flagged"
  | "list"
  | "bullets"
  | "fitness"
  | "education"
  | "home"
  | "idea"
  | "sun"
  | "folder"
  | "plus"
  | "info"
  | "chevron"
  | "search";

const PATHS: Record<GlyphName, React.ReactNode> = {
  // A calendar page with the day marked — what Today means.
  today: (
    <>
      <rect x="2.5" y="3.5" width="11" height="10" rx="2.2" />
      <path d="M2.5 6.5h11" />
      <circle cx="8" cy="10" r="1.6" fill="currentColor" stroke="none" />
    </>
  ),
  // The same page, with days on it rather than one.
  scheduled: (
    <>
      <rect x="2.5" y="3.5" width="11" height="10" rx="2.2" />
      <path d="M2.5 6.5h11M5.5 2.5v2M10.5 2.5v2" />
      <path d="M5.2 9.3h1.4M9.4 9.3h1.4M5.2 11.4h1.4M9.4 11.4h1.4" />
    </>
  ),
  // Everything, as a list.
  all: (
    <>
      <path d="M5.5 4.6h8M5.5 8h8M5.5 11.4h8" />
      <circle cx="2.9" cy="4.6" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="2.9" cy="8" r="0.9" fill="currentColor" stroke="none" />
      <circle cx="2.9" cy="11.4" r="0.9" fill="currentColor" stroke="none" />
    </>
  ),
  flagged: (
    <>
      <path d="M4.2 13.5V3.2" />
      <path d="M4.2 3.6h7.2l-1.7 2.6 1.7 2.6H4.2z" fill="currentColor" stroke="none" />
    </>
  ),
  // A list you made yourself, in its own colour.
  list: <circle cx="8" cy="8" r="4.4" fill="currentColor" stroke="none" />,
  // The glyph a list gets when it has chosen none: Apple's bulleted list.
  bullets: (
    <>
      <path d="M6.2 4.8h6.3M6.2 8h6.3M6.2 11.2h6.3" />
      <circle cx="3.8" cy="4.8" r="0.95" fill="currentColor" stroke="none" />
      <circle cx="3.8" cy="8" r="0.95" fill="currentColor" stroke="none" />
      <circle cx="3.8" cy="11.2" r="0.95" fill="currentColor" stroke="none" />
    </>
  ),
  fitness: (
    <>
      <path d="M5.5 8h5" />
      <rect x="3.2" y="5" width="2.3" height="6" rx="0.8" />
      <rect x="10.5" y="5" width="2.3" height="6" rx="0.8" />
      <path d="M1.9 6.8v2.4M14.1 6.8v2.4" />
    </>
  ),
  education: (
    <>
      <path d="M8 4.6c-1.6-1-3.6-1.2-5.3-.7v8.2c1.7-.5 3.7-.3 5.3.7 1.6-1 3.6-1.2 5.3-.7V3.9c-1.7-.5-3.7-.3-5.3.7z" />
      <path d="M8 4.6v8.2" />
    </>
  ),
  home: (
    <>
      <path d="M2.6 7.4 8 3l5.4 4.4" />
      <path d="M4.2 6.3v6.4h7.6V6.3" />
      <path d="M6.8 12.7V9.6h2.4v3.1" />
    </>
  ),
  idea: (
    <>
      <path d="M5.9 10.4c-.9-.7-1.6-1.8-1.6-3.1a3.7 3.7 0 0 1 7.4 0c0 1.3-.7 2.4-1.6 3.1v1.2H5.9z" />
      <path d="M6.3 13.4h3.4" />
    </>
  ),
  sun: (
    <>
      <circle cx="8" cy="8" r="2.6" />
      <path d="M8 1.9v1.5M8 12.6v1.5M1.9 8h1.5M12.6 8h1.5M3.7 3.7l1 1M11.3 11.3l1 1M3.7 12.3l1-1M11.3 4.7l1-1" />
    </>
  ),
  folder: (
    <path d="M2.4 4.6c0-.6.5-1.1 1.1-1.1h2.6l1.4 1.5h5c.6 0 1.1.5 1.1 1.1v5.8c0 .6-.5 1.1-1.1 1.1h-9c-.6 0-1.1-.5-1.1-1.1z" />
  ),
  plus: <path d="M8 3.5v9M3.5 8h9" />,
  info: (
    <>
      <circle cx="8" cy="8" r="5.6" />
      <path d="M8 7.4v3.6" />
      <circle cx="8" cy="5.2" r="0.8" fill="currentColor" stroke="none" />
    </>
  ),
  chevron: <path d="M6 4.5 9.5 8 6 11.5" />,
  search: (
    <>
      <circle cx="7" cy="7" r="4" />
      <path d="m10 10 3 3" />
    </>
  ),
};

/**
 * The glyph for one of Apple's list emblems.
 *
 * Apple names them by family and number — `fitness`, `education1`, `lifestyle2` — and draws
 * them with SF Symbols, which cannot be shipped outside Apple's platforms. Each family gets
 * the drawn glyph nearest its meaning; one this does not know gets the plain bulleted list
 * rather than a guess.
 */
export function emblemGlyph(emblem: string | null | undefined): GlyphName {
  const family = (emblem ?? "").replace(/\d+$/, "").toLowerCase();
  switch (family) {
    case "fitness":
    case "sports":
    case "health":
      return "fitness";
    case "education":
    case "school":
    case "book":
      return "education";
    case "lifestyle":
    case "home":
    case "house":
      return "home";
    case "concept":
    case "idea":
      return "idea";
    case "weather":
    case "nature":
      return "sun";
    default:
      return "bullets";
  }
}

export function Glyph({ name, className }: { name: GlyphName; className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {PATHS[name]}
    </svg>
  );
}
