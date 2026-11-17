// ./desktop/src/lib/window-state-controller.ts
// Theme publishing, serialized preference writes, and native window size restore/save, extracted
// from the route (window-state slice B). The route's state is reached through a small typed
// `WindowStateHost`; the native bridge, window, monitor, storage and document root are injected so
// every branch (clamping, debouncing, failures) is unit-tested in node
// (pnpm test:window-state-controller). No direct DOM or Tauri imports.

import { normalizeTheme } from './normalizers.ts';
import { themeAttributes } from './themes.ts';
import type { AppPreferences, PreferencesPayload, Theme } from './ui-contracts.js';

export interface WindowStateHost {
  appPreferences: AppPreferences;
  preferenceStorage: PreferencesPayload | null;
  theme: Theme;
  metadataLanguageDraft: string;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface WindowLike {
  scaleFactor(): Promise<number>;
  setSize(size: unknown): Promise<void>;
  maximize(): Promise<void>;
  isMaximized(): Promise<boolean>;
  innerSize(): Promise<{ width: number; height: number }>;
}

export interface MonitorLike {
  scaleFactor: number;
  workArea: { size: { toLogical(scaleFactor: number): { width: number; height: number } } };
}

export interface WindowStateDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: WindowStateHost;
  /** Wraps the visible theme switch (cross-fade). Defaults to switching immediately. */
  themeTransition?: (update: () => void) => void;
  windowApi: {
    currentWindow(): WindowLike;
    currentMonitor(): Promise<MonitorLike | null>;
    logicalSize(width: number, height: number): unknown;
  };
  storage: { setItem(key: string, value: string): void };
  browserPreferencesKey: string;
  root: { dataset: Record<string, string | undefined> };
}

export function createWindowStateController(deps: WindowStateDeps) {
  const { invoke, host: h, windowApi, storage, browserPreferencesKey, root } = deps;
  const runThemeTransition = deps.themeTransition ?? ((update: () => void) => update());
  let preferenceSaveQueue: Promise<void> = Promise.resolve();
  let windowStateSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let windowPreferencesReady = false;
  let restoringWindowState = false;

  function publishTheme(value: Theme) {
    const attributes = themeAttributes(value);
    h.theme = value;
    root.dataset.theme = attributes.theme;
    root.dataset.palette = attributes.palette;
    if (attributes.instrument) root.dataset.instrument = attributes.instrument;
    else delete root.dataset.instrument;
  }

  function persistPreferences(patch: Partial<AppPreferences>, successMessage?: string) {
    h.appPreferences = { ...h.appPreferences, ...patch };
    if (patch.metadataLanguage) h.metadataLanguageDraft = patch.metadataLanguage;
    const snapshot = JSON.parse(JSON.stringify(h.appPreferences)) as AppPreferences;
    const operation = preferenceSaveQueue.catch(() => {}).then(async () => {
      if (deps.isDesktop()) {
        h.preferenceStorage = await invoke<PreferencesPayload>("save_app_preferences", { preferences: snapshot });
      } else {
        storage.setItem(browserPreferencesKey, JSON.stringify(snapshot));
      }
      if (successMessage) h.setNotice(successMessage, "success");
    });
    preferenceSaveQueue = operation.then(() => {}, () => {});
    return operation.then(() => true, (error) => {
      h.setNotice(`Could not save application settings: ${String(error)}`, "error");
      return false;
    });
  }

  function applyTheme(value: string) {
    const theme = normalizeTheme(value);
    // The switch may be deferred by the cross-fade, so persist the chosen value, not h.theme.
    runThemeTransition(() => publishTheme(theme));
    void persistPreferences({ theme });
  }

  async function restoreWindowState(saved: AppPreferences["windowState"]) {
    if (!deps.isDesktop()) return;
    const currentWindow = windowApi.currentWindow();
    restoringWindowState = true;
    try {
      const monitor = await windowApi.currentMonitor();
      const scaleFactor = monitor?.scaleFactor ?? await currentWindow.scaleFactor();
      const workArea = monitor?.workArea.size.toLogical(scaleFactor);
      const maxWidth = workArea ? Math.max(1120, Math.floor(workArea.width - 40)) : 4096;
      const maxHeight = workArea ? Math.max(720, Math.floor(workArea.height - 40)) : 4096;
      const width = Math.min(Math.max(saved.width, 1120), maxWidth);
      const height = Math.min(Math.max(saved.height, 720), maxHeight);
      await currentWindow.setSize(windowApi.logicalSize(width, height));
      if (saved.maximized) await currentWindow.maximize();
    } catch (error) {
      h.setNotice(`Could not restore the previous window size: ${String(error)}`, "error");
    } finally {
      setTimeout(() => {
        restoringWindowState = false;
        windowPreferencesReady = true;
      }, 350);
    }
  }

  function scheduleWindowStateSave() {
    if (!deps.isDesktop() || !windowPreferencesReady || restoringWindowState) return;
    if (windowStateSaveTimer !== null) clearTimeout(windowStateSaveTimer);
    windowStateSaveTimer = setTimeout(() => {
      windowStateSaveTimer = null;
      void saveCurrentWindowState();
    }, 500);
  }

  async function saveCurrentWindowState() {
    if (!deps.isDesktop() || !windowPreferencesReady || restoringWindowState) return;
    try {
      const currentWindow = windowApi.currentWindow();
      const maximized = await currentWindow.isMaximized();
      const next = { ...h.appPreferences.windowState, maximized };
      if (!maximized) {
        const [size, scaleFactor] = await Promise.all([currentWindow.innerSize(), currentWindow.scaleFactor()]);
        next.width = Math.round(size.width / scaleFactor);
        next.height = Math.round(size.height / scaleFactor);
      }
      if (JSON.stringify(next) === JSON.stringify(h.appPreferences.windowState)) return;
      h.appPreferences = { ...h.appPreferences, windowState: next };
      await persistPreferences({ windowState: next });
    } catch (error) {
      h.setNotice(`Could not save the window size: ${String(error)}`, "error");
    }
  }

  return { publishTheme, applyTheme, persistPreferences, restoreWindowState, scheduleWindowStateSave, saveCurrentWindowState };
}
