// ./desktop/tests/poster-urls.test.mjs
// Characterization tests for the poster/artwork resolvers extracted from the route
// (slice 2 of the +page.svelte segregation). They pin the precedence rules the route had.
// Run: pnpm test:poster-urls

import assert from "node:assert/strict";
import test from "node:test";
import {
  createPosterResolvers,
  posterCandidateForUrl,
  providerPosterUrl,
  remoteArtworkUrl,
} from "../src/lib/poster-urls.ts";
import { queuePathKey } from "../src/lib/queue-paths.ts";

const candidate = (overrides = {}) => ({
  id: "tmdb-1",
  provider: "tmdb",
  title: "Poster",
  imageUrl: "https://image.example/p.jpg",
  thumbnailUrl: "https://image.example/t.jpg",
  sourceUrl: "https://source.example/1",
  ...overrides,
});

function resolvers({ previews = {}, localPosters = {}, suggestions = {} } = {}) {
  return createPosterResolvers(() => previews, () => localPosters, () => suggestions);
}

test("posterCandidateForUrl only derives candidates on the same https host", () => {
  const base = candidate();
  const derived = posterCandidateForUrl(base, "https://image.example/season-2.jpg", "Show season 2 poster", "season-2");
  assert.deepEqual(derived, {
    ...base,
    id: "tmdb-1-season-2",
    title: "Show season 2 poster",
    imageUrl: "https://image.example/season-2.jpg",
    thumbnailUrl: "https://image.example/season-2.jpg",
  });
  assert.equal(posterCandidateForUrl(base, "https://other.example/x.jpg", "t", "s"), null);
  assert.equal(posterCandidateForUrl(base, "http://image.example/x.jpg", "t", "s"), null);
  assert.equal(posterCandidateForUrl(candidate({ imageUrl: "file:///x.jpg" }), "https://image.example/x.jpg", "t", "s"), null);
  assert.equal(posterCandidateForUrl(candidate({ sourceUrl: "" }), "https://image.example/x.jpg", "t", "s"), null);
  assert.equal(posterCandidateForUrl(base, "https://", "t", "s"), null);
});

test("posterCandidateForUrl keeps the base title when the new title is empty and bounds id and title length", () => {
  const base = candidate({ id: "x".repeat(600) });
  const derived = posterCandidateForUrl(base, "https://image.example/a.jpg", "", "season-1");
  assert.equal(derived.title, "Poster");
  assert.equal(derived.id.length, 512);
  assert.equal(posterCandidateForUrl(candidate(), "https://image.example/a.jpg", "t".repeat(2000), "s").title.length, 1024);
});

test("remoteArtworkUrl prefers imageUrl, falls back to thumbnailUrl, and rejects non-https", () => {
  assert.equal(remoteArtworkUrl(null), "");
  assert.equal(remoteArtworkUrl(undefined), "");
  assert.equal(remoteArtworkUrl(candidate()), "https://image.example/p.jpg");
  assert.equal(remoteArtworkUrl(candidate({ imageUrl: "http://x/p.jpg" })), "https://image.example/t.jpg");
  assert.equal(remoteArtworkUrl(candidate({ imageUrl: "", thumbnailUrl: "file:///t.jpg" })), "");
});

test("providerPosterUrl takes the first https image, then the first https thumbnail", () => {
  assert.equal(providerPosterUrl(undefined), "");
  assert.equal(providerPosterUrl({ artworkCandidates: [] }), "");
  assert.equal(
    providerPosterUrl({ artworkCandidates: [candidate({ imageUrl: "x", thumbnailUrl: "https://t/1" }), candidate({ imageUrl: "https://i/2" })] }),
    "https://i/2",
  );
  assert.equal(
    providerPosterUrl({ artworkCandidates: [candidate({ imageUrl: "x", thumbnailUrl: "https://t/1" }), candidate({ imageUrl: "y", thumbnailUrl: "z" })] }),
    "https://t/1",
  );
});

test("localPosterUrl and displayPosterUrl resolve local previews by normalized queue key", () => {
  const path = "/media/Show/poster.jpg";
  const { localPosterUrl, displayPosterUrl } = resolvers({ previews: { [queuePathKey(path)]: "asset://poster" } });
  assert.equal(localPosterUrl(path), "asset://poster");
  assert.equal(localPosterUrl(""), null);
  assert.equal(localPosterUrl("/missing.jpg"), null);
  assert.equal(displayPosterUrl(path), "asset://poster");
  assert.equal(displayPosterUrl("https://image.example/p.jpg"), "https://image.example/p.jpg");
  assert.equal(displayPosterUrl("http://image.example/p.jpg"), null);
});

test("resolvers read their state lazily so later changes are visible", () => {
  let previews = {};
  const { localPosterUrl } = createPosterResolvers(() => previews, () => ({}), () => ({}));
  assert.equal(localPosterUrl("/a.jpg"), null);
  previews = { [queuePathKey("/a.jpg")]: "asset://a" };
  assert.equal(localPosterUrl("/a.jpg"), "asset://a");
});

test("chosenPoster accepts https, known library posters and previewed local paths only", () => {
  const known = "/media/Show/poster.jpg";
  const previewed = "/media/Other/poster.jpg";
  const { chosenPoster, knownLocalPoster } = resolvers({
    localPosters: { [queuePathKey("/media/Show")]: known },
    previews: { [queuePathKey(previewed)]: "asset://o" },
  });
  assert.equal(chosenPoster(""), "");
  assert.equal(chosenPoster("https://image.example/p.jpg"), "https://image.example/p.jpg");
  assert.equal(chosenPoster(known), known);
  assert.equal(chosenPoster(previewed), previewed);
  assert.equal(chosenPoster("/media/unknown.jpg"), "");
  assert.equal(knownLocalPoster(known), known);
  assert.equal(knownLocalPoster("https://image.example/p.jpg"), "");
  assert.equal(knownLocalPoster(""), "");
});

test("visiblePosterUrl prefers a local preview, then remote, then the local path", () => {
  const local = "/media/Show/poster.jpg";
  const { visiblePosterUrl } = resolvers({ previews: { [queuePathKey(local)]: "asset://local" } });
  assert.equal(visiblePosterUrl(local, "https://image.example/p.jpg"), "asset://local");
  assert.equal(visiblePosterUrl("", "https://image.example/p.jpg"), "https://image.example/p.jpg");
  assert.equal(visiblePosterUrl("https://image.example/l.jpg", "https://image.example/r.jpg"), "https://image.example/r.jpg");
  assert.equal(visiblePosterUrl("/nothing.jpg", ""), null);
});

test("seasonArtworkPath prefers on-disk poster, then the saved choice, then the provider poster", () => {
  const dir = "/media/Show/Season 1";
  const review = {
    localSeasonPosterPaths: { 1: "https://image.example/saved.jpg" },
    artworkCandidates: [candidate()],
  };
  const withDisk = resolvers({ localPosters: { [queuePathKey(dir)]: "/media/Show/Season 1/poster.jpg" } });
  assert.equal(withDisk.seasonArtworkPath(review, 1, dir), "/media/Show/Season 1/poster.jpg");
  const noDisk = resolvers();
  assert.equal(noDisk.seasonArtworkPath(review, 1, dir), "https://image.example/saved.jpg");
  assert.equal(noDisk.seasonArtworkPath({ ...review, localSeasonPosterPaths: {} }, 1, dir), "https://image.example/p.jpg");
  assert.equal(noDisk.seasonArtworkPath(undefined, 1, ""), "");
});

test("seasonPosterSources splits the saved choice into local and remote paths", () => {
  const dir = "/media/Show/Season 1";
  const known = "/media/Show/Season 1/poster.jpg";
  const { seasonPosterSources } = resolvers({ localPosters: { [queuePathKey(dir)]: known } });
  assert.deepEqual(seasonPosterSources({ localSeasonPosterPaths: { 1: "https://image.example/saved.jpg" } }, 1, dir), {
    localPath: known,
    remotePath: "https://image.example/saved.jpg",
  });
  const noDisk = resolvers().seasonPosterSources;
  assert.deepEqual(noDisk({ localSeasonPosterPaths: { 2: "https://image.example/s2.jpg" } }, 2, ""), {
    localPath: "",
    remotePath: "https://image.example/s2.jpg",
  });
  assert.deepEqual(noDisk({ localSeasonPosterPaths: { 2: "/unknown.jpg" } }, 2, ""), { localPath: "", remotePath: "" });
  assert.deepEqual(noDisk(undefined, 1, ""), { localPath: "", remotePath: "" });
});

test("seriesSuggestionArtwork returns the first episode suggestion that has https artwork", () => {
  const art = candidate();
  const { seriesSuggestionArtwork } = resolvers({
    suggestions: {
      "/s/e1.mkv": { artwork: candidate({ imageUrl: "x", thumbnailUrl: "y" }) },
      "/s/e2.mkv": { artwork: art },
      "/s/e3.mkv": { artwork: candidate({ imageUrl: "https://other/p.jpg" }) },
    },
  });
  assert.equal(seriesSuggestionArtwork({ episodePaths: ["/s/e0.mkv", "/s/e1.mkv", "/s/e2.mkv", "/s/e3.mkv"] }), art);
  assert.equal(seriesSuggestionArtwork({ episodePaths: ["/s/e1.mkv"] }), null);
  assert.equal(seriesSuggestionArtwork({ episodePaths: [] }), null);
});
