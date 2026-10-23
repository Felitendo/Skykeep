// Which links Skykeep opens: a web page, an email, a call, a place. A shared note's, list's or
// invitation's link is someone else's choice, and anything else (a file on a share, a search
// query, a program's own scheme) is never handed to the computer.
import { test } from "node:test";
import assert from "node:assert/strict";
import { outsideHref, linkedNote } from "../src/lib/links.ts";

test("web pages, email, calls and places are opened", () => {
  assert.equal(outsideHref("https://example.org/agenda"), "https://example.org/agenda");
  assert.equal(outsideHref("http://example.org"), "http://example.org/");
  assert.equal(outsideHref("mailto:someone@example.org"), "mailto:someone@example.org");
  assert.equal(outsideHref("tel:+15550100"), "tel:+15550100");
  assert.equal(outsideHref("geo:0,0?q=Main%20Street"), "geo:0,0?q=Main%20Street");
  assert.equal(outsideHref("  HTTPS://EXAMPLE.ORG/ "), "https://example.org/");
});

test("every other kind of link is refused", () => {
  for (const href of [
    "file:///home/someone/x.desktop",
    "file://attacker.example@SSL/DavWWWRoot/a.lnk",
    "\\\\attacker.example\\share\\a.lnk",
    "search-ms:query=x&crumb=location:%5C%5Cattacker.example",
    "search:query=x",
    "ms-msdt:/id PCWDiagnostic",
    "smb://attacker.example/share",
    "javascript:alert(1)",
    "java\tscript:alert(1)",
    " javascript:alert(1)",
    "data:text/html,<h1>Sign in</h1>",
    "blob:tauri://localhost/0b1c",
    "quince://photo/ABC",
    "vbscript:msgbox(1)",
    "webcal://example.org/a.ics",
    "applenotes:note/0b1c2d3e-0000-0000-0000-000000000000",
    "",
    null,
    undefined,
  ]) {
    assert.equal(outsideHref(href), null, String(href));
  }
});

test("a link to another note is the app's own", () => {
  assert.equal(linkedNote("applenotes:note/0b1c2d3e-aaaa-bbbb-cccc-000000000000?ownerIdentifier=x"), "0B1C2D3E-AAAA-BBBB-CCCC-000000000000");
  assert.equal(linkedNote("https://example.org/note/0b1c2d3e-aaaa-bbbb-cccc-000000000000"), null);
});
