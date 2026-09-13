import { useEffect, useRef, useState } from "react";
import { api, messageOf } from "../lib/ipc";
import { CalendarView } from "./CalendarView";
import { RemindersView } from "./RemindersView";
import { NotesView } from "./NotesView";
import { ContactsView } from "./ContactsView";

const NAMES: Record<string, string> = {
  photos: "Photos",
  sharedlibrary: "Shared library",
  sharedstreams: "Shared albums",
  photosupload: "Photo upload",
  calendar: "Calendar",
  contacts: "Contacts",
  contactsweb: "Contacts (web)",
  reminders: "Reminders",
  notes: "Notes",
  mail: "Mail",
  premiummailsettings: "Mail settings",
  drivews: "iCloud Drive",
  docws: "Documents",
  ubiquity: "Drive (legacy)",
  archivews: "Archives",
  ckdatabasews: "CloudKit data",
  cksharews: "CloudKit sharing",
  ckdeviceservice: "CloudKit devices",
  findme: "Find My",
  keyvalue: "App data",
  account: "Account",
  settings: "Account settings",
  iwmb: "iWork",
  iworkexportws: "iWork export",
  iworkthumbnailws: "iWork previews",
  uploadimagews: "Image upload",
  geows: "Location",
  push: "Push",
  mpp: "Private Relay",
  mcc: "Account services",
  mccgateway: "Account gateway",
  developerapi: "Developer",
  schoolwork: "Schoolwork",
};

/** Services with a real screen behind them. Opening on one of these beats opening on a
 *  placeholder, so selection prefers them. */
const BUILT = new Set(["calendar", "reminders", "notes", "contacts"]);

/** What a person actually came for, in the order they'd look for it. */
const PRIMARY = [
  "photos",
  "calendar",
  "reminders",
  "notes",
  "contacts",
  "drivews",
  "mail",
  "findme",
  "sharedlibrary",
  "sharedstreams",
];

export function Library({ onSignOut }: { onSignOut: () => void }) {
  const [services, setServices] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .services()
      .then((list) => {
        // Apple advertises several keys that resolve to the same surface; show each once.
        const seen = new Set<string>();
        const unique = list.filter((k) => {
          const label = NAMES[k] ?? k;
          if (seen.has(label)) return false;
          seen.add(label);
          return true;
        });
        setServices(unique);
        setSelected(
          PRIMARY.find((k) => unique.includes(k) && BUILT.has(k)) ??
            PRIMARY.find((k) => unique.includes(k)) ??
            unique[0] ??
            null
        );
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  const primary = PRIMARY.filter((k) => services.includes(k));
  // Everything else is infrastructure — real, but not something anyone navigates to.
  const rest = services.filter((k) => !PRIMARY.includes(k));

  // Native sidebars move with the arrow keys and occupy a single tab stop. Without this a
  // keyboard user has to Tab through every row to reach the content.
  const rows = useRef<(HTMLButtonElement | null)[]>([]);
  const order = showAll ? [...primary, ...rest] : primary;

  function onKeyDown(e: React.KeyboardEvent, index: number) {
    const move = (to: number) => {
      e.preventDefault();
      const clamped = Math.max(0, Math.min(to, order.length - 1));
      rows.current[clamped]?.focus();
      setSelected(order[clamped]);
    };
    if (e.key === "ArrowDown") move(index + 1);
    else if (e.key === "ArrowUp") move(index - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(order.length - 1);
  }

  const item = (key: string) => {
    const index = order.indexOf(key);
    return (
      <button
        key={key}
        ref={(el) => {
          rows.current[index] = el;
        }}
        className="nav-item"
        aria-current={selected === key}
        tabIndex={selected === key ? 0 : -1}
        onKeyDown={(e) => onKeyDown(e, index)}
        onClick={() => setSelected(key)}
      >
        <span>{NAMES[key] ?? key}</span>
      </button>
    );
  };

  return (
    <div className="library enter-library">
      <nav className="sidebar" aria-label="Your iCloud">
        <h2>Your iCloud</h2>
        {primary.map(item)}
        {services.length === 0 && !error && <p className="badge">Loading…</p>}

        {rest.length > 0 && (
          <>
            <button
              className="nav-item nav-toggle"
              aria-expanded={showAll}
              onClick={() => setShowAll((v) => !v)}
            >
              <span>{showAll ? "Hide" : "Show"} {rest.length} more</span>
            </button>
            {showAll && rest.map(item)}
          </>
        )}
      </nav>

      <main className="content">
        {selected === "calendar" ? (
          <CalendarView />
        ) : selected === "reminders" ? (
          <RemindersView />
        ) : selected === "notes" ? (
          <NotesView />
        ) : selected === "contacts" ? (
          <ContactsView />
        ) : (
          <>
        <div className="toolbar">
          <button className="quiet" onClick={onSignOut}>Sign out</button>
        </div>
        <h1>{selected ? (NAMES[selected] ?? selected) : "You're signed in"}</h1>
        <p>
          Quince is holding a live session with Apple, and it survived quitting and reopening
          the app — no password, no code. The list on the left is what Apple reports for your
          account, nothing invented.
        </p>
        <p>
          Reading this into a local library is what comes next, so it opens instantly and
          works offline. Until then this screen tells you the truth: the connection works,
          the data layer isn't built yet.
        </p>
        {error && <p className="error" role="alert">{error}</p>}
          </>
        )}
      </main>
    </div>
  );
}
