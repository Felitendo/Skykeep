// Every third-party step a build runs is pinned to one commit, never a tag or a branch that
// can be moved under it: a moved tag runs someone else's code on the machine that builds the
// release. A local workflow (`./.github/…`) is this repository's own.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";

const dir = new URL("../.github/workflows/", import.meta.url);

test("every action a workflow uses is pinned to a full commit", () => {
  const loose = [];
  for (const file of readdirSync(dir).filter((f) => /\.ya?ml$/.test(f))) {
    for (const [i, line] of readFileSync(new URL(file, dir), "utf8").split("\n").entries()) {
      const m = /^\s*-?\s*uses:\s*([^\s#]+)\s*(#\s*(\S+))?/.exec(line);
      if (!m || m[1].startsWith("./")) continue;
      if (!/@[0-9a-f]{40}$/.test(m[1]) || !m[3]) loose.push(`${file}:${i + 1} ${m[1]}`);
    }
  }
  assert.deepEqual(loose, []);
});
