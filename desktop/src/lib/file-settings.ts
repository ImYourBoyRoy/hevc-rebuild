// ./desktop/src/lib/file-settings.ts
// Per-file run settings extracted from the route (helper slice): YouTube estimates and resource forecasts, tune
// overrides, effective settings per file, default and chosen streams, the run-settings editor, current-run
// overrides and HDR10+ loss approvals. The route's state is reached through a typed host; every branch is
// unit-tested in node (pnpm test:file-settings). The code is the route's code moved verbatim.

import type { FileRunSettingsOverride, MediaEntry, MediaQueueSource, QueueMovieReview, QueueSeriesReview, RunMode, Settings } from "./types.js";
import type { AppPreferences } from "./ui-contracts.js";
import { queuePathKey } from "./queue-paths.ts";
import { metadataSuggestsAnimation, streamIsRealVideo, streamIsSelectable } from "./entry-display.ts";

export interface FileSettingsHost {
  youtubeMode: AppPreferences["youtubeMode"];
  youtubeResolution: AppPreferences["youtubeResolution"];
  youtubeQualityMode: AppPreferences["youtubeQualityMode"];
  youtubeSourceMultiplier: number;
  youtubeHdr10plusLossPaths: Set<string>;
  mode: RunMode;
  fileTuneOverrides: Record<string, "animation" | "none">;
  fileStreamOverrides: Record<string, number[]>;
  fileSettingsOverrides: Record<string, FileRunSettingsOverride>;
  runSettingsEditorOpen: boolean;
  runSettingsEditorPaths: string[];
  settings: Settings | null;
  queueMovieReviews: Record<string, QueueMovieReview>;
  readonly queueSources: MediaQueueSource[];
  readonly activeRoot: string | null;
  readonly selectedRunEntries: MediaEntry[];
  readonly workflowBusy: boolean;
  setNotice(message: string, kind?: "info" | "success" | "error"): void;
}

export interface FileSettingsActions {
  reviewedSeriesForPath(path: string): QueueSeriesReview | undefined;
}

export interface FileSettingsDeps {

  host: FileSettingsHost;
  actions: FileSettingsActions;
}

export function createFileSettings(deps: FileSettingsDeps) {
  const { host: h, actions } = deps;
  const { reviewedSeriesForPath } = actions;
  const setNotice = h.setNotice.bind(h);

  function youtubeEstimateFor(entry: MediaEntry) {
    if (h.youtubeMode === "optimize") return undefined; // Optimize decides copy vs re-encode per file at run time.
    const estimate = entry.youtube_estimates?.[h.youtubeResolution]?.[h.youtubeQualityMode];
    const profile = entry.profile;
    if (!estimate || !profile || h.youtubeQualityMode !== "balanced" || !Number.isFinite(profile.duration_seconds) || profile.duration_seconds <= 0) return estimate;
    // Keep the interactive forecast aligned with youtube._video_rate_kbps and youtube_output_estimate.
    const sourceAverageKbps = entry.size_bytes * 8 / profile.duration_seconds / 1000;
    const outputAudioKbps = profile.audio_count * 384;
    const videoBitrateKbps = Math.min(
      estimate.recommended_video_bitrate_kbps,
      Math.max(250, Math.round(sourceAverageKbps * h.youtubeSourceMultiplier - outputAudioKbps)),
    );
    return {
      ...estimate,
      video_bitrate_kbps: videoBitrateKbps,
      estimated_output_bytes: Math.max(0, Math.round(profile.duration_seconds * (videoBitrateKbps + outputAudioKbps) * 1000 / 8 * 1.12)),
    };
  }

  function resourceForecastFor(entry: MediaEntry) {
    return h.mode === "youtube"
      ? entry.youtube_resource_forecasts?.[h.youtubeResolution] ?? entry.resource_forecast
      : entry.resource_forecast;
  }

  function setFileTuneOverride(path: string, value: string) {
    if (value === "animation" || value === "none") {
      h.fileTuneOverrides = { ...h.fileTuneOverrides, [path]: value };
      return;
    }
    const remaining = { ...h.fileTuneOverrides };
    delete remaining[path];
    h.fileTuneOverrides = remaining;
  }

  function settingsForEntry(entry: MediaEntry): Settings | null | undefined {
    const source = h.queueSources.find((candidate) =>
      candidate.scanResult?.entries.some((item) => queuePathKey(item.path) === queuePathKey(entry.path)),
    );
    if (!source) return h.settings;
    return source.root === h.activeRoot
      ? h.settings ?? source.scanResult?.settings
      : source.scanResult?.settings ?? h.settings;
  }

  function effectiveContentTuneForEntry(
    entry: MediaEntry,
    review: QueueMovieReview | QueueSeriesReview | undefined,
    effectiveSettings: Settings | null | undefined,
  ): "animation" | "grain" | "none" {
    if (h.fileTuneOverrides[entry.path] === "animation" || h.fileTuneOverrides[entry.path] === "none") {
      return h.fileTuneOverrides[entry.path];
    }
    if (metadataSuggestsAnimation(review) || entry.profile?.content?.animation_detected) return "animation";
    const globalTune = effectiveSettings?.content.tune ?? "auto";
    if (globalTune === "animation" || globalTune === "grain" || globalTune === "none") return globalTune;
    return "none";
  }

  function fileTuneOverridesForBatch(entries: MediaEntry[], _effectiveSettings: Settings): Record<string, "animation" | "none"> {
    const paths = new Set(entries.map((entry) => entry.path));
    const overrides = Object.fromEntries(
      Object.entries(h.fileTuneOverrides).filter(([path]) => paths.has(path)),
    ) as Record<string, "animation" | "none">;
    for (const entry of entries) {
      if (overrides[entry.path]) continue;
      const review = h.queueMovieReviews[entry.path] ?? reviewedSeriesForPath(entry.path);
      if (metadataSuggestsAnimation(review) || entry.profile?.content?.animation_detected) overrides[entry.path] = "animation";
    }
    return overrides;
  }

  function defaultStreamIndices(entry: MediaEntry): number[] {
    const tracks = entry.profile?.tracks ?? [];
    const videos = tracks
      .filter((track) => track.type === "video" && !track.attached_picture && (track.width ?? 0) > 0 && (track.height ?? 0) > 0)
      .sort((a, b) => {
        const pixelDifference = (b.width ?? 0) * (b.height ?? 0) - (a.width ?? 0) * (a.height ?? 0);
        return pixelDifference || (b.width ?? 0) - (a.width ?? 0) || (b.height ?? 0) - (a.height ?? 0) || Number(b.default) - Number(a.default) || Number(b.bit_depth ?? 0) - Number(a.bit_depth ?? 0) || a.index - b.index;
      });
    const effectiveSettings = settingsForEntry(entry);
    const selected = effectiveSettings?.streams.secondary_video === "drop" ? videos.slice(0, 1) : [...videos];
    selected.push(...tracks.filter((track) => ["audio", "subtitle", "attachment"].includes(track.type)));
    if ((effectiveSettings?.streams.data_streams ?? "preserve") === "preserve") {
      selected.push(...tracks.filter((track) => track.type === "data"));
    }
    return [...new Set(selected.map((track) => track.index))].sort((a, b) => a - b);
  }

  function selectedStreamIndices(entry: MediaEntry): number[] {
    return h.fileStreamOverrides[entry.path] ?? defaultStreamIndices(entry);
  }

  function runEntriesForFolder(folder: string): MediaEntry[] {
    const base = queuePathKey(folder);
    const prefix = base.endsWith("/") ? base : `${base}/`;
    return h.selectedRunEntries.filter((entry) => {
      const path = queuePathKey(entry.path);
      return path === base || path.startsWith(prefix);
    });
  }

  function runOverrideSummary(entry: MediaEntry): string {
    const override = h.fileSettingsOverrides[entry.path];
    if (!override) return "Uses reviewed run settings";
    const choices: string[] = [];
    if (override.crf !== undefined) choices.push(`CRF ${override.crf}`);
    if (override.preset) choices.push(override.preset);
    if (override.tune) choices.push(override.tune === "grain" ? "film grain" : override.tune);
    if (override.promote_8bit_to_10bit !== undefined) choices.push(override.promote_8bit_to_10bit ? "10-bit promotion" : "8-bit output");
    if (override.preserve_hdr10plus !== undefined) choices.push(override.preserve_hdr10plus ? "HDR10+" : "no HDR10+");
    if (override.dolby_vision) choices.push(override.dolby_vision === "hdr10-fallback" ? "HDR10 fallback" : `Dolby ${override.dolby_vision}`);
    if (override.stream_indices) choices.push(`${override.stream_indices.length} tracks`);
    return choices.length ? `Current run: ${choices.join(" · ")}` : "Uses reviewed run settings";
  }

  function openRunSettingsEditor(paths: string[]) {
    if (h.workflowBusy) return;
    const wanted = new Set(paths.map(queuePathKey));
    const entries = h.selectedRunEntries.filter((entry) => wanted.has(queuePathKey(entry.path)));
    if (!entries.length) {
      setNotice("Select or scan at least one ready media file before adjusting this run.", "error");
      return;
    }
    h.runSettingsEditorPaths = entries.map((entry) => entry.path);
    h.runSettingsEditorOpen = true;
  }

  function streamIndicesForMode(entry: MediaEntry, streamMode: "default" | "all" | "primary" | "no-data"): number[] {
    if (streamMode === "default") return defaultStreamIndices(entry);
    const tracks = entry.profile?.tracks ?? [];
    const selectable = tracks.filter((track) => streamIsSelectable(track.type));
    if (streamMode === "all") return [...new Set(selectable.map((track) => track.index))].sort((a, b) => a - b);
    const videos = tracks
      .filter((track) => streamIsRealVideo(entry, track.index))
      .sort((a, b) => (b.width ?? 0) * (b.height ?? 0) - (a.width ?? 0) * (a.height ?? 0) || Number(b.default) - Number(a.default) || a.index - b.index);
    const selected = videos.slice(0, 1);
    selected.push(...tracks.filter((track) => ["audio", "subtitle", "attachment"].includes(track.type)));
    if (streamMode !== "no-data") selected.push(...tracks.filter((track) => track.type === "data"));
    return [...new Set(selected.map((track) => track.index))].sort((a, b) => a - b);
  }

  function applyRunSettingsOverride(paths: string[], override: FileRunSettingsOverride, streamMode: "keep" | "default" | "all" | "primary" | "no-data" | "exact") {
    const next = { ...h.fileSettingsOverrides };
    for (const path of paths) {
      const current = { ...(next[path] ?? {}), ...override };
      const entry = h.selectedRunEntries.find((item) => queuePathKey(item.path) === queuePathKey(path));
      if (entry && ["default", "all", "primary", "no-data"].includes(streamMode)) current.stream_indices = streamIndicesForMode(entry, streamMode as "default" | "all" | "primary" | "no-data");
      if (!Object.keys(current).length) delete next[path];
      else next[path] = current;
    }
    h.fileSettingsOverrides = next;
    setNotice(`Applied current-run choices to ${paths.length} file${paths.length === 1 ? "" : "s"}. Saved defaults were not changed.`, "success");
  }

  function clearRunSettingsOverride(paths: string[]) {
    const next = { ...h.fileSettingsOverrides };
    for (const path of paths) delete next[path];
    h.fileSettingsOverrides = next;
    setNotice(`Cleared current-run choices for ${paths.length} file${paths.length === 1 ? "" : "s"}.`, "info");
  }

  function setFileStreamSelected(entry: MediaEntry, index: number, included: boolean) {
    const current = selectedStreamIndices(entry);
    if (!included && streamIsRealVideo(entry, index) && current.filter((selected) => streamIsRealVideo(entry, selected)).length <= 1) {
      setNotice("Keep at least one real video track selected for a rebuild.", "error");
      return;
    }
    const next = included
      ? [...new Set([...current, index])].sort((a, b) => a - b)
      : current.filter((selected) => selected !== index);
    const defaults = defaultStreamIndices(entry);
    const overrides = { ...h.fileStreamOverrides };
    if (next.length === defaults.length && next.every((value, position) => value === defaults[position])) {
      delete overrides[entry.path];
    } else {
      overrides[entry.path] = next;
    }
    h.fileStreamOverrides = overrides;
  }

  function selectedTrackCount(entry: MediaEntry): number {
    return selectedStreamIndices(entry).length;
  }

  function youtubeHdr10plusLossAccepted(path: string): boolean {
    return h.youtubeHdr10plusLossPaths.has(path);
  }

  function setYoutubeHdr10plusLossAccepted(path: string, accepted: boolean) {
    const next = new Set(h.youtubeHdr10plusLossPaths);
    if (accepted && !next.has(path) && next.size >= 512) {
      setNotice("Per-run HDR10+ loss approval is limited to 512 files. Split this library into smaller export batches.", "error");
      return;
    }
    if (accepted) next.add(path);
    else next.delete(path);
    h.youtubeHdr10plusLossPaths = next;
  }

  return { youtubeEstimateFor, resourceForecastFor, setFileTuneOverride, settingsForEntry, effectiveContentTuneForEntry, fileTuneOverridesForBatch, defaultStreamIndices, selectedStreamIndices, runEntriesForFolder, runOverrideSummary, openRunSettingsEditor, streamIndicesForMode, applyRunSettingsOverride, clearRunSettingsOverride, setFileStreamSelected, selectedTrackCount, youtubeHdr10plusLossAccepted, setYoutubeHdr10plusLossAccepted };
}
