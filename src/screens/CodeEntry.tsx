import { useEffect, useState } from "react";
import { Mark } from "../components/Mark";
import { CodeInput } from "../components/CodeInput";
import { StatusLine } from "../components/StatusLine";
import { sentence } from "./SignIn";
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
  const [verifying, setVerifying] = useState(false);
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

  const texted = options?.phones.find((p) => p.id === smsPhoneId);
  const devices = options?.trustedDeviceCount ?? 0;

  async function verify(value: string) {
    setBusy(true);
    setVerifying(true);
    setError(null);
    setNotice(null);
    try {
      if (smsPhoneId !== null) await api.submitSmsCode(smsPhoneId, value);
      else await api.submitCode(value);
      onVerified();
    } catch (err) {
      setError(sentence(messageOf(err)));
      setCode("");
      setBusy(false);
      setVerifying(false);
    }
  }

  async function run(action: () => Promise<void>, done: string) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await action();
      setNotice(done || null);
    } catch (err) {
      setError(sentence(messageOf(err)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="sheet enter-sheet" aria-busy={verifying}>
      <Mark className="mark" />
      <h1>Enter the code</h1>
      <p className="lede">
        {texted
          ? `Apple texted a code to ${texted.maskedNumber}.`
          : options === null
            ? "Apple sent a code to your devices."
            : nowhereToSend
              ? "Apple has nowhere to send a code for this account."
              : nothingSent
                ? "No device is reachable, but a code can be texted to you."
                : `Apple sent a code to ${devices === 1 ? "one of your devices" : `${devices} of your devices`}.`}
      </p>

      <CodeInput
        value={code}
        onChange={(next) => {
          setCode(next);
          if (error) setError(null);
        }}
        onComplete={verify}
        disabled={busy}
        invalid={!!error}
      />

      {/* One line under the boxes whose height is kept, so checking, a wrong code and a
          sent text all take the same place and nothing below moves. */}
      <div className="sheet-status">
        {verifying ? (
          <StatusLine tone="pending">Checking</StatusLine>
        ) : error ? (
          <StatusLine tone="error">{error}</StatusLine>
        ) : notice ? (
          <StatusLine tone="note">{notice}</StatusLine>
        ) : null}
      </div>

      {!showHelp && (
        <button
          className="link"
          type="button"
          // The button goes as the help opens; focus moves to the first way to get a code
          // rather than falling back to the page.
          onClick={() => {
            setShowHelp(true);
            requestAnimationFrame(() =>
              document.querySelector<HTMLButtonElement>(".help button:not(:disabled)")?.focus(),
            );
          }}
        >
          Didn't get a code?
        </button>
      )}

      {showHelp && (
        <div className="help">
          {options?.tooManyCodesSent && (
            <p className="help-warn">
              Apple is limiting new codes. Wait a few minutes before asking again.
            </p>
          )}

          {nowhereToSend ? (
            <p>
              This account has no device or number to send to. Use a device you're still
              signed in on, or add a number at <strong>account.apple.com</strong>.
            </p>
          ) : nothingSent ? null : (
            <p>
              The code appears once you tap <strong>Allow</strong> on your iPhone.
            </p>
          )}

          {!nowhereToSend && (
            <div className="help-actions">
              {!nothingSent && (
                <button
                  className="quiet"
                  type="button"
                  disabled={busy}
                  onClick={() => run(api.resendCode, "Sent again. Check your devices.")}
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
                    // The heading says where it went; the help folds away again.
                    run(async () => {
                      await api.sendSms(phone.id);
                      setSmsPhoneId(phone.id);
                      setShowHelp(false);
                    }, "")
                  }
                >
                  Text {phone.maskedNumber}
                </button>
              ))}
            </div>
          )}

          <p className="help-tip">
            No prompt? On iPhone, turn on <strong>Airplane Mode</strong>, then open Settings →
            your name → Sign-In &amp; Security. iOS offers a code there only while offline.
          </p>
        </div>
      )}

      <div className="sheet-foot">
        <p className="assurance">You'll stay signed in on this computer.</p>
        <button className="link" type="button" onClick={onCancel} disabled={verifying}>
          Use a different Apple ID
        </button>
      </div>
    </div>
  );
}
