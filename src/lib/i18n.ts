import { useSyncExternalStore } from "react";
import en from "../locales/en.json" with { type: "json" };
import zhCN from "../locales/zh-CN.json" with { type: "json" };

export type LanguageChoice = "auto" | "en" | "zh-CN";
export type ResolvedLanguage = "en" | "zh-CN";

const KEY = "quince.language";

const dictionaries: Record<ResolvedLanguage, Record<string, unknown>> = {
  en,
  "zh-CN": zhCN,
};

const listeners = new Set<() => void>();

function notify() {
  listeners.forEach((l) => l());
}

/** Subscribe to language changes (compatible with useSyncExternalStore). */
export function subscribeLanguage(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Detect the system's preferred language. */
export function systemLanguage(): ResolvedLanguage {
  try {
    const nav = (typeof navigator !== "undefined" && (navigator.languages?.[0] || navigator.language)) || "en";
    return nav.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
  } catch {
    return "en";
  }
}

let memoryLanguage: LanguageChoice | null = null;

/** The language choice saved in local storage. Defaults to "auto". */
export function storedLanguage(): LanguageChoice {
  if (memoryLanguage !== null) return memoryLanguage;
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      const val = window.localStorage.getItem(KEY);
      return val === "en" || val === "zh-CN" ? val : "auto";
    }
  } catch {
    // Fall back to auto.
  }
  return "auto";
}

/** The currently active language ("en" or "zh-CN"). */
export function activeLanguage(): ResolvedLanguage {
  const choice = storedLanguage();
  return choice === "auto" ? systemLanguage() : choice;
}

/** Set the HTML document's lang attribute. */
export function applyLanguage(lang: ResolvedLanguage) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = lang;
  }
}

/** Save the user's language choice and notify listeners. */
export function setLanguage(choice: LanguageChoice) {
  memoryLanguage = choice;
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      if (choice === "auto") {
        window.localStorage.removeItem(KEY);
      } else {
        window.localStorage.setItem(KEY, choice);
      }
    }
  } catch {
    // Ignore storage errors.
  }
  applyLanguage(activeLanguage());
  notify();
}

/**
 * Follow language changes from other windows, mirroring followTheme().
 */
export function followLanguage() {
  if (typeof window === "undefined") return;
  window.addEventListener("storage", (e) => {
    if (e.key === KEY || e.key === null) {
      memoryLanguage = null;
      applyLanguage(activeLanguage());
      notify();
    }
  });
}

/** Human-readable native name for a resolved language. */
export function languageName(lang: ResolvedLanguage): string {
  switch (lang) {
    case "zh-CN":
      return "简体中文";
    case "en":
    default:
      return "English";
  }
}

function getNested(obj: unknown, path: string): string | undefined {
  if (!obj || typeof obj !== "object") return undefined;
  const direct = (obj as Record<string, unknown>)[path];
  if (typeof direct === "string") return direct;
  const parts = path.split(".");
  let curr: unknown = obj;
  for (const p of parts) {
    if (curr === null || curr === undefined || typeof curr !== "object") return undefined;
    curr = (curr as Record<string, unknown>)[p];
  }
  return typeof curr === "string" ? curr : undefined;
}

/**
 * Translate a key into the active language, falling back to English.
 * Supports `{param}` interpolation. An optional `langOverride` can specify
 * the target language directly.
 */
export function t(key: string, params?: Record<string, string | number>, langOverride?: ResolvedLanguage): string {
  const lang = langOverride ?? activeLanguage();
  let text = getNested(dictionaries[lang], key);
  if (text === undefined && lang !== "en") {
    text = getNested(dictionaries.en, key);
  }
  if (text === undefined) {
    text = key;
  }
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (_, k) => (k in params ? String(params[k]) : `{${k}}`));
}

/**
 * React hook for language state and translation.
 * Re-renders automatically when the language is changed.
 */
export function useI18n() {
  const active = useSyncExternalStore(subscribeLanguage, activeLanguage, activeLanguage);
  return {
    t,
    lang: active,
    choice: storedLanguage(),
    setLanguage,
  };
}
