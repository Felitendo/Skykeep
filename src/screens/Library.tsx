import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, messageOf, OPEN_REMINDER } from "../lib/ipc";
import { SyncProvider } from "../lib/sync";
import { SyncStatus } from "../components/SyncStatus";
import { CalendarView } from "./calendar/CalendarView";
import { RemindersView } from "./RemindersView";
import { NotesView } from "./NotesView";
import { ContactsView } from "./ContactsView";
import { DriveView } from "./DriveView";

/**
 * What the sidebar holds: the services with a real screen behind them, most used first.
 *
 * Apple advertises thirty-odd services for an account — "CloudKit data", "Push", "Account
 * gateway" — and they were all listed, behind "Show 23 more", along with Photos, Mail and Find
 * My, which opened onto a page saying they were not built. None of that is something a person
 * can use here, so none of it is shown. A service joins this list when it works.
 */
const SCREENS: { key: string; name: string }[] = [
  { key: "reminders", name: "Reminders" },
  { key: "notes", name: "Notes" },
  { key: "drivews", name: "iCloud Drive" },
  { key: "calendar", name: "Calendar" },
  { key: "contacts", name: "Contacts" },
];

/** The screen open last time, so the app reopens where it was left. */
const LAST_KEY = "quince.library.screen";

export function Library({ onSignOut }: { onSignOut: () => void }) {
  const [services, setServices] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    api
      .services()
      .then((list) => {
        const offered = SCREENS.map((s) => s.key).filter((k) => list.includes(k));
        setServices(offered);
        let last: string | null = null;
        try {
          last = window.localStorage.getItem(LAST_KEY);
        } catch {
          // Without storage it opens on the first screen, which is fine.
        }
        setSelected(last && offered.includes(last) ? last : (offered[0] ?? null));
        setReady(true);
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  const choose = (key: string) => {
    setSelected(key);
    try {
      window.localStorage.setItem(LAST_KEY, key);
    } catch {
      // Remembering is a convenience; choosing still works.
    }
  };

  // Clicking a reminder's alert brings the window back on Reminders.
  useEffect(() => {
    const stop = listen<string>(OPEN_REMINDER, () => {
      if (services.includes("reminders")) choose("reminders");
    });
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [services]);

  // Native sidebars move with the arrow keys and occupy a single tab stop. Without this a
  // keyboard user has to Tab through every row to reach the content.
  const rows = useRef<(HTMLButtonElement | null)[]>([]);
  const order = services;

  function onKeyDown(e: React.KeyboardEvent, index: number) {
    const move = (to: number) => {
      e.preventDefault();
      const clamped = Math.max(0, Math.min(to, order.length - 1));
      rows.current[clamped]?.focus();
      choose(order[clamped]);
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
        onClick={() => choose(key)}
      >
        <span>{SCREENS.find((s) => s.key === key)?.name ?? key}</span>
      </button>
    );
  };

  return (
    <SyncProvider>
      <div className="library enter-library">
        <nav className="sidebar" aria-label="Your iCloud">
          <h2>Your iCloud</h2>
          {services.map(item)}
          {!ready && !error && <p className="badge">Loading…</p>}
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}

          <div className="sidebar-foot">
            <SyncStatus />
            <button className="quiet sidebar-signout" onClick={onSignOut}>
              Sign out
            </button>
          </div>
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
          ) : selected === "drivews" ? (
            <DriveView />
          ) : (
            ready &&
            services.length === 0 && (
              <p className="cal-empty">
                Apple hasn’t offered Reminders, Notes, iCloud Drive, Calendar or Contacts to
                this account yet.
              </p>
            )
          )}
        </main>
      </div>
    </SyncProvider>
  );
}
