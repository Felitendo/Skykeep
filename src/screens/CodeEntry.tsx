import { useEffect, useState } from "react";
import { Mark } from "../components/Mark";
import { CodeInput } from "../components/CodeInput";
import { api, messageOf, type TwoFactorOptions } from "../lib/ipc";

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
  const [notice, setNotice] = useState<string | null>(null);
  const [options, setOptions] = useState<TwoFactorOptions | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  /** Set once a code has been texted, so it verifies through the SMS endpoint instead. */
  const [smsPhoneId, setSmsPhoneId] = useState<number | null>(null);

  useEffect(() => {
    api.twoFactorOptions().then(setOptions).catch(() => setOptions(null));
  }, []);

  // If Apple pushed to nothing, say so immediately rather than letting someone wait.
  const nothingSent = options?.trustedDeviceCount === 0;
  /**
   * No device to prompt and no number to text.
   *
   * Rare, and real — an account whose trusted number was removed, or one reached from a place
   * its devices are not. Without saying so the screen offers a code that can never arrive and
   * no way out of it, which is a dead end with a text box in the middle.
   */
  const nowhereToSend = nothingSent && options.phones.length === 0;
  useEffect(() => {
    if (nothingSent) setShowHelp(true);
  }, [nothingSent]);

  async function verify(value: string) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      if (smsPhoneId !== null) await api.submitSmsCode(smsPhoneId, value);
      else await api.submitCode(value);
      onVerified();
    } catch (err) {
      setError(messageOf(err));
      setCode("");
      setBusy(false);
    }
  }

  async function run(action: () => Promise<void>, done: string) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await action();
      setNotice(done);
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="sheet enter-sheet">
      <Mark className="mark" />
      <h1>Enter the code</h1>
      <p className="lede">
        {options === null
          ? "Apple sent a six-digit code to your trusted devices."
          : nowhereToSend
            ? "Apple has nowhere to send a code for this account."
            : nothingSent
              ? "None of your devices are reachable, but a code can still be texted to you."
              : `Apple sent a six-digit code to ${options.trustedDeviceCount} of your devices.`}
      </p>

      <CodeInput value={code} onChange={setCode} onComplete={verify} disabled={busy} />

      {error && <p className="error" role="alert">{error}</p>}
      {notice && <p className="notice" role="status">{notice}</p>}

      {!showHelp && (
        <button className="quiet" type="button" onClick={() => setShowHelp(true)}>
          Didn't get a code?
        </button>
      )}

      {showHelp && (
        <div className="help">
          {options?.tooManyCodesSent && (
            <p className="error">
              Apple is limiting new codes. Wait a few minutes — repeated tries risk a
              temporary lock.
            </p>
          )}

          {nowhereToSend ? (
            <p>
              This account has no trusted device and no trusted phone number, so there is
              nothing here that can produce a code. Sign in on a device you still have, or add
              a trusted number at <strong>account.apple.com</strong>, and then come back.
            </p>
          ) : (
            <p>
              The code is only created once you tap <strong>Allow</strong> on your iPhone. If
              no prompt appeared, no code exists yet.
            </p>
          )}

          <div className="help-actions">
            {!nothingSent && (
              <button
                className="quiet"
                type="button"
                disabled={busy}
                onClick={() => run(api.resendCode, "Sent again — check your devices.")}
              >
                Send to my devices again
              </button>
            )}
            {options?.phones.map((phone) => (
              <button
                key={phone.id}
                className="quiet"
                type="button"
                disabled={busy}
                onClick={() =>
                  run(async () => {
                    await api.sendSms(phone.id);
                    setSmsPhoneId(phone.id);
                  }, `Texted ${phone.maskedNumber}.`)
                }
              >
                Text {phone.maskedNumber}
              </button>
            ))}
          </div>

          <p>
            Still nothing? On iPhone, turn on <strong>Airplane Mode</strong>, then open
            Settings → your name → Sign-In &amp; Security. A popup offers a code while the
            phone is offline. (iOS hides that option when the phone is online.)
          </p>
        </div>
      )}

      <p className="assurance">
        After this, Pomander stays signed in. You won't be asked again unless Apple retires the
        session, which takes about two months.
      </p>

      <button className="quiet" type="button" onClick={onCancel} disabled={busy}>
        Use a different Apple ID
      </button>
    </div>
  );
}
