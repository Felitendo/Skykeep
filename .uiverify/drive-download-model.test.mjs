// How a download into Downloads is counted for the tray: one line for a whole folder, the
// words it shows at each step, what can be gone on with, and a hundred thousand files counted
// without a row each.
import { test } from "node:test";
import assert from "node:assert/strict";
import { NAMED, apply, finish, heading, newLine, pausedLine, view } from "../src/lib/downloadModel.ts";

const ev = (id, state, extra = {}) => ({ id: `b:${id}`, batch: "b", name: `${id}.jpg`, within: "Photos", sent: 0, total: 100, state, folder: false, error: null, ...extra });
const tally = (state, files, bytes) => ({ id: `b:${state}`, batch: "b", name: "", within: "", sent: files, total: bytes, state, folder: true, error: null });
const report = (extra = {}) => ({ key: "k1", saved: ["/home/me/Downloads/Photos"], failed: [], moreFailed: 0, cancelled: false, problem: null, resumable: false, ...extra });
const items = [{ drivewsid: "FOLDER::z::photos", name: "Photos", kind: "FOLDER" }];

test("a folder is one line: counted while it is read, then to a total that will not move", () => {
  const line = newLine("b", "Photos", items);
  assert.equal(view(line).status, "Preparing…");
  apply(line, tally("found", 2, 200));
  assert.equal(view(line).status, "Preparing… 2 found");
  assert.equal(view(line).percent, null, "no total yet, so no percent");
  apply(line, tally("planned", 2, 2_000_000));
  apply(line, ev("big", "downloading", { sent: 500_000, total: 1_000_000 }));
  assert.equal(view(line).status, "0 of 2");
  assert.equal(view(line).percent, 25);
  apply(line, ev("big", "done", { sent: 1_000_000 }));
  apply(line, ev("small", "done", { sent: 1_000_000 }));
  assert.equal(view(line).status, "2 of 2");
  assert.equal(view(line).percent, 100);
  assert.equal(heading([view(line)]), "Downloading Photos");
  finish(line, report(), null);
  const v = view(line);
  assert.equal(v.status, "Downloaded");
  assert.equal(v.tone, "done");
  assert.equal(v.reveal, "/home/me/Downloads/Photos");
  assert.equal(v.again, null, "nothing to go on with");
  assert.equal(heading([v]), "Photos downloaded");
});

test("a late report for a finished file changes nothing", () => {
  const line = newLine("b", "Photos", items);
  apply(line, ev(0, "downloading", { sent: 90 }));
  apply(line, ev(0, "done", { sent: 100 }));
  apply(line, ev(0, "downloading", { sent: 10 }));
  assert.equal(line.done, 1);
  assert.equal(line.inflight.size, 0);
  assert.equal(line.bytesDone, 100);
});

test("what did not download is named, with why; the rest are counted", () => {
  const line = newLine("b", "3 items", items);
  apply(line, ev(0, "failed", { error: "its path is too long for this computer" }));
  apply(line, { ...ev("x", "failed", { error: "iCloud Drive is busy" }), id: "b:x:0", folder: true, name: "Broken", within: "Photos" });
  finish(line, report({ resumable: true }), null);
  const v = view(line);
  assert.equal(v.status, "2 didn’t download");
  assert.equal(v.tone, "failed");
  assert.deepEqual(v.failures.map((f) => f.path), ["Photos/0.jpg", "Photos/Broken"]);
  assert.equal(v.again, "Resume", "it can be gone on with");
  assert.equal(v.forgettable, "k1");
  assert.equal(heading([v]), "Some downloads didn’t finish");
  const many = newLine("c", "Big", items);
  for (let i = 0; i < NAMED + 50; i++) apply(many, { ...ev(i, "failed", { error: "no" }), batch: "c" });
  finish(many, report(), null);
  assert.equal(view(many).failures.length, NAMED);
  assert.equal(view(many).unnamed, 50);
});

test("cancelled, out of room, connection lost, sign-in lapsed: each said as itself, and offered again", () => {
  const cancelled = newLine("b", "Photos", items);
  apply(cancelled, ev(0, "done", { sent: 100 }));
  finish(cancelled, report({ cancelled: true, resumable: true }), null);
  assert.equal(view(cancelled).status, "Cancelled after 1");
  assert.equal(view(cancelled).again, "Resume");
  assert.equal(heading([view(cancelled)]), "Download cancelled");

  const full = newLine("b", "Photos", items);
  finish(full, report({ saved: [], problem: "There isn’t room in Downloads for this: it needs 3.2 GB and 1.1 GB is free." }), null);
  assert.equal(view(full).status, "Stopped");
  assert.match(view(full).note, /isn’t room/);
  assert.equal(view(full).reveal, null, "nothing to show");
  assert.equal(view(full).again, "Try Again", "nothing to resume, so it starts again");

  const lost = newLine("b", "Photos", items);
  apply(lost, ev(0, "done", { sent: 100 }));
  finish(lost, report({ problem: "The connection to iCloud was lost, so the download paused.", resumable: true }), null);
  assert.equal(view(lost).status, "1 downloaded");
  assert.equal(view(lost).again, "Resume");

  const lapsed = newLine("b", "Photos", items);
  finish(lapsed, null, "this needs a fresh sign-in");
  assert.equal(view(lapsed).status, "Didn’t download");
  assert.equal(view(lapsed).note, "this needs a fresh sign-in");
});

test("a download left unfinished last time is offered again, with what of it is there", () => {
  const v = view(pausedLine("k9", "Archive", items, ["/home/me/Downloads/Archive"]));
  assert.equal(v.status, "Paused");
  assert.equal(v.again, "Resume");
  assert.equal(v.reveal, "/home/me/Downloads/Archive");
  assert.equal(v.forgettable, "k9");
  assert.equal(v.running, false);
  assert.equal(heading([v]), "A download didn’t finish");
});

test("a hundred thousand files are counts, not rows", () => {
  const line = newLine("b", "Archive", items);
  apply(line, tally("planned", 100_000, 100_000));
  for (let i = 0; i < 100_000; i++) apply(line, ev(i, "done", { sent: 1, total: 1 }));
  assert.equal(line.inflight.size, 0);
  assert.equal(line.ended.size, 0, "small files leave nothing behind");
  assert.equal(line.done, 100_000);
  assert.equal(view(line).percent, 100);
});
