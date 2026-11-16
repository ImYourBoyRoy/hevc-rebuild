// ./desktop/src/lib/organizer-rename.ts
// Organizer selection and rename flow extracted from MetadataWorkbench (T10.7 slice 10): the selected files, episode lookups, the years of a
// show, assigning a film, the rename request with all its safety checks, previewing it and applying it (path moves, assignment moves,
// rescan and every failure report). The state and props are reached through a typed host, the bridge and callbacks are injected so every
// branch is unit-tested in node (pnpm test:organizer-rename). The code is the component's code moved verbatim.

import type { Detail, Episode, MediaKind, MovieAssignment, OrganizerFile, OrganizerScan, OrganizerTemplates, Provider, RenamePreview, RenameTemplateInput, Result } from "./metadata-workbench-types.js";
import { movedPathForSource } from "./metadata-candidates.ts";

export interface OrganizerRenameHost {
  movieAssignments: Record<string, MovieAssignment>;
  selectedMediaPaths: string[];
  readonly organizerTemplates: OrganizerTemplates;
  readonly textBlacklistPatterns: string[];
  readonly compactItemMode: boolean;
  readonly visibleOrganizerFiles: OrganizerFile[];
  organizerScan: OrganizerScan | null;
  organizerSource: "analysis" | "folder" | null;
  tvRenameScope: "series-tree" | "episodes-only";
  seriesYearConflictConfirmed: boolean;
  episodeAssignments: Record<string, string>;
  selectedArtwork: Record<string, string>;
  mediaKind: MediaKind;
  provider: Provider;
  selected: Result | null;
  detail: Detail | null;
  renamePreview: RenamePreview | null;
  renameError: string;
  renameMessage: string;
  renameBusy: boolean;
  confirmRename: boolean;
}

export interface OrganizerRenameActions {
  currentSeriesYearMismatch(): { text: string } | null;
  onRenameState(mediaPaths: string[], status: "renaming" | "renamed" | "failed", message?: string): void;
  onPathsRenamed(paths: Record<string, string>): void;
  onOrganizerTemplatesChange(templates: OrganizerTemplates): void;
  onStartMetadataOnly(mediaPaths: string[]): void;
}

export interface OrganizerRenameDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  isDesktop: () => boolean;
  host: OrganizerRenameHost;
  actions: OrganizerRenameActions;
}

export function createOrganizerRename(deps: OrganizerRenameDeps) {
  const { host: h, actions, invoke, isDesktop } = deps;
  const { currentSeriesYearMismatch, onRenameState, onPathsRenamed, onOrganizerTemplatesChange, onStartMetadataOnly } = actions;

  function buildRenameRequest(): { root: string; items: RenameTemplateInput[]; textBlacklistPatterns: string[] } {
    if (!h.organizerScan) throw new Error("Choose a media folder first.");
    const items: RenameTemplateInput[] = [];
    const selectedForOrganizer = selectedPathsInOrganizer();
    if (h.mediaKind === "movie") {
      if (!selectedForOrganizer.length) throw new Error("Select at least one movie file for this reviewed title.");
      for (const path of selectedForOrganizer) {
        const assignment = h.movieAssignments[path];
        const file = h.organizerScan.files.find((item) => item.path === path);
        if (!assignment) {
          throw new Error(`Assign a reviewed film to ${file?.name ?? path} before previewing or applying this batch.`);
        }
        items.push({ sourcePath: path, kind: "movie", template: h.organizerTemplates.movie, values: {
          title: assignment.title, year: assignment.year, certification: assignment.certification,
        } });
      }
      return { root: h.organizerScan.root, items, textBlacklistPatterns: h.textBlacklistPatterns };
    }

    if (!h.detail || !h.selected) throw new Error("Search for and review a TV series before planning its rename.");
    const yearMismatch = currentSeriesYearMismatch();
    if (yearMismatch && !h.seriesYearConflictConfirmed) {
      throw new Error(`${yearMismatch.text}. Confirm that this is the correct series before preparing a rename.`);
    }

    const includeSeriesTree = !h.compactItemMode && h.tvRenameScope === "series-tree";
    if (includeSeriesTree) {
      if (h.organizerSource === "analysis") {
        throw new Error("The current library analysis may exclude files below its size filter. Scan this series folder before planning a full-series rename.");
      }
      if (h.organizerScan.truncated) throw new Error("Folder analysis reached its 10,000-file safety limit. Narrow the selected series folder before a full-series rename.");
      if (h.organizerScan.errors.length) throw new Error(`Folder analysis found ${h.organizerScan.errors.length} unreadable entries. Resolve these before a full-series rename.`);
      const selectedPaths = new Set(selectedForOrganizer);
      const unselected = h.organizerScan.files.filter((file) => !file.multipleEpisodes && !selectedPaths.has(file.path));
      if (unselected.length) {
        throw new Error(`${unselected.length} supported episode file(s) are not selected. Select all supported episodes or choose “Selected episode files only.”`);
      }
      const years = showYears();
      items.push({ sourcePath: h.organizerScan.root, kind: "series", template: h.organizerTemplates.series, values: {
        series_title: h.detail.title, first_year: years.first, last_year: years.last, year_range: years.range,
      } });
      for (const season of h.organizerScan.seasonDirectories) {
        items.push({ sourcePath: season.path, kind: "season", template: h.organizerTemplates.season, values: { season: String(season.season) } });
      }
    } else if (!selectedForOrganizer.length) {
      throw new Error("Select one or more episode files, or choose the full-series rename scope.");
    }

    for (const path of selectedForOrganizer) {
      const file = h.organizerScan.files.find((item) => item.path === path);
      if (!file) throw new Error("A selected episode is no longer present in the folder scan. Scan the folder again.");
      if (file.multipleEpisodes) {
        throw new Error(`${file.name} contains multiple episode numbers and cannot be safely assigned to one episode filename. Handle it separately.`);
      }
      const match = episodeForKey(h.episodeAssignments[path]);
      if (!match || match.season === null || match.episode === null) {
        throw new Error(`Choose and review a provider episode for ${file.name} before previewing or applying the rename.`);
      }
      items.push({ sourcePath: path, kind: "episode", template: h.organizerTemplates.episode, values: {
        series_title: h.detail.title, season: String(match.season), episode: String(match.episode), episode_title: match.title,
      } });
    }
    return { root: includeSeriesTree ? h.organizerScan.renameRoot : h.organizerScan.root, items, textBlacklistPatterns: h.textBlacklistPatterns };
  }

  async function previewNames() {
    if (!isDesktop() || !h.organizerScan) return;
    h.renameBusy = true;
    h.renameError = "";
    h.renameMessage = "";
    h.confirmRename = false;
    try {
      h.renamePreview = await invoke<RenamePreview>("preview_library_rename", buildRenameRequest());
    } catch (cause) {
      h.renamePreview = null;
      h.renameError = String(cause);
    } finally {
      h.renameBusy = false;
    }
  }

  async function applyNames() {
    if (!isDesktop() || !h.organizerScan || !h.renamePreview?.canApply) return;
    h.renameBusy = true;
    h.renameError = "";
    const appliedPreview = h.renamePreview;
    const affectedSources = appliedPreview.items.filter((item) => !item.isDirectory && !item.unchanged).map((item) => item.sourcePath);
    onRenameState(affectedSources, "renaming", "Applying the reviewed filename changes.");
    let pathsApplied = false;
    try {
      const request = buildRenameRequest();
      await invoke("apply_library_rename", {
        ...request, previewId: appliedPreview.previewId, confirmed: true,
      });
      const movedPaths = new Map(appliedPreview.items.map((item) => [item.sourcePath, item.targetPath]));
      const changedPaths = [...movedPaths].filter(([source, target]) => source !== target);
      onPathsRenamed(Object.fromEntries(movedPaths));
      pathsApplied = true;
      onRenameState(changedPaths.map(([, target]) => target), "renamed", `Renamed ${changedPaths.length} path${changedPaths.length === 1 ? "" : "s"}.`);
      const nextRoot = movedPaths.get(h.organizerScan.root) ?? h.organizerScan.root;
      h.selectedMediaPaths = h.selectedMediaPaths.map((path) => movedPaths.get(path) ?? path);
      if (h.mediaKind === "movie") {
        const updated = { ...h.movieAssignments };
        for (const [sourcePath, targetPath] of movedPaths) {
          const assignment = updated[sourcePath];
          if (assignment && sourcePath !== targetPath) {
            updated[targetPath] = assignment;
            delete updated[sourcePath];
          }
        }
        h.movieAssignments = updated;
      } else {
        const updated = { ...h.episodeAssignments };
        for (const [sourcePath, targetPath] of movedPaths) {
          const assignment = h.episodeAssignments[sourcePath];
          if (assignment && sourcePath !== targetPath) {
            updated[targetPath] = assignment;
            delete updated[sourcePath];
          }
        }
        h.episodeAssignments = updated;
      }
      h.organizerScan = await invoke<OrganizerScan>("scan_organizer_folder", { root: nextRoot });
      if (h.mediaKind === "tv") syncEpisodeAssignments(h.organizerScan.files);
      h.renamePreview = null;
      h.confirmRename = false;
      h.renameMessage = `Renamed ${changedPaths.length} path${changedPaths.length === 1 ? "" : "s"}. Media contents were not changed.`;
    } catch (cause) {
      h.renameError = String(cause);
      onRenameState(pathsApplied ? [...new Set(affectedSources.map((path) => movedPathForSource(path, appliedPreview.items)))]: affectedSources, pathsApplied ? "renamed" : "failed", pathsApplied ? `Files were renamed, but the follow-up refresh failed: ${String(cause)}` : `Rename did not complete: ${String(cause)}`);
      h.renamePreview = null;
    } finally {
      h.renameBusy = false;
    }
  }

  function compactRenameReady(): boolean {
    const paths = selectedPathsInOrganizer();
    if (!h.compactItemMode || !paths.length) return false;
    if (h.mediaKind === "movie") return paths.every((path) => Boolean(h.movieAssignments[path]));
    return Boolean(h.detail && h.selected && paths.every((path) => {
      const episode = episodeForKey(h.episodeAssignments[path]);
      return episode && episode.season !== null && episode.episode !== null;
    }));
  }

  function selectedPathsInOrganizer(): string[] {
    if (!h.organizerScan) return [];
    const known = new Set(h.visibleOrganizerFiles.map((file) => file.path));
    return h.selectedMediaPaths.filter((path) => known.has(path));
  }

  function episodeForKey(key: string | undefined): Episode | undefined {
    if (!key || !h.detail) return undefined;
    const [season, episode] = key.split("/").map(Number);
    return h.detail.episodes.find((item) => item.season === season && item.episode === episode);
  }

  function updateEpisode(episode: Episode, field: "title" | "airdate" | "summary", value: string) {
    if (!h.detail) return;
    const sameEpisode = (item: Episode) => item.season === episode.season && item.episode === episode.episode;
    h.detail = {
      ...h.detail,
      episodes: h.detail.episodes.map((item) => sameEpisode(item) ? { ...item, [field]: value || null } : item),
    };
    h.renamePreview = null;
  }

  function toggleMedia(path: string, selectedNow: boolean) {
    h.selectedMediaPaths = selectedNow
      ? [...new Set([...h.selectedMediaPaths, path])]
      : h.selectedMediaPaths.filter((item) => item !== path);
    h.renamePreview = null;
  }

  function selectAllMedia(select: boolean) {
    if (!h.organizerScan) return;
    const scopeFiles = h.compactItemMode ? h.visibleOrganizerFiles : h.organizerScan.files;
    const eligible = new Set(scopeFiles.filter((file) => !file.multipleEpisodes).map((file) => file.path));
    h.selectedMediaPaths = select
      ? [...new Set([...h.selectedMediaPaths, ...eligible])]
      : h.selectedMediaPaths.filter((path) => !eligible.has(path));
    h.renamePreview = null;
    h.confirmRename = false;
  }

  function showYears() {
    const candidates = [h.detail?.year, ...(h.detail?.seasons ?? []).flatMap((season) => [season.yearStart, season.yearEnd])]
      .filter((value): value is string => typeof value === "string")
      .flatMap((value) => value.match(/\b\d{4}\b/g) ?? []);
    const years = [...new Set(candidates)].sort();
    const first = years[0] ?? "";
    const last = years.at(-1) ?? "";
    return { first, last, range: first && last && first !== last ? `${first}-${last}` : first || last };
  }

  function saveOrganizerTemplates() {
    onOrganizerTemplatesChange({ ...h.organizerTemplates });
  }

  function createMetadataOnlyCopies() {
    const paths = selectedPathsInOrganizer();
    if (paths.length) onStartMetadataOnly(paths);
  }

  function syncEpisodeAssignments(files: OrganizerFile[]) {
    if (!h.detail) return;
    const known = new Set(h.detail.episodes.filter((episode) => episode.season !== null && episode.episode !== null)
      .map((episode) => `${episode.season}/${episode.episode}`));
    const next = { ...h.episodeAssignments };
    for (const file of files) {
      if (next[file.path] || file.multipleEpisodes || file.season === null || file.episode === null) continue;
      const key = `${file.season}/${file.episode}`;
      if (known.has(key)) next[file.path] = key;
    }
    h.episodeAssignments = next;
  }

  function assignCurrentMovie(paths: string[]) {
    if (!h.detail || h.mediaKind !== "movie" || paths.length === 0) return;
    const assignment = {
      title: h.detail.title,
      year: h.detail.year ?? "",
      certification: h.detail.certification ?? "",
      recordId: `${h.provider}:${h.selected?.id ?? ""}`,
      shortDescription: h.detail.shortDescription,
      plot: h.detail.longDescription,
      genresText: h.detail.genres.join(", "),
      ratingText: h.detail.rating === null ? "" : String(h.detail.rating),
      sourceUrl: h.detail.sourceUrl,
      artworkCandidates: h.detail.artwork.map((image) => ({ ...image })),
      posterId: h.selectedArtwork.movie ?? "",
      localPosterPath: "",
    };
    h.movieAssignments = {
      ...h.movieAssignments,
      ...Object.fromEntries(paths.map((path) => [path, { ...assignment }])),
    };
    h.renamePreview = null;
    h.renameError = "";
  }

  return { buildRenameRequest, previewNames, applyNames, compactRenameReady, selectedPathsInOrganizer, episodeForKey, updateEpisode, toggleMedia, selectAllMedia, showYears, saveOrganizerTemplates, createMetadataOnlyCopies, syncEpisodeAssignments, assignCurrentMovie };
}
