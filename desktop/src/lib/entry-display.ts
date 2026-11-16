// ./desktop/src/lib/entry-display.ts
// Pure display and classification helpers for media entries (profile and media-info lines, durations,
// HDR10+ signals and consent text, metadata value formatting, status labels, stream classification and
// the animation hint), extracted from the route (helper slice). Tested by pnpm test:entry-display.

import type { MediaEntry, QueueMovieReview, QueueSeriesReview } from "./types.js";
import { bytes } from "./format.ts";

export function profileSummary(entry: MediaEntry) {
  if (!entry.profile) return "Profile unavailable";
  const p = entry.profile;
  const hdr = p.hdr.dolby_vision ? "Dolby Vision" : p.hdr.hdr10plus ? "HDR10+" : p.hdr.hdr10plus_hint ? "HDR10+ candidate" : p.hdr.hdr10 ? "HDR10" : p.hdr.hlg ? "HLG" : "SDR";
  return `${p.video.width} × ${p.video.height} · ${p.video.source_bit_depth}-bit · ${hdr}`;
}

export function hasHdr10plusSignal(entry: MediaEntry): boolean {
  return Boolean(entry.profile?.hdr.hdr10plus || entry.profile?.hdr.hdr10plus_hint);
}

export function youtubeHdr10plusConsentMessage(entry: MediaEntry): string {
  if (entry.profile?.hdr.hdr10plus) {
    return "HDR10+ was detected and hdr10plus_tool is missing, so a re-encode would drop it. Install the helper to preserve it, or approve static HDR10/PQ-only export for this file.";
  }
  return "HDR10+ is indicated but not confirmed from frame metadata, and hdr10plus_tool is missing. Install the helper to preserve it, or approve static HDR10/PQ-only export after review.";
}

export function clockDuration(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remain = total % 60;
  if (hours) return `${hours}:${String(minutes).padStart(2, "0")}:${String(remain).padStart(2, "0")}`;
  return `${minutes}:${String(remain).padStart(2, "0")}`;
}

export function mediaInfoLine(entry: MediaEntry): string {
  const profile = entry.profile;
  if (!profile) return bytes(entry.size_bytes);
  const resolution = profile.video.width && profile.video.height ? `${profile.video.width}×${profile.video.height}` : "";
  const hdr = profile.hdr.dolby_vision ? "Dolby Vision" : profile.hdr.hdr10plus ? "HDR10+" : profile.hdr.hdr10 ? "HDR10" : profile.hdr.hlg ? "HLG" : "";
  return [
    profile.video.codec || "Video",
    resolution,
    profile.video.source_bit_depth ? `${profile.video.source_bit_depth}-bit` : "",
    profile.duration_seconds > 0 ? clockDuration(profile.duration_seconds) : "",
    `${profile.audio_count} audio`,
    `${profile.subtitle_count} subtitles`,
    hdr,
    bytes(entry.size_bytes),
  ].filter(Boolean).join(" · ");
}

export function formatMetadataValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value, null, 2) ?? String(value);
  } catch {
    return String(value);
  }
}

export function statusLabel(status: string) {
  if (status === "Ready") return "Ready";
  if (status === "Backed up") return "Has backup";
  if (status === "Tools required") return "Tools needed";
  return status;
}

export function streamIsSelectable(type: string): boolean {
  return ["video", "audio", "subtitle", "attachment", "data"].includes(type);
}

export function streamIsRealVideo(entry: MediaEntry, index: number): boolean {
  const stream = entry.profile?.tracks.find((track) => track.index === index);
  return Boolean(stream && stream.type === "video" && !stream.attached_picture && (stream.width ?? 0) > 0 && (stream.height ?? 0) > 0);
}

export function metadataSuggestsAnimation(review: QueueMovieReview | QueueSeriesReview | undefined): boolean {
  return /\b(?:animation|anime|animated|cartoon|cartoons)\b/i.test(review?.genresText ?? "");
}
