// Environment checks, APT update status and the app artwork cache report.

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
    package_manager?: string | null;
    package_manager_available?: boolean;
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
