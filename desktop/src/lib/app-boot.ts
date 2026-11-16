// ./desktop/src/lib/app-boot.ts
// App boot and native listener wiring extracted from the route (boot slice): initialize() (stored
// preferences, legacy migration, window restore, queue and the parallel startup loads), the browser
// preview preference load, the mount sequence (preview vs desktop), every native event listener
// (scan, run, repair, drag-drop, resize, agent control), cleanup, and the external-link click
// handler. The route's state is reached through a typed `AppBootHost`; the DOM, window, storage,
// Tauri listen/window API and the route's actions are injected so every branch is unit-tested in
// node (pnpm test:app-boot). The code is the route's code moved verbatim with `h.` prefixes.

import type { UnlistenFn } from '@tauri-apps/api/event';
import type { AppPreferences, PreferencesPayload } from './ui-contracts.js';
import type { OrganizerTemplates, Environment, Page, RunFinishedEvent, RunLineEvent, RunProgressEvent, RunResourceEvent, ScanProgress } from './types.js';
import type { RepairFinished } from './tool-setup-controller.ts';
import { parseBrowserPreferences } from './browser-preferences.ts';
import { normalizePage, normalizeTheme, normalizeToolPaths } from './normalizers.ts';
import { normalizeDroppedPaths } from './queue-paths.ts';

export type AgentControlRequest = { id: string; action: "press" | "snapshot" | "state" | "screenshot"; key?: string | null };
export type AgentControlWindow = Window & {
  __TAURI_INTERNALS__?: unknown;
  isTauri?: boolean;
  __hevcAgentControl?: (request: AgentControlRequest) => void;
};

export interface AppBootHost {
  preferenceStorage: PreferencesPayload | null;
  appPreferences: AppPreferences;
  metadataLanguageDraft: string;
  minimumSizeMib: string;
  recursive: boolean;
  stopOnError: boolean;
  youtubeResolution: AppPreferences["youtubeResolution"];
  youtubeQualityMode: AppPreferences["youtubeQualityMode"];
  youtubeSourceMultiplier: number;
  youtubeMode: AppPreferences["youtubeMode"];
  youtubeSdrCodec: AppPreferences["youtubeSdrCodec"];
  youtubePreset: AppPreferences["youtubePreset"];
  youtubeShrink: AppPreferences["youtubeShrink"];
  youtubeQualityCheck: AppPreferences["youtubeQualityCheck"];
  youtubeOutputDir: string;
  root: string;
  page: Page;
  environmentError: string;
  booting: boolean;
  desktopShell: boolean;
  scanProgress: ScanProgress | null;
  libraryDropActive: boolean;
  runLines: string[];
  runResources: unknown;
  readonly theme: AppPreferences["theme"];
  readonly environment: Environment | null;
  readonly workflowBusy: boolean;
  readonly runId: string;
  readonly runStarting: boolean;
  readonly queueActiveRoot: string | null;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface AppBootActions {
  restoreWindowState(state: AppPreferences["windowState"]): Promise<void> | void;
  publishTheme(theme: AppPreferences["theme"]): void;
  applyToolPaths(paths: AppPreferences["toolPaths"]): void;
  persistPreferences(patch: Partial<AppPreferences>, successMessage?: string): Promise<boolean> | boolean | void;
  loadMediaQueue(): Promise<unknown> | unknown;
  loadEnvironment(): Promise<unknown> | unknown;
  loadSettings(folder?: string): Promise<unknown> | unknown;
  loadReports(): Promise<unknown> | unknown;
  refreshToolSetupLogs(preferredJobId?: string): Promise<unknown> | unknown;
  refreshTmdbCredentialStatus(): Promise<unknown> | unknown;
  refreshTvdbCredentialStatus(): Promise<unknown> | unknown;
  checkUbuntuPackageUpdates(silent?: boolean): Promise<unknown> | unknown;
  noteRunProgress(payload: RunProgressEvent): void;
  finishRun(payload: RunFinishedEvent): Promise<unknown> | unknown;
  handleRepairLine(payload: RunLineEvent): void;
  handleRepairLogWarning(payload: RunLineEvent): void;
  handleRepairFinished(payload: RepairFinished): Promise<unknown> | unknown;
  addQueueSources(paths: string[]): Promise<unknown> | unknown;
  openPage(page: Page): void;
  scheduleWindowStateSave(): void;
  handleAgentControlRequest(request: AgentControlRequest): Promise<unknown> | unknown;
  handleUtilityContextMenu(event: MouseEvent): void;
  closeContextMenu(): void;
}

export interface AppBootEnv {
  document: Pick<Document, "addEventListener" | "removeEventListener">;
  window: Window;
  localStorage: Pick<Storage, "getItem" | "setItem" | "removeItem">;
  isTauri(): boolean;
  listen<T>(event: string, handler: (event: { payload: T }) => void): Promise<UnlistenFn>;
  getCurrentWindow(): {
    onDragDropEvent(handler: (event: { payload: { type: string; paths?: string[] } }) => void): Promise<UnlistenFn>;
    onResized(handler: () => void): Promise<UnlistenFn>;
  };
  openUrl(url: string): Promise<unknown>;
}

export interface AppBootDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  host: AppBootHost;
  actions: AppBootActions;
  env: AppBootEnv;
  runEventBuffer: { line(p: RunLineEvent): void; update(p: RunProgressEvent): void; sample(p: RunResourceEvent): void; finish(p: RunFinishedEvent): void };
  constants: { browserPreferencesKey: string; defaultFilenameExcludePatterns: string[]; defaultOrganizerTemplates: OrganizerTemplates };
}

export function createAppBoot(deps: AppBootDeps) {
  const { invoke, host: h, actions, env, runEventBuffer, constants } = deps;
  const { document, window, localStorage, isTauri, listen, getCurrentWindow, openUrl } = env;
  const { browserPreferencesKey, defaultFilenameExcludePatterns, defaultOrganizerTemplates } = constants;
  const {
    restoreWindowState, publishTheme, applyToolPaths, persistPreferences, loadMediaQueue, loadEnvironment, loadSettings, loadReports, refreshToolSetupLogs,
    refreshTmdbCredentialStatus, refreshTvdbCredentialStatus, checkUbuntuPackageUpdates, noteRunProgress, finishRun, handleRepairLine, handleRepairLogWarning,
    handleRepairFinished, addQueueSources, openPage, scheduleWindowStateSave, handleAgentControlRequest, handleUtilityContextMenu, closeContextMenu,
  } = actions;
  const setNotice = h.setNotice.bind(h);
  let unlisteners: UnlistenFn[] = [];

  function loadBrowserPreferences() {
    const { saved, legacyTheme } = parseBrowserPreferences({
      key: browserPreferencesKey,
      readItem: (key) => localStorage.getItem(key),
      base: { metadataSources: h.appPreferences.metadataSources },
      defaults: { filenameExcludePatterns: defaultFilenameExcludePatterns, organizerTemplates: defaultOrganizerTemplates },
    });

    h.appPreferences = { ...h.appPreferences, ...saved };
    h.metadataLanguageDraft = h.appPreferences.metadataLanguage;
    publishTheme(normalizeTheme(h.appPreferences.theme));
    h.appPreferences.theme = h.theme;
    h.root = h.appPreferences.root ?? "";
    applyToolPaths(h.appPreferences.toolPaths);
    h.recursive = h.appPreferences.recursive;
    h.minimumSizeMib = h.appPreferences.minimumSizeMib;
    h.stopOnError = h.appPreferences.stopOnError;
    h.youtubeResolution = h.appPreferences.youtubeResolution;
    h.youtubeQualityMode = h.appPreferences.youtubeQualityMode;
    h.youtubeSourceMultiplier = h.appPreferences.youtubeSourceMultiplier;
    h.youtubeMode = h.appPreferences.youtubeMode;
    h.youtubeSdrCodec = h.appPreferences.youtubeSdrCodec;
    h.youtubePreset = h.appPreferences.youtubePreset;
    h.youtubeShrink = h.appPreferences.youtubeShrink;
    h.youtubeQualityCheck = h.appPreferences.youtubeQualityCheck;
    h.youtubeOutputDir = h.appPreferences.youtubeOutputDir ?? "";
    if (legacyTheme) localStorage.setItem(browserPreferencesKey, JSON.stringify(h.appPreferences));
  }

  function openExternalLink(event: MouseEvent) {
    if (event.defaultPrevented || event.button !== 0) return;
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!(anchor instanceof HTMLAnchorElement) || anchor.protocol !== "https:") return;
    event.preventDefault();
    void openUrl(anchor.href).catch((error) => h.setNotice(`Could not open ${anchor.hostname} in your browser: ${String(error)}`, "error"));
  }

  async function initialize() {
    try {
      const stored = await invoke<PreferencesPayload>("load_app_preferences");
      h.preferenceStorage = stored;
      h.appPreferences = stored.preferences;
      await restoreWindowState(h.appPreferences.windowState);
      h.metadataLanguageDraft = h.appPreferences.metadataLanguage;
      const migration: Partial<AppPreferences> = {};
      if (h.appPreferences.organizerTemplates.series === "{series_title} [[({year_range})]]") {
        migration.organizerTemplates = { ...h.appPreferences.organizerTemplates, series: defaultOrganizerTemplates.series };
      }
      if (!stored.hasSavedSettings) {
        try {
          const oldTheme = localStorage.getItem("hevc-rebuild-theme");
          const oldRoot = localStorage.getItem("hevc-rebuild.root");
          const oldFfmpeg = localStorage.getItem("hevc-rebuild.ffmpeg");
          const oldRecursive = localStorage.getItem("hevc-rebuild.recursive");
          const oldMinimum = localStorage.getItem("hevc-rebuild.minimumSizeMib");
          if (oldTheme) migration.theme = normalizeTheme(oldTheme);
          if (oldRoot) migration.root = oldRoot;
          if (oldFfmpeg) migration.toolPaths = normalizeToolPaths(h.appPreferences.toolPaths, oldFfmpeg);
          if (oldRecursive !== null) migration.recursive = oldRecursive !== "false";
          if (oldMinimum !== null) migration.minimumSizeMib = oldMinimum;
        } catch { /* Native preferences remain available if legacy browser storage is disabled. */ }
      }
      h.appPreferences = { ...h.appPreferences, ...migration };
      publishTheme(normalizeTheme(h.appPreferences.theme));
      h.appPreferences.theme = h.theme;
      applyToolPaths(h.appPreferences.toolPaths);
      h.minimumSizeMib = h.appPreferences.minimumSizeMib;
      h.recursive = h.appPreferences.recursive;
      h.stopOnError = h.appPreferences.stopOnError;
      h.youtubeResolution = h.appPreferences.youtubeResolution;
      h.youtubeQualityMode = h.appPreferences.youtubeQualityMode;
      h.youtubeSourceMultiplier = h.appPreferences.youtubeSourceMultiplier;
      h.youtubeMode = h.appPreferences.youtubeMode;
      h.youtubeSdrCodec = h.appPreferences.youtubeSdrCodec;
      h.youtubePreset = h.appPreferences.youtubePreset;
      h.youtubeShrink = h.appPreferences.youtubeShrink;
      h.youtubeQualityCheck = h.appPreferences.youtubeQualityCheck;
      h.youtubeOutputDir = h.appPreferences.youtubeOutputDir ?? "";
      h.root = h.appPreferences.root ?? await invoke<string>("default_media_root");
      await loadMediaQueue();
      h.page = normalizePage(h.appPreferences.lastPage);
      if (h.queueActiveRoot) migration.root = h.queueActiveRoot;
      if (!stored.hasSavedSettings || migration.root === undefined) {
        const saved = await persistPreferences({ ...migration, root: h.root });
        if (saved && !stored.hasSavedSettings) {
          try {
            ["hevc-rebuild-theme", "hevc-rebuild.root", "hevc-rebuild.ffmpeg", "hevc-rebuild.recursive", "hevc-rebuild.minimumSizeMib"].forEach((key) => localStorage.removeItem(key));
          } catch { /* The native preference file is already authoritative. */ }
        }
      } else if (Object.keys(migration).length) {
        await persistPreferences(migration);
      }
      if (stored.storageWarning) h.setNotice(stored.storageWarning, "error");
      await Promise.all([loadEnvironment(), loadSettings(h.root), loadReports(), refreshToolSetupLogs(), refreshTmdbCredentialStatus(), refreshTvdbCredentialStatus()]);
      if (h.environment?.os.package_manager === "apt" && h.environment?.os.package_manager_available !== false) await checkUbuntuPackageUpdates(true);
    } catch (error) {
      h.environmentError = String(error);
      h.setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    } finally {
      h.booting = false;
    }
  }

  function startApp(): () => void {
    let disposed = false;
    document.addEventListener("contextmenu", handleUtilityContextMenu);
    publishTheme(h.theme);
    const nativeWindow = window as AgentControlWindow;
    nativeWindow.__hevcAgentControl = (request) => {
      void handleAgentControlRequest(request);
    };
    h.desktopShell = isTauri() || Boolean(nativeWindow.__TAURI_INTERNALS__ || nativeWindow.isTauri);
    if (h.desktopShell) {
      void invoke("frontend_ready").catch((error) => {
        console.warn("Native agent-control readiness could not be reported", error);
      });
    }
    if (!h.desktopShell) {
      loadBrowserPreferences();
      h.environmentError = "Open HEVC Rebuild Studio from the desktop app to inspect this computer and connect to the media tools.";
      void Promise.all([refreshTmdbCredentialStatus(), refreshTvdbCredentialStatus()]);
      h.booting = false;
      return () => {
        disposed = true;
        document.removeEventListener("contextmenu", handleUtilityContextMenu);
      };
    }
    const register = async () => {
      const currentWindow = getCurrentWindow();
      let agentControlListener: UnlistenFn | null = null;
      try {
        agentControlListener = await listen<AgentControlRequest>("agent-control-request", (event) => {
          void handleAgentControlRequest(event.payload);
        });
      } catch (error) {
        console.warn("Native agent-control listener could not be registered", error);
      }
      const listeners = await Promise.all([
        listen<ScanProgress>("scan-progress", (event) => { h.scanProgress = event.payload; }),
        listen<RunLineEvent>("run-line", (event) => {
          if (event.payload.runId === h.runId) h.runLines = [...h.runLines, event.payload.line].slice(-240);
          else if (h.runStarting) runEventBuffer.line(event.payload);
        }),
        listen<RunProgressEvent>("run-progress", (event) => {
          if (event.payload.runId === h.runId) {
            noteRunProgress(event.payload);
          }
          else if (h.runStarting) runEventBuffer.update(event.payload);
        }),
        listen<RunResourceEvent>("run-resources", (event) => {
          if (event.payload.runId === h.runId) h.runResources = event.payload.snapshot;
          else if (h.runStarting) runEventBuffer.sample(event.payload);
        }),
        listen<RunFinishedEvent>("run-finished", async (event) => {
          if (event.payload.runId !== h.runId) {
            if (h.runStarting) runEventBuffer.finish(event.payload);
            return;
          }
          await finishRun(event.payload);
        }),
        listen<RunLineEvent>("repair-line", (event) => handleRepairLine(event.payload)),
        listen<RunLineEvent>("repair-log-warning", (event) => handleRepairLogWarning(event.payload)),
        listen<RepairFinished>("repair-finished", (event) => handleRepairFinished(event.payload)),
        currentWindow.onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over") {
            h.libraryDropActive = h.page === "library" && !h.workflowBusy;
            return;
          }
          h.libraryDropActive = false;
          if (event.payload.type !== "drop") return;
          const paths = normalizeDroppedPaths(event.payload.paths);
          if (!paths.length) return;
          if (h.workflowBusy) {
            h.setNotice("Finish the current scan or run before adding dropped files.", "error");
            return;
          }
          openPage("library");
          h.setNotice(`Adding ${paths.length} dropped path${paths.length === 1 ? "" : "s"} to the queue.`, "info");
          void addQueueSources(paths);
        }),
      ]);
      if (agentControlListener) listeners.push(agentControlListener);
      const windowResizeListener = await currentWindow.onResized(() => scheduleWindowStateSave());
      listeners.push(windowResizeListener);
      if (disposed) listeners.forEach((unlisten) => unlisten());
      else unlisteners = listeners;
    };
    void register().then(() => initialize()).catch((error) => {
      h.environmentError = String(error);
      h.booting = false;
      h.setNotice(`The desktop backend did not start: ${String(error)}`, "error");
    });
    document.addEventListener("click", openExternalLink);
    return () => {
      disposed = true;
      document.removeEventListener("contextmenu", handleUtilityContextMenu);
      document.removeEventListener("click", openExternalLink);
      unlisteners.forEach((unlisten) => unlisten());
      closeContextMenu();
    };
  }

  return { loadBrowserPreferences, openExternalLink, initialize, startApp };
}
