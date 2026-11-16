//! Request parameter types for the MCP tools. They are plain data: each field is
//! documented here because the schema is what an agent reads next to the tool text.

use super::*;

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EnvironmentParams {
    pub(super) ffmpeg_path: Option<String>,
    pub(super) tool_paths: Option<ToolPaths>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RootParams {
    pub(super) root: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkDirectoryParams {
    pub(super) root: String,
    /// Optional `auto` or absolute folder override. Omit to use effective settings.
    pub(super) work_root: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ScanParams {
    pub(super) root: String,
    pub(super) recursive: Option<bool>,
    pub(super) minimum_size_mib: Option<u32>,
    pub(super) filename_include_patterns: Option<Vec<String>>,
    pub(super) filename_exclude_patterns: Option<Vec<String>>,
    pub(super) ffmpeg_path: Option<String>,
    pub(super) tool_paths: Option<ToolPaths>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct IntegrityParams {
    pub(super) files: Vec<String>,
    pub(super) level: Option<String>,
    pub(super) ffmpeg_path: Option<String>,
    pub(super) tool_paths: Option<ToolPaths>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct JobParams {
    pub(super) job_id: String,
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QueuePageParams {
    pub(super) offset: Option<usize>,
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct QueueSelectionParams {
    pub(super) selected_media_paths: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtworkCachePurgeParams {
    pub(super) preview_id: String,
    pub(super) confirm_purge: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct LocalArtworkCacheParams {
    pub(super) source_path: String,
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StartRunParams {
    pub(super) root: String,
    pub(super) mode: Option<String>,
    /// `auto` keeps temporary work beside each source; an absolute folder uses a shared work root.
    pub(super) work_root: Option<String>,
    /// youtube mode: `source`, `1440p` (default) or `4k`. Upscaling creates a higher-tier upload copy; it adds no detail.
    pub(super) youtube_resolution: Option<String>,
    /// youtube mode: `balanced` (default) or `recommended` (can be much larger).
    pub(super) youtube_quality_mode: Option<String>,
    /// youtube mode: balanced bitrate as 0.5 to 4 times the source average (default 2), capped at YouTube's recommended rate.
    pub(super) youtube_source_multiplier: Option<f64>,
    /// `upscale` (default) resizes to 1440p/4K; `optimize` keeps resolution and only retags,
    /// copies, or shrinks 1440p+ sources.
    pub(super) youtube_mode: Option<String>,
    /// SDR codec: `h264` (default) or `hevc`. HDR always uses HEVC.
    pub(super) youtube_sdr_codec: Option<String>,
    /// Encoder preset: `slow` (default), `medium`, or `slower`.
    pub(super) youtube_preset: Option<String>,
    /// Optimize re-encode policy: `auto` (default), `off`, or `aggressive`.
    pub(super) youtube_shrink: Option<String>,
    /// Optimize SSIM gate: `standard` (default), `off`, `relaxed`, or `strict`.
    pub(super) youtube_quality_check: Option<String>,
    /// youtube mode: absolute folder for upload copies.
    pub(super) upload_output_dir: Option<String>,
    /// metadata/test/full: existing folder that receives outputs, mirroring the input's folder structure. Omit for in-place full rebuilds.
    pub(super) output_dir: Option<String>,
    pub(super) recursive: Option<bool>,
    pub(super) minimum_size_mib: Option<u32>,
    pub(super) ffmpeg_path: Option<String>,
    pub(super) tool_paths: Option<ToolPaths>,
    /// Full settings object for this run only (never saved). `metadata.text_blacklist` holds one case-insensitive glob per entry (`*` matches anything). Omit to use the effective settings.
    pub(super) settings_json: Option<String>,
    /// Stop after the first failed file instead of continuing.
    pub(super) stop_on_error: Option<bool>,
    /// Required for mode=test.
    pub(super) confirm_test_output: Option<bool>,
    /// Required for mode=full.
    pub(super) confirm_full_rebuild: Option<bool>,
    /// Required for mode=youtube.
    pub(super) confirm_youtube_upload: Option<bool>,
    /// Case-insensitive basename globs (`*`, `?`) to keep.
    pub(super) filename_include_patterns: Option<Vec<String>>,
    /// Basename or folder-name globs to skip; they beat includes. Omitted means `["old"]` (the legacy backup folder; the current hidden `.hevc-rebuild-originals` folder is always skipped); pass an explicit list to change that.
    pub(super) filename_exclude_patterns: Option<Vec<String>>,
    /// Absolute media paths inside root to process; omit for every eligible file.
    pub(super) selected_files: Option<Vec<String>>,
    /// path -> `animation` or `none`, overriding the automatic content tune.
    pub(super) file_tune_overrides: Option<BTreeMap<String, String>>,
    /// test/full: path -> FFprobe stream indices to keep. Selected real video tracks are encoded to HEVC and the rest are copied. Not accepted for metadata or youtube.
    pub(super) file_stream_overrides: Option<BTreeMap<String, Vec<u16>>>,
    /// check/test/full: path -> settings for this run only (crf, preset, 8-bit promotion, tune, HDR10+, Dolby Vision policy, stream choices). Omitted fields inherit the effective settings.
    pub(super) file_settings_overrides: Option<BTreeMap<String, Value>>,
    /// path -> JPEG, PNG or WebP artwork inside the input or returned by cache_library_artwork; attached to new MKV outputs when metadata.embed_artwork is true.
    pub(super) file_poster_overrides: Option<BTreeMap<String, String>>,
    /// path -> reviewed title, year, certification, summaries, plot, genres, rating, source URL, show title, season, episode, air date. These are the only source of output title/description tags; animation genres select the animation tune.
    pub(super) file_metadata_overrides: Option<BTreeMap<String, Value>>,
    /// path -> existing destination folder that stores that file's output directly.
    pub(super) file_output_dirs: Option<BTreeMap<String, String>>,
    /// youtube mode: files whose HDR10+ dynamic metadata may be lost when hdr10plus_tool is unavailable. Dolby Vision sources are always skipped.
    pub(super) allow_youtube_hdr10plus_loss_paths: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StartQueueRunParams {
    /// A list of single-source run requests. Every request must include its explicit selected_files list.
    pub(super) runs: Vec<StartRunParams>,
    /// Stop before starting another source after a non-zero engine result. Defaults to true.
    pub(super) stop_on_error: Option<bool>,
    /// Explicitly acknowledge review of any selected interrupted or uncertain queue outputs.
    pub(super) acknowledge_review_paths: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanBatchParams {
    /// Absolute path of the library folder to rebuild.
    pub(super) root: String,
    /// Include subfolders. Defaults to true.
    pub(super) recursive: Option<bool>,
    pub(super) minimum_size_mib: Option<u32>,
    pub(super) filename_include_patterns: Option<Vec<String>>,
    pub(super) filename_exclude_patterns: Option<Vec<String>>,
    /// Free space to protect, in GiB. Omit for the default: the larger of 10 GiB and 5% of the volume.
    pub(super) reserve_gib: Option<f64>,
    /// `auto` (beside each source) or an absolute shared folder. Omit to use effective settings.
    pub(super) work_root: Option<String>,
    /// Optional full settings object; omit to use the effective settings for this folder.
    pub(super) settings_json: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StartPlannedParams {
    /// The plan_id returned by plan_library_batch.
    pub(super) plan_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StopParams {
    pub(super) job_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RepairParams {
    pub(super) confirmed_package_names: Vec<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct AptUpdateParams {}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct AptRefreshParams {
    pub(super) confirm_refresh: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct HelperParams {
    pub(super) confirm_installation: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportsParams {
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct LogParams {
    pub(super) path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct MetadataSearchParams {
    pub(super) query: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct FilmMetadataParams {
    pub(super) entity_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct TvMetadataParams {
    pub(super) show_id: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OrganizerFolderParams {
    pub(super) root: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RenamePreviewParams {
    pub(super) root: String,
    pub(super) items: Vec<crate::library_organizer::RenameTemplateInput>,
    pub(super) text_blacklist_patterns: Option<Vec<String>>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RenameApplyParams {
    pub(super) root: String,
    pub(super) items: Vec<crate::library_organizer::RenameTemplateInput>,
    pub(super) text_blacklist_patterns: Option<Vec<String>>,
    pub(super) preview_id: String,
    pub(super) confirm_rename: bool,
}
