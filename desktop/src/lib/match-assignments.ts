// ./desktop/src/lib/match-assignments.ts
// Saved matches and assignments extracted from MetadataWorkbench (T10.7 slice 6): saved suggestions as match sets, whether a film still needs
// a search and its button label, poster choices, automatic and seeded movie and series assignments, filename drafts for films and the catalog
// and season text applied to a series. The state and props are reached through a typed host and the bridge is injected so every branch is
// unit-tested in node (pnpm test:match-assignments). The code is the component's code moved verbatim.

import type { QueueMetadataSuggestion, QueueTvEpisode, ScanResult } from "./types.js";
import type { AnalysisSeriesGroup, Artwork, FilenameClue, MatchSuggestion, MovieAssignment, OrganizerFile, OrganizerScan, LocalAssetsScan, Result, Season, SeriesAssignment } from "./metadata-workbench-types.js";
import { episodeIdentity, fillEpisodeFromCatalog, fillSeasonFromProvider } from "./library-browse.ts";
import { candidatePreview, reviewableArtwork, suggestionMatchSet } from "./metadata-candidates.ts";
import { normalizeMatchTitle } from "./metadata-match.ts";
import { basename, buildAnalysisSeriesGroups, normalizedPath } from "./organizer-analysis.ts";

export interface MatchAssignmentsHost {
  movieAssignments: Record<string, MovieAssignment>;
  seriesAssignments: Record<string, SeriesAssignment>;
  readonly queueSuggestions: Record<string, QueueMetadataSuggestion>;
  readonly scanResult: ScanResult | null;
  readonly textBlacklistPatterns: string[];
  organizerScan: OrganizerScan | null;
  localAssetScanError: string;
  lookupMessage: string;
  lookupMessageScope: "analyzed films" | "series folders" | null;
}

export interface MatchAssignmentsActions {
  ensureLocalAssets(root: string, files: OrganizerFile[], directories?: string[]): Promise<LocalAssetsScan | null>;
}

export interface MatchAssignmentsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: MatchAssignmentsHost;
  actions: MatchAssignmentsActions;
}

export function createMatchAssignments(deps: MatchAssignmentsDeps) {
  const { host: h, actions, invoke, isDesktop } = deps;
  const { ensureLocalAssets } = actions;

  function suggestionForPath(path: string): QueueMetadataSuggestion | undefined {
    return h.queueSuggestions[path] ?? Object.entries(h.queueSuggestions).find(([key]) => normalizedPath(key) === normalizedPath(path))?.[1];
  }

  function savedMovieMatchSet(file: OrganizerFile): MatchSuggestion | null {
    return suggestionMatchSet(suggestionForPath(file.path), file.name, "movie");
  }

  function savedSeriesMatchSet(group: AnalysisSeriesGroup): MatchSuggestion | null {
    const suggestion = group.files.map((file) => suggestionForPath(file.path)).find((item) => item?.kind === "tv");
    return suggestionMatchSet(suggestion, basename(group.root), "tv");
  }

  function movieNeedsSearch(file: OrganizerFile): boolean {
    const suggestion = suggestionForPath(file.path);
    return !h.movieAssignments[file.path]?.recordId
      && suggestion?.status !== "ready";
  }

  function movieSearchCount(files: OrganizerFile[]): number {
    return files.filter(movieNeedsSearch).length;
  }

  function movieLookupLabel(file: OrganizerFile, matchSet: MatchSuggestion | null): string {
    if (matchSet?.status === "searching") return "Searching…";
    if (suggestionForPath(file.path)?.status === "ready") return "Review suggestion";
    return "Find matches";
  }

  function reviewPosterOptions(path: string, assignment: MovieAssignment | undefined, candidates: Result[] = []): Artwork[] {
    const choices = [...(assignment?.artworkCandidates ?? [])];
    const add = (image: Artwork | null | undefined, first = false) => {
      if (!image || (!image.imageUrl && !image.thumbnailUrl)) return;
      if (choices.some((existing) => existing.id === image.id)) return;
      if (first) choices.unshift(image);
      else choices.push(image);
    };
    add(suggestionForPath(path)?.artwork, true);
    for (const row of candidates) add(candidatePreview(row));
    return choices;
  }

  function autoAssignMovieMatch(path: string, clue: FilenameClue, candidate: Result): boolean {
    const previous = h.movieAssignments[path];
    if (previous?.recordId) return false;
    if (previous && normalizeMatchTitle(previous.title) !== normalizeMatchTitle(clue.title)) return false;
    const artwork = reviewableArtwork(candidate.provider, candidate.title, candidate.artwork);
    const year = candidate.year?.match(/(?:19|20)\d{2}/)?.[0] ?? clue.year ?? "";
    h.movieAssignments = {
      ...h.movieAssignments,
      [path]: {
        ...previous,
        title: candidate.title,
        year,
        certification: previous?.certification ?? "",
        recordId: `${candidate.provider}:${candidate.id}`,
        shortDescription: previous?.shortDescription || candidate.description || "",
        plot: previous?.plot || candidate.description || "",
        genresText: previous?.genresText ?? "",
        ratingText: previous?.ratingText || (candidate.rating === null ? "" : String(candidate.rating)),
        sourceUrl: candidate.sourceUrl,
        artworkCandidates: artwork,
        posterId: previous?.posterId || artwork[0]?.id || "",
        localPosterPath: previous?.localPosterPath ?? "",
      },
    };
    return true;
  }

  function autoAssignSeriesMatch(root: string, clue: FilenameClue, candidate: Result): boolean {
    const previous = h.seriesAssignments[root];
    if (previous?.recordId) return false;
    if (previous && normalizeMatchTitle(previous.title) !== normalizeMatchTitle(clue.title)) return false;
    const artwork = reviewableArtwork(candidate.provider, candidate.title, candidate.artwork);
    const year = candidate.year?.match(/(?:19|20)\d{2}/)?.[0] ?? clue.year ?? "";
    h.seriesAssignments = {
      ...h.seriesAssignments,
      [root]: {
        ...previous,
        title: candidate.title,
        year,
        certification: previous?.certification ?? "",
        recordId: `${candidate.provider}:${candidate.id}`,
        shortDescription: previous?.shortDescription || candidate.description || "",
        plot: previous?.plot || candidate.description || "",
        genresText: previous?.genresText ?? "",
        ratingText: previous?.ratingText || (candidate.rating === null ? "" : String(candidate.rating)),
        sourceUrl: candidate.sourceUrl,
        artworkCandidates: artwork,
        posterId: previous?.posterId || artwork[0]?.id || "",
        localPosterPath: previous?.localPosterPath ?? "",
        localSeasonPosterPaths: previous?.localSeasonPosterPaths ?? {},
        seasonRecords: previous?.seasonRecords ?? {},
        episodes: previous?.episodes ?? [],
        episodeAssignments: previous?.episodeAssignments ?? {},
      },
    };
    return true;
  }

  function seedSeriesAssignment(root: string, row: Result) {
    if (h.seriesAssignments[root]) return;
    const artwork = row.artwork ?? [];
    h.seriesAssignments = {
      ...h.seriesAssignments,
      [root]: {
        title: row.title,
        year: row.year ?? "",
        certification: "",
        recordId: `${row.provider}:${row.id}`,
        shortDescription: row.description ?? "",
        plot: row.description ?? "",
        genresText: "",
        ratingText: row.rating === null || row.rating === undefined ? "" : String(row.rating),
        sourceUrl: row.sourceUrl,
        artworkCandidates: artwork,
        posterId: artwork[0]?.id ?? "",
        localPosterPath: "",
        localSeasonPosterPaths: {},
        seasonRecords: {},
        episodes: [],
        episodeAssignments: {},
      },
    };
  }

  async function seedMovieDrafts(files: OrganizerFile[]) {
    if (!isDesktop() || !files.length) return;
    try {
      const assets = h.organizerScan
        ? await ensureLocalAssets(h.organizerScan.root, files, [h.organizerScan.root, ...h.organizerScan.seasonDirectories.map((season) => season.path)])
        : null;
      if (!assets) {
        if (h.localAssetScanError) {
          h.lookupMessage = `Could not inspect local artwork files: ${h.localAssetScanError}`;
          h.lookupMessageScope = "analyzed films";
        }
        return;
      }
      const clues = await invoke<FilenameClue[]>("metadata_filename_clues", {
        filenames: files.map((file) => file.name),
        patterns: h.textBlacklistPatterns,
      });
      const next = { ...h.movieAssignments };
      for (const [index, file] of files.entries()) {
        if (next[file.path] || !clues[index]) continue;
        const clue = clues[index];
        next[file.path] = {
          title: clue.title, year: clue.year ?? "",
          certification: clue.certification ?? "", recordId: "",
          shortDescription: "", plot: "",
          genresText: "", ratingText: "",
          sourceUrl: "", artworkCandidates: [], posterId: "",
          localPosterPath: "",
        };
      }
      h.movieAssignments = next;
    } catch (cause) {
      h.lookupMessage = `Could not prepare filename-based metadata drafts: ${String(cause)}`;
      h.lookupMessageScope = "analyzed films";
    }
  }

  function applyCatalogToFiles(root: string, existing: SeriesAssignment, catalog: QueueTvEpisode[]): SeriesAssignment["episodeAssignments"] {
    const assignments = { ...existing.episodeAssignments };
    const seen = new Set<string>();
    for (const file of seriesGroupForHydrate(root)?.files ?? []) {
      seen.add(file.path);
      if (file.season === null || file.episode === null || file.episode <= 0 || file.multipleEpisodes) continue;
      const match = catalog.find((item) => item.season === file.season && item.episode === file.episode);
      const current = assignments[file.path];
      if (!match && !current) continue;
      const identityTitle = episodeIdentity(file.name)?.title ?? "";
      const base = current ?? {
        season: file.season,
        episode: file.episode,
        title: identityTitle || file.name,
        airdate: null,
        summary: "",
        sourceUrl: "",
      };
      assignments[file.path] = fillEpisodeFromCatalog(base, match, existing.title, existing.plot, identityTitle);
    }
    for (const [path, episode] of Object.entries(assignments)) {
      if (seen.has(path)) continue;
      const match = catalog.find((item) => item.season === episode.season && item.episode === episode.episode);
      const name = path.split(/[\\/]/u).filter(Boolean).at(-1) ?? "";
      assignments[path] = fillEpisodeFromCatalog(episode, match, existing.title, existing.plot, episodeIdentity(name)?.title ?? "");
    }
    return assignments;
  }

  function applySeasonPublication(root: string, existing: SeriesAssignment, providerSeasons: Season[]): SeriesAssignment["seasonRecords"] {
    const records = { ...existing.seasonRecords };
    const numbers = new Set<number>();
    for (const season of providerSeasons) {
      if (typeof season.number === "number" && season.number >= 0) numbers.add(season.number);
    }
    for (const season of seriesGroupForHydrate(root)?.seasonDirectories ?? []) {
      if (season.season > 0) numbers.add(season.season);
    }
    for (const number of numbers) {
      const providerSeason = providerSeasons.find((season) => season.number === number);
      const key = String(number);
      records[key] = fillSeasonFromProvider(records[key], {
        number,
        title: providerSeason?.title ?? "",
        overview: providerSeason?.overview ?? "",
        rating: typeof providerSeason?.rating === "number" ? providerSeason.rating : null,
      }, existing.genresText);
    }
    return records;
  }

  function seriesGroupForHydrate(root: string): AnalysisSeriesGroup | undefined {
    if (!h.scanResult) return undefined;
    const wanted = normalizedPath(root);
    return buildAnalysisSeriesGroups(h.scanResult).find((group) => normalizedPath(group.root) === wanted);
  }

  return { suggestionForPath, savedMovieMatchSet, savedSeriesMatchSet, movieNeedsSearch, movieSearchCount, movieLookupLabel, reviewPosterOptions, autoAssignMovieMatch, autoAssignSeriesMatch, seedSeriesAssignment, seedMovieDrafts, applyCatalogToFiles, applySeasonPublication, seriesGroupForHydrate };
}
