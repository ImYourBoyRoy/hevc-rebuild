// ./desktop/src/lib/metadata-candidates.ts
// Pure candidate and artwork helpers extracted from MetadataWorkbench (T10.7 slice 1): saved suggestions as search results and
// match sets, the scan fingerprint, artwork de-duplication and review filtering, safe URLs, artwork file targets and the
// provider and episode text checks. No state, no IO; tested by test:metadata-candidates. The code is the component's code moved verbatim.

import type { QueueMetadataSuggestion, ScanResult } from "./types.js";
import type { AnalysisSeriesGroup, Artwork, ArtworkTarget, FilenameClue, MatchSuggestion, MediaKind, MovieAssignment, OrganizerFile, Provider, RenamePreview, Result, SeriesAssignment } from "./metadata-workbench-types.js";
import { normalizedPath } from "./organizer-analysis.ts";
import { metadataMatchLabel } from "./metadata-match.ts";

export function suggestionResult(suggestion: QueueMetadataSuggestion | undefined, expectedKind?: MediaKind): Result | null {
  if (!suggestion || suggestion.status !== "ready" || (expectedKind && suggestion.kind !== expectedKind)) return null;
  const record = suggestion.recordId ?? "";
  const split = record.indexOf(":");
  const providerValue = split > 0 ? record.slice(0, split) : suggestion.provider ?? "";
  const id = split > 0 ? record.slice(split + 1) : "";
  const allowed = suggestion.kind === "movie" ? ["tmdb", "wikidata"] : ["tmdb", "tvdb", "tvmaze"];
  if (!id || !allowed.includes(providerValue)) return null;
  const provider = providerValue as Provider;
  return {
    id,
    provider,
    kind: suggestion.kind === "movie" ? "movie" : "tv",
    title: suggestion.title || suggestion.artwork?.title || "Untitled",
    year: suggestion.year,
    description: suggestion.description,
    rating: null,
    sourceUrl: suggestion.sourceUrl || suggestion.artwork?.sourceUrl || "",
    confidence: suggestion.autoAssociated ? 1 : 0,
    artwork: suggestion.artwork ? [suggestion.artwork] : [],
  };
}

export function suggestionMatchSet(suggestion: QueueMetadataSuggestion | undefined, fallbackTitle: string, expectedKind: MediaKind): MatchSuggestion | null {
  if (!suggestion || suggestion.kind !== expectedKind) return null;
  const result = suggestionResult(suggestion, expectedKind);
  return {
    clue: {
      title: suggestion.title?.trim() || suggestion.artwork?.title?.trim() || fallbackTitle.replace(/\.[^.]+$/, ""),
      year: suggestion.year,
      yearEnd: null,
      certification: null,
    },
    results: result ? [result] : [],
    warnings: [],
    status: suggestion.status,
    error: suggestion.error,
  };
}

export function suggestionDetail(suggestion: QueueMetadataSuggestion | undefined): Result | null {
  return suggestionResult(suggestion, "tv");
}

export function dedupeArtwork(images: Artwork[]): Artwork[] {
  const seen = new Set<string>();
  return images.filter((image) => {
    const key = image.imageUrl || image.thumbnailUrl || image.id;
    if (!key || seen.has(key)) return false;
    seen.add(key);
    return Boolean(image.imageUrl || image.thumbnailUrl);
  });
}

export function metadataScanKey(scan: ScanResult): string {
  let hash = 2166136261;
  const source = `${normalizedPath(scan.root)}\n${scan.entries.map((entry) => `${normalizedPath(entry.path)}\0${entry.size_bytes}\0${entry.modified_unix_ms ?? ""}`).join("\n")}`;
  for (let index = 0; index < source.length; index += 1) {
    hash ^= source.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `${scan.entries.length}:${hash >>> 0}`;
}

export function candidateEvidence(result: Result, clue: FilenameClue): { text: string; conflict: boolean } {
  return metadataMatchLabel(result, { ...clue, kind: result.kind });
}

export function uniqueArtwork(artwork: Artwork[]): Artwork[] {
  const seen = new Set<string>();
  return artwork.filter((image) => {
    if (!image.id || image.id.length > 1024 || seen.has(image.id)) return false;
    seen.add(image.id);
    return true;
  });
}

export function clueCaption(clue: FilenameClue): string {
  const years = clue.year ? ` · ${clue.year}${clue.yearEnd ? `–${clue.yearEnd}` : ""}` : "";
  const certification = clue.certification ? ` · ${clue.certification}` : "";
  return `${clue.title}${years}${certification}`;
}

export function safeHttpUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "http:" || parsed.protocol === "https:" ? parsed.href : null;
  } catch {
    return null;
  }
}

export function assignedMoviePoster(assignment: MovieAssignment): Artwork | undefined {
  return assignment.artworkCandidates.find((image) => image.id === assignment.posterId);
}

export function artworkFileExtension(candidate: Artwork): string {
  try {
    const extension = new URL(candidate.imageUrl).pathname.split(".").at(-1)?.toLowerCase() ?? "";
    return ["jpg", "jpeg", "png", "webp"].includes(extension) ? extension : "jpg";
  } catch {
    return "jpg";
  }
}

export function movieArtworkTarget(file: OrganizerFile): ArtworkTarget {
  const extension = file.name.lastIndexOf(".");
  const stem = extension > 0 ? file.name.slice(0, extension) : file.name;
  return { directory: file.parentPath, basename: `${stem}-poster` };
}

export function seriesArtworkTargets(seriesRoot: string): ArtworkTarget[] {
  return [
    { directory: seriesRoot, basename: "folder" },
    { directory: seriesRoot, basename: "poster" },
  ];
}

export function seasonArtworkTargets(seriesRoot: string, seasonDirectory: string, seasonNumber: number): ArtworkTarget[] {
  const kodiName = seasonNumber === 0 ? "season-specials-poster" : `season${seasonNumber}-poster`;
  return [
    { directory: seasonDirectory, basename: "poster" },
    { directory: seasonDirectory, basename: seasonNumber === 0 ? "season-specials-poster" : `Season${String(seasonNumber).padStart(2, "0")}` },
    { directory: seriesRoot, basename: kodiName },
  ];
}

export function movedPathForSource(sourcePath: string, items: RenamePreview["items"]): string {
  return items.find((item) => item.sourcePath === sourcePath)?.targetPath ?? sourcePath;
}

export function providerLabel(value: Provider): string {
  return ({ tmdb: "TMDB", wikidata: "Wikidata", tvmaze: "TVmaze", tvdb: "TheTVDB", local: "Earlier review" })[value];
}

export const normalizeArt = (items: unknown): Artwork[] => Array.isArray(items) ? items as Artwork[] : [];

export function reviewableArtwork(provider: Provider, title: string, value: unknown): Artwork[] {
  const artwork = uniqueArtwork(normalizeArt(value));
  if (provider !== "wikidata") return artwork;

  // Commons search can return books, documents, and other weakly related images.
  // Show a Wikidata image only when its file title contains every meaningful title word.
  const ignored = new Set(["the", "and", "for", "from", "with", "film", "movie", "poster", "artwork"]);
  const titleWords = title.toLowerCase().match(/[a-z0-9]+/g)?.filter((word) => word.length > 2 && !ignored.has(word)) ?? [];
  if (!titleWords.length) return [];
  return artwork.filter((image) => {
    const imageTitle = image.title.toLowerCase().replace(/^file\s*:\s*/, "");
    const imageWords = new Set(imageTitle.match(/[a-z0-9]+/g) ?? []);
    return titleWords.every((word) => imageWords.has(word));
  });
}

export function candidatePreview(result: Result): Artwork | undefined {
  return reviewableArtwork(result.provider, result.title, result.artwork)[0];
}

export function providerEpisodeTextReady(review: SeriesAssignment, group: AnalysisSeriesGroup): boolean {
  if (!review.episodes.length) return false;
  for (const file of group.files) {
    if (file.season === null || file.episode === null || file.episode <= 0 || file.multipleEpisodes) continue;
    const match = review.episodes.find((item) => item.season === file.season && item.episode === file.episode);
    if (!match) continue;
    const saved = review.episodeAssignments[file.path];
    if (!saved) return false;
    if ((match.summary ?? "").trim() && !(saved.summary ?? "").trim()) return false;
    if ((match.airdate ?? "").trim() && !(saved.airdate ?? "").trim()) return false;
    if (match.title.trim() && !saved.title.trim()) return false;
  }
  return group.seasonDirectories
    .filter((season) => season.season > 0)
    .every((season) => Boolean(review.seasonRecords[String(season.season)]));
}
