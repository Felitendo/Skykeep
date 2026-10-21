import { useEffect, useState } from "react";
import { SignIn } from "./screens/SignIn";
import { CodeEntry } from "./screens/CodeEntry";
import { Library } from "./screens/Library";
import { ImportSheet } from "./components/ImportSheet";
import { StatusLine } from "./components/StatusLine";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, NEEDS_CODE, NEEDS_SIGNIN } from "./lib/ipc";

type Phase = "checking" | "signin" | "code" | "library";

export default function App() {
  const [phase, setPhase] = useState<Phase>("checking");
  // The window is shown once the first real screen (the library, or sign-in) has been drawn:
  // never the session check before it, never a half-drawn frame (see window_ready, main.rs).
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (shown || phase === "checking") return;
    setShown(true);
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        // Started at login, the window stays in the tray unless the person is needed here.
        invoke("window_ready", { needsYou: phase !== "library" })
          .catch(() => {});
      }),
    );
  }, [phase, shown]);

  // On launch, try the stored session before showing anything. This is the whole promise:
  // a returning user should never see the sign-in screen.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await api.status();
        // Always resume rather than trusting stored metadata. Apple's service endpoints
        // authenticate by cookie, and holding a token is not the same as being able to call
        // them — resuming is what establishes the cookies that make the session usable.
        if (status.hasStoredSession) {
          const next = await api.resume();
          if (next !== "signin") return !cancelled && setPhase(next);
        }
      } catch {
        /* fall through to sign-in */
      }
      if (!cancelled) setPhase("signin");
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // A session that lapsed while the window was open, and that Apple will take back only
  // with a code: the code screen, not the sign-in screen — the password is already kept.
  useEffect(() => {
    const stop = listen(NEEDS_CODE, () => setPhase("code"));
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  // Apple signed the session out: the library stays — still readable — with one line over it
  // saying so, and the sign-in a click away rather than forced in its place.
  const [lapsed, setLapsed] = useState(false);
  useEffect(() => {
    const stop = listen(NEEDS_SIGNIN, () => setLapsed(true));
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  // Signed out, the window starts over: nothing of the account is left in its memory either —
  // no picture, no list, no cached answer — for whoever signs in next.
  async function signOut(keep: boolean) {
    await api.signOut(keep);
    window.location.reload();
  }

  // Filling the library is the library's own business now: one refresh covers every
  // service, runs the moment the window opens, and repeats on its own thereafter.

  if (phase === "library")
    return (
      <>
        <div className="app-ground" aria-hidden="true" />
        {lapsed && (
          <div className="toast lapsed" role="alert">
            <p>Apple signed you out. Your copy on this computer is still here.</p>
            <button className="primary" onClick={() => setPhase("signin")}>
              Sign In
            </button>
            <button className="quiet" onClick={() => setLapsed(false)}>
              Not Now
            </button>
          </div>
        )}
        <Library onSignOut={signOut} />
        <ImportSheet />
      </>
    );

  return (
    <div className="shell">
      <div className="app-ground" aria-hidden="true" />
      {phase === "checking" && <StatusLine tone="pending">Checking your session…</StatusLine>}
      {phase === "signin" && (
        <SignIn
          onCancel={lapsed ? () => setPhase("library") : undefined}
          onResult={(r) => {
            setLapsed(false);
            setPhase(r === "complete" ? "library" : "code");
          }}
        />
      )}
      {phase === "code" && (
        <CodeEntry onVerified={() => setPhase("library")} onCancel={() => setPhase("signin")} />
      )}
    </div>
  );
}
