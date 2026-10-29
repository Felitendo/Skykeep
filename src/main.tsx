import React, { lazy, Suspense } from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/kit.css";
import "./styles/app.css";
import "./styles/reminders.css";
import "./styles/calendar.css";
import "./styles/notes.css";
import "./styles/photos.css";
import "./styles/a11y.css";
import { applyTheme, followContrast, followTheme, storedTheme } from "./lib/theme";
import { keepTinted } from "./lib/tint";
import { fadeSheetsIn } from "./lib/fade";
import { followClick } from "./lib/follow";

// Before the first paint, so the window opens in the appearance chosen.
applyTheme(storedTheme());
followTheme();
followContrast();
keepTinted();
fadeSheetsIn();

/*
 * Every link goes the one way out: through `followClick` (lib/follow.ts), which checks what
 * kind of address it is before anything happens — the engine never follows a link by itself.
 * A permitted one leaves as a navigation, which the window refuses and hands to the app this
 * computer uses for it (see the "links" plugin in main.rs). Capturing, so a link whose own
 * handler stops the click from spreading is covered too; a middle click, which would ask for
 * a new window, is refused the same way.
 */
document.addEventListener("click", (e) => void followClick(e), true);
document.addEventListener("auxclick", (e) => {
  if ((e.target as Element | null)?.closest?.("a[href]")) e.preventDefault();
}, true);

/*
 * The engine's own menu (Back, Forward, Reload, Inspect Element) is not this app's: a reload
 * from it flashed the whole window. Text fields keep their Cut, Copy and Paste; every menu the
 * app draws itself has already taken its right-click.
 */
document.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented) return;
  const editable = (e.target as HTMLElement | null)?.closest?.("input, textarea, [contenteditable=''], [contenteditable='true'], [contenteditable='plaintext-only']");
  if (!editable) e.preventDefault();
});

// The quick-capture window (`skykeep --new reminder`) is one field, not the library — and the
// library's window never reads its code.
const capture = new URLSearchParams(window.location.search).get("capture");
const QuickCapture = lazy(() => import("./components/QuickCapture").then((m) => ({ default: m.QuickCapture })));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {capture === "reminder" || capture === "note" ? (
      <Suspense fallback={null}>
        <QuickCapture initial={capture} />
      </Suspense>
    ) : (
      <App />
    )}
  </React.StrictMode>
);

