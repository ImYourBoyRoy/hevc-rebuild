import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

export { isTauri };

export type UpdateUiState = "idle" | "checking" | "current" | "available" | "installing" | "error";

export type UpdateCheckResult = {
  state: Extract<UpdateUiState, "current" | "available" | "error">;
  update?: Update;
  message: string;
};

export type InstallUpdateResult = {
  installed: true;
  relaunched: boolean;
};

const DISMISSED_UPDATE_KEY = "hevc-rebuild-studio-dismissed-update";
const CHECK_TIMEOUT_MS = 10_000;
const MAX_UPDATE_NOTES_LENGTH = 4_000;

let dismissedUpdateMemory: string | undefined;

export function formatVersionLabel(version: string | undefined | null): string {
  const trimmed = (version ?? "").trim();
  if (!trimmed) return "v?";
  return /^v/i.test(trimmed) ? `v${trimmed.slice(1)}` : `v${trimmed}`;
}

function comparableVersion(version: string | undefined | null): string | undefined {
  const trimmed = (version ?? "").trim();
  if (!trimmed) return undefined;
  return trimmed.replace(/^v/i, "");
}

export function formatUpdateNotes(body: string | undefined | null): string | undefined {
  const notes = (body ?? "")
    .replace(/\r\n?/g, "\n")
    .replace(/^[ \t]{0,3}#{1,6}[ \t]+/gm, "")
    .replace(/\*\*(.*?)\*\*/g, "$1")
    .replace(/__(.*?)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^[ \t]*[-*][ \t]+/gm, "• ")
    .trim();
  if (!notes) return undefined;
  if (notes.length <= MAX_UPDATE_NOTES_LENGTH) return notes;
  return `${notes.slice(0, MAX_UPDATE_NOTES_LENGTH - 1).trimEnd()}…`;
}

export function updaterErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message.trim();
  if (typeof error === "string" && error.trim()) return error.trim();
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    for (const key of ["message", "detail", "error"]) {
      const value = record[key];
      if (typeof value === "string" && value.trim()) return value.trim();
    }
  }
  return "The signed updater could not complete the installation.";
}

export function getDismissedUpdateVersion(): string | undefined {
  try {
    const value = globalThis.localStorage?.getItem(DISMISSED_UPDATE_KEY)?.trim();
    if (value) return value;
  } catch {
    // Fall through to the in-memory fallback.
  }
  return dismissedUpdateMemory;
}

export function setDismissedUpdateVersion(version: string | undefined): void {
  const comparable = comparableVersion(version);
  dismissedUpdateMemory = comparable;
  try {
    if (!comparable) globalThis.localStorage?.removeItem(DISMISSED_UPDATE_KEY);
    else globalThis.localStorage?.setItem(DISMISSED_UPDATE_KEY, comparable);
  } catch {
    // The in-memory value still prevents repeated prompts in this session.
  }
}

export function shouldShowUpdateBanner(
  availableVersion: string | undefined,
  dismissedVersion: string | undefined = getDismissedUpdateVersion(),
): boolean {
  const available = comparableVersion(availableVersion);
  if (!available) return false;
  return available !== comparableVersion(dismissedVersion);
}

export function updateProgressPercent(downloaded: number, contentLength: number): number {
  if (contentLength <= 0) return 0;
  return Math.min(100, Math.round((downloaded / contentLength) * 100));
}

export async function getAppVersion(): Promise<string> {
  if (!isTauri()) return "0.0.0";
  return getVersion();
}

export async function checkForAppUpdate(options: { quiet?: boolean; previous?: Update } = {}): Promise<UpdateCheckResult> {
  if (!isTauri()) {
    return {
      state: "error",
      message: "Update checks are available in the desktop build",
    };
  }

  try {
    const update = await check({ timeout: CHECK_TIMEOUT_MS });
    if (options.previous) await options.previous.close().catch(() => undefined);
    if (!update) {
      return {
        state: "current",
        message: "HEVC Rebuild Studio is up to date.",
      };
    }
    return {
      state: "available",
      update,
      message: `Update available: ${formatVersionLabel(update.version)}`,
    };
  } catch (error) {
    return {
      state: "error",
      message: options.quiet ? "" : updaterErrorMessage(error),
    };
  }
}

export async function installAppUpdate(
  update: Update,
  options: { confirmed: boolean; onProgress?: (percent: number) => void },
): Promise<InstallUpdateResult> {
  if (!options.confirmed) throw new Error("Confirm that you want to download and install this update.");
  if (!isTauri()) throw new Error("Update installation is available in the desktop build.");

  let contentLength = 0;
  let downloaded = 0;
  let installed = false;
  try {
    await update.downloadAndInstall((event: DownloadEvent) => {
      if (event.event === "Started") {
        contentLength = event.data.contentLength ?? 0;
        downloaded = 0;
        options.onProgress?.(0);
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
        options.onProgress?.(updateProgressPercent(downloaded, contentLength));
      } else {
        options.onProgress?.(100);
      }
    }, { restartAfterInstall: true });
    installed = true;
  } finally {
    // Keep a failed update resource available so the user can retry it.
    if (installed) await update.close().catch(() => undefined);
  }

  try {
    await relaunch();
    return { installed: true, relaunched: true };
  } catch {
    // Installation completed even if the current session could not relaunch.
    return { installed: true, relaunched: false };
  }
}
