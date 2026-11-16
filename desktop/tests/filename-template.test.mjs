// ./desktop/tests/filename-template.test.mjs
// Characterization tests for the organizer filename templating extracted from the route
// (slice 4 of the +page.svelte segregation). They pin token rendering, optional [[...]] groups,
// rejection of malformed templates, proposal selection and destination paths.
// Run: pnpm test:filename-template

import assert from "node:assert/strict";
import test from "node:test";
import {
  proposedFilenameForEntry,
  renderFilenameChunk,
  renderFilenameTemplate,
  reviewDestinationPath,
} from "../src/lib/filename-template.ts";

const movieTokens = ["title", "year", "certification", "ext"];
const templates = {
  movie: "{title} [[({year} {certification})]]{ext}",
  series: "{series_title}",
  season: "Season {season:02}",
  episode: "S{season:02}E{episode:02}.{episode_title}{ext}",
};

test("renderFilenameChunk substitutes tokens and reports whether any had a value", () => {
  assert.deepEqual(renderFilenameChunk("{title} - {year}", movieTokens, { title: "Film", year: "2001" }), { text: "Film - 2001", hasValue: true });
  assert.deepEqual(renderFilenameChunk("({year})", movieTokens, { year: "" }), { text: "()", hasValue: false });
  assert.deepEqual(renderFilenameChunk("plain", movieTokens, {}), { text: "plain", hasValue: false });
  assert.deepEqual(renderFilenameChunk("{title}", movieTokens, {}), { text: "", hasValue: false });
});

test("renderFilenameChunk pads numeric tokens and rejects non-numeric padded values", () => {
  const tokens = ["season", "episode"];
  assert.equal(renderFilenameChunk("{season:02}{episode:03}", tokens, { season: "1", episode: "7" })?.text, "01007");
  assert.equal(renderFilenameChunk("{season:04}", tokens, { season: "12" })?.text, "0012");
  assert.equal(renderFilenameChunk("{season:02}", tokens, { season: "x" }), null);
  assert.equal(renderFilenameChunk("{season:02}", tokens, { season: "" })?.text, "");
  assert.equal(renderFilenameChunk("{season:05}", tokens, { season: "1" }), null);
});

test("renderFilenameChunk rejects unknown tokens, unbalanced braces and nested braces", () => {
  for (const chunk of ["{nope}", "{title", "title}", "{}", "{{title}}", "}{title}", "{Title}", "{title:}"]) {
    assert.equal(renderFilenameChunk(chunk, movieTokens, { title: "x" }), null, chunk);
  }
});

test("renderFilenameTemplate renders the default movie template with and without optional parts", () => {
  assert.equal(renderFilenameTemplate(templates.movie, movieTokens, { title: "Film", year: "2001", certification: "PG", ext: ".mkv" }), "Film (2001 PG).mkv");
  assert.equal(renderFilenameTemplate(templates.movie, movieTokens, { title: "Film", year: "2001", certification: "", ext: ".mkv" }), "Film (2001).mkv");
  assert.equal(renderFilenameTemplate(templates.movie, movieTokens, { title: "Film", year: "", certification: "", ext: ".mkv" }), "Film .mkv");
  // Existing behavior, pinned deliberately: the space before the dropped optional group stays.
});

test("renderFilenameTemplate collapses whitespace and removes empty bracket pairs", () => {
  assert.equal(renderFilenameTemplate("{title}   {year}", movieTokens, { title: "A", year: "B" }), "A B");
  assert.equal(renderFilenameTemplate("{title} ({year})", movieTokens, { title: "A", year: "" }), "A");
  assert.equal(renderFilenameTemplate("{title} [{year}]", movieTokens, { title: "A", year: "" }), "A");
  assert.equal(renderFilenameTemplate("{title} ( {year} )", movieTokens, { title: "A", year: "1999" }), "A (1999)");
});

test("renderFilenameTemplate returns null for empty, oversized or malformed templates and empty results", () => {
  assert.equal(renderFilenameTemplate("   ", movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("x".repeat(1025), movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("x".repeat(1024), movieTokens, {})?.length, 1024);
  assert.equal(renderFilenameTemplate("{title}[[{year}", movieTokens, { title: "A" }), null);
  assert.equal(renderFilenameTemplate("a]]b", movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("[[a[[b]]]]", movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("{unknown}", movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("{year}", movieTokens, { year: "" }), null);
});

test("renderFilenameTemplate measures the size limit in UTF-8 bytes", () => {
  assert.equal(renderFilenameTemplate("é".repeat(513), movieTokens, {}), null);
  assert.equal(renderFilenameTemplate("é".repeat(512), movieTokens, {})?.length, 512);
});

test("renderFilenameTemplate passes characters it does not own straight through", () => {
  assert.equal(renderFilenameTemplate("{title}: part/1?{ext}", movieTokens, { title: "A", ext: ".mkv" }), "A: part/1?.mkv");
});

const entry = (name, path = `/media/${name}`) => ({ name, path });
const emptySeries = (extra = {}) => ({ title: "Show", episodeAssignments: {}, ...extra });

test("proposedFilenameForEntry uses a reviewed movie and keeps the source extension", () => {
  const proposal = proposedFilenameForEntry(entry("raw.name.MKV"), { title: "Film", year: "2001", certification: "PG" }, undefined, undefined, templates);
  assert.deepEqual(proposal, { name: "Film (2001 PG).MKV", needsTitleReview: false });
  assert.deepEqual(proposedFilenameForEntry(entry("noext"), { title: "Film", year: "", certification: "" }, undefined, undefined, templates), { name: "Film", needsTitleReview: false });
});

test("proposedFilenameForEntry falls back to a ready movie suggestion and flags it for review", () => {
  const suggestion = { kind: "movie", status: "ready", title: " Film ", year: "1999" };
  assert.deepEqual(proposedFilenameForEntry(entry("a.mkv"), undefined, undefined, suggestion, templates), { name: "Film (1999).mkv", needsTitleReview: true });
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), undefined, undefined, { ...suggestion, status: "pending" }, templates), null);
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), undefined, undefined, { ...suggestion, kind: "series" }, templates), null);
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), undefined, undefined, undefined, templates), null);
});

test("proposedFilenameForEntry names a series episode from its assignment, matching paths by queue key", () => {
  const assignment = { season: 2, episode: 5, title: "The Title" };
  const exact = emptySeries({ episodeAssignments: { "/media/a.mkv": assignment } });
  assert.deepEqual(proposedFilenameForEntry(entry("a.mkv"), undefined, exact, undefined, templates), { name: "S02E05.The Title.mkv", needsTitleReview: false });
  const windows = emptySeries({ episodeAssignments: { "C:\\Media\\A.mkv": assignment } });
  assert.deepEqual(proposedFilenameForEntry(entry("A.mkv", "c:/media/a.mkv"), undefined, windows, undefined, templates)?.name, "S02E05.The Title.mkv");
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), undefined, emptySeries(), undefined, templates), null);
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), undefined, emptySeries({ episodeAssignments: { "/media/a.mkv": { ...assignment, title: "  " } } }), undefined, templates), null);
});

test("proposedFilenameForEntry returns null when the template cannot render", () => {
  assert.equal(proposedFilenameForEntry(entry("a.mkv"), { title: "Film", year: "", certification: "" }, undefined, undefined, { ...templates, movie: "{bad}" }), null);
});

test("reviewDestinationPath defaults to the source folder with the proposed or original name", () => {
  const none = { fileOutputDirs: {}, rebuildOutputDir: null };
  assert.deepEqual(reviewDestinationPath({ path: "/media/Movies/a.mkv", name: "a.mkv", relative_path: "a.mkv" }, "Film (2001).mkv", none), { path: "/media/Movies/Film (2001).mkv", note: "Proposed name in the source folder" });
  assert.deepEqual(reviewDestinationPath({ path: "C:\\Media\\a.mkv", name: "a.mkv", relative_path: "a.mkv" }, null, none), { path: "C:\\Media\\a.mkv", note: "Proposed name in the source folder" });
  assert.deepEqual(reviewDestinationPath({ path: "a.mkv", name: "a.mkv", relative_path: "a.mkv" }, null, none), { path: "a.mkv", note: "Proposed name in the source folder" });
});

test("reviewDestinationPath mirrors source subfolders into the default output folder and forces .mkv", () => {
  const entry = { path: "/media/Show/Season 1/a.mp4", name: "a.mp4", relative_path: "Show/Season 1/a.mp4" };
  assert.deepEqual(reviewDestinationPath(entry, "S01E01.Pilot.mp4", { fileOutputDirs: {}, rebuildOutputDir: "/out/" }), { path: "/out/Show/Season 1/S01E01.Pilot.mkv", note: "Default output folder" });
  assert.deepEqual(reviewDestinationPath(entry, null, { fileOutputDirs: {}, rebuildOutputDir: "D:\\Out\\" }), { path: "D:\\Out\\Show\\Season 1\\a.mkv", note: "Default output folder" });
});

test("reviewDestinationPath sends a per-file output folder the file directly", () => {
  const entry = { path: "/media/Show/a.mkv", name: "a.mkv", relative_path: "Show/a.mkv" };
  assert.deepEqual(reviewDestinationPath(entry, "B.mkv", { fileOutputDirs: { "/media/Show/a.mkv": "/special//" }, rebuildOutputDir: "/out" }), { path: "/special/B.mkv", note: "Per-file output folder" });
});
