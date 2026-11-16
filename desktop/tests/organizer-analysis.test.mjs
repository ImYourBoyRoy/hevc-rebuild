// ./desktop/tests/organizer-analysis.test.mjs
// Characterization tests for the organizer analysis helpers extracted from MetadataWorkbench (T10.7 slice 1):
// path normalizing and joining, season and episode recognition from names, series grouping of an analyzed scan,
// episode gap detection and the organizer scan built from an analysis. Pure functions, no state.
// Run: pnpm test:organizer-analysis

import assert from "node:assert/strict";
import test from "node:test";
import {
  analysisOrganizerScan, analyzedLibraryRoot, basename, buildAnalysisSeriesGroups, episodeGapsFromFiles, episodeIdFromName, genericSeriesRoot,
  inferredSeriesRoot, isAnalyzedEpisode, joinPath, normalizedPath, organizerFileFromEntry, parentPath, seasonNumberFromDirectory, seriesFilenameTitle,
} from "../src/lib/organizer-analysis.ts";

const entry = (root, relative, name = relative.split("/").at(-1)) => ({ path: `${root}/${relative}`, relative_path: relative, name });
const scan = (root, entries, input_kind = "folder") => ({ root, input_kind, entries });

test("normalizedPath unifies separators, collapses repeats and keeps a network prefix", () => {
  assert.equal(normalizedPath("C:\\a\\\\b"), "C:/a/b");
  assert.equal(normalizedPath("/a//b///c"), "/a/b/c");
  assert.equal(normalizedPath("//server//share///x"), "//server/share/x");
  assert.equal(normalizedPath("\\\\server\\share"), "//server/share");
  assert.equal(normalizedPath("/a/b/"), "/a/b/");
});

test("parentPath returns the folder, the root slash or a drive root, and empty for a bare name", () => {
  assert.equal(parentPath("/a/b/c.mkv"), "/a/b");
  assert.equal(parentPath("/a"), "/");
  assert.equal(parentPath("C:/x"), "C:/");
  assert.equal(parentPath("c:\\Movies\\f.mkv"), "c:/Movies");
  assert.equal(parentPath("plain"), "");
  assert.equal(parentPath("/a/b/"), "/a");
  assert.equal(parentPath("/"), "");
});

test("basename returns the last segment, ignoring a trailing slash", () => {
  assert.equal(basename("/a/b/c.mkv"), "c.mkv");
  assert.equal(basename("/a/b/"), "b");
  assert.equal(basename("c.mkv"), "c.mkv");
  assert.equal(basename("C:\\x\\y.mkv"), "y.mkv");
});

test("joinPath appends a child to folders, the root slash and drive letters", () => {
  assert.equal(joinPath("/lib", "x"), "/lib/x");
  assert.equal(joinPath("/lib/", "x"), "/lib/x");
  assert.equal(joinPath("/", "x"), "/x");
  assert.equal(joinPath("C:", "x"), "C:/x");
  assert.equal(joinPath("C:/", "x"), "C:/x");
  assert.equal(joinPath("C:\\lib", "x"), "C:/lib/x");
});

test("seasonNumberFromDirectory accepts Specials and Season N names only", () => {
  assert.equal(seasonNumberFromDirectory("Specials"), 0);
  assert.equal(seasonNumberFromDirectory("specials"), 0);
  assert.equal(seasonNumberFromDirectory("Season 1"), 1);
  assert.equal(seasonNumberFromDirectory("season_02"), 2);
  assert.equal(seasonNumberFromDirectory("Season-10"), 10);
  assert.equal(seasonNumberFromDirectory("SEASON3"), 3);
  assert.equal(seasonNumberFromDirectory("Season 12345"), null);
  assert.equal(seasonNumberFromDirectory("Season 1 Extras"), null);
  assert.equal(seasonNumberFromDirectory("S01"), null);
});

test("genericSeriesRoot recognizes container folder names case-insensitively", () => {
  for (const name of ["TV Shows", " Movies ", "Library", "SERIES", "tv", "Videos", "media", "shows"]) assert.equal(genericSeriesRoot(name), true, name);
  for (const name of ["My Show", "TV Show", "Anime", ""]) assert.equal(genericSeriesRoot(name), false, name);
});

test("seriesFilenameTitle uses a named root, or the most common filename prefix under a generic root", () => {
  const file = (name) => ({ name });
  assert.equal(seriesFilenameTitle({ root: "/lib/My Show", files: [file("Other.S01E01.mkv")] }), "My Show");
  const generic = (names) => ({ root: "/lib/TV", files: names.map(file) });
  assert.equal(seriesFilenameTitle(generic(["Alpha.S01E01.mkv", "Alpha.S01E02.mkv", "Beta.S01E01.mkv"])), "Alpha");
  assert.equal(seriesFilenameTitle(generic(["Show A - S01E01.mkv"])), "Show A");
  assert.equal(seriesFilenameTitle(generic(["No marker.mkv", "S01E01.mkv"])), "TV");
  assert.equal(seriesFilenameTitle(generic([])), "TV");
});

test("episodeIdFromName reads SxxEyy, flags double episodes and rejects embedded letters", () => {
  assert.deepEqual(episodeIdFromName("Show.S01E02.mkv"), { season: 1, episode: 2, multipleEpisodes: false });
  assert.deepEqual(episodeIdFromName("Show.S01E02E03.mkv"), { season: 1, episode: 2, multipleEpisodes: true });
  assert.deepEqual(episodeIdFromName("Show.S01E02.E03.mkv"), { season: 1, episode: 2, multipleEpisodes: true });
  assert.deepEqual(episodeIdFromName("s1e5.mkv"), { season: 1, episode: 5, multipleEpisodes: false });
  assert.deepEqual(episodeIdFromName("Show S10 - E100.mkv"), { season: 10, episode: 100, multipleEpisodes: false });
  assert.deepEqual(episodeIdFromName("Movie (2001).mkv"), { season: null, episode: null, multipleEpisodes: false });
  assert.deepEqual(episodeIdFromName("xs01e02.mkv"), { season: null, episode: null, multipleEpisodes: false });
});

test("organizerFileFromEntry normalizes the path and derives name, folder, extension and episode ids", () => {
  assert.deepEqual(organizerFileFromEntry({ path: "C:\\lib\\Show\\Show.S01E02.mkv", name: "Show.S01E02.mkv" }), {
    path: "C:/lib/Show/Show.S01E02.mkv", name: "Show.S01E02.mkv", parentPath: "C:/lib/Show", extension: "mkv", season: 1, episode: 2, multipleEpisodes: false,
  });
  const unnamed = organizerFileFromEntry({ path: "/lib/Film (2001).mkv", name: "" });
  assert.deepEqual([unnamed.name, unnamed.extension, unnamed.season], ["Film (2001).mkv", "mkv", null]);
  assert.equal(organizerFileFromEntry({ path: "/lib/README", name: "README" }).extension, "");
});

test("analyzedLibraryRoot is the folder itself, or the parent of a single analyzed file", () => {
  assert.equal(analyzedLibraryRoot({ root: "C:\\lib", input_kind: "folder" }), "C:/lib");
  assert.equal(analyzedLibraryRoot({ root: "/lib/Film.mkv", input_kind: "file" }), "/lib");
});

test("isAnalyzedEpisode accepts episode names, season roots and season subfolders only", () => {
  const folder = scan("/lib", []);
  assert.equal(isAnalyzedEpisode(folder, entry("/lib", "Show.S01E01.mkv")), true);
  assert.equal(isAnalyzedEpisode(folder, entry("/lib", "Show/Season 1/clip.mkv")), true);
  assert.equal(isAnalyzedEpisode(folder, entry("/lib", "Movies/Film (2001).mkv")), false);
  assert.equal(isAnalyzedEpisode(scan("/lib/Show/Season 2", []), entry("/lib/Show/Season 2", "clip.mkv")), true);
  assert.equal(isAnalyzedEpisode(scan("/lib/Season 1/clip.mkv", [], "file"), entry("/lib/Season 1", "clip.mkv")), true);
  assert.equal(isAnalyzedEpisode(scan("/lib/Film.mkv", [], "file"), entry("/lib", "Season 1/Film.mkv")), false);
});

test("inferredSeriesRoot is the folder above the first season folder, or the first folder, or the root", () => {
  const folder = scan("/lib", []);
  assert.equal(inferredSeriesRoot(folder, entry("/lib", "Show A/Season 1/e.mkv")), "/lib/Show A");
  assert.equal(inferredSeriesRoot(folder, entry("/lib", "Show A/Extras/Season 2/e.mkv")), "/lib/Show A/Extras");
  assert.equal(inferredSeriesRoot(folder, entry("/lib", "Show B/e.mkv")), "/lib/Show B");
  assert.equal(inferredSeriesRoot(folder, entry("/lib", "e.mkv")), "/lib");
  assert.equal(inferredSeriesRoot(scan("/lib/Show/Season 1", []), entry("/lib/Show/Season 1", "e.mkv")), "/lib/Show");
  assert.equal(inferredSeriesRoot(scan("/lib/Show/Season 1/e.mkv", [], "file"), entry("/lib/Show/Season 1", "e.mkv")), "/lib/Show");
});

test("buildAnalysisSeriesGroups groups episodes by series root with sorted files and seasons", () => {
  const library = scan("/lib", [
    entry("/lib", "Show A/Season 2/Show.A.S02E01.mkv"), entry("/lib", "Show A/Season 1/Show.A.S01E01.mkv"), entry("/lib", "Show B/Show.B.S01E01.mkv"), entry("/lib", "Movies/Film (2001).mkv"),
  ]);
  const groups = buildAnalysisSeriesGroups(library);
  assert.deepEqual(groups.map((group) => group.root), ["/lib/Show A", "/lib/Show B"]);
  assert.deepEqual(groups[0].files.map((file) => file.name), ["Show.A.S01E01.mkv", "Show.A.S02E01.mkv"]);
  assert.deepEqual(groups[0].seasonDirectories, [{ path: "/lib/Show A/Season 1", name: "Season 1", season: 1 }, { path: "/lib/Show A/Season 2", name: "Season 2", season: 2 }]);
  assert.deepEqual(groups.map((group) => group.ambiguous), [false, false]);
  assert.deepEqual(groups[1].seasonDirectories, []);
});

test("buildAnalysisSeriesGroups marks mixed or unnamed series folders ambiguous", () => {
  const generic = buildAnalysisSeriesGroups(scan("/lib/TV", [entry("/lib/TV", "Alpha.S01E01.mkv"), entry("/lib/TV", "Beta.S01E01.mkv")]));
  assert.deepEqual(generic.map((group) => [group.root, group.ambiguous]), [["/lib/TV", true]]);
  const genericSingle = buildAnalysisSeriesGroups(scan("/lib/TV", [entry("/lib/TV", "Alpha.S01E01.mkv"), entry("/lib/TV", "Alpha.S01E02.mkv")]));
  assert.equal(genericSingle[0].ambiguous, false);
  const mixed = buildAnalysisSeriesGroups(scan("/lib/Mixed", [entry("/lib/Mixed", "Alpha.S01E01.mkv"), entry("/lib/Mixed", "Beta.S01E01.mkv")]));
  assert.equal(mixed[0].ambiguous, true);
  const withSeasons = buildAnalysisSeriesGroups(scan("/lib", [entry("/lib", "Mixed/Season 1/Alpha.S01E01.mkv"), entry("/lib", "Mixed/Season 1/Beta.S01E02.mkv")]));
  assert.equal(withSeasons[0].ambiguous, false);
  assert.deepEqual(buildAnalysisSeriesGroups(scan("/lib", [entry("/lib", "Film (2001).mkv")])), []);
});

test("episodeGapsFromFiles reports missing episodes between the first and last, with a cap of eight examples", () => {
  const ep = (season, episode, multipleEpisodes = false) => ({ season, episode, multipleEpisodes });
  assert.deepEqual(episodeGapsFromFiles([ep(1, 1), ep(1, 2), ep(1, 5)]), [{ season: 1, missingCount: 2, examples: [3, 4] }]);
  assert.deepEqual(episodeGapsFromFiles([ep(1, 1), ep(1, 2), ep(1, 3)]), []);
  assert.deepEqual(episodeGapsFromFiles([ep(1, 1)]), []);
  assert.deepEqual(episodeGapsFromFiles([ep(1, 1), ep(1, 5, true)]), []);
  assert.deepEqual(episodeGapsFromFiles([ep(2, 1), ep(2, 20)]), [{ season: 2, missingCount: 18, examples: [2, 3, 4, 5, 6, 7, 8, 9] }]);
  assert.deepEqual(episodeGapsFromFiles([ep(null, 1), ep(3, null), ep(3, 1), ep(3, 3), ep(1, 1), ep(1, 3)]), [{ season: 1, missingCount: 1, examples: [2] }, { season: 3, missingCount: 1, examples: [2] }]);
  assert.deepEqual(episodeGapsFromFiles([]), []);
});

test("analysisOrganizerScan builds the movie list, or the chosen unambiguous series", () => {
  const library = scan("/lib", [
    entry("/lib", "Movies/B Film.mkv"), entry("/lib", "Movies/A Film.mkv"), entry("/lib", "Show/Season 1/Show.S01E01.mkv"), entry("/lib", "Show/Season 1/Show.S01E03.mkv"),
  ]);
  assert.equal(analysisOrganizerScan(scan("/lib", []), "movie"), null);
  const movies = analysisOrganizerScan(library, "movie");
  assert.deepEqual(movies.files.map((file) => file.name), ["A Film.mkv", "B Film.mkv"]);
  assert.deepEqual([movies.root, movies.renameRoot, movies.seasonDirectories, movies.episodeGaps, movies.errors, movies.truncated], ["/lib", "/lib", [], [], [], false]);
  const series = analysisOrganizerScan(library, "tv", "/lib/Show");
  assert.deepEqual([series.root, series.renameRoot, series.files.length, series.seasonDirectories.length, series.episodeGaps], ["/lib/Show", "/lib/Show", 2, 1, [{ season: 1, missingCount: 1, examples: [2] }]]);
  assert.equal(analysisOrganizerScan(library, "tv", "/lib/Unknown"), null);
  assert.equal(analysisOrganizerScan(library, "tv"), null);
  const ambiguous = scan("/lib/TV", [entry("/lib/TV", "Alpha.S01E01.mkv"), entry("/lib/TV", "Beta.S01E01.mkv")]);
  assert.equal(analysisOrganizerScan(ambiguous, "tv", "/lib/TV"), null);
});
