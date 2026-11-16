// ./desktop/src/lib/search-panel.ts
// Title search panel extracted from MetadataWorkbench (T10.7 slice 5): opening a search for a movie or series record, running the
// search with its guards and state resets, closing it, the series year mismatch, the collapsed candidate lists and the series root
// being reviewed. The state instances are reached through a typed host, the provider calls are injected so every branch is unit-tested
// in node (pnpm test:search-panel). The code is the component's code moved verbatim.

import type { AnalysisSeriesGroup, Detail, FilenameClue, MatchSuggestion, MediaKind, OrganizerFile, OrganizerScan, PendingArtworkSave, PublicSearchResponse, RenamePreview, Result, SearchTarget } from "./metadata-workbench-types.js";
import { candidateEvidence } from "./metadata-candidates.ts";
import { rankMetadataMatches } from "./metadata-match.ts";
import { basename, seriesFilenameTitle } from "./organizer-analysis.ts";

export interface SearchPanelHost {
  searchTarget: SearchTarget | null;
  mediaKind: MediaKind;
  query: string;
  searching: boolean;
  error: string;
  results: Result[];
  searchWarnings: string[];
  selected: Result | null;
  detail: Detail | null;
  manualSearchClue: FilenameClue | null;
  expandedMatchLists: Record<string, boolean>;
  organizerScan: OrganizerScan | null;
  organizerSource: "analysis" | "folder" | null;
  selectedAnalysisSeriesRoot: string;
  seriesYearConflictConfirmed: boolean;
  renamePreview: RenamePreview | null;
  confirmRename: boolean;
  pendingArtworkSave: PendingArtworkSave | null;
  artworkError: string;
  artworkMessage: string;
  seriesSuggestions: Record<string, MatchSuggestion>;
  selectedMediaPaths: string[];
}

export interface SearchPanelActions {
  chooseAnalyzedSeries(root: string): void;
  filenameClue(filename: string): Promise<FilenameClue>;
  searchPublicTitles(kind: MediaKind, title: string): Promise<PublicSearchResponse>;
}

export interface SearchPanelDeps {
  isDesktop: () => boolean;
  host: SearchPanelHost;
  actions: SearchPanelActions;
}

export const initialCandidateCount = 4;

export function createSearchPanel(deps: SearchPanelDeps) {
  const { host: h, actions, isDesktop } = deps;
  const { chooseAnalyzedSeries, filenameClue, searchPublicTitles } = actions;

  function prepareSearchTarget(target: SearchTarget, title: string) {
    h.searchTarget = target;
    h.mediaKind = target.kind;
    h.query = title;
    h.results = [];
    h.searchWarnings = [];
    h.selected = null;
    h.detail = null;
    h.manualSearchClue = null;
    h.seriesYearConflictConfirmed = false;
    h.expandedMatchLists = { ...h.expandedMatchLists, "manual-search": false };
    h.error = "";
    h.renamePreview = null;
    h.confirmRename = false;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    void filenameClue(title).then((clue) => {
      if (h.searchTarget === target && h.query === title) h.query = clue.title;
    }).catch(() => {});
  }

  function searchMovieRecord(file: OrganizerFile, preferredTitle?: string | null) {
    h.selectedMediaPaths = [...new Set([...h.selectedMediaPaths, file.path])];
    prepareSearchTarget({ kind: "movie", path: file.path, label: file.name }, preferredTitle?.trim() || file.name);
  }

  function searchSeriesRecord(group: AnalysisSeriesGroup, preferredTitle?: string | null) {
    if (group.ambiguous) return;
    if (h.organizerSource === "analysis" && h.selectedAnalysisSeriesRoot !== group.root) chooseAnalyzedSeries(group.root);
    prepareSearchTarget({ kind: "tv", root: group.root, label: basename(group.root) }, preferredTitle?.trim() || seriesFilenameTitle(group));
  }

  function searchCurrentSeriesRecord() {
    const current = h.organizerScan;
    if (!current || h.mediaKind !== "tv") return;
    searchSeriesRecord({
      root: current.root,
      files: current.files,
      seasonDirectories: current.seasonDirectories,
      ambiguous: false,
    });
  }

  function closeTitleSearch() {
    h.searchTarget = null;
    h.results = [];
    h.searchWarnings = [];
    h.error = "";
  }

  async function searchTitles() {
    const entered = h.query.trim();
    if (!isDesktop()) {
      h.error = "Metadata search runs in the installed desktop app. The browser preview cannot access the native provider client.";
      return;
    }
    if (!h.searchTarget) {
      h.error = "Open title search from a movie or series record so the result stays connected to that item.";
      return;
    }
    if (!entered) {
      h.error = "Enter a movie or series title to search.";
      return;
    }
    h.searching = true;
    h.error = "";
    h.searchWarnings = [];
    h.selected = null;
    h.detail = null;
    h.renamePreview = null;
    h.confirmRename = false;
    h.pendingArtworkSave = null;
    h.artworkError = "";
    h.artworkMessage = "";
    try {
      const clue = await filenameClue(entered);
      h.manualSearchClue = clue;
      const response = await searchPublicTitles(h.mediaKind, clue.title);
      h.results = rankByFilenameYear(response.results, clue);
      h.searchWarnings = response.warnings;
      if (!h.results.length) h.error = "No likely matches came back. Try the original title, a shorter title, or a year.";
    } catch (cause) {
      h.error = String(cause);
      h.results = [];
    } finally {
      h.searching = false;
    }
  }

  function currentSeriesYearMismatch(): { text: string; clue: FilenameClue } | null {
    if (h.mediaKind !== "tv" || !h.selected || !h.detail) return null;
    const root = currentReviewedSeriesRoot();
    const clue = h.seriesSuggestions[root]?.clue ?? h.manualSearchClue;
    if (!clue || !candidateEvidence(h.selected, clue).conflict) return null;
    return { text: candidateEvidence(h.selected, clue).text, clue };
  }

  function visibleCandidates<T>(items: T[], key: string): T[] {
    return h.expandedMatchLists[key] ? items : items.slice(0, initialCandidateCount);
  }

  function toggleCandidateList(key: string) {
    h.expandedMatchLists = { ...h.expandedMatchLists, [key]: !h.expandedMatchLists[key] };
  }

  function candidateListExpanded(key: string): boolean {
    return Boolean(h.expandedMatchLists[key]);
  }

  function rankByFilenameYear(rows: Result[], clue: FilenameClue, kind: MediaKind = h.mediaKind): Result[] {
    return rankMetadataMatches(rows, { ...clue, kind });
  }

  function currentReviewedSeriesRoot(): string {
    return h.searchTarget?.kind === "tv" ? h.searchTarget.root : h.organizerScan?.root || h.selectedAnalysisSeriesRoot;
  }

  return { prepareSearchTarget, searchMovieRecord, searchSeriesRecord, searchCurrentSeriesRecord, closeTitleSearch, searchTitles, currentSeriesYearMismatch, visibleCandidates, toggleCandidateList, candidateListExpanded, rankByFilenameYear, currentReviewedSeriesRoot };
}
