// ./desktop/src/lib/organizer-analysis.ts
// Pure organizer analysis helpers extracted from MetadataWorkbench (T10.7 slice 1): path normalizing and joining, season and
// episode recognition from names, series grouping of an analyzed scan, episode gap detection and the organizer scan
// built from an analysis. No state, no IO; tested by test:organizer-analysis. The code is the component's code moved verbatim.

import type { MediaEntry, ScanResult } from "./types.js";
import type { AnalysisSeriesGroup, MediaKind, OrganizerEpisodeGap, OrganizerFile, OrganizerScan, SeasonDirectory } from "./metadata-workbench-types.js";


export function normalizedPath(path: string): string {
  const separators = path.replaceAll("\\", "/");
  const networkPrefix = separators.startsWith("//") ? "//" : "";
  return networkPrefix + separators.slice(networkPrefix.length).replace(/\/{2,}/g, "/");
}

export function parentPath(path: string): string {
  const normalized = normalizedPath(path).replace(/\/$/, "");
  const separator = normalized.lastIndexOf("/");
  if (separator < 0) return "";
  if (separator === 0) return "/";
  if (separator === 2 && /^[a-z]:\//i.test(normalized)) return normalized.slice(0, 3);
  return normalized.slice(0, separator);
}

export function basename(path: string): string {
  const normalized = normalizedPath(path).replace(/\/$/, "");
  return normalized.slice(normalized.lastIndexOf("/") + 1);
}

export function joinPath(root: string, child: string): string {
  const normalizedRoot = normalizedPath(root).replace(/\/$/, "");
  if (/^[a-z]:$/i.test(normalizedRoot)) return `${normalizedRoot}/${child}`;
  return normalizedRoot === "/" ? `/${child}` : `${normalizedRoot}/${child}`;
}

export function seasonNumberFromDirectory(name: string): number | null {
  if (name.toLowerCase() === "specials") return 0;
  const match = name.match(/^season[\s_-]*(\d{1,4})$/i);
  return match ? Number(match[1]) : null;
}

export function genericSeriesRoot(name: string): boolean {
  return new Set(["tv", "tv shows", "shows", "series", "library", "media", "videos", "movies"])
    .has(name.trim().toLocaleLowerCase());
}

export function seriesFilenameTitle(group: AnalysisSeriesGroup): string {
  const rootName = basename(group.root);
  if (!genericSeriesRoot(rootName)) return rootName;
  const prefixCounts = new Map<string, number>();
  for (const file of group.files) {
    const stem = file.name.replace(/\.[^.]+$/, "");
    const marker = /(?:^|[\s._-])s\d{1,4}[\s._-]*e\d{1,4}/i.exec(stem);
    if (!marker) continue;
    const prefix = stem.slice(0, marker.index).replace(/[._-]+/g, " ").trim();
    if (prefix) prefixCounts.set(prefix, (prefixCounts.get(prefix) ?? 0) + 1);
  }
  return [...prefixCounts.entries()].sort((left, right) => right[1] - left[1])[0]?.[0] ?? rootName;
}

export function episodeIdFromName(name: string): { season: number | null; episode: number | null; multipleEpisodes: boolean } {
  const stem = name.replace(/\.[^.]+$/, "");
  const match = stem.match(/(?:^|[^a-z0-9])s(\d{1,4})[\s._-]*e(\d{1,4})(?:[\s._-]*e\d{1,4})?/i);
  if (!match) return { season: null, episode: null, multipleEpisodes: false };
  const compactMatch = match[0].replace(/[\s._-]+/g, "");
  return {
    season: Number(match[1]),
    episode: Number(match[2]),
    multipleEpisodes: /(?:e\d{1,4}){2}$/i.test(compactMatch),
  };
}

export function organizerFileFromEntry(entry: MediaEntry): OrganizerFile {
  const path = normalizedPath(entry.path);
  const name = entry.name || basename(path);
  const id = episodeIdFromName(name);
  return {
    path,
    name,
    parentPath: parentPath(path),
    extension: name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "",
    ...id,
  };
}

export function isAnalyzedEpisode(scan: ScanResult, entry: MediaEntry): boolean {
  const file = organizerFileFromEntry(entry);
  if (file.season !== null && file.episode !== null) return true;
  if (seasonNumberFromDirectory(basename(analyzedLibraryRoot(scan))) !== null) return true;
  if (scan.input_kind === "file") return false;
  const relativeDirectories = normalizedPath(entry.relative_path).split("/").filter(Boolean).slice(0, -1);
  return relativeDirectories.some((directory) => seasonNumberFromDirectory(directory) !== null);
}

export function analyzedLibraryRoot(scan: ScanResult): string {
  return normalizedPath(scan.input_kind === "file" ? parentPath(scan.root) : scan.root);
}

export function inferredSeriesRoot(scan: ScanResult, entry: MediaEntry): string {
  const root = analyzedLibraryRoot(scan);
  const relativeParts = scan.input_kind === "file"
    ? []
    : normalizedPath(entry.relative_path).split("/").filter(Boolean).slice(0, -1);
  const directoryChain = [root];
  for (const part of relativeParts) directoryChain.push(joinPath(directoryChain.at(-1) ?? root, part));

  const firstSeasonDirectory = directoryChain.findIndex((path) => seasonNumberFromDirectory(basename(path)) !== null);
  if (firstSeasonDirectory >= 0) {
    return firstSeasonDirectory === 0
      ? parentPath(directoryChain[0]) || directoryChain[0]
      : directoryChain[firstSeasonDirectory - 1];
  }
  return relativeParts.length ? directoryChain[1] : root;
}

export function buildAnalysisSeriesGroups(scan: ScanResult): AnalysisSeriesGroup[] {
  const groups = new Map<string, OrganizerFile[]>();
  for (const entry of scan.entries) {
    if (!isAnalyzedEpisode(scan, entry)) continue;
    const root = inferredSeriesRoot(scan, entry);
    const files = groups.get(root) ?? [];
    files.push(organizerFileFromEntry(entry));
    groups.set(root, files);
  }
  return [...groups.entries()].map(([root, files]) => {
    const seasons = new Map<string, SeasonDirectory>();
    for (const file of files) {
      const relativeParent = normalizedPath(file.parentPath).slice(normalizedPath(root).replace(/\/$/, "").length).replace(/^\//, "");
      const firstDirectory = relativeParent.split("/").filter(Boolean)[0];
      if (!firstDirectory) continue;
      const season = seasonNumberFromDirectory(firstDirectory);
      if (season === null) continue;
      const path = joinPath(root, firstDirectory);
      seasons.set(path, { path, name: firstDirectory, season });
    }
    const titlePrefixes = new Set(files.flatMap((file) => {
      const stem = file.name.replace(/\.[^.]+$/, "");
      const marker = /(?:^|[\s._-])s\d{1,4}[\s._-]*e\d{1,4}/i.exec(stem);
      if (!marker) return [];
      const prefix = stem.slice(0, marker.index).replace(/[._-]+/g, " ").trim().toLocaleLowerCase();
      return prefix ? [prefix] : [];
    }));
    const genericRoot = genericSeriesRoot(basename(root));
    const seasonDirectories = [...seasons.values()].sort((left, right) => left.season - right.season);
    return {
      root,
      files: files.sort((left, right) => left.path.localeCompare(right.path)),
      seasonDirectories,
      ambiguous: genericRoot ? titlePrefixes.size !== 1 : seasonDirectories.length === 0 && titlePrefixes.size > 1,
    };
  }).sort((left, right) => left.root.localeCompare(right.root));
}

export function episodeGapsFromFiles(files: OrganizerFile[]): OrganizerEpisodeGap[] {
  const bySeason = new Map<number, { episodes: Set<number>; hasMultiEpisodeFile: boolean }>();
  for (const file of files) {
    if (file.season === null || file.episode === null) continue;
    const season = bySeason.get(file.season) ?? { episodes: new Set<number>(), hasMultiEpisodeFile: false };
    season.episodes.add(file.episode);
    season.hasMultiEpisodeFile ||= file.multipleEpisodes;
    bySeason.set(file.season, season);
  }
  return [...bySeason].flatMap(([season, observed]) => {
    if (observed.hasMultiEpisodeFile || observed.episodes.size < 2) return [];
    const sorted = [...observed.episodes].sort((left, right) => left - right);
    const first = sorted[0];
    const last = sorted.at(-1) ?? first;
    const present = observed.episodes;
    const examples: number[] = [];
    let missingCount = 0;
    for (let episode = first + 1; episode < last; episode += 1) {
      if (present.has(episode)) continue;
      missingCount += 1;
      if (examples.length < 8) examples.push(episode);
    }
    return missingCount ? [{ season, missingCount, examples }] : [];
  }).sort((left, right) => left.season - right.season);
}

export function analysisOrganizerScan(scan: ScanResult, kind: MediaKind, seriesRoot = ""): OrganizerScan | null {
  if (!scan.entries.length) return null;
  if (kind === "movie") {
    const root = analyzedLibraryRoot(scan);
    const files = scan.entries.filter((entry) => !isAnalyzedEpisode(scan, entry)).map(organizerFileFromEntry);
    return {
      root,
      renameRoot: root,
      files: files.sort((left, right) => left.path.localeCompare(right.path)),
      seasonDirectories: [],
      episodeGaps: [],
      errors: [],
      truncated: false,
    };
  }
  const group = buildAnalysisSeriesGroups(scan).find((candidate) => candidate.root === seriesRoot);
  if (!group || group.ambiguous) return null;
  return { root: group.root, renameRoot: group.root, files: group.files, seasonDirectories: group.seasonDirectories, episodeGaps: episodeGapsFromFiles(group.files), errors: [], truncated: false };
}
