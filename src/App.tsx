import { useEffect, useState } from "react";
import { SignIn } from "./screens/SignIn";
import { CodeEntry } from "./screens/CodeEntry";
import { Library } from "./screens/Library";
import { api } from "./lib/ipc";

type Phase = "checking" | "signin" | "code" | "library";

export default function App() {
  const [phase, setPhase] = useState<Phase>("checking");

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
        if (status.hasStoredSession && (await api.resume())) {
          return !cancelled && setPhase("library");
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

  async function signOut() {
    await api.signOut().catch(() => {});
    setPhase("signin");
  }

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
