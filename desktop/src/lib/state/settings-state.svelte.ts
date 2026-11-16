// ./desktop/src/lib/state/settings-state.svelte.ts
// Effective settings draft and baselines, work-directory preview, language draft and the filter/blacklist drafts.
// A plain rune class: its instance is the typed host the controller works on, so the route needs no
// per-field accessors. Members marked `declare` are supplied by the route (see withGetters).
import type { Settings, WorkDirectoryPreview } from "../types.js";
import type { AppPreferences } from "../ui-contracts.js";

export class SettingsState {
  settings = $state<Settings | null>(null);
  settingsSources = $state<string[]>([]);
  settingsPath = $state("");
  workDirectory = $state<WorkDirectoryPreview | null>(null);
  settingsBaseline = $state("");
  librarySettingsBaseline = $state("");
  settingsRoot = $state("");
  metadataLanguageDraft = $state("en-US");
  newMetadataBlacklistPattern = $state("");
  metadataBlacklistError = $state("");
  newFilenameIncludePattern = $state("");
  newFilenameExcludePattern = $state("");
  filenameFilterError = $state("");
  setNotice: (message: string, kind?: "info" | "success" | "error") => void = () => {};
  declare readonly appPreferences: AppPreferences;
  declare readonly root: string;
  declare readonly activeRoot: string | null;
  declare readonly workflowBusy: boolean;
  declare readonly settingsChanged: boolean;
}
