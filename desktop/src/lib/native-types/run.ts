// Run requests, live run events, resource snapshots and report rows.

import type { RunMode, YoutubeMode, YoutubePreset, YoutubeQualityCheck, YoutubeSdrCodec, YoutubeShrink } from './modes.js';
import type { Settings } from './settings.js';

export interface ReportRow {
  input: string;
  output?: string;
  status: string;
  message: string;
  primary_video?: string;
  resolution?: string;
  audio_count?: string;
  subtitle_count?: string;
  subtitle_note?: string;
  hdr10?: string;
  hdr10plus?: string;
  hdr10plus_note?: string;
  dolby_vision?: string;
  dovi_profile?: string;
  dovi_source?: string;
  dolby_vision_action?: string;
  memory_forecast?: string;
  memory_available_gib?: string;
  memory_total_gib?: string;
  memory_swap_used_percent?: string;
  memory_budget_gib?: string;
  memory_worker_cap?: string;
  planned_workers?: string;
  planned_frame_threads?: string;
  memory_forecast_message?: string;
  calibration_source?: string;
  calibration_workers?: string;
  calibration_frame_threads?: string;
  calibration_candidate_count?: string;
  calibration_note?: string;
  min_memory_available_gib?: string;
  max_encoder_rss_gib?: string;
  max_swap_used_percent?: string;
  max_encoder_swap_gib?: string;
  max_swap_in_mib_s?: string;
  max_swap_out_mib_s?: string;
  max_memory_full_psi_percent?: string;
  input_bytes?: string;
  output_bytes?: string;
  upload_resolution?: string;
  upload_quality_mode?: string;
  upload_source_multiplier?: string;
  output_resolution?: string;
  upload_video_bitrate_kbps?: string;
  youtube_recommended_video_bitrate_kbps?: string;
  estimated_output_bytes?: string;
  metadata_text_scrub_count?: string;
  metadata_text_scrub_details?: string;
  release_group_scrub_count?: string;
  release_group_scrub_details?: string;
  content_tune?: string;
  elapsed_seconds?: string;
  upload_mode?: string;
  upload_color_mode?: string;
  source_video_bitrate_kbps?: string;
  optimize_action?: string;
  optimize_reason?: string;
  optimize_note?: string;
  quality_ssim_min?: string;
  video_payload_identical?: string;
}

export interface RunReport {
  csv_path: string;
  log_path: string | null;
  created: number;
  counts: Record<string, number>;
  rows: ReportRow[];
}

export interface RunRequest {
  root: string;
  recursive: boolean;
  minimumSizeMib: number;
  ffmpegPath: string | null;
  mode: RunMode;
  youtubeResolution: 'source' | '1440p' | '4k';
  youtubeQualityMode: 'balanced' | 'recommended';
  youtubeSourceMultiplier: number;
  youtubeMode: YoutubeMode;
  youtubeSdrCodec: YoutubeSdrCodec;
  youtubePreset: YoutubePreset;
  youtubeShrink: YoutubeShrink;
  youtubeQualityCheck: YoutubeQualityCheck;
  youtubeOutputDir: string | null;
  settings: Settings;
  stopOnError: boolean;
  fullRebuildConfirmed: boolean;
  fileTuneOverrides: Record<string, 'animation' | 'none'>;
  fileSettingsOverrides: Record<string, FileRunSettingsOverride>;
}

/**
 * Settings applied only to the current run and only to the mapped media path.
 * An omitted field inherits the effective library/app/session value.
 */
export interface FileRunSettingsOverride {
  crf?: number;
  preset?: string;
  promote_8bit_to_10bit?: boolean;
  tune?: 'auto' | 'animation' | 'grain' | 'none';
  preserve_hdr10plus?: boolean;
  dolby_vision?: 'safe' | 'skip' | 'hdr10-fallback';
  stream_indices?: number[];
}

export interface RunLineEvent {
  runId: string;
  line: string;
}

export interface RunProgressEvent {
  runId: string;
  fileName: string;
  filePath?: string | null;
  phase: string;
  batchNumber: number;
  batchTotal: number;
  filePercent: number | null;
  queuePercent: number | null;
  speed: number | null;
  etaSeconds: number | null;
  elapsedSeconds: number | null;
  inputBytes: number | null;
  outputBytes: number | null;
  estimatedOutputBytes: number | null;
}

export interface RuntimeResourceSnapshot {
  sampled_at_unix_ms: number;
  cpu_percent: number | null;
  load1: number | null;
  memory_total_gib: number | null;
  memory_available_gib: number | null;
  swap_total_gib: number | null;
  swap_free_gib: number | null;
  swap_used_gib: number | null;
  swap_used_percent: number | null;
  swap_in_bytes_per_second: number | null;
  swap_out_bytes_per_second: number | null;
  memory_some_avg10: number | null;
  memory_full_avg10: number | null;
  io_some_avg10: number | null;
  io_full_avg10: number | null;
  cpu_some_avg10: number | null;
  disk_total_bytes: number | null;
  disk_used_bytes: number | null;
  disk_free_bytes: number | null;
  disk_free_percent: number | null;
  engine_process_rss_gib: number | null;
  engine_process_swap_gib: number | null;
  ffmpeg_process_rss_gib: number | null;
  ffmpeg_process_swap_gib: number | null;
  temperature_c: number | null;
}

export interface RunResourceEvent {
  runId: string;
  snapshot: RuntimeResourceSnapshot;
}

export interface ScanProgress {
  current: number;
  total: number;
  file: string;
}

export interface RunFinishedEvent {
  runId: string;
  exitCode: number | null;
  stopRequested: boolean;
  logPath: string | null;
  reportPath: string | null;
  error: string | null;
}
