export type RunMode = 'check' | 'test' | 'full';
export type Page = 'overview' | 'library' | 'run' | 'reports';

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
  ready: boolean;
  missing_apt_packages: string[];
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
  };
  audio_count: number;
  subtitle_count: number;
  chapters_count: number;
  duration_seconds: number;
  interlaced: boolean;
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
  per_frame_allowance_gib?: number;
  per_worker_allowance_gib?: number;
  modeled_plan_gib?: number | null;
  memory_worker_cap?: number | null;
  requested_workers?: number | null;
  requested_frame_threads?: number | null;
  planned_workers: number | null;
  planned_frame_threads: number | null;
  messages: string[];
  plan_reason?: string;
}

export interface MediaEntry {
  path: string;
  relative_path: string;
  name: string;
  size_bytes: number;
  status: 'Ready' | 'Backed up' | 'Tools required' | 'Analysis failed' | string;
  already_processed: boolean;
  profile: MediaProfile | null;
  resource_forecast?: ResourceForecast | null;
  error: string | null;
}

export interface ScanResult {
  root: string;
  recursive: boolean;
  minimum_size_mib: number;
  candidate_count: number;
  total_input_bytes: number;
  disk_free_bytes: number;
  entries: MediaEntry[];
  environment: Environment;
  settings: Settings;
  settings_sources: string[];
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
  hdr10?: string;
  hdr10plus?: string;
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
  settings: Settings;
  stopOnError: boolean;
  fullRebuildConfirmed: boolean;
}

export interface RunLineEvent {
  runId: string;
  line: string;
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
