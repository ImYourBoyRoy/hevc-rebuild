// ./desktop/src/lib/queue-paths.ts
// Pure, cross-platform path helpers for the media queue: normalization,
// containment, rename remapping and display names. No IPC, no DOM, no state.
// Used by the route and by node tests (pnpm test:queue-paths).
// Handles POSIX, Windows drive and UNC paths; Windows comparisons are
// case-insensitive while the original casing of the returned tail is kept.

import type { ScanResult } from './types.js';

export function normalizeDroppedPaths(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value
    .filter((path): path is string => typeof path === "string")
    .map((path) => path.trim())
    .filter(Boolean))];
}

export function fileName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

export function archivedOriginalPath(source: string): string {
  const parts = source.split(/[\\/]/);
  const name = parts.pop() || source;
  const slash = source.includes("\\") && !source.includes("/") ? "\\" : "/";
  return [...parts, "old", name].join(slash);
}

export function pathIsWithin(directory: string, candidate: string): boolean {
  const separator = directory.includes("\\") ? "\\" : "/";
  const base = directory.replace(/[\\/]+$/, "");
  const prefix = base ? `${base}${separator}` : separator;
  const windowsPath = separator === "\\";
  const left = windowsPath ? candidate.toLowerCase() : candidate;
  const normalizedBase = windowsPath ? base.toLowerCase() : base;
  const normalizedPrefix = windowsPath ? prefix.toLowerCase() : prefix;
  return left === normalizedBase || left.startsWith(normalizedPrefix);
}

export function queuePathKey(path: string): string {
  const separators = path.replaceAll("\\", "/");
  const networkPrefix = separators.startsWith("//") ? "//" : "";
  let normalized = networkPrefix + separators.slice(networkPrefix.length).replace(/\/{2,}/g, "/");
  if (normalized.length > 1 && !/^[a-z]:\/$/i.test(normalized)) normalized = normalized.replace(/\/$/, "");
  return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : normalized;
}

export function remapQueuePath(path: string, movedPaths: Map<string, string>): string {
  const direct = movedPaths.get(path);
  if (direct) return direct;
  const separator = path.includes("\\") ? "\\" : "/";
  const normalizedPath = separator === "\\" ? path.toLowerCase() : path;
  const ancestor = [...movedPaths.entries()]
    .filter(([source]) => {
      const prefix = source.endsWith(separator) ? source : `${source}${separator}`;
      return normalizedPath.startsWith(separator === "\\" ? prefix.toLowerCase() : prefix);
    })
    .sort(([left], [right]) => right.length - left.length)[0];
  return ancestor ? `${ancestor[1]}${path.slice(ancestor[0].length)}` : path;
}

export function relativeQueuePath(rootPath: string, mediaPath: string, inputKind: ScanResult["input_kind"], previous: string): string {
  if (inputKind === "file") return fileName(mediaPath);
  const separator = rootPath.includes("\\") ? "\\" : "/";
  const prefix = rootPath.endsWith(separator) ? rootPath : `${rootPath}${separator}`;
  const normalizedPath = separator === "\\" ? mediaPath.toLowerCase() : mediaPath;
  const normalizedPrefix = separator === "\\" ? prefix.toLowerCase() : prefix;
  return normalizedPath.startsWith(normalizedPrefix) ? mediaPath.slice(prefix.length) : previous;
}
