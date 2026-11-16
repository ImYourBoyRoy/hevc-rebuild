// ./desktop/src/lib/browser-preferences.ts
// Parser for the browser-preview preferences kept in localStorage, extracted from the route
// (window-state slice A). It accepts older saved shapes, validates every field on its own and
// never throws: unavailable or invalid storage simply yields fewer fields. The caller merges the
// result into its defaults and rewrites the entry when the legacy "hacker" skin was found.
// Pure: storage reads are injected. Tested by pnpm test:browser-preferences.

import { normalizePage, normalizeTheme, normalizeToolPaths } from './normalizers.ts';
import type { MetadataSources, AppPreferences } from './ui-contracts.js';
import type { OrganizerTemplates } from './types.js';

export interface BrowserPreferencesInput {
  /** Storage key of the saved preferences object. */
  key: string;
  readItem: (key: string) => string | null;
  /** Current values used as fallbacks for partially saved sub-objects. */
  base: { metadataSources: MetadataSources };
  defaults: { filenameExcludePatterns: readonly string[]; organizerTemplates: OrganizerTemplates };
}

export function parseBrowserPreferences({ key, readItem, base, defaults }: BrowserPreferencesInput): { saved: Partial<AppPreferences>; legacyTheme: boolean } {
    let saved: Partial<AppPreferences> = {};
    let legacyTheme = false;
    try {
      const raw = readItem(key);
      if (raw) {
        const value: unknown = JSON.parse(raw);
        if (value && typeof value === "object") {
          const candidate = value as Omit<Partial<AppPreferences>, "theme"> & { theme?: unknown; ffmpegPath?: unknown };
          if (candidate.theme === "hacker") legacyTheme = true;
          if (typeof candidate.theme === "string") saved.theme = normalizeTheme(candidate.theme);
          if (typeof candidate.lastPage === "string") saved.lastPage = normalizePage(candidate.lastPage);
          if (typeof candidate.root === "string" || candidate.root === null) saved.root = candidate.root;
          if (candidate.toolPaths && typeof candidate.toolPaths === "object") {
            saved.toolPaths = normalizeToolPaths(candidate.toolPaths, candidate.ffmpegPath);
          } else if (typeof candidate.ffmpegPath === "string" || candidate.ffmpegPath === null) {
            saved.toolPaths = normalizeToolPaths({}, candidate.ffmpegPath);
          }
          if (typeof candidate.recursive === "boolean") saved.recursive = candidate.recursive;
          if (typeof candidate.minimumSizeMib === "string") saved.minimumSizeMib = candidate.minimumSizeMib;
          if (typeof candidate.stopOnError === "boolean") saved.stopOnError = candidate.stopOnError;
          if (candidate.youtubeResolution === "source" || candidate.youtubeResolution === "1440p" || candidate.youtubeResolution === "4k") {
            saved.youtubeResolution = candidate.youtubeResolution === "source" && candidate.youtubeQualityMode === undefined
              ? "1440p"
              : candidate.youtubeResolution;
          }
          if (candidate.youtubeQualityMode === "balanced" || candidate.youtubeQualityMode === "recommended") saved.youtubeQualityMode = candidate.youtubeQualityMode;
          if (typeof candidate.youtubeSourceMultiplier === "number" && Number.isFinite(candidate.youtubeSourceMultiplier) && candidate.youtubeSourceMultiplier >= 0.5 && candidate.youtubeSourceMultiplier <= 4) saved.youtubeSourceMultiplier = candidate.youtubeSourceMultiplier;
          if (candidate.youtubeMode === "upscale" || candidate.youtubeMode === "optimize") saved.youtubeMode = candidate.youtubeMode;
          if (candidate.youtubeSdrCodec === "h264" || candidate.youtubeSdrCodec === "hevc") saved.youtubeSdrCodec = candidate.youtubeSdrCodec;
          if (candidate.youtubePreset === "slow" || candidate.youtubePreset === "medium" || candidate.youtubePreset === "slower") saved.youtubePreset = candidate.youtubePreset;
          if (candidate.youtubeShrink === "auto" || candidate.youtubeShrink === "off" || candidate.youtubeShrink === "aggressive") saved.youtubeShrink = candidate.youtubeShrink;
          if (candidate.youtubeQualityCheck === "standard" || candidate.youtubeQualityCheck === "off" || candidate.youtubeQualityCheck === "relaxed" || candidate.youtubeQualityCheck === "strict") saved.youtubeQualityCheck = candidate.youtubeQualityCheck;
          if (typeof candidate.youtubeOutputDir === "string" || candidate.youtubeOutputDir === null) saved.youtubeOutputDir = candidate.youtubeOutputDir;
          if (typeof candidate.rebuildOutputDir === "string" || candidate.rebuildOutputDir === null) saved.rebuildOutputDir = candidate.rebuildOutputDir;
          if (candidate.fileOutputDirs && typeof candidate.fileOutputDirs === "object" && !Array.isArray(candidate.fileOutputDirs)) {
            const entries = Object.entries(candidate.fileOutputDirs as Record<string, unknown>).filter((item): item is [string, string] => typeof item[1] === "string");
            if (entries.length <= 512) saved.fileOutputDirs = Object.fromEntries(entries);
          }
          if (Array.isArray(candidate.filenameIncludePatterns) && candidate.filenameIncludePatterns.every((item) => typeof item === "string")) saved.filenameIncludePatterns = candidate.filenameIncludePatterns as string[];
          if (Array.isArray(candidate.filenameExcludePatterns) && candidate.filenameExcludePatterns.every((item) => typeof item === "string")) {
            const excludes = candidate.filenameExcludePatterns as string[];
            const defaultsApplied = candidate.filenameExcludeDefaultsVersion === 2;
            saved.filenameExcludePatterns = defaultsApplied
              ? excludes
              : [...defaults.filenameExcludePatterns, ...excludes.filter((pattern) => !defaults.filenameExcludePatterns.some((fallback) => fallback.toLocaleLowerCase() === pattern.toLocaleLowerCase()))];
            saved.filenameExcludeDefaultsVersion = 2;
          }
          if (typeof candidate.metadataAutoLookup === "boolean") saved.metadataAutoLookup = candidate.metadataAutoLookup;
          if (typeof candidate.metadataLanguage === "string" && /^[a-zA-Z]{2}(?:-[a-zA-Z]{2})?$/.test(candidate.metadataLanguage)) saved.metadataLanguage = candidate.metadataLanguage;
          if (candidate.metadataSources && typeof candidate.metadataSources === "object" && !Array.isArray(candidate.metadataSources)) {
            const sources = candidate.metadataSources as Partial<MetadataSources>;
            saved.metadataSources = {
              tmdb: typeof sources.tmdb === "boolean" ? sources.tmdb : base.metadataSources.tmdb,
              wikidata: typeof sources.wikidata === "boolean" ? sources.wikidata : base.metadataSources.wikidata,
              tvmaze: typeof sources.tvmaze === "boolean" ? sources.tvmaze : base.metadataSources.tvmaze,
              tvdb: typeof sources.tvdb === "boolean" ? sources.tvdb : base.metadataSources.tvdb,
            };
          }
          if (candidate.organizerTemplates && typeof candidate.organizerTemplates === "object") {
            const templates = candidate.organizerTemplates as Partial<OrganizerTemplates>;
            saved.organizerTemplates = {
              movie: typeof templates.movie === "string" ? templates.movie : defaults.organizerTemplates.movie,
              series: templates.series === "{series_title} [[({year_range})]]"
                ? defaults.organizerTemplates.series
                : typeof templates.series === "string" ? templates.series : defaults.organizerTemplates.series,
              season: typeof templates.season === "string" ? templates.season : defaults.organizerTemplates.season,
              episode: typeof templates.episode === "string" ? templates.episode : defaults.organizerTemplates.episode,
            };
          }
          if (candidate.settings === null || (candidate.settings && typeof candidate.settings === "object")) saved.settings = candidate.settings;
        }
      } else {
        const oldTheme = readItem("hevc-rebuild-theme");
        const oldRoot = readItem("hevc-rebuild.root");
        const oldFfmpeg = readItem("hevc-rebuild.ffmpeg");
        const oldRecursive = readItem("hevc-rebuild.recursive");
        const oldMinimum = readItem("hevc-rebuild.minimumSizeMib");
        if (oldTheme === "hacker") legacyTheme = true;
        if (oldTheme) saved.theme = normalizeTheme(oldTheme);
        if (oldRoot) saved.root = oldRoot;
        if (oldFfmpeg) saved.toolPaths = normalizeToolPaths(saved.toolPaths, oldFfmpeg);
        if (oldRecursive !== null) saved.recursive = oldRecursive !== "false";
        if (oldMinimum !== null) saved.minimumSizeMib = oldMinimum;
      }
    } catch {
      // The browser preview is optional; unavailable or invalid browser storage does not block the UI.
    }
  return { saved, legacyTheme };
}
