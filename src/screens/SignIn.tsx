import { useState } from "react";
import { Mark } from "../components/Mark";
import { api, messageOf, type SignInResult } from "../lib/ipc";

export function SignIn({ onResult }: { onResult: (r: SignInResult) => void }) {
  const [appleId, setAppleId] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const ready = appleId.trim().length > 0 && password.length > 0 && !busy;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      onResult(await api.signIn(appleId.trim(), password));
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setPassword("");
      setBusy(false);
    }
  }

  return (
    <form className="sheet enter-sheet" onSubmit={submit}>
      <Mark className="mark" />
      <h1>Sign in with your Apple ID</h1>
      <p className="lede">
        Quince keeps your iCloud on this computer, so it opens instantly and works offline.
      </p>

      <div className="field-group">
        <input
          value={appleId}
          onChange={(e) => setAppleId(e.target.value)}
          type="email"
          autoComplete="username"
          placeholder="Apple ID"
          aria-label="Apple ID"
          autoFocus
          disabled={busy}
        />
        <input
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          type="password"
          autoComplete="current-password"
          placeholder="Password"
          aria-label="Password"
          disabled={busy}
        />
      </div>

      <button className="primary" type="submit" disabled={!ready}>
        {busy ? "Signing in…" : "Sign in"}
      </button>

      {error && <p className="error" role="alert">{error}</p>}

      <p className="assurance">
        Your password never leaves this computer. Quince proves you know it without sending
        it, and keeps the result in your system keyring.
      </p>
      <p className="disclaimer">An independent project. Not affiliated with Apple.</p>
    </form>
  );
}
