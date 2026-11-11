import { useEffect, useState } from "react";
import { Mark } from "../components/Mark";
import { CodeInput } from "../components/CodeInput";
import { StatusLine } from "../components/StatusLine";
import { sentence } from "./SignIn";
import { api, messageOf, type TwoFactorOptions } from "../lib/ipc";
import { rich, useI18n } from "../lib/i18n";

export function CodeEntry({
  onVerified,
  onCancel,
  request,
}: {
  onVerified: () => void;
  onCancel: () => void;
  /**
   * How the code is asked for, when Skykeep signed itself back in and Apple wants one: `now`,
   * the person came here to get it; `ask`, they did not (the window opened with Skykeep), so
   * it is theirs to send. Absent, the sign-in a moment ago already asked.
   */
  request?: "now" | "ask";
}) {
  const { t } = useI18n();
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

  /** Apple has been asked for this sign-in's code. */
  const [asked, setAsked] = useState(request === undefined);
  const askForCode = () =>
    run(async () => {
      await api.requestCode();
      setAsked(true);
    }, "");
  useEffect(() => {
    if (request === "now") void askForCode();
    // Once, as the screen opens.
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
  /** A number Apple calls with the code rather than texting it (a landline). */
  const calls = (phone: { mode?: string }) => phone.mode === "voice";
  const onlyCalls = !!options && options.phones.length > 0 && options.phones.every(calls);
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

  // An account that signs in with a security key gets no code: Apple asks for the key. Said
  // plainly, rather than a code screen waiting for something that never comes.
  if (options?.securityKey) {
    return (
      <div className="sheet enter-sheet">
        <Mark className="mark" />
        <h1>{t("codeEntry.securityKeyTitle")}</h1>
        <p className="lede">{t("codeEntry.securityKeyLede")}</p>
        <div className="sheet-foot">
          <button className="link" type="button" onClick={onCancel}>
            {t("codeEntry.useDifferentAccount")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="sheet enter-sheet" aria-busy={verifying}>
      <Mark className="mark" />
      <h1>{t("codeEntry.enterCode")}</h1>
      <p className="lede">
        {!asked && !texted
          ? t("codeEntry.ledeLapse")
          : texted
          ? calls(texted)
            ? t("codeEntry.ledeCalling", { number: texted.maskedNumber })
            : t("codeEntry.ledeTexted", { number: texted.maskedNumber })
          : options === null
            ? t("codeEntry.ledeDevices")
            : nowhereToSend
              ? t("codeEntry.ledeNowhere")
              : nothingSent
                ? onlyCalls
                  ? t("codeEntry.ledeOnlyCalls")
                  : t("codeEntry.ledeCanText")
                : devices === 1 ? t("codeEntry.ledeOneDevice") : t("codeEntry.ledeManyDevices", { devices })}
      </p>

      {!asked && !texted && (
        <button className="primary" type="button" disabled={busy} onClick={() => void askForCode()}>
          {t("codeEntry.sendCode")}
        </button>
      )}

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
          <StatusLine tone="pending">{t("codeEntry.checking")}</StatusLine>
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
          {t("codeEntry.didntGetCode")}
        </button>
      )}

      {showHelp && (
        <div className="help">
          {options?.tooManyCodesSent && (
            <p className="help-warn">
              {t("codeEntry.tooManyCodes")}
            </p>
          )}

          {nowhereToSend ? (
            <p>{rich(t("codeEntry.nowhereHelp"))}</p>
          ) : nothingSent ? null : (
            <p>{rich(t("codeEntry.allowHelp"))}</p>
          )}

          {!nowhereToSend && (
            <div className="help-actions">
              {!nothingSent && (
                <button
                  className="quiet"
                  type="button"
                  disabled={busy}
                  // Back on the devices: the code they show is checked as theirs, not as the
                  // number's it was texted to before — that was refused as wrong every time.
                  onClick={() =>
                    run(async () => {
                      await api.resendCode();
                      setSmsPhoneId(null);
                    }, t("codeEntry.sentAgainNotice"))
                  }
                >
                  {t("codeEntry.sendToDevicesAgain")}
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
                  {calls(phone)
                    ? t("codeEntry.callNumber", { number: phone.maskedNumber })
                    : t("codeEntry.textNumber", { number: phone.maskedNumber })}
                </button>
              ))}
            </div>
          )}

          <p className="help-tip">
            {rich(t("codeEntry.offlineTip"))}
          </p>
        </div>
      )}

      <div className="sheet-foot">
        <p className="assurance">{t("codeEntry.staySignedInAssurance")}</p>
        <button className="link" type="button" onClick={onCancel} disabled={verifying}>
          {t("codeEntry.useDifferentAccount")}
        </button>
      </div>
    </div>
  );
}
