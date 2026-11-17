// ./desktop/src/lib/normalizers.ts
// Normalizers for saved preferences (theme, page, tool paths) and page labels, extracted
// from the route. Pure: no IPC, no DOM, no state. Tested by pnpm test:normalizers.

import type { Page, ToolPaths } from './types.js';
import type { Theme } from './ui-contracts.js';
import { isTheme } from './themes.ts';

export type ToolPathKey = keyof ToolPaths;

export const workspacePages: Page[] = ["home", "library", "metadata", "youtube", "run", "settings", "reports", "help"];

export function normalizeTheme(value: unknown): Theme {
  if (value === "hacker") return "phosphor";
  if (value === "contrast") return "light"; // retired high-contrast theme, replaced by DOS
  if (isTheme(value)) return value;
  return "light";
}

export function normalizeToolPaths(value: unknown, legacyFfmpeg?: unknown): ToolPaths {
  const candidate = value && typeof value === "object" ? value as Partial<Record<ToolPathKey, unknown>> : {};
  const normalize = (key: ToolPathKey, legacy?: unknown): string | null => {
    const raw = Object.hasOwn(candidate, key) ? candidate[key] : legacy;
    return typeof raw === "string" && raw.trim() ? raw.trim() : null;
  };
  return {
    ffmpeg: normalize("ffmpeg", legacyFfmpeg),
    ffprobe: normalize("ffprobe"),
    mediainfo: normalize("mediainfo"),
    mkvpropedit: normalize("mkvpropedit"),
    mkvmerge: normalize("mkvmerge"),
    mkvextract: normalize("mkvextract"),
  };
}

export function pageLabel(id: Page): string {
  if (id === "home" || id === "library") return "Library";
  if (id === "metadata") return "Organize titles";
  if (id === "youtube") return "YouTube";
  if (id === "run") return "Run";
  if (id === "settings") return "Settings";
  if (id === "reports") return "Reports & errors";
  if (id === "help") return "Help & commands";
  return "Library";
}

export function normalizePage(value: unknown): Page {
  if (value === "home") return "library";
  return typeof value === "string" && workspacePages.includes(value as Page) ? value as Page : "library";
}
