// Tool paths, effective settings and the resolved work directory.

export interface ToolPaths {
  ffmpeg: string | null;
  ffprobe: string | null;
  mediainfo: string | null;
  mkvpropedit: string | null;
  mkvmerge: string | null;
  mkvextract: string | null;
}

export interface Settings {
  paths: {
    /** `auto` resolves beside each source; otherwise this is an absolute folder. */
    work_root: string;
  };
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
  /** Absent in profiles saved before the quality check existed; the engine then uses its defaults. */
  quality?: {
    ssim_gate: "off" | "report" | "enforce";
    min_ssim: number;
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

export interface WorkDirectoryPreview {
  source_path: string;
  configured_root: string;
  directory: string;
  resolution: 'source-adjacent' | 'configured' | 'environment-override' | string;
  fallback_directory: string | null;
}
