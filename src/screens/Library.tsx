import { useEffect, useState } from "react";
import { api, messageOf } from "../lib/ipc";

/** Which services Quince can actually read yet. Empty on purpose: the sync engine is Phase 2,
 *  and showing anything else here would be inventing data the app doesn't have. */
const IMPLEMENTED = new Set<string>();

const NAMES: Record<string, string> = {
  calendar: "Calendar",
  contacts: "Contacts",
  reminders: "Reminders",
  ckdatabasews: "Photos & Notes",
  drivews: "iCloud Drive",
  docws: "Documents",
  findme: "Find My",
  photos: "Photos",
  mail: "Mail",
  keyvalue: "App data",
  settings: "Account settings",
  account: "Account",
  premiummailsettings: "Mail settings",
  iworkthumbnailws: "iWork previews",
  uploadimagews: "Photo upload",
  streams: "Shared albums",
  iwmb: "iWork",
  ubiquity: "Legacy Drive",
  fmf: "Find My Friends",
  schoolwork: "Schoolwork",
};

export function Library({ onSignOut }: { onSignOut: () => void }) {
  const [services, setServices] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
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
        setSelected(unique[0] ?? null);
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  return (
    <div className="library enter-library">
      <nav className="sidebar" aria-label="Your iCloud services">
        <h2>On your account</h2>
        {services.map((key) => (
          <button
            key={key}
            className="nav-item"
            aria-current={selected === key}
            onClick={() => setSelected(key)}
          >
            <span>{NAMES[key] ?? key}</span>
            {!IMPLEMENTED.has(key) && <span className="badge">soon</span>}
          </button>
        ))}
        {services.length === 0 && !error && <p className="badge">Loading…</p>}
      </nav>

      <main className="content">
        <div className="toolbar">
          <button className="quiet" onClick={onSignOut}>Sign out</button>
        </div>
        <h1>You're signed in</h1>
        <p>
          Quince is holding a live session with Apple, and it will still be here after you
          quit and reopen the app. The list on the left is what Apple actually reports for
          your account — nothing invented.
        </p>
        <p>
          Reading your calendar, reminders and photos into a local library comes next. Until
          then this screen tells you the truth: the connection works, the data layer isn't
          built yet.
        </p>
        {error && <p className="error" role="alert">{error}</p>}
      </main>
    </div>
  );
}
