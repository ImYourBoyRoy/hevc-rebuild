// ./desktop/tests/browser-preferences.test.mjs
// Characterization tests for the browser-preview preference parser extracted from the route
// (window-state slice A). It must accept older saved shapes, ignore invalid fields one by one,
// and never throw. Run: pnpm test:browser-preferences

import assert from "node:assert/strict";
import test from "node:test";
import { parseBrowserPreferences } from "../src/lib/browser-preferences.ts";

const KEY = "prefs";
const defaults = {
  filenameExcludePatterns: ["old", ".*", "extras"],
  organizerTemplates: { movie: "M", series: "S", season: "Se", episode: "E" },
};
const base = { metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false } };

function parse(stored, legacy = {}) {
  const items = { ...(stored === undefined ? {} : { [KEY]: typeof stored === "string" ? stored : JSON.stringify(stored) }), ...legacy };
  return parseBrowserPreferences({ key: KEY, readItem: (key) => (key in items ? items[key] : null), base, defaults });
}

test("nothing stored gives an empty result", () => {
  assert.deepEqual(parse(undefined), { saved: {}, legacyTheme: false });
});

test("invalid JSON, non-objects and throwing storage never throw and give nothing", () => {
  assert.deepEqual(parse("{oops"), { saved: {}, legacyTheme: false });
  assert.deepEqual(parse("42"), { saved: {}, legacyTheme: false });
  assert.deepEqual(parse("null"), { saved: {}, legacyTheme: false });
  const result = parseBrowserPreferences({ key: KEY, readItem: () => { throw new Error("blocked"); }, base, defaults });
  assert.deepEqual(result, { saved: {}, legacyTheme: false });
});

test("theme and page are normalized, with the legacy hacker skin flagged for rewrite", () => {
  assert.deepEqual(parse({ theme: "dark", lastPage: "settings" }).saved, { theme: "dark", lastPage: "settings" });
  const legacy = parse({ theme: "hacker", lastPage: "home" });
  assert.deepEqual([legacy.saved.theme, legacy.saved.lastPage, legacy.legacyTheme], ["phosphor", "library", true]);
  assert.deepEqual(parse({ theme: "neon", lastPage: "nowhere" }).saved, { theme: "light", lastPage: "library" });
  assert.deepEqual(parse({ theme: 3, lastPage: 3 }).saved, {});
});

test("root accepts a string or null, simple flags need the right type", () => {
  assert.deepEqual(parse({ root: "/m", recursive: false, minimumSizeMib: "9", stopOnError: true }).saved, { root: "/m", recursive: false, minimumSizeMib: "9", stopOnError: true });
  assert.deepEqual(parse({ root: null }).saved, { root: null });
  assert.deepEqual(parse({ root: 5, recursive: "yes", minimumSizeMib: 9, stopOnError: 1 }).saved, {});
});

test("tool paths are normalized, with the single legacy ffmpeg path as a fallback", () => {
  assert.deepEqual(parse({ toolPaths: { ffmpeg: " /f ", mkvmerge: "" } }).saved.toolPaths, { ffmpeg: "/f", ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null });
  assert.equal(parse({ toolPaths: { ffprobe: "/p" }, ffmpegPath: "/legacy" }).saved.toolPaths.ffmpeg, "/legacy");
  assert.equal(parse({ ffmpegPath: " /legacy " }).saved.toolPaths.ffmpeg, "/legacy");
  assert.equal(parse({ ffmpegPath: null }).saved.toolPaths.ffmpeg, null);
  assert.equal("toolPaths" in parse({ ffmpegPath: 5 }).saved, false);
});

test("a source-resolution default without a quality mode falls back to 1440p; valid ones are kept", () => {
  assert.equal(parse({ youtubeResolution: "source" }).saved.youtubeResolution, "1440p");
  assert.equal(parse({ youtubeResolution: "source", youtubeQualityMode: "balanced" }).saved.youtubeResolution, "source");
  assert.equal(parse({ youtubeResolution: "4k" }).saved.youtubeResolution, "4k");
  assert.equal("youtubeResolution" in parse({ youtubeResolution: "8k" }).saved, false);
});

test("YouTube settings accept only their known values and a bounded multiplier", () => {
  const good = { youtubeQualityMode: "recommended", youtubeSourceMultiplier: 4, youtubeMode: "optimize", youtubeSdrCodec: "hevc", youtubePreset: "slower", youtubeShrink: "aggressive", youtubeQualityCheck: "strict", youtubeOutputDir: "/yt" };
  assert.deepEqual(parse(good).saved, good);
  for (const bad of [{ youtubeSourceMultiplier: 0.4 }, { youtubeSourceMultiplier: 4.1 }, { youtubeSourceMultiplier: Number.NaN }, { youtubeSourceMultiplier: "2" }, { youtubeMode: "x" }, { youtubeSdrCodec: "av1" }, { youtubePreset: "fast" }, { youtubeShrink: "x" }, { youtubeQualityCheck: "x" }, { youtubeQualityMode: "x" }, { youtubeOutputDir: 5 }]) {
    assert.deepEqual(parse(bad).saved, {}, JSON.stringify(bad));
  }
  assert.equal(parse({ youtubeSourceMultiplier: 0.5 }).saved.youtubeSourceMultiplier, 0.5);
  assert.equal(parse({ youtubeOutputDir: null }).saved.youtubeOutputDir, null);
});

test("output folders keep only string values and at most 512 per-file entries", () => {
  assert.deepEqual(parse({ rebuildOutputDir: "/out" }).saved, { rebuildOutputDir: "/out" });
  assert.deepEqual(parse({ rebuildOutputDir: 3 }).saved, {});
  assert.deepEqual(parse({ fileOutputDirs: { "/a": "/x", "/b": 5 } }).saved.fileOutputDirs, { "/a": "/x" });
  const many = Object.fromEntries(Array.from({ length: 513 }, (_, index) => [`/f${index}`, "/x"]));
  assert.equal("fileOutputDirs" in parse({ fileOutputDirs: many }).saved, false);
  assert.equal(Object.keys(parse({ fileOutputDirs: Object.fromEntries(Object.entries(many).slice(0, 512)) }).saved.fileOutputDirs).length, 512);
  assert.equal("fileOutputDirs" in parse({ fileOutputDirs: ["/a"] }).saved, false);
});

test("include patterns must all be strings; exclude patterns get the defaults merged unless already migrated", () => {
  assert.deepEqual(parse({ filenameIncludePatterns: ["a", "b"] }).saved.filenameIncludePatterns, ["a", "b"]);
  assert.equal("filenameIncludePatterns" in parse({ filenameIncludePatterns: ["a", 1] }).saved, false);
  const migrated = parse({ filenameExcludePatterns: ["mine", "OLD"], filenameExcludeDefaultsVersion: 2 }).saved;
  assert.deepEqual([migrated.filenameExcludePatterns, migrated.filenameExcludeDefaultsVersion], [["mine", "OLD"], 2]);
  const legacy = parse({ filenameExcludePatterns: ["mine", "OLD", "Extras", "mine2"] }).saved;
  assert.deepEqual(legacy.filenameExcludePatterns, ["old", ".*", "extras", "mine", "mine2"]);
  assert.equal(legacy.filenameExcludeDefaultsVersion, 2);
  assert.equal("filenameExcludePatterns" in parse({ filenameExcludePatterns: [1] }).saved, false);
});

test("metadata options are validated and missing sources fall back to the current ones", () => {
  assert.deepEqual(parse({ metadataAutoLookup: false, metadataLanguage: "fr-CA" }).saved, { metadataAutoLookup: false, metadataLanguage: "fr-CA" });
  assert.equal(parse({ metadataLanguage: "en" }).saved.metadataLanguage, "en");
  for (const bad of ["english", "e", "en_US", "en-USA", 5]) assert.equal("metadataLanguage" in parse({ metadataLanguage: bad }).saved, false, String(bad));
  assert.deepEqual(parse({ metadataSources: { tmdb: false, tvdb: true, tvmaze: "yes" } }).saved.metadataSources, { tmdb: false, wikidata: true, tvmaze: true, tvdb: true });
  assert.equal("metadataSources" in parse({ metadataSources: [] }).saved, false);
});

test("organizer templates fall back per field and replace the retired series template", () => {
  assert.deepEqual(parse({ organizerTemplates: { movie: "m", series: "{series_title} [[({year_range})]]", season: 3 } }).saved.organizerTemplates, { movie: "m", series: "S", season: "Se", episode: "E" });
  assert.equal(parse({ organizerTemplates: { series: "custom" } }).saved.organizerTemplates.series, "custom");
  assert.equal("organizerTemplates" in parse({ organizerTemplates: "x" }).saved, false);
});

test("saved app settings may be null or an object", () => {
  assert.equal(parse({ settings: null }).saved.settings, null);
  assert.deepEqual(parse({ settings: { a: 1 } }).saved.settings, { a: 1 });
  assert.equal("settings" in parse({ settings: 3 }).saved, false);
});

test("without a saved object the pre-1.0 individual keys are read", () => {
  const legacy = { "hevc-rebuild-theme": "hacker", "hevc-rebuild.root": "/old", "hevc-rebuild.ffmpeg": "/ff", "hevc-rebuild.recursive": "false", "hevc-rebuild.minimumSizeMib": "9" };
  const result = parse(undefined, legacy);
  assert.deepEqual(result.saved, {
    theme: "phosphor", root: "/old", toolPaths: { ffmpeg: "/ff", ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null }, recursive: false, minimumSizeMib: "9",
  });
  assert.equal(result.legacyTheme, true);
  assert.deepEqual(parse(undefined, { "hevc-rebuild.recursive": "true" }).saved, { recursive: true });
  assert.deepEqual(parse(undefined, { "hevc-rebuild-theme": "dark" }), { saved: { theme: "dark" }, legacyTheme: false });
});

test("the legacy keys are ignored once a saved object exists", () => {
  assert.deepEqual(parse({ recursive: true }, { "hevc-rebuild-theme": "dark", "hevc-rebuild.root": "/old" }).saved, { recursive: true });
});
