import { useEffect, useState } from "react";
import { SignIn } from "./screens/SignIn";
import { CodeEntry } from "./screens/CodeEntry";
import { Library } from "./screens/Library";
import { listen } from "@tauri-apps/api/event";
import { api, NEEDS_CODE } from "./lib/ipc";

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
        import("@tauri-apps/api/core")
          .then(({ invoke }) => invoke("window_ready"))
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

  async function signOut() {
    await api.signOut().catch(() => {});
    setPhase("signin");
  }

  // Filling the library is the library's own business now: one refresh covers every
  // service, runs the moment the window opens, and repeats on its own thereafter.

  if (phase === "library")
    return (
      <>
        <div className="app-ground" aria-hidden="true" />
        <Library onSignOut={signOut} />
      </>
    );

  return (
    <div className="shell">
      <div className="app-ground" aria-hidden="true" />
      {phase === "checking" && <span className="badge">Checking your session…</span>}
      {phase === "signin" && (
        <SignIn
          onResult={(r) => setPhase(r === "complete" ? "library" : "code")}
        />
      )}
      {phase === "code" && (
        <CodeEntry onVerified={() => setPhase("library")} onCancel={() => setPhase("signin")} />
      )}
    </div>
  );
}
