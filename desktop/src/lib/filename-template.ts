// ./desktop/src/lib/filename-template.ts
// Organizer filename templating extracted from the route: token rendering with optional
// [[...]] groups, proposed names for movies and episodes, and the destination path preview.
// Pure: no IPC, no DOM, no state. Tested by pnpm test:filename-template.

import { queuePathKey } from './queue-paths.ts';
import type { MediaEntry, OrganizerTemplates, QueueMetadataSuggestion, QueueMovieReview, QueueSeriesReview } from './types.js';

export function renderFilenameChunk(
  chunk: string,
  allowed: readonly string[],
  values: Record<string, string>,
): { text: string; hasValue: boolean } | null {
  let text = "";
  let hasValue = false;
  let cursor = 0;
  while (cursor < chunk.length) {
    const open = chunk.indexOf("{", cursor);
    const close = chunk.indexOf("}", cursor);
    if (open < 0 && close < 0) {
      text += chunk.slice(cursor);
      break;
    }
    if (open < 0 || (close >= 0 && close < open)) return null;
    text += chunk.slice(cursor, open);
    const end = chunk.indexOf("}", open + 1);
    if (end < 0) return null;
    const token = chunk.slice(open + 1, end);
    if (!token || token.includes("{")) return null;
    const match = token.match(/^([a-z_]+)(?::(02|03|04))?$/);
    if (!match || !allowed.includes(match[1])) return null;
    const value = values[match[1]] ?? "";
    if (match[2] && value && !/^\d+$/.test(value)) return null;
    text += match[2] && value ? value.padStart(Number(match[2]), "0") : value;
    hasValue ||= Boolean(value);
    cursor = end + 1;
  }
  return { text, hasValue };
}

export function renderFilenameTemplate(
  template: string,
  allowed: readonly string[],
  values: Record<string, string>,
): string | null {
  if (!template.trim() || new TextEncoder().encode(template).length > 1024) return null;
  let output = "";
  let rest = template;
  while (rest.length) {
    const open = rest.indexOf("[[");
    const actualClose = rest.indexOf("]]");
    if (open < 0 && actualClose < 0) {
      const tail = renderFilenameChunk(rest, allowed, values);
      if (!tail) return null;
      output += tail.text;
      break;
    }
    if (open < 0 || (actualClose >= 0 && actualClose < open)) return null;
    const prefix = renderFilenameChunk(rest.slice(0, open), allowed, values);
    const bodyStart = open + 2;
    const bodyEnd = rest.indexOf("]]", bodyStart);
    if (!prefix || bodyEnd < 0 || rest.slice(bodyStart, bodyEnd).includes("[[")) return null;
    const optional = renderFilenameChunk(rest.slice(bodyStart, bodyEnd), allowed, values);
    if (!optional) return null;
    output += prefix.text + (optional.hasValue ? optional.text : "");
    rest = rest.slice(bodyEnd + 2);
  }
  let cleaned = output.split(/\s+/u).filter(Boolean).join(" ")
    .replaceAll("( ", "(").replaceAll(" )", ")")
    .replaceAll("[ ", "[").replaceAll(" ]", "]")
    .trim();
  for (const [left, right] of [["(", ")"], ["[", "]"]] as const) {
    cleaned = cleaned.replaceAll(`${left}${right}`, "");
  }
  return cleaned.trim() || null;
}

export function proposedFilenameForEntry(
  entry: MediaEntry,
  movie: QueueMovieReview | undefined,
  series: QueueSeriesReview | undefined,
  suggestion: QueueMetadataSuggestion | undefined,
  templates: OrganizerTemplates,
): { name: string; needsTitleReview: boolean } | null {
  const extension = entry.name.match(/\.[^.]+$/)?.[0] ?? "";
  const movieTitle = movie?.title.trim() || (suggestion?.kind === "movie" && suggestion.status === "ready" ? suggestion.title?.trim() : "") || "";
  if (movieTitle) {
    const isSuggestion = !movie;
    const name = renderFilenameTemplate(templates.movie, ["title", "year", "certification", "ext"], {
      title: movie?.title ?? movieTitle,
      year: movie?.year ?? suggestion?.year ?? "",
      certification: movie?.certification ?? "",
      ext: extension,
    });
    return name ? { name, needsTitleReview: isSuggestion } : null;
  }

  if (!series) return null;
  const assignment = series.episodeAssignments[entry.path] ??
    Object.entries(series.episodeAssignments).find(([path]) => queuePathKey(path) === queuePathKey(entry.path))?.[1];
  if (!assignment?.title.trim()) return null;
  const name = renderFilenameTemplate(templates.episode, ["series_title", "season", "episode", "episode_title", "ext"], {
    series_title: series.title,
    season: String(assignment.season),
    episode: String(assignment.episode),
    episode_title: assignment.title,
    ext: extension,
  });
  return name ? { name, needsTitleReview: false } : null;
}

export function reviewDestinationPath(
  entry: MediaEntry,
  proposedName: string | null,
  outputs: { fileOutputDirs: Record<string, string>; rebuildOutputDir: string | null },
): { path: string; note: string } {
  const name = proposedName || entry.name;
  const perFile = outputs.fileOutputDirs[entry.path];
  const folder = (perFile || outputs.rebuildOutputDir || "").replace(/[\\/]+$/, "");
  if (folder) {
    const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
    const stem = name.replace(/\.[^.]+$/, "");
    // The default folder mirrors source subfolders; a per-file folder receives the file directly.
    const mirrored = perFile ? [] : (entry.relative_path ?? "").split(/[\\/]/).filter(Boolean).slice(0, -1);
    return {
      path: [folder, ...mirrored, `${stem}.mkv`].join(separator),
      note: perFile ? "Per-file output folder" : "Default output folder",
    };
  }
  const slash = Math.max(entry.path.lastIndexOf("/"), entry.path.lastIndexOf("\\"));
  const directory = slash >= 0 ? entry.path.slice(0, slash + 1) : "";
  return { path: `${directory}${name}`, note: "Proposed name in the source folder" };
}
