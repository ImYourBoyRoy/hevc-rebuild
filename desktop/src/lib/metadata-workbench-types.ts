// ./desktop/src/lib/metadata-workbench-types.ts
// Types shared by the metadata workbench and the modules extracted from it (T10.7): providers and search results,
// filename clues, match suggestions, provider details, organizer files and scans, local assets, artwork saves and
// rename previews. Moved verbatim from MetadataWorkbench.svelte; the component keeps only its own Props type.

import type { QueueArtworkCandidate, QueueMovieReview, QueueSeriesReview } from "./types.js";

export type Provider = "wikidata" | "tvmaze" | "tvdb" | "tmdb" | "local";
export type MetadataSources = { tmdb: boolean; wikidata: boolean; tvmaze: boolean; tvdb: boolean };
export type TmdbCredentialStatus = { configured: boolean; secureStoreAvailable: boolean; message: string | null };
export type TvdbCredentialStatus = { configured: boolean; secureStoreAvailable: boolean; subscriberPinConfigured: boolean; message: string | null };
export type SearchTarget = { kind: "movie"; path: string; label: string } | { kind: "tv"; root: string; label: string };
export type MediaKind = "movie" | "tv";
export type Artwork = QueueArtworkCandidate;
export type Result = {
  id: string;
  provider: Provider;
  kind: MediaKind;
  title: string;
  year: string | null;
  description: string | null;
  rating: number | null;
  sourceUrl: string;
  confidence: number;
  artwork: Artwork[];
};
export type FilenameClue = { title: string; year: string | null; yearEnd: string | null; certification: string | null };
export type MatchSuggestion = { clue: FilenameClue; results: Result[]; warnings: string[]; status: "searching" | "ready" | "error"; error: string };
export type Season = { number: number | null; title: string | null; yearStart: string | null; yearEnd: string | null; overview?: string | null; rating?: number | null; artwork: Artwork[] };
export type Episode = { season: number; episode: number; title: string; airdate: string | null; summary: string | null; sourceUrl: string; artwork: Artwork[] };
export type Detail = {
  title: string;
  originalTitle: string | null;
  year: string | null;
  sourceUrl: string;
  shortDescription: string;
  longDescription: string;
  certification: string | null;
  runtime: number | null;
  genres: string[];
  rating: number | null;
  artwork: Artwork[];
  seasons: Season[];
  episodes: Episode[];
  attribution: string;
  episodeOrderLabel?: string;
};
export type OrganizerFile = {
  path: string; name: string; parentPath: string; extension: string;
  season: number | null; episode: number | null; multipleEpisodes: boolean;
};
export type OrganizerEpisodeGap = { season: number; missingCount: number; examples: number[] };
export type LocalArtworkFile = { path: string; name: string; role: string };
export type LocalMediaAssets = { path: string; artwork: LocalArtworkFile[] };
export type LocalDirectoryAssets = { path: string; artwork: LocalArtworkFile[] };
export type LocalAssetsScan = { root: string; files: LocalMediaAssets[]; directories: LocalDirectoryAssets[] };
export type PublicSearchResponse = { results: Result[]; warnings: string[]; successfulProviders: Provider[] };
export type MovieAssignment = QueueMovieReview;
export type SeriesAssignment = QueueSeriesReview;
export type SeasonDirectory = { path: string; name: string; season: number };
export type AnalysisSeriesGroup = { root: string; files: OrganizerFile[]; seasonDirectories: SeasonDirectory[]; ambiguous: boolean };
export type OrganizerScan = {
  root: string; renameRoot: string; files: OrganizerFile[];
  seasonDirectories: SeasonDirectory[]; episodeGaps: OrganizerEpisodeGap[]; errors: string[]; truncated: boolean;
};
export type TemplateKind = "movie" | "series" | "season" | "episode";
export type TvRenameScope = "series-tree" | "episodes-only";
export type OrganizerTemplates = { movie: string; series: string; season: string; episode: string };
export type ArtworkTarget = { directory: string; basename: string };
export type ArtworkQueueBinding =
  | { kind: "movie"; mediaPath: string; candidateId: string }
  | { kind: "series"; seriesRoot: string; candidateId: string }
  | { kind: "season"; seriesRoot: string; seasonNumber: number; candidateId: string };
export type PendingArtworkSave = { candidate: Artwork; targets: ArtworkTarget[]; label: string; queueBindings: (ArtworkQueueBinding | null)[]; saveBesideMedia: boolean };
export type PendingLocalPoster = { sourcePath: string; targetPath: string; target: ArtworkTarget; mediaPath: string; saveBesideMedia: boolean };
export type ArtworkSaveResult = { files: { path: string; attributionPath: string }[]; bytesPerFile: number; message: string };
export type RenameTemplateInput = { sourcePath: string; kind: TemplateKind; template: string; values: Record<string, string> };
export type RenamePreview = {
  previewId: string; root: string; canApply: boolean;
  items: { sourcePath: string; targetPath: string; targetName: string; isDirectory: boolean; unchanged: boolean }[];
  conflicts: { sourcePath: string | null; targetPath: string; reason: string }[];
};
