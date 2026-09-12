import { useState } from "react";
import { Mark } from "../components/Mark";
import { CodeInput } from "../components/CodeInput";
import { api, messageOf } from "../lib/ipc";

export function CodeEntry({
  onVerified,
  onCancel,
}: {
  onVerified: () => void;
  onCancel: () => void;
}) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function verify(value: string) {
    setBusy(true);
    setError(null);
    try {
      await api.submitCode(value);
      onVerified();
    } catch (err) {
      setError(messageOf(err));
      setCode("");
      setBusy(false);
    }
  }

  return (
    <div className="sheet enter-sheet">
      <Mark className="mark" />
      <h1>Enter the code</h1>
      <p className="lede">Apple sent a six-digit code to your trusted devices.</p>

      <CodeInput value={code} onChange={setCode} onComplete={verify} disabled={busy} />

      {error && <p className="error" role="alert">{error}</p>}

      <p className="assurance">
        After this, Quince stays signed in. You won't be asked again unless Apple retires
        the session, which takes about two months.
      </p>

      <button className="quiet" type="button" onClick={onCancel} disabled={busy}>
        Use a different Apple ID
      </button>
    </div>
  );
}
