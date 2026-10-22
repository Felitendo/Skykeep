// The Join button's link: found by each service's own host and path, never by a word, and
// never on a look-alike host.
import { test } from "node:test";
import assert from "node:assert/strict";
import { meetingOf } from "../src/screens/calendar/model.ts";

test("each service's join link is found where calendars put it", () => {
  assert.deepEqual(meetingOf({ url: "https://us02web.zoom.us/j/8123456789?pwd=abc" }), { href: "https://us02web.zoom.us/j/8123456789?pwd=abc", service: "Zoom" });
  assert.equal(meetingOf({ location: "https://meet.google.com/abc-defg-hij" })?.service, "Google Meet");
  assert.equal(meetingOf({ notes: "Join: https://teams.microsoft.com/l/meetup-join/19%3ameeting_x/0?context=y" })?.service, "Microsoft Teams");
  assert.equal(meetingOf({ notes: "https://acme.webex.com/meet/jo" })?.service, "Webex");
  assert.equal(meetingOf({ url: "https://facetime.apple.com/join#v=1&p=AbC" })?.service, "FaceTime");
});

test("the URL field comes first, then the location, then the notes", () => {
  const e = { url: "https://meet.google.com/aaa-bbbb-ccc", location: "https://zoom.us/j/1", notes: "https://zoom.us/j/2" };
  assert.equal(meetingOf(e)?.service, "Google Meet");
  assert.equal(meetingOf({ location: "Room 4", notes: "dial in https://zoom.us/j/99. Thanks!" })?.href, "https://zoom.us/j/99");
});

test("look-alikes, plain words and other links make no button", () => {
  for (const text of [
    "https://zoom.us.evil.example/j/1",
    "https://notzoom.us/j/1",
    "https://meet.google.com.example.org/abc-defg-hij",
    "http://zoom.us/j/1",
    "Let's zoom in on the plan",
    "https://zoom.us/pricing",
    "https://meet.google.com/",
    "https://example.com/teams.microsoft.com/l/meetup-join/x",
    "",
  ]) {
    assert.equal(meetingOf({ notes: text }), null, text);
  }
  assert.equal(meetingOf({}), null);
  assert.equal(meetingOf({ url: null, location: null, notes: null }), null);
});

test("trailing punctuation and brackets are left off", () => {
  assert.equal(meetingOf({ notes: "(https://zoom.us/j/123)." })?.href, "https://zoom.us/j/123");
  assert.equal(meetingOf({ notes: "link: https://meet.google.com/abc-defg-hij!" })?.href, "https://meet.google.com/abc-defg-hij");
});
