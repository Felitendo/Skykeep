import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/reminders.css";
import "./styles/calendar.css";
import "./styles/notes.css";

/*
 * Every link goes the one way out: as a navigation, which the window refuses and hands to
 * the app this computer uses for it (see the "links" plugin in main.rs). A link meant for a
 * new window would otherwise slip past that — WebKitGTK asks for a window instead — so it
 * is followed in place, where the refusal catches it.
 */
document.addEventListener("click", (e) => {
  const a = (e.target as HTMLElement | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
  if (!a || e.defaultPrevented || a.target !== "_blank") return;
  e.preventDefault();
  window.location.assign(a.href);
  // Capturing, so a link whose own handler stops the click from spreading is covered too.
}, true);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
