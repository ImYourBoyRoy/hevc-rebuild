// ./desktop/src/lib/tool-paths.ts
// Executable tool paths extracted from the route (helper slice): the six path drafts, normalizing and applying saved
// paths, saving and rechecking them, choosing or clearing one, the shortcut to the toolchain settings and copying the
// host setup command. Dialogs and DOM access are injected so every branch is unit-tested in node (pnpm test:tool-paths).
// The code is the route's code moved verbatim.

import type { Page, ToolPaths } from "./types.js";
import type { AppPreferences } from "./ui-contracts.js";
import { normalizeToolPaths } from "./normalizers.ts";

export type ToolPathKey = "ffmpeg" | "ffprobe" | "mediainfo" | "mkvpropedit" | "mkvmerge" | "mkvextract";

export interface ToolPathsHost {
  ffmpegPath: string;
  ffprobePath: string;
  mediainfoPath: string;
  mkvpropeditPath: string;
  mkvmergePath: string;
  mkvextractPath: string;
  desktopShell: boolean;
  readonly runId: string;
  readonly runStarting: boolean;
  readonly repairId: string;
  readonly workflowBusy: boolean;
  readonly hostSetupCommand: string;
  readonly hostSetupShell: string;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface ToolPathsActions {
  persistPreferences(patch: Partial<AppPreferences>, message?: string): Promise<boolean>;
  loadEnvironment(): Promise<boolean>;
  openDialog(options: Record<string, unknown>): Promise<string | string[] | null>;
  openPage(page: Page): void;
  tick(): Promise<void>;
}

export interface ToolPathsDeps {
  env: { clipboard: { writeText(value: string): Promise<void> }; getElementById(id: string): { scrollIntoView(options?: ScrollIntoViewOptions): void } | null };
  host: ToolPathsHost;
  actions: ToolPathsActions;
}

export function createToolPaths(deps: ToolPathsDeps) {
  const { host: h, actions, env } = deps;
  const { persistPreferences, loadEnvironment, openDialog, openPage, tick } = actions;
  const setNotice = h.setNotice.bind(h);

  async function copyHostSetupCommand() {
    try {
      await env.clipboard.writeText(h.hostSetupCommand);
      setNotice(`Setup command copied. Paste it into ${h.hostSetupShell}.`, "success");
    } catch {
      setNotice("Clipboard access is unavailable. Select the command and copy it instead.", "error");
    }
  }

  function currentToolPaths(): ToolPaths {
    return normalizeToolPaths({
      ffmpeg: h.ffmpegPath,
      ffprobe: h.ffprobePath,
      mediainfo: h.mediainfoPath,
      mkvpropedit: h.mkvpropeditPath,
      mkvmerge: h.mkvmergePath,
      mkvextract: h.mkvextractPath,
    });
  }

  function setToolPathValue(key: ToolPathKey, value: string) {
    switch (key) {
      case "ffmpeg": h.ffmpegPath = value; break;
      case "ffprobe": h.ffprobePath = value; break;
      case "mediainfo": h.mediainfoPath = value; break;
      case "mkvpropedit": h.mkvpropeditPath = value; break;
      case "mkvmerge": h.mkvmergePath = value; break;
      case "mkvextract": h.mkvextractPath = value; break;
    }
  }

  function toolPathValue(key: ToolPathKey): string {
    switch (key) {
      case "ffmpeg": return h.ffmpegPath;
      case "ffprobe": return h.ffprobePath;
      case "mediainfo": return h.mediainfoPath;
      case "mkvpropedit": return h.mkvpropeditPath;
      case "mkvmerge": return h.mkvmergePath;
      case "mkvextract": return h.mkvextractPath;
    }
  }

  function applyToolPaths(paths: ToolPaths) {
    h.ffmpegPath = paths.ffmpeg ?? "";
    h.ffprobePath = paths.ffprobe ?? "";
    h.mediainfoPath = paths.mediainfo ?? "";
    h.mkvpropeditPath = paths.mkvpropedit ?? "";
    h.mkvmergePath = paths.mkvmerge ?? "";
    h.mkvextractPath = paths.mkvextract ?? "";
  }

  async function saveToolPaths(recheck = true) {
    if (h.workflowBusy) return;
    const paths = currentToolPaths();
    if (!(await persistPreferences({ toolPaths: paths }))) return;
    if (recheck && !(await loadEnvironment())) return;
    setNotice("Saved executable paths and checked the selected tools.", "success");
  }

  async function chooseToolPath(key: ToolPathKey, label: string) {
    if (!h.desktopShell || h.runId || h.runStarting || h.repairId) return;
    const picked = await openDialog({ directory: false, multiple: false, title: `Choose ${label} executable` });
    if (typeof picked !== "string") return;
    setToolPathValue(key, picked);
    await saveToolPaths();
  }

  async function clearToolPath(key: ToolPathKey) {
    if (h.workflowBusy) return;
    setToolPathValue(key, "");
    await saveToolPaths();
  }

  async function chooseFfmpeg() {
    await chooseToolPath("ffmpeg", "FFmpeg");
  }

  async function clearFfmpegOverride() {
    await clearToolPath("ffmpeg");
  }

  async function openToolchainSettings() {
    openPage("settings");
    await tick();
    env.getElementById("environment-capabilities")?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  return { copyHostSetupCommand, currentToolPaths, setToolPathValue, toolPathValue, applyToolPaths, saveToolPaths, chooseToolPath, clearToolPath, chooseFfmpeg, clearFfmpegOverride, openToolchainSettings };
}
