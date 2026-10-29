// The name a pasted copy shows under while it is made: Finder's, the same rule as the backend's
// copy_name (crates/quince-icloud/src/drive.rs), which names the real one.
import { test } from "node:test";
import assert from "node:assert/strict";
import { copyName } from "../src/lib/clipboard.ts";

test("its own name where that is free", () => {
  assert.equal(copyName("Report", "pdf", []), "Report.pdf");
  assert.equal(copyName("Trip", null, ["Report.pdf"]), "Trip");
});

test("then “copy”, then “copy 2”, whatever the case of what is there", () => {
  assert.equal(copyName("Report", "pdf", ["Report.pdf"]), "Report copy.pdf");
  assert.equal(copyName("Report", "pdf", ["report.PDF", "Report copy.pdf"]), "Report copy 2.pdf");
  assert.equal(copyName("Report", "pdf", ["Report.pdf", "Report copy.pdf", "Report copy 2.pdf"]), "Report copy 3.pdf");
  assert.equal(copyName("Trip", null, ["Trip"]), "Trip copy");
});

test("a copy of a copy counts on, never “copy copy”", () => {
  assert.equal(copyName("Report copy", "pdf", ["Report copy.pdf"]), "Report copy 2.pdf");
  assert.equal(copyName("Report copy 2", "pdf", ["Report copy.pdf", "Report copy 2.pdf"]), "Report copy 3.pdf");
  assert.equal(copyName("copy", "txt", ["copy.txt"]), "copy copy.txt");
  assert.equal(copyName("Hard copy 2b", null, ["Hard copy 2b"]), "Hard copy 2b copy");
});
