// ./desktop/tests/normalizers.test.mjs
// Characterization tests for the preference normalizers extracted from the route (slice 5).
// Run: pnpm test:normalizers

import assert from "node:assert/strict";
import test from "node:test";
import { normalizePage, normalizeTheme, normalizeToolPaths, pageLabel } from "../src/lib/normalizers.ts";

test("normalizeTheme keeps the five skins, upgrades the legacy hacker skin and defaults to light", () => {
  for (const theme of ["light", "dark", "phosphor", "ion", "breach"]) assert.equal(normalizeTheme(theme), theme);
  assert.equal(normalizeTheme("hacker"), "phosphor");
  for (const bad of ["", "Dark", "neon", null, undefined, 3, {}]) assert.equal(normalizeTheme(bad), "light", String(bad));
});

test("normalizePage maps the saved home page to library and rejects unknown pages", () => {
  assert.equal(normalizePage("home"), "library");
  for (const page of ["library", "metadata", "youtube", "run", "settings", "reports", "help"]) assert.equal(normalizePage(page), page);
  for (const bad of ["", "Library", "nope", null, undefined, 1]) assert.equal(normalizePage(bad), "library", String(bad));
});

test("pageLabel names every page and treats home as Library", () => {
  assert.deepEqual(
    ["home", "library", "metadata", "youtube", "run", "settings", "reports", "help"].map(pageLabel),
    ["Library", "Library", "Organize titles", "YouTube", "Run", "Settings", "Reports & errors", "Help & commands"],
  );
  assert.equal(pageLabel("unknown"), "Library");
});

test("normalizeToolPaths trims strings and turns blanks and non-strings into null", () => {
  assert.deepEqual(
    normalizeToolPaths({ ffmpeg: " /usr/bin/ffmpeg ", ffprobe: "", mediainfo: "   ", mkvpropedit: 5, mkvmerge: null, other: "x" }),
    { ffmpeg: "/usr/bin/ffmpeg", ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null },
  );
  const empty = { ffmpeg: null, ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null };
  for (const bad of [undefined, null, "text", 4]) assert.deepEqual(normalizeToolPaths(bad), empty, String(bad));
});

test("normalizeToolPaths applies the legacy ffmpeg path only when the key is absent", () => {
  assert.equal(normalizeToolPaths({}, " /legacy/ffmpeg ").ffmpeg, "/legacy/ffmpeg");
  assert.equal(normalizeToolPaths(undefined, "/legacy/ffmpeg").ffmpeg, "/legacy/ffmpeg");
  assert.equal(normalizeToolPaths({ ffmpeg: null }, "/legacy/ffmpeg").ffmpeg, null);
  assert.equal(normalizeToolPaths({ ffmpeg: "/new" }, "/legacy/ffmpeg").ffmpeg, "/new");
  assert.equal(normalizeToolPaths({}, 7).ffmpeg, null);
  assert.equal(normalizeToolPaths({}, "/legacy").ffprobe, null);
});
