import type { MediaEntry, MediaQueueRunState, Page, ReportRow, Settings, ToolPaths, OrganizerTemplates, YoutubeMode, YoutubeSdrCodec, YoutubePreset, YoutubeShrink, YoutubeQualityCheck } from './types.js';

export type Theme = "light" | "dark" | "phosphor" | "ion" | "breach";

export type MetadataSources = { tmdb: boolean; wikidata: boolean; tvmaze: boolean; tvdb: boolean };

export type TmdbCredentialStatus = { configured: boolean; secureStoreAvailable: boolean; message: string | null };

export type TvdbCredentialStatus = { configured: boolean; subscriberPinConfigured: boolean; secureStoreAvailable: boolean; message: string | null };

export type AppPreferences = {
    theme: Theme;
    windowState: { width: number; height: number; maximized: boolean };
    lastPage: Page; // "home" is accepted from older profiles and normalized to "library"
    root: string | null;
    toolPaths: ToolPaths;
    recursive: boolean;
    minimumSizeMib: string;
    stopOnError: boolean;
    youtubeResolution: "source" | "1440p" | "4k";
    youtubeQualityMode: "balanced" | "recommended";
    youtubeSourceMultiplier: number;
    youtubeMode: YoutubeMode;
    youtubeSdrCodec: YoutubeSdrCodec;
    youtubePreset: YoutubePreset;
    youtubeShrink: YoutubeShrink;
    youtubeQualityCheck: YoutubeQualityCheck;
    youtubeOutputDir: string | null;
    rebuildOutputDir: string | null;
    fileOutputDirs: Record<string, string>;
    filenameIncludePatterns: string[];
    filenameExcludePatterns: string[];
    filenameExcludeDefaultsVersion: number;
    metadataAutoLookup: boolean;
    metadataLanguage: string;
    metadataSources: MetadataSources;
    organizerTemplates: OrganizerTemplates;
    savePosterBesideMedia: boolean;
    settings: Settings | null;
  };

export type PreferencesPayload = {
    preferences: AppPreferences;
    settingsFile: string;
    hasSavedSettings: boolean;
    portable: boolean;
    storageWarning: string | null;
  };

export type RecoveryReviewItem = {
    entry: MediaEntry;
    state: MediaQueueRunState;
    report: ReportRow | null;
    targetPath: string;
    hasRecordedOutput: boolean;
  };

export type RepairToolOutcome = {
    key: string;
    label: string;
    before: string;
    after: string;
    status: "Installed" | "Updated or re-resolved" | "Ready · unchanged" | "Still missing";
  };

export type ToolSetupLogSummary = { jobId: string; modifiedUnix: number; sizeBytes: number; truncated: boolean };
