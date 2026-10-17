import { useId } from "react";
import { dateText } from "../lib/when";

/**
 * Each screen's mark in the sidebar, drawn after the app it stands in for — Reminders' three
 * coloured dots, Notes' yellow pad, Drive's blue folder, Calendar's day tile, Contacts' card —
 * so a row is found by its picture before its name is read.
 *
 * Drawn here, not Apple's artwork, which is Apple's to ship. Like icloud.com's launcher they
 * come in two finishes: a light tile, and in dark mode the deep graphite tile Apple's dark
 * icons sit on, with a soft top light and a hairline edge so they read as objects, not
 * flat stickers. Vector, so they are crisp at any scale on any desktop.
 */
export function AppIcon({ app, size = 22 }: { app: string; size?: number }) {
  const id = useId().replace(/:/g, "");
  const today = new Date();
  const s = { width: size, height: size, flex: "none" } as const;

  // The tile every icon stands on: light or graphite by the theme, a top light, an edge.
  const tile = (
    <>
      <defs>
        <linearGradient id={`${id}t`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" className="ai-tile-top" />
          <stop offset="1" className="ai-tile-bottom" />
        </linearGradient>
        <linearGradient id={`${id}g`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#fff" stopOpacity="0.22" />
          <stop offset="0.5" stopColor="#fff" stopOpacity="0" />
        </linearGradient>
      </defs>
      <rect width="24" height="24" rx="5.4" fill={`url(#${id}t)`} />
    </>
  );
  const shine = (
    <>
      <rect width="24" height="24" rx="5.4" fill={`url(#${id}g)`} />
      <rect x="0.25" y="0.25" width="23.5" height="23.5" rx="5.2" fill="none" className="ai-edge" strokeWidth="0.5" />
    </>
  );

  let art: React.ReactNode = null;
  switch (app) {
    case "reminders":
      art = [
        [7.2, "#0a84ff"],
        [12, "#ff453a"],
        [16.8, "#ff9f0a"],
      ].map(([y, c]) => (
        <g key={String(y)}>
          <circle cx="6.6" cy={y as number} r="2.15" fill={c as string} />
          <circle cx="6.6" cy={y as number} r="2.15" fill="none" stroke="#000" strokeOpacity="0.12" strokeWidth="0.3" />
          <rect x="10.2" y={(y as number) - 0.55} width="9.4" height="1.1" rx="0.55" className="ai-rule" />
        </g>
      ));
      break;
    case "notes":
      art = (
        <>
          <defs>
            <linearGradient id={`${id}n`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#ffe066" />
              <stop offset="1" stopColor="#ffc800" />
            </linearGradient>
          </defs>
          <path d="M0 5.4A5.4 5.4 0 0 1 5.4 0h13.2A5.4 5.4 0 0 1 24 5.4V7.6H0z" fill={`url(#${id}n)`} />
          <path d="M0 7.6h24" stroke="#000" strokeOpacity="0.12" strokeWidth="0.4" strokeDasharray="0.5 0.7" />
          {[11.8, 15.2, 18.6].map((y) => (
            <rect key={y} x="3.4" y={y} width="17.2" height="0.8" rx="0.4" className="ai-rule" />
          ))}
        </>
      );
      break;
    case "drivews":
      art = (
        <>
          <defs>
            <linearGradient id={`${id}d`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#64d2ff" />
              <stop offset="1" stopColor="#0a84ff" />
            </linearGradient>
            <linearGradient id={`${id}b`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#0a6ad6" />
              <stop offset="1" stopColor="#0957b3" />
            </linearGradient>
          </defs>
          <path d="M3.6 7.6a1.6 1.6 0 0 1 1.6-1.6h4.1l1.7 1.8h7.8a1.6 1.6 0 0 1 1.6 1.6V10H3.6z" fill={`url(#${id}b)`} />
          <path d="M3.6 9.6A1.6 1.6 0 0 1 5.2 8h13.6a1.6 1.6 0 0 1 1.6 1.6v7.6a1.6 1.6 0 0 1-1.6 1.6H5.2a1.6 1.6 0 0 1-1.6-1.6z" fill={`url(#${id}d)`} />
          <path d="M3.6 10.4h16.8" stroke="#fff" strokeOpacity="0.35" strokeWidth="0.4" />
        </>
      );
      break;
    case "calendar":
      art = (
        <>
          <text x="12" y="8" textAnchor="middle" fontSize="5" fontWeight="600" fill="#ff453a"
            fontFamily="-apple-system, 'SF Pro Text', Inter, system-ui, sans-serif" letterSpacing="0.2">
            {dateText(today, { weekday: "short" }).slice(0, 3)}
          </text>
          <text x="12" y="19.4" textAnchor="middle" fontSize="11.6" fontWeight="300" className="ai-ink"
            fontFamily="-apple-system, 'SF Pro Display', Inter, system-ui, sans-serif" letterSpacing="-0.4">
            {today.getDate()}
          </text>
        </>
      );
      break;
    case "contacts":
      art = (
        <>
          <rect x="17.6" y="4.2" width="2.2" height="4.4" rx="0.6" fill="#ff9f0a" />
          <rect x="17.6" y="9.8" width="2.2" height="4.4" rx="0.6" fill="#30d158" />
          <rect x="17.6" y="15.4" width="2.2" height="4.4" rx="0.6" fill="#0a84ff" />
          <rect x="3.8" y="3.6" width="13" height="16.8" rx="2" className="ai-card" />
          <circle cx="10.3" cy="10.2" r="2.9" className="ai-person" />
          <path d="M5.4 18.4c.7-2.7 2.7-4 4.9-4s4.2 1.3 4.9 4z" className="ai-person" />
        </>
      );
      break;
    default:
      return <span style={s} aria-hidden="true" />;
  }

  return (
    <svg viewBox="0 0 24 24" style={s} aria-hidden="true" className={`app-icon ai-${app}`}>
      {tile}
      {art}
      {shine}
    </svg>
  );
}
