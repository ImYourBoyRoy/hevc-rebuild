// ./desktop/src/lib/state/app-state.svelte.ts
// Application-wide state: page, theme, saved preferences, boot flags, library root, scan options and tool path drafts.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { Page } from "../types.js";
import type { Theme, AppPreferences, PreferencesPayload } from "../ui-contracts.js";
import { defaultOrganizerTemplates, defaultFilenameExcludePatterns } from "./defaults.ts";

export class AppState {
  page = $state<Page>("library");
  theme = $state<Theme>("light");
  appPreferences = $state<AppPreferences>({ theme: "light", windowState: { width: 1600, height: 1000, maximized: false }, lastPage: "library", root: null, toolPaths: { ffmpeg: null, ffprobe: null, mediainfo: null, mkvpropedit: null, mkvmerge: null, mkvextract: null }, recursive: true, minimumSizeMib: "5", stopOnError: false, youtubeResolution: "1440p", youtubeQualityMode: "balanced", youtubeSourceMultiplier: 2, youtubeMode: "upscale", youtubeSdrCodec: "h264", youtubePreset: "slow", youtubeShrink: "auto", youtubeQualityCheck: "standard", youtubeOutputDir: null, rebuildOutputDir: null, fileOutputDirs: {}, filenameIncludePatterns: [], filenameExcludePatterns: [...defaultFilenameExcludePatterns], filenameExcludeDefaultsVersion: 2, metadataAutoLookup: true, metadataLanguage: "en-US", metadataSources: { tmdb: true, wikidata: true, tvmaze: true, tvdb: false }, organizerTemplates: { ...defaultOrganizerTemplates }, savePosterBesideMedia: true, autoUpdateTools: false, settings: null });
  preferenceStorage = $state<PreferencesPayload | null>(null);
  booting = $state(true);
  desktopShell = $state(false);
  root = $state("");
  recursive = $state(true);
  minimumSizeMib = $state("5");
  ffmpegPath = $state("");
  ffprobePath = $state("");
  mediainfoPath = $state("");
  mkvpropeditPath = $state("");
  mkvmergePath = $state("");
  mkvextractPath = $state("");
}
