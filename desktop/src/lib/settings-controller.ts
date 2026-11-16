// ./desktop/src/lib/settings-controller.ts
// Effective-settings load/reset/save, the work-directory preview, the metadata language draft, the
// metadata provider toggles, the title-cleanup blacklist and the filename include/exclude filters,
// extracted from the route (settings slice). The route's state is reached through a typed
// `SettingsHost`; the native bridge and preference persistence are injected so every branch is
// unit-tested in node (pnpm test:settings-controller). The code is the route's code moved verbatim
// with `h.` prefixes.

import type { AppPreferences, MetadataSources } from './ui-contracts.js';
import type { Settings, WorkDirectoryPreview } from './types.js';

export type SettingsLoad = { settings: Settings; sources: string[]; user_config: string; work_directory: WorkDirectoryPreview };

export interface SettingsHost {
  settings: Settings | null;
  settingsBaseline: string;
  librarySettingsBaseline: string;
  settingsRoot: string;
  settingsSources: string[];
  settingsPath: string;
  workDirectory: WorkDirectoryPreview | null;
  metadataLanguageDraft: string;
  newMetadataBlacklistPattern: string;
  metadataBlacklistError: string;
  newFilenameIncludePattern: string;
  newFilenameExcludePattern: string;
  filenameFilterError: string;
  readonly appPreferences: AppPreferences;
  readonly root: string;
  readonly activeRoot: string | null;
  readonly workflowBusy: boolean;
  readonly settingsChanged: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface SettingsDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: SettingsHost;
  persistPreferences: (patch: Partial<AppPreferences>, successMessage?: string) => Promise<boolean> | boolean | void;
}

export function createSettingsController(deps: SettingsDeps) {
  const { invoke, isDesktop, host: h, persistPreferences } = deps;

  async function loadSettings(folder = h.root) {
    if (!folder.trim()) return;
    try {
      const payload = await invoke<SettingsLoad>("load_settings", { root: folder.trim() });
      h.librarySettingsBaseline = JSON.stringify(payload.settings);
      h.settings = payload.settings;
      h.settingsBaseline = JSON.stringify(h.settings);
      h.settingsRoot = folder.trim();
      h.settingsSources = payload.sources;
      h.settingsPath = payload.user_config;
      h.workDirectory = payload.work_directory;
    } catch (error) {
      h.setNotice(`Could not load the effective settings: ${String(error)}`, "error");
    }
  }

  async function previewWorkDirectory(workRoot: string) {
    const source = h.root.trim() || h.activeRoot;
    if (!isDesktop() || !source) return;
    try {
      h.workDirectory = await invoke<WorkDirectoryPreview>("resolve_work_directory_command", {
        root: source,
        workRoot: workRoot.trim() || "auto",
      });
    } catch {
      // The settings editor validates the value on save/start. Keep the last
      // valid source preview visible while a new value is being edited.
    }
  }

  async function resetSettings() {
    if (!h.settingsBaseline || h.workflowBusy) return;
    try {
      if (h.appPreferences.settings) {
        const folder = h.root.trim() || h.settingsRoot;
        if (await persistPreferences({ settings: null }, "Removed the saved application profile.") && folder) await loadSettings(folder);
      } else {
        h.settings = JSON.parse(h.settingsBaseline) as Settings;
        h.setNotice("Restored the effective settings for this library.", "success");
      }
    } catch (error) {
      h.setNotice(`Could not restore the effective settings: ${String(error)}`, "error");
    }
  }

  async function saveSettings() {
    if (!h.settings || !h.settingsChanged || h.workflowBusy) return;
    const snapshot = JSON.parse(JSON.stringify(h.settings)) as Settings;
    if (await persistPreferences({ settings: snapshot }, "Saved these rebuild settings as this app's defaults.")) {
      h.settingsBaseline = JSON.stringify(snapshot);
      h.librarySettingsBaseline = h.librarySettingsBaseline || h.settingsBaseline;
      if (!h.settingsSources.includes("Saved application profile")) h.settingsSources = [...h.settingsSources, "Saved application profile"];
    }
  }

  function saveMetadataLanguageDraft() {
    const parts = h.metadataLanguageDraft.trim().match(/^([a-zA-Z]{2})(?:-([a-zA-Z]{2}))?$/);
    if (!parts) {
      h.metadataLanguageDraft = h.appPreferences.metadataLanguage;
      h.setNotice("Use a two-letter language code with an optional two-letter region, such as en-US, fr-FR, or ja-JP.", "error");
      return;
    }
    const normalized = parts[2] ? `${parts[1].toLowerCase()}-${parts[2].toUpperCase()}` : parts[1].toLowerCase();
    h.metadataLanguageDraft = normalized;
    if (normalized !== h.appPreferences.metadataLanguage) void persistPreferences({ metadataLanguage: normalized });
  }

  function updateMetadataSources(patch: Partial<MetadataSources>) {
    void persistPreferences({ metadataSources: { ...h.appPreferences.metadataSources, ...patch } });
  }

  function addMetadataBlacklistPattern() {
    if (!h.settings) return;
    const pattern = h.newMetadataBlacklistPattern.trim();
    if (!pattern) {
      h.metadataBlacklistError = "Enter a pattern first.";
      return;
    }
    if (new TextEncoder().encode(pattern).length > 128) {
      h.metadataBlacklistError = "Patterns can be up to 128 UTF-8 bytes.";
      return;
    }
    if (/^\*+$/.test(pattern)) {
      h.metadataBlacklistError = "Add at least one letter or number; a wildcard-only pattern would remove all text.";
      return;
    }
    if (/[\u0000-\u001f\u007f]/.test(pattern)) {
      h.metadataBlacklistError = "Control characters are not allowed in patterns.";
      return;
    }
    const existing = h.settings.metadata.text_blacklist ?? [];
    if (existing.some((item) => item.toLocaleLowerCase() === pattern.toLocaleLowerCase())) {
      h.metadataBlacklistError = "That pattern is already in the list.";
      return;
    }
    if (existing.length >= 100) {
      h.metadataBlacklistError = "The list is limited to 100 patterns.";
      return;
    }
    h.settings.metadata.text_blacklist = [...existing, pattern];
    h.newMetadataBlacklistPattern = "";
    h.metadataBlacklistError = "";
  }

  function removeMetadataBlacklistPattern(index: number) {
    if (!h.settings) return;
    h.settings.metadata.text_blacklist = h.settings.metadata.text_blacklist.filter((_, itemIndex) => itemIndex !== index);
    h.metadataBlacklistError = "";
  }

  function clearMetadataBlacklist() {
    if (!h.settings) return;
    h.settings.metadata.text_blacklist = [];
    h.metadataBlacklistError = "";
  }

  function addFilenamePattern(kind: "include" | "exclude") {
    const draft = kind === "include" ? h.newFilenameIncludePattern : h.newFilenameExcludePattern;
    const pattern = draft.trim();
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    const current = h.appPreferences[key];
    if (!pattern) { h.filenameFilterError = "Enter a filename pattern first."; return; }
    if (new TextEncoder().encode(pattern).length > 128) { h.filenameFilterError = "Filename patterns can be up to 128 UTF-8 bytes."; return; }
    if (/[\\/\u0000-\u001f\u007f]/.test(pattern)) { h.filenameFilterError = "Use a filename pattern without folder separators or control characters."; return; }
    if ([...pattern].every((character) => character === "*" || character === "?")) { h.filenameFilterError = "Add at least one filename character; wildcard-only filters can hide the entire library."; return; }
    if (current.some((value) => value.toLocaleLowerCase() === pattern.toLocaleLowerCase())) { h.filenameFilterError = "That pattern is already in this list."; return; }
    if (current.length >= 100) { h.filenameFilterError = "Each list is limited to 100 patterns."; return; }
    const updated = [...current, pattern];
    void persistPreferences({ [key]: updated } as Partial<AppPreferences>);
    if (kind === "include") h.newFilenameIncludePattern = "";
    else h.newFilenameExcludePattern = "";
    h.filenameFilterError = "";
  }

  function removeFilenamePattern(kind: "include" | "exclude", index: number) {
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    const updated = h.appPreferences[key].filter((_, itemIndex) => itemIndex !== index);
    void persistPreferences({ [key]: updated } as Partial<AppPreferences>);
    h.filenameFilterError = "";
  }

  function clearFilenamePatterns(kind: "include" | "exclude") {
    const key = kind === "include" ? "filenameIncludePatterns" : "filenameExcludePatterns";
    void persistPreferences({ [key]: [] } as Partial<AppPreferences>);
    h.filenameFilterError = "";
  }

  return { loadSettings, previewWorkDirectory, resetSettings, saveSettings, saveMetadataLanguageDraft, updateMetadataSources, addMetadataBlacklistPattern, removeMetadataBlacklistPattern, clearMetadataBlacklist, addFilenamePattern, removeFilenamePattern, clearFilenamePatterns };
}
