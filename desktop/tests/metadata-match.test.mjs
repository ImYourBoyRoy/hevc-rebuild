import assert from "node:assert/strict";
import test from "node:test";
import {
  automaticMetadataMatch,
  metadataMatchLabel,
  normalizeMatchTitle,
  rankMetadataMatches,
} from "../src/lib/metadata-match.ts";

const movieClue = {
  title: "Gnomeo & Juliet",
  year: "2011",
  yearEnd: null,
  kind: "movie",
};

function candidate(id, provider, title, year, kind = "movie") {
  return { id, provider, title, year, kind };
}

test("normalizes punctuation, ampersands, and Unicode compatibility forms", () => {
  assert.equal(normalizeMatchTitle("Gnomeo & Juliet"), "gnomeo and juliet");
  assert.equal(normalizeMatchTitle("Gnomeo and Juliet"), "gnomeo and juliet");
  assert.equal(normalizeMatchTitle("Ｇｎｏｍｅｏ—Juliet"), "gnomeo juliet");
});

test("associates an exact title and matching year from one provider", () => {
  const match = candidate("42", "tmdb", "Gnomeo and Juliet", "2011");
  assert.equal(automaticMetadataMatch([match], movieClue)?.id, "42");

  const exact = candidate("43", "tmdb", "Gnomeo & Juliet", "2011");
  assert.equal(automaticMetadataMatch([exact], movieClue)?.id, "43");
});

test("leaves missing-year and year-conflicting matches for review", () => {
  assert.equal(automaticMetadataMatch([
    candidate("1", "tmdb", "Gnomeo & Juliet", null),
  ], movieClue), null);
  assert.equal(automaticMetadataMatch([
    candidate("1", "tmdb", "Gnomeo & Juliet", "2005"),
  ], movieClue), null);
});

test("does not auto-associate when the same title has a competing year", () => {
  const candidates = [
    candidate("2011", "tmdb", "Gnomeo & Juliet", "2011"),
    candidate("2005", "wikidata", "Gnomeo & Juliet", "2005"),
  ];
  assert.equal(automaticMetadataMatch(candidates, movieClue), null);
});

test("uses cross-source agreement and rejects provider disagreement", () => {
  const agreed = [
    candidate("43", "tmdb", "Gnomeo & Juliet", "2011"),
    candidate("Q123", "wikidata", "Gnomeo & Juliet", "2011"),
  ];
  assert.equal(automaticMetadataMatch(agreed, movieClue)?.provider, "tmdb");

  const disagreed = [
    ...agreed,
    candidate("wrong", "tvdb", "Gnomeo & Juliet: The Sequel", "2011"),
  ];
  assert.equal(automaticMetadataMatch(disagreed, movieClue), null);
});

test("does not auto-associate duplicate same-provider title/year records", () => {
  const candidates = [
    candidate("a", "tmdb", "Gnomeo & Juliet", "2011"),
    candidate("b", "tmdb", "Gnomeo & Juliet", "2011"),
  ];
  assert.equal(automaticMetadataMatch(candidates, movieClue), null);
});

test("requires the candidate media kind to match the search kind", () => {
  assert.equal(automaticMetadataMatch([
    candidate("1", "tvmaze", "Gnomeo & Juliet", "2011", "tv"),
  ], movieClue), null);
});

test("ranks year agreement and cross-source agreement ahead of provider order", () => {
  const clue = { ...movieClue, title: "The Thing", year: "1982" };
  const candidates = [
    candidate("wrong-year", "tmdb", "The Thing", "1951"),
    candidate("right-year-secondary", "wikidata", "The Thing", "1982"),
    candidate("right-year-primary", "tmdb", "The Thing", "1982"),
  ];
  const ranked = rankMetadataMatches(candidates, clue);
  assert.equal(ranked[0].id, "right-year-primary");
  assert.equal(ranked[1].id, "right-year-secondary");
  assert.deepEqual(metadataMatchLabel(ranked[0], clue), {
    text: "Exact title · year matches filename",
    conflict: false,
  });
  assert.equal(metadataMatchLabel(ranked[2], clue).conflict, true);
});

test("does not fabricate a numeric probability for its evidence label", () => {
  const label = metadataMatchLabel(candidate("1", "tmdb", "Gnomeo & Juliet", "2011"), movieClue);
  assert.match(label.text, /Exact title/);
  assert.doesNotMatch(label.text, /\d+%/);
});
