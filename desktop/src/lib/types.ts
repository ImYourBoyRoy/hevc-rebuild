export type RunMode = 'check' | 'metadata' | 'test' | 'full' | 'youtube';
export type Page = 'home' | 'library' | 'metadata' | 'youtube' | 'run' | 'settings' | 'reports' | 'help';

export interface ToolPaths {
  ffmpeg: string | null;
  ffprobe: string | null;
  mediainfo: string | null;
  mkvpropedit: string | null;
  mkvmerge: string | null;
  mkvextract: string | null;
}

export interface Settings {
  video: {
    crf: number;
    preset: string;
    promote_8bit_to_10bit: boolean;
  };
  content: {
    tune: string;
    detect_animation: boolean;
  };
  hdr: {
    preserve_hdr10plus: boolean;
    dolby_vision: string;
  };
  streams: {
    secondary_video: string;
    data_streams: string;
  };
  metadata: {
    comment: string;
    text_blacklist: string[];
    embed_artwork: boolean;
  };
  safety: {
    verify_copy_hashes: boolean;
  };
  calibration: {
    mode: string;
    cache_hours: number;
    large_file_gib: number;
    sample_seconds: number;
    max_candidates: number;
    throughput_tolerance_percent: number;
  };
  resources: {
    threading: string;
    reserved_cpu_threads: number;
    max_worker_threads: number;
    frame_threads: number;
    min_free_memory_gib: number;
    memory_reserve_percent: number;
    min_free_disk_gib: number;
    preflight_output_ratio: number;
    disk_projection_margin_percent: number;
  };
}

export interface CommandCheck {
  key: string;
  label: string;
  path: string | null;
  version: string | null;
  available: boolean;
  required: boolean;
  detail: string;
}

export interface Environment {
  os: {
    name: string;
    version: string;
    supported: boolean;
    note: string;
  };
  hardware: {
    cpu_model: string;
    logical_cpus: number;
    memory_total_gib: number | null;
    memory_available_gib: number | null;
    swap_total_gib: number | null;
    swap_free_gib: number | null;
    swap_used_gib: number | null;
    swap_used_percent: number | null;
    gpu_devices: string[];
    encoder: string;
  };
  checks: CommandCheck[];
  x265_pixel_formats: string[];
  dolby_vision_x265: boolean;
  can_analyze: boolean;
  can_probe_media?: boolean;
  can_enrich_with_mediainfo?: boolean;
  youtube_ready: boolean;
  metadata_ready?: boolean;
  ready: boolean;
  missing_apt_packages: string[];
}

export interface AptPackageUpdate {
  name: string;
  installed_version: string;
  candidate_version: string;
}

export interface AptUpdateStatus {
  packages: AptPackageUpdate[];
  catalog_updated_at: number | null;
  catalog_stale: boolean;
  source: string;
  checked_at: number;
}

export interface ArtworkCacheInfo {
  available: boolean;
  imageCount: number;
  totalFileCount: number;
  totalBytes: number;
  purgeableImageCount: number;
  purgeableBytes: number;
  protectedImageCount: number;
  protectedBytes: number;
  unmanagedEntryCount: number;
  previewId: string;
}

export interface ArtworkCachePurgeResult {
  deletedImageCount: number;
  deletedBytes: number;
  failedFileCount: number;
  remaining: ArtworkCacheInfo;
  message: string;
}

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
  warnings?: string[];
  discovery_limit_reached?: boolean;
}

export interface MediaQueueSource {
  root: string;
  status: 'scanning' | 'ready' | 'failed' | 'interrupted';
  scanResult: ScanResult | null;
  error: string | null;
  updatedAtMs: number;
  changedFiles: string[];
  missingFiles: string[];
}

export interface MediaQueueDocument {
  version: number;
  sources: MediaQueueSource[];
  selectedMediaPaths: string[];
  fileTuneOverrides: Record<string, 'animation' | 'none'>;
  fileStreamOverrides: Record<string, number[]>;
  movieReviews: Record<string, QueueMovieReview>;
  seriesReviews: Record<string, QueueSeriesReview>;
  matchSuggestions: Record<string, QueueMetadataSuggestion>;
  runStates: Record<string, MediaQueueRunState>;
  activeRoot: string | null;
  updatedAtMs: number;
}

export type MediaQueueRunStatus = 'queued' | 'running' | 'scanning' | 'renaming' | 'remuxing' | 'rebuilding' | 'validating' | 'renamed' | 'completed' | 'failed' | 'skipped' | 'interrupted' | 'needs_review';
export type MediaQueueOperationMode = RunMode | 'organize';

export interface InterruptedRecovery {
  skipped: boolean;
  completed: number;
  cleared: number;
  held: number;
  removedTemporaryFiles: number;
  runStates: Record<string, MediaQueueRunState>;
  summary: string;
}

export interface MediaQueueRunState {
  status: MediaQueueRunStatus;
  mode: MediaQueueOperationMode | null;
  message: string | null;
  outputPath: string | null;
  reportPath: string | null;
  reportStatus: string | null;
  updatedAtMs: number;
}

export interface PlannedOutputTarget {
  source: string;
  target: string;
  destination: 'source_folder' | 'default_folder' | 'per_file_folder';
  replacesSource: boolean;
  note: string | null;
}

export interface OutputCollision {
  kind: 'existing_output' | 'duplicate_target' | string;
  source: string;
  target: string;
  conflictsWith: string | null;
}

export interface OutputPreview {
  mode: 'metadata' | 'test' | 'full';
  sourceCount: number;
  fileCount: number;
  eligible: boolean;
  targets: PlannedOutputTarget[];
  omittedTargetCount: number;
  collisions: OutputCollision[];
  collisionCount: number;
  omittedCollisionCount: number;
  testOutputsMayReceiveUniqueSuffixes: boolean;
}

export interface QueueArtworkCandidate {
  id: string;
  provider: string;
  title: string;
  imageUrl: string;
  thumbnailUrl: string;
  sourceUrl: string;
  creator: string | null;
  license: string | null;
  licenseUrl: string | null;
  attribution: string | null;
}

export interface QueueMetadataSuggestion {
  kind: 'movie' | 'tv';
  status: 'searching' | 'ready' | 'error';
  title: string | null;
  year: string | null;
  provider: string | null;
  recordId: string | null;
  sourceUrl: string | null;
  description: string | null;
  autoAssociated: boolean;
  artwork: QueueArtworkCandidate | null;
  error: string;
}

export interface MetadataLookupStatus {
  status: 'searching' | 'ready' | 'error';
  done: number;
  total: number;
  label: string;
  message: string;
  sourceRoot?: string;
}

export interface QueueMovieReview {
  title: string;
  year: string;
  certification: string;
  recordId: string;
  shortDescription: string;
  plot: string;
  genresText: string;
  ratingText: string;
  sourceUrl: string;
  comment?: string;
  artworkCandidates: QueueArtworkCandidate[];
  posterId: string;
  localPosterPath: string;
  nfoPaths: string[];
  nfoError: string;
}

export interface QueueTvEpisode {
  season: number;
  episode: number;
  title: string;
  airdate: string | null;
  summary: string | null;
  sourceUrl: string;
  comment?: string;
  certification?: string;
}

export interface QueueSeasonReview {
  title: string;
  year: string;
  certification: string;
  plot: string;
  genresText: string;
  ratingText: string;
  comment?: string;
}

export interface QueueSeriesReview {
  title: string;
  year: string;
  certification: string;
  recordId: string;
  shortDescription: string;
  plot: string;
  genresText: string;
  ratingText: string;
  sourceUrl: string;
  comment?: string;
  artworkCandidates: QueueArtworkCandidate[];
  posterId: string;
  localPosterPath: string;
  localSeasonPosterPaths: Record<string, string>;
  seasonRecords: Record<string, QueueSeasonReview>;
  nfoPaths: string[];
  nfoError: string;
  episodes: QueueTvEpisode[];
  episodeAssignments: Record<string, QueueTvEpisode>;
}

export interface MediaQueueLoadResult {
  queue: MediaQueueDocument;
  queueFile: string;
  warning: string | null;
}

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
  youtubeOutputDir: string | null;
  settings: Settings;
  stopOnError: boolean;
  fullRebuildConfirmed: boolean;
  fileTuneOverrides: Record<string, 'animation' | 'none'>;
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
