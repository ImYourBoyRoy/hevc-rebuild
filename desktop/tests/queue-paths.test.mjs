// ./desktop/tests/queue-paths.test.mjs
// Characterization tests for the pure queue path helpers extracted from the
// route (slice 1 of the +page.svelte segregation). They pin the behavior the
// route had before the move, including Windows and network-path handling.
// Run: pnpm test:queue-paths

import assert from "node:assert/strict";
import test from "node:test";
import {
  archivedOriginalPath,
  fileName,
  normalizeDroppedPaths,
  pathIsWithin,
  queuePathKey,
  relativeQueuePath,
  remapQueuePath,
} from "../src/lib/queue-paths.ts";

test("normalizeDroppedPaths trims, drops blanks and non-strings, and de-duplicates", () => {
  assert.deepEqual(normalizeDroppedPaths([" /a/b.mkv ", "/a/b.mkv", "", 7, null, "/c.mkv"]), ["/a/b.mkv", "/c.mkv"]);
  assert.deepEqual(normalizeDroppedPaths("not-an-array"), []);
  assert.deepEqual(normalizeDroppedPaths(undefined), []);
});

test("fileName handles both separators and trailing separators", () => {
  assert.equal(fileName("/media/Movies/Film (2001).mkv"), "Film (2001).mkv");
  assert.equal(fileName("C:\\Media\\Film.mkv"), "Film.mkv");
  assert.equal(fileName("/media/Movies/"), "Movies");
  assert.equal(fileName(""), "");
});

test("archivedOriginalPath puts the source in an old/ sibling folder using the source separator", () => {
  assert.equal(archivedOriginalPath("/media/Film.mkv"), "/media/old/Film.mkv");
  assert.equal(archivedOriginalPath("C:\\Media\\Film.mkv"), "C:\\Media\\old\\Film.mkv");
  assert.equal(archivedOriginalPath("Film.mkv"), "old/Film.mkv");
});

test("pathIsWithin matches the directory itself and descendants only", () => {
  assert.equal(pathIsWithin("/media/tv", "/media/tv"), true);
  assert.equal(pathIsWithin("/media/tv", "/media/tv/Show/S01E01.mkv"), true);
  assert.equal(pathIsWithin("/media/tv", "/media/tv-extra/file.mkv"), false);
  assert.equal(pathIsWithin("/media/tv/", "/media/tv/file.mkv"), true);
  assert.equal(pathIsWithin("/media/tv", "/media/TV/file.mkv"), false, "POSIX paths are case-sensitive");
});

test("pathIsWithin is case-insensitive for Windows-style directories", () => {
  assert.equal(pathIsWithin("C:\\Media", "c:\\media\\Show\\E01.mkv"), true);
  assert.equal(pathIsWithin("C:\\Media", "C:\\Media-Other\\E01.mkv"), false);
});

test("queuePathKey normalizes separators, duplicate slashes and trailing slashes", () => {
  assert.equal(queuePathKey("/media//tv/"), "/media/tv");
  assert.equal(queuePathKey("/"), "/");
  assert.equal(queuePathKey("/media/TV"), "/media/TV", "POSIX keys keep their case");
});

test("queuePathKey lowercases drive and network paths and keeps the UNC prefix", () => {
  assert.equal(queuePathKey("C:\\Media\\Show\\"), "c:/media/show");
  assert.equal(queuePathKey("C:\\"), "c:/");
  assert.equal(queuePathKey("\\\\server\\share\\Folder"), "//server/share/folder");
});

test("remapQueuePath prefers an exact move, then the longest moved ancestor", () => {
  const moves = new Map([
    ["/media/a", "/media/renamed-a"],
    ["/media/a/b", "/media/renamed-b"],
    ["/media/x.mkv", "/media/y.mkv"],
  ]);
  assert.equal(remapQueuePath("/media/x.mkv", moves), "/media/y.mkv");
  assert.equal(remapQueuePath("/media/a/file.mkv", moves), "/media/renamed-a/file.mkv");
  assert.equal(remapQueuePath("/media/a/b/file.mkv", moves), "/media/renamed-b/file.mkv");
  assert.equal(remapQueuePath("/media/ab/file.mkv", moves), "/media/ab/file.mkv", "a sibling that shares a prefix is not an ancestor");
  assert.equal(remapQueuePath("/other/file.mkv", moves), "/other/file.mkv");
});

test("remapQueuePath compares Windows ancestors case-insensitively but keeps the original tail", () => {
  const moves = new Map([["C:\\Media\\Old", "C:\\Media\\New"]]);
  assert.equal(remapQueuePath("c:\\media\\old\\Ep 1.mkv", moves), "C:\\Media\\New\\Ep 1.mkv");
});

test("relativeQueuePath returns the file name for a single-file source and the tail for a folder", () => {
  assert.equal(relativeQueuePath("/media/Film.mkv", "/media/Film.mkv", "file", "prev"), "Film.mkv");
  assert.equal(relativeQueuePath("/media/tv", "/media/tv/Show/E01.mkv", "folder", "prev"), "Show/E01.mkv");
  assert.equal(relativeQueuePath("/media/tv/", "/media/tv/Show/E01.mkv", "folder", "prev"), "Show/E01.mkv");
  assert.equal(relativeQueuePath("/media/tv", "/elsewhere/E01.mkv", "folder", "prev"), "prev", "outside the root keeps the previous value");
  assert.equal(relativeQueuePath("C:\\TV", "c:\\tv\\Show\\E01.mkv", "folder", "prev"), "Show\\E01.mkv");
});
