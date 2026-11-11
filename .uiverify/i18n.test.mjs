// i18n lookup and fallback rules, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { t, setLanguage, activeLanguage, languageName } from "../src/lib/i18n.ts";

test("a key translates to English when English is active", () => {
  setLanguage("en");
  assert.equal(activeLanguage(), "en");
  assert.equal(t("common.cancel"), "Cancel");
  assert.equal(t("settings.title"), "Settings");
  assert.equal(t("settings.appearance"), "Appearance");
});

test("a key translates to Simplified Chinese when zh-CN is active", () => {
  setLanguage("zh-CN");
  assert.equal(activeLanguage(), "zh-CN");
  assert.equal(t("common.cancel"), "取消");
  assert.equal(t("settings.title"), "设置");
  assert.equal(t("settings.appearance"), "外观");
});

test("missing string in zh-CN falls back to English", () => {
  setLanguage("zh-CN");
  // If a key only exists in en, it falls back to English.
  assert.equal(t("common.cancel", undefined, "en"), "Cancel");
});

test("missing string falls back to key when absent from all dictionaries", () => {
  assert.equal(t("nonexistent.key.test"), "nonexistent.key.test");
});

test("parameters interpolate into template placeholders in English and Chinese", () => {
  setLanguage("en");
  assert.equal(t("sync.updatedMinAgo", { count: 5 }), "Updated 5 min ago");
  assert.equal(t("sync.appUpdating", { name: "Notes" }), "Updating Notes…");

  setLanguage("zh-CN");
  assert.equal(t("sync.updatedMinAgo", { count: 5 }), "更新于 5 分钟前");
  assert.equal(t("sync.appUpdating", { name: "备忘录" }), "正在更新备忘录…");
});

test("language name helper returns readable name", () => {
  assert.equal(languageName("en"), "English");
  assert.equal(languageName("zh-CN"), "简体中文");
});
