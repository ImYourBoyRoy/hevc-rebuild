// ./desktop/src/lib/provider-search.ts
// Provider title search extracted from MetadataWorkbench (T10.7 slice 3): which sources are asked for a movie or a series, how each
// provider's rows become results, skipped sources and their warnings, partial and total failures, de-duplication and the result
// cap, plus the filename clue call. The native bridge is injected so every branch is unit-tested in node (pnpm test:provider-search).
// The code is the component's code moved verbatim; the component's props are read through a getter host.

import type { FilenameClue, MediaKind, MetadataSources, Provider, PublicSearchResponse, Result, TmdbCredentialStatus, TvdbCredentialStatus } from "./metadata-workbench-types.js";
import { normalizeArt } from "./metadata-candidates.ts";

export interface ProviderSearchHost {
  readonly metadataSources: MetadataSources;
  readonly tmdbCredentialStatus: TmdbCredentialStatus;
  readonly tvdbCredentialStatus: TvdbCredentialStatus;
  readonly metadataLanguage: string;
  readonly textBlacklistPatterns: string[];
}

export interface ProviderSearchDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: ProviderSearchHost;
}

export function createProviderSearch(deps: ProviderSearchDeps) {
  const { host: h, invoke, isDesktop } = deps;

  async function filenameClue(filename: string): Promise<FilenameClue> {
    if (!isDesktop()) throw new Error("Filename-assisted matching is available in the installed desktop app.");
    return invoke<FilenameClue>("metadata_filename_clue", { filename, patterns: h.textBlacklistPatterns });
  }

  async function searchPublicTitles(kind: MediaKind, title: string): Promise<PublicSearchResponse> {
    const requests: { source: string; provider: Provider; run: Promise<Result[]> }[] = [];
    const warnings: string[] = [];

    if (h.metadataSources.tmdb) {
      if (!h.tmdbCredentialStatus.configured) {
        warnings.push("TMDB was skipped because no API Read Access Token is saved. Add one in Settings to include TMDB results.");
      } else {
        requests.push({
          source: "TMDB",
          provider: "tmdb",
          run: invoke<Array<Record<string, unknown>>>(kind === "movie" ? "search_tmdb_movies" : "search_tmdb_tv", {
            query: title, language: h.metadataLanguage,
          }).then((rows) => rows.map((row) => ({
            id: String(row.id), provider: "tmdb", kind,
            title: String(row.title ?? "Untitled"),
            year: typeof row.year === "string" ? row.year : null,
            description: typeof row.overview === "string" ? row.overview : null,
            rating: typeof row.rating === "number" ? row.rating : null,
            sourceUrl: String(row.sourceUrl ?? "https://www.themoviedb.org"),
            confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
          }))),
        });
      }
    }

    if (kind === "movie" && h.metadataSources.wikidata) {
      requests.push({
        source: "Wikidata",
        provider: "wikidata",
        run: invoke<Array<Record<string, unknown>>>("search_public_films", { query: title, language: h.metadataLanguage }).then((rows) => rows.map((row) => ({
          id: String(row.entityId), provider: "wikidata", kind,
          title: String(row.title ?? "Untitled"),
          year: typeof row.year === "string" ? row.year : null,
          description: typeof row.description === "string" ? row.description : null,
          rating: null, sourceUrl: String(row.sourceUrl ?? "https://www.wikidata.org"),
          confidence: Number(row.confidence ?? 0), artwork: [],
        }))),
      });
    }

    if (h.metadataSources.tvdb) {
      if (!h.tvdbCredentialStatus.configured) {
        warnings.push("TheTVDB was skipped because its project API key is not saved. Set up TheTVDB access in Settings to include its results.");
      } else {
        requests.push({
          source: "TheTVDB",
          provider: "tvdb",
          run: invoke<Array<Record<string, unknown>>>("search_tvdb_titles", {
            query: title, kind: kind === "movie" ? "movie" : "series", language: h.metadataLanguage,
          }).then((rows) => rows.map((row) => ({
            id: String(row.id), provider: "tvdb", kind,
            title: String(row.title ?? "Untitled"),
            year: typeof row.year === "string" ? row.year : null,
            description: typeof row.overview === "string" ? row.overview : null,
            rating: null, sourceUrl: String(row.sourceUrl ?? "https://www.thetvdb.com"),
            confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
          }))),
        });
      }
    }

    if (kind === "tv" && h.metadataSources.tvmaze) {
      requests.push({
        source: "TVmaze",
        provider: "tvmaze",
        run: invoke<Array<Record<string, unknown>>>("search_public_tv_shows", { query: title }).then((rows) => rows.map((row) => ({
          id: String(row.id), provider: "tvmaze", kind,
          title: String(row.title ?? "Untitled"),
          year: [row.yearStart, row.yearEnd].filter((year) => typeof year === "string").join("–") || null,
          description: typeof row.summary === "string" ? row.summary : null,
          rating: typeof row.audienceRating === "number" ? row.audienceRating : null,
          sourceUrl: String(row.sourceUrl ?? "https://www.tvmaze.com"),
          confidence: Number(row.confidence ?? 0), artwork: normalizeArt(row.artwork),
        }))),
      });
    }

    if (!requests.length) {
      const enableHint = kind === "movie"
        ? "No movie source is ready. Turn on Wikidata, save a TMDB token, or set up TheTVDB in Settings."
        : "No series source is ready. Turn on TVmaze, save a TMDB token, or set up TheTVDB in Settings.";
      throw new Error([...warnings, enableHint].join(" "));
    }

    const settled = await Promise.allSettled(requests.map((request) => request.run));
    const results: Result[] = [];
    const successfulProviders: Provider[] = [];
    settled.forEach((outcome, index) => {
      if (outcome.status === "fulfilled") {
        successfulProviders.push(requests[index].provider);
        results.push(...outcome.value);
      } else {
        warnings.push(`${requests[index].source} lookup failed: ${String(outcome.reason)}`);
      }
    });
    if (!successfulProviders.length) {
      throw new Error(warnings.join(" ") || "All enabled metadata sources failed.");
    }

    const unique = new Map<string, Result>();
    for (const result of results) unique.set(`${result.provider}:${result.id}`, result);
    return { results: [...unique.values()].slice(0, 45), warnings, successfulProviders };
  }

  return { filenameClue, searchPublicTitles };
}
