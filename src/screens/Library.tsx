import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { PanelLeft } from "lucide-react";
import { api, messageOf, OPEN_REMINDER } from "../lib/ipc";
import { SyncProvider } from "../lib/sync";
import { AccountCard } from "../components/Account";

/*
 * Each app is its own chunk, so the window opens on the code for the app it shows, not all
 * five — Notes' editor alone is a few hundred kilobytes. The rest are fetched as soon as the
 * window is up, before anyone could reach them, so switching never waits on a load.
 */
const loaders = {
  calendar: () => import("./calendar/CalendarView").then((m) => ({ default: m.CalendarView })),
  reminders: () => import("./RemindersView").then((m) => ({ default: m.RemindersView })),
  notes: () => import("./NotesView").then((m) => ({ default: m.NotesView })),
  contacts: () => import("./ContactsView").then((m) => ({ default: m.ContactsView })),
  drivews: () => import("./DriveView").then((m) => ({ default: m.DriveView })),
  settings: () => import("./SettingsView").then((m) => ({ default: m.SettingsView })),
};
const CalendarView = lazy(loaders.calendar);
const RemindersView = lazy(loaders.reminders);
const NotesView = lazy(loaders.notes);
const ContactsView = lazy(loaders.contacts);
const DriveView = lazy(loaders.drivews);
const SettingsView = lazy(loaders.settings);
import { ScreenBoundary } from "../components/ScreenBoundary";
import { AppIcon } from "../components/AppIcon";

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
const RAIL_KEY = "quince.library.rail";

export function Library({ onSignOut }: { onSignOut: () => void }) {
  const [services, setServices] = useState<string[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  /** In a narrow window the sidebar slides over the content, and only when asked for. */
  const [sidebarOpen, setSidebarOpen] = useState(false);
  /** A wide window can fold the sidebar to its icons, as Finder and Mail can; remembered. */
  const [rail, setRail] = useState(() => {
    try {
      return window.localStorage.getItem(RAIL_KEY) === "1";
    } catch {
      return false;
    }
  });
  const content = useRef<HTMLElement | null>(null);
  /**
   * Folding the sidebar moves the content without laying it out on every frame. Animating
   * the content's padding made the whole screen — a calendar grid, a long list — lay itself
   * out sixty times a second, and that was the stutter. Instead it takes its new width at
   * once and slides from where it was, which the compositor does alone.
   */
  const slideContent = (from: number) => {
    const el = content.current;
    if (!el || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    requestAnimationFrame(() => {
      const to = el.getBoundingClientRect().left + parseFloat(getComputedStyle(el).paddingLeft);
      const by = from - to;
      if (Math.abs(by) < 1) return;
      el.animate([{ transform: `translateX(${by}px)` }, { transform: "none" }], {
        duration: 300,
        easing: getComputedStyle(document.documentElement).getPropertyValue("--spring-quick").trim() || "ease-out",
      });
    });
  };
  const toggleRail = () => {
    const el = content.current;
    if (el) slideContent(el.getBoundingClientRect().left + parseFloat(getComputedStyle(el).paddingLeft));
    setRailState();
  };
  const setRailState = () =>
    setRail((was) => {
      try {
        window.localStorage.setItem(RAIL_KEY, was ? "0" : "1");
      } catch {
        // Folds for now; forgotten on restart.
      }
      return !was;
    });

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
        setSelected(last && (offered.includes(last) || last === "settings") ? last : (offered[0] ?? null));
        setReady(true);
      })
      .catch((e) => setError(messageOf(e)));
  }, []);

  // The open app's accent belongs to the whole window — menus and popovers drawn over it
  // too — as each Mac app colours itself: Notes yellow, Calendar red, the rest blue.
  useEffect(() => {
    document.documentElement.dataset.app = selected ?? "";
    return () => {
      delete document.documentElement.dataset.app;
    };
  }, [selected]);

  // The other apps' code, fetched while nothing else is happening.
  useEffect(() => {
    if (!ready) return undefined;
    const t = window.setTimeout(() => {
      for (const load of Object.values(loaders)) void load().catch(() => {});
    }, 300);
    return () => window.clearTimeout(t);
  }, [ready]);

  const choose = (key: string) => {
    setSelected(key);
    setSidebarOpen(false);
    try {
      window.localStorage.setItem(LAST_KEY, key);
    } catch {
      // Remembering is a convenience; choosing still works.
    }
  };

  useEffect(() => {
    if (!sidebarOpen) return undefined;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setSidebarOpen(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sidebarOpen]);

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
        title={rail ? (SCREENS.find((s) => s.key === key)?.name ?? key) : undefined}
      >
        <AppIcon app={key} />
        <span className="nav-name">{SCREENS.find((s) => s.key === key)?.name ?? key}</span>
      </button>
    );
  };

  return (
    <SyncProvider>
      <div className={`library enter-library${sidebarOpen ? " is-sidebar-open" : ""}${rail ? " is-rail" : ""}`}>
        {/* Only in a narrow window, where the sidebar gives its room to the content. */}
        <button
          className="sidebar-toggle"
          aria-label={sidebarOpen ? "Hide Sidebar" : "Show Sidebar"}
          aria-expanded={sidebarOpen}
          title={sidebarOpen ? "Hide Sidebar" : "Show Sidebar"}
          onClick={() => setSidebarOpen((open) => !open)}
        >
          <PanelLeft size={17} strokeWidth={2} aria-hidden="true" />
        </button>
        <div className="sidebar-scrim" aria-hidden="true" onClick={() => setSidebarOpen(false)} />
        <nav className="sidebar" aria-label="Your iCloud">
          <div className="sidebar-head">
            <h2>iCloud</h2>
            <button
              className="sidebar-fold"
              aria-label={rail ? "Show Sidebar Names" : "Fold Sidebar to Icons"}
              aria-pressed={rail}
              title={rail ? "Show Sidebar Names" : "Fold Sidebar to Icons"}
              onClick={toggleRail}
            >
              <PanelLeft size={16} strokeWidth={2} aria-hidden="true" />
            </button>
          </div>
          {services.map(item)}
          {!ready && !error && <p className="badge">Loading…</p>}
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}

          <div className="sidebar-foot">
            <AccountCard current={selected === "settings"} onOpen={() => choose("settings")} />
          </div>
        </nav>

        <main className="content" ref={content}>
          {selected === "settings" ? (
            <ScreenBoundary key="settings" name="Settings">
              <Suspense fallback={null}>
                <SettingsView onSignOut={onSignOut} />
              </Suspense>
            </ScreenBoundary>
          ) : selected && SCREENS.some((s) => s.key === selected) ? (
            // Keyed by screen, so moving to another screen also clears a failed one.
            <ScreenBoundary key={selected} name={SCREENS.find((s) => s.key === selected)?.name ?? "This screen"}>
              <Suspense fallback={null}>
              {selected === "calendar" ? (
                <CalendarView />
              ) : selected === "reminders" ? (
                <RemindersView />
              ) : selected === "notes" ? (
                <NotesView />
              ) : selected === "contacts" ? (
                <ContactsView />
              ) : (
                <DriveView />
              )}
              </Suspense>
            </ScreenBoundary>
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
