/**
 * Each screen's mark in the sidebar, drawn after the app it stands in for — Reminders' three
 * coloured dots on white, Notes' yellow pad, Drive's blue folder, Calendar's day-of-the-week
 * tile, Contacts' grey card — as icloud.com's launcher and macOS's sidebar show them, so a
 * row is found by its picture before its name is read. Drawn, not bitmaps: crisp at any
 * scale and on any desktop.
 */
export function AppIcon({ app, size = 22 }: { app: string; size?: number }) {
  const today = new Date();
  const s = { width: size, height: size, flex: "none" } as const;
  switch (app) {
    case "reminders":
      return (
        <svg viewBox="0 0 24 24" style={s} aria-hidden="true">
          <rect width="24" height="24" rx="5.5" fill="#fff" />
          {[
            [7, "#0a84ff"],
            [12, "#ff3b30"],
            [17, "#ff9f0a"],
          ].map(([y, c]) => (
            <g key={String(y)}>
              <circle cx="6.5" cy={y as number} r="2.1" fill={c as string} />
              <rect x="10" y={(y as number) - 0.6} width="9" height="1.2" rx="0.6" fill="#c7c7cc" />
            </g>
          ))}
        </svg>
      );
    case "notes":
      return (
        <svg viewBox="0 0 24 24" style={s} aria-hidden="true">
          <rect width="24" height="24" rx="5.5" fill="#fff" />
          <path d="M0 5.5A5.5 5.5 0 0 1 5.5 0h13A5.5 5.5 0 0 1 24 5.5V8H0z" fill="#ffd60a" />
          {[12, 15.5, 19].map((y) => (
            <rect key={y} x="3.5" y={y} width="17" height="0.9" rx="0.45" fill="#d1d1d6" />
          ))}
        </svg>
      );
    case "drivews":
      return (
        <svg viewBox="0 0 24 24" style={s} aria-hidden="true">
          <defs>
            <linearGradient id="qi-drive" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#5ac8fa" />
              <stop offset="1" stopColor="#0a84ff" />
            </linearGradient>
          </defs>
          <rect width="24" height="24" rx="5.5" fill="#fff" />
          <path d="M4 7.5A1.5 1.5 0 0 1 5.5 6h4l1.6 1.8h7.4A1.5 1.5 0 0 1 20 9.3V17a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 17z" fill="url(#qi-drive)" />
        </svg>
      );
    case "calendar":
      return (
        <svg viewBox="0 0 24 24" style={s} aria-hidden="true">
          <rect width="24" height="24" rx="5.5" fill="#fff" />
          <text x="12" y="8.2" textAnchor="middle" fontSize="5.4" fontWeight="600" fill="#ff3b30"
            fontFamily="-apple-system, system-ui, sans-serif">
            {today.toLocaleDateString(undefined, { weekday: "short" }).toUpperCase().slice(0, 3)}
          </text>
          <text x="12" y="19.6" textAnchor="middle" fontSize="11.5" fontWeight="300" fill="#1c1c1e"
            fontFamily="-apple-system, system-ui, sans-serif">
            {today.getDate()}
          </text>
        </svg>
      );
    case "contacts":
      return (
        <svg viewBox="0 0 24 24" style={s} aria-hidden="true">
          <defs>
            <linearGradient id="qi-contacts" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="#d1d1d6" />
              <stop offset="1" stopColor="#aeaeb2" />
            </linearGradient>
          </defs>
          <rect width="24" height="24" rx="5.5" fill="url(#qi-contacts)" />
          <circle cx="12" cy="9.5" r="3.6" fill="#fff" />
          <path d="M5.5 19.5c.9-3.4 3.4-5 6.5-5s5.6 1.6 6.5 5z" fill="#fff" />
        </svg>
      );
    default:
      return <span style={s} aria-hidden="true" />;
  }
}
