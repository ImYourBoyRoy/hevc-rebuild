// Scanned media: profiles, streams, resource forecasts, entries and scan results.

import type { Environment } from './environment.js';
import type { Settings, WorkDirectoryPreview } from './settings.js';

export interface MediaProfile {
  video: {
    codec: string | null;
    width: number;
    height: number;
    source_bit_depth: number;
    source_chroma: string;
    target_pixel_format: string;
    secondary_count: number;
  };
  hdr: {
    hdr10: boolean;
    hdr10plus: boolean;
    hlg: boolean;
    dolby_vision: boolean;
    dolby_vision_profile: number | null;
    hdr10plus_hint?: boolean;
    dolby_vision_level?: number | null;
    dolby_vision_rpu?: boolean;
    dolby_vision_enhancement_layer?: boolean;
    dolby_vision_base_layer?: boolean;
    dolby_vision_compatibility_id?: number | null;
    dolby_vision_evidence?: string | null;
  };
  audio_count: number;
  subtitle_count: number;
  attachment_count: number;
  data_stream_count: number;
  tracks: MediaStream[];
  mediainfo_general: Record<string, unknown>;
  content?: {
    genres: string[];
    sources: string[];
    animation_detected: boolean;
    resolved_tune: 'animation' | 'grain' | null;
    reason: string;
  };
  chapters_count: number;
  duration_seconds: number;
  interlaced: boolean;
  color_primaries?: string | null;
  color_transfer?: string | null;
  color_matrix?: string | null;
  color_range_x265?: string | null;
  color_range_ffmpeg?: string | null;
  mastering_display?: string | null;
  mastering_display_source?: string | null;
  max_cll?: number | null;
  max_fall?: number | null;
  chroma_location?: number | null;
  primary_stream_index?: number;
  mediainfo_available?: boolean;
  frame_metadata_available?: boolean;
  source_metadata?: {
    format?: Record<string, unknown> | null;
    chapters?: Array<Record<string, unknown>>;
  };
}

export interface MediaStream {
  index: number;
  type: string;
  ordinal: number;
  codec: string | null;
  profile?: string | null;
  width?: number | null;
  height?: number | null;
  pixel_format?: string | null;
  bit_depth?: number | string | null;
  color_primaries?: string | null;
  color_transfer?: string | null;
  color_matrix?: string | null;
  frame_rate?: string | null;
  channels?: number | null;
  channel_layout?: string | null;
  sample_rate?: string | null;
  bit_rate?: string | null;
  language?: string | null;
  title?: string | null;
  default: boolean;
  forced: boolean;
  attached_picture: boolean;
  filename?: string | null;
  mime_type?: string | null;
  mediainfo: Record<string, unknown>;
  ffprobe?: Record<string, unknown>;
}

export interface ResourceForecast {
  model: string;
  level: 'guarded' | 'constrained' | 'high' | 'unknown' | 'blocked' | string;
  available_gib: number | null;
  total_gib: number | null;
  swap_free_gib: number | null;
  swap_used_gib?: number | null;
  swap_used_percent?: number | null;
  reserve_gib: number;
  planning_budget_gib: number | null;
  profile_factor?: number;
  preset_factor?: number;
  per_frame_allowance_gib?: number;
  per_worker_allowance_gib?: number;
  modeled_plan_gib?: number | null;
  memory_worker_cap?: number | null;
  requested_workers?: number | null;
  requested_frame_threads?: number | null;
  planned_workers: number | null;
  planned_frame_threads: number | null;
  retry_plan?: {
    level: string;
    planned_workers: number;
    planned_frame_threads: number;
    modeled_plan_gib: number;
    reduces_parallelism: boolean;
  } | null;
  messages: string[];
  plan_reason?: string;
}

export interface MediaEntry {
  path: string;
  relative_path: string;
  name: string;
  size_bytes: number;
  modified_unix_ms?: number | null;
  queue_stale?: 'changed' | 'missing';
  status: 'Ready' | 'Backed up' | 'Tools required' | 'Analysis failed' | string;
  already_processed: boolean;
  profile: MediaProfile | null;
  youtube_estimates?: Record<'source' | '1440p' | '4k', Record<'balanced' | 'recommended', YouTubeEstimate>>;
  youtube_resource_forecasts?: Record<'source' | '1440p' | '4k', ResourceForecast>;
  resource_forecast?: ResourceForecast | null;
  error: string | null;
  warning?: string | null;
}

export interface YouTubeEstimate {
  estimated_output_bytes: number;
  video_bitrate_kbps: number;
  recommended_video_bitrate_kbps: number;
  output_width: number;
  output_height: number;
  source_bytes: number;
}

export interface ScanResult {
  root: string;
  input_kind: 'file' | 'folder';
  recursive: boolean;
  minimum_size_mib: number;
  candidate_count: number;
  total_input_bytes: number;
  disk_free_bytes: number;
  entries: MediaEntry[];
  environment: Environment;
  settings: Settings;
  settings_sources: string[];
  work_directory: WorkDirectoryPreview;
  warnings?: string[];
  discovery_limit_reached?: boolean;
  interrupted?: boolean;
}
