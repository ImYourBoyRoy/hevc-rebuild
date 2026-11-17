// ./desktop/src/lib/file-pickers.ts
// Native file and folder pickers extracted from the route (helper slice): adding queue sources from a folder or file
// dialog, the YouTube, default-rebuild and per-file output folders, the typed-path analyze action, rescanning a source,
// opening a file location and cancelling a scan. The dialog and native bridge are injected so every branch is
// unit-tested in node (pnpm test:file-pickers). The code is the route's code moved verbatim.

import type { AppPreferences } from "./ui-contracts.js";
import { fileName } from "./queue-paths.ts";

export interface FilePickersHost {
  desktopShell: boolean;
  root: string;
  appPreferences: AppPreferences;
  youtubeOutputDir: string;
  readonly activeRoot: string | null;
  readonly runId: string;
  readonly runStarting: boolean;
  readonly repairId: string;
  readonly scanning: boolean;
  readonly workflowBusy: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface FilePickersActions {
  openDialog(options: Record<string, unknown>): Promise<string | string[] | null>;
  addQueueSources(paths: string[]): Promise<void>;
  scanQueueSource(root: string, activate: boolean, refresh: boolean): Promise<void>;
  persistPreferences(patch: Partial<AppPreferences>, message?: string): Promise<boolean>;
}

export interface FilePickersDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  host: FilePickersHost;
  actions: FilePickersActions;
}

export function createFilePickers(deps: FilePickersDeps) {
  const { host: h, actions, invoke } = deps;
  const { openDialog, addQueueSources, scanQueueSource, persistPreferences } = actions;
  const setNotice = h.setNotice.bind(h);

  async function browseFolder() {
    if (!h.desktopShell || h.runId || h.runStarting || h.repairId || h.scanning) return;
    let defaultPath = h.root.trim() || h.activeRoot || h.appPreferences.root || undefined;
    if (defaultPath && /\.(mkv|mp4|m4v|mov|avi|ts|m2ts|mts|webm)$/i.test(defaultPath)) {
      const separator = Math.max(defaultPath.lastIndexOf("/"), defaultPath.lastIndexOf("\\"));
      defaultPath = separator > 0 ? defaultPath.slice(0, separator) : undefined;
    }
    if (!defaultPath) {
      try { defaultPath = await invoke<string>("default_media_root"); } catch { /* The native dialog can use its platform default. */ }
    }
    const picked = await openDialog({ directory: true, multiple: true, title: "Add media folder(s) to the queue", defaultPath });
    const paths = typeof picked === "string" ? [picked] : Array.isArray(picked) ? picked : [];
    if (paths.length) await addQueueSources(paths);
  }

  async function browseFile() {
    if (!h.desktopShell || h.runId || h.runStarting || h.repairId || h.scanning) return;
    const picked = await openDialog({
      directory: false,
      multiple: true,
      title: "Add media file(s) to the queue",
      filters: [{ name: "Video files", extensions: ["mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm"] }],
    });
    const paths = typeof picked === "string" ? [picked] : Array.isArray(picked) ? picked : [];
    if (paths.length) await addQueueSources(paths);
  }

  async function chooseYoutubeOutput() {
    if (!h.desktopShell || h.runId || h.runStarting || h.repairId || h.scanning) return;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose the YouTube export folder" });
    if (typeof picked !== "string") return;
    h.youtubeOutputDir = picked;
    await persistPreferences({ youtubeOutputDir: h.youtubeOutputDir });
    setNotice("YouTube exports will be saved to the selected folder.", "success");
  }

  async function chooseRebuildOutput() {
    if (!h.desktopShell || h.workflowBusy) return;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose the default rebuild output folder" });
    if (typeof picked !== "string") return;
    await persistPreferences({ rebuildOutputDir: picked }, "Default rebuild outputs will be saved to this folder. Original files will stay where they are.");
  }

  async function chooseFileOutput(path: string) {
    if (!h.desktopShell || h.workflowBusy) return;
    const picked = await openDialog({ directory: true, multiple: false, title: `Choose output folder for ${fileName(path)}` });
    if (typeof picked !== "string") return;
    const fileOutputDirs = { ...h.appPreferences.fileOutputDirs, [path]: picked };
    if (Object.keys(fileOutputDirs).length > 512) {
      setNotice("Per-file output folders are limited to 512 media files.", "error");
      return;
    }
    await persistPreferences({ fileOutputDirs });
  }

  function clearFileOutput(path: string) {
    const fileOutputDirs = { ...h.appPreferences.fileOutputDirs };
    delete fileOutputDirs[path];
    void persistPreferences({ fileOutputDirs });
  }

  async function analyzeFolder() {
    if (!h.root.trim()) {
      setNotice("Choose a media file or folder to add to the queue.", "error");
      return;
    }
    await scanQueueSource(h.root.trim(), true, true);
  }

  function refreshQueueSource(sourceRoot: string) {
    void scanQueueSource(sourceRoot, true, true);
  }

  async function openFileLocation(path: string) {
    if (!path || path.includes("::season:")) {
      setNotice("That folder is not available.", "error");
      return;
    }
    try {
      await invoke("open_file_location", { path });
    } catch (cause) {
      setNotice(String(cause), "error");
    }
  }

  async function cancelScan() {
    try {
      await invoke("cancel_scan");
      setNotice("Stopping the scan after the current file…", "info");
    } catch (error) {
      if (h.scanning) setNotice(`Could not cancel the scan: ${String(error)}`, "error");
    }
  }

  return { browseFolder, browseFile, chooseYoutubeOutput, chooseRebuildOutput, chooseFileOutput, clearFileOutput, analyzeFolder, refreshQueueSource, openFileLocation, cancelScan };
}
