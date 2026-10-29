import {
  BookOpen,
  Calendar,
  Check,
  CalendarDays,
  ChevronRight,
  Dumbbell,
  Flag,
  Folder,
  House,
  Inbox,
  Info,
  Lightbulb,
  List,
  Plus,
  Search,
  Sun,
  type LucideIcon,
} from "lucide-react";

/** The icons the app's screens mark things with, by what they mean rather than how drawn. */
export type GlyphName =
  | "today"
  | "scheduled"
  | "all"
  | "flagged"
  | "completed"
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


/**
 * Each glyph from one professionally drawn set, on one grid and one stroke — the nearest
 * open equivalents of the SF Symbols Apple's apps use, which cannot be shipped outside
 * Apple's platforms. The hand-drawn paths these replaced read as rough beside Apple's.
 */
const ICONS: Record<GlyphName, { icon: LucideIcon; filled?: boolean }> = {
  today: { icon: Calendar },
  scheduled: { icon: CalendarDays },
  all: { icon: Inbox },
  flagged: { icon: Flag, filled: true },
  completed: { icon: Check },
  list: { icon: List },
  bullets: { icon: List },
  fitness: { icon: Dumbbell },
  education: { icon: BookOpen },
  home: { icon: House },
  idea: { icon: Lightbulb },
  sun: { icon: Sun },
  folder: { icon: Folder },
  plus: { icon: Plus },
  info: { icon: Info },
  chevron: { icon: ChevronRight },
  search: { icon: Search },
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
  const { icon: Icon, filled } = ICONS[name];
  return (
    <Icon
      className={className}
      size={16}
      strokeWidth={2.1}
      absoluteStrokeWidth={false}
      fill={filled ? "currentColor" : "none"}
      aria-hidden="true"
      focusable="false"
    />
  );
}
