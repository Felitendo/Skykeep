/**
 * The small icons the Reminders sidebar marks its built-in views with.
 *
 * Drawn rather than typed: the glyphs here used to be a mix of emoji and dingbat
 * characters — a literal 📅 next to a ☀ next to a ≡ — which every font renders at a
 * different weight, size and baseline, so the row of them never lined up and never looked
 * like it came from the same hand. These are one stroke weight on one grid.
 */
export type GlyphName = "today" | "scheduled" | "all" | "flagged" | "list";

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
};

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
