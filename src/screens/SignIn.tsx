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
        Pomander keeps your iCloud on this computer, so it opens instantly and works offline.
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
        Your password never leaves this computer. Pomander proves you know it without sending
        it, and keeps it in your system keyring so it can sign itself back in. Signing out
        erases it.
      </p>
      {/*
        * What somebody is owed before they type an Apple ID into software a stranger wrote.
        *
        * Folded away rather than laid across the screen: a wall of warnings is not read, and
        * a page nobody reads discloses nothing. One line that says the important part, and
        * the rest a click away for anyone who wants it.
        */}
      <details className="disclosure">
        <summary>An independent project. Not affiliated with Apple.</summary>
        <div className="disclosure-body">
          <p>
            <strong>Unofficial.</strong> Pomander reaches iCloud through interfaces Apple has
            not published. Apple can change them without notice, and when that happens
            something here will stop working until it is put right.
          </p>
          <p>
            <strong>Your password never leaves this computer.</strong> Pomander proves you know
            it without sending it, and keeps the result in your system keyring. There is no
            Pomander account and no Pomander server — there is nowhere else for any of this to go.
          </p>
          <p>
            <strong>What it keeps, and where.</strong> A copy of your calendar, reminders,
            notes, contacts and the listing of your Drive, in a database under{" "}
            <code>~/.local/share/dev.quince.client</code>. Signing out removes the session;
            deleting that folder removes the copy.
          </p>
          <p>
            <strong>What no client can read.</strong> Health, Messages, Keychain, Home, Safari
            history and Screen Time are encrypted end to end — Apple's own servers hold no key
            to them, so no app can show them. Pomander says they are unavailable rather than
            quietly leaving them out.
          </p>
        </div>
      </details>
    </form>
  );
}
