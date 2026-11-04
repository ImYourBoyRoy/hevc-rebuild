//! ./desktop/src-tauri/src/rebuild_run.rs
//! Native Short-Test and Full Rebuild coordinator. Rust owns the whole job with
//! no Python process: discovery, profiling, stream selection, content tune,
//! Dolby Vision / HDR10+ handling, memory planning, optional hardware
//! calibration, FFmpeg encode with adaptive memory retries, metadata/poster
//! embedding, validation, and durable install (in-place `old/` backup or
//! separate export). Check-only stays in `run_coordinator`; Metadata-only and
//! YouTube stay in their own modules.
//!
//! Run: GUI `start_run`, MCP `start_run`, and the CLI hand the same engine-style
//! argument vector to `RebuildRunOptions::from_engine_args`, then call
//! `run_rebuild`. Sources are never replaced until validation succeeds.

use crate::encode_run::{run_encode, EncodeFailure, EncodeProgress, EncodeSpec};
use crate::media_scan::{
    discover_media_paths, disk_free_bytes, engine_profile, engine_track_profiles,
    EngineProfileRequest, EngineTrackProfilesRequest,
};
use crate::output_targets::canonicalize_allow_missing;
use crate::resource_plan::{memory_forecast, retry_plan, MemoryForecastInput};
use crate::run_arguments::ToolPaths;
use crate::run_coordinator::optional_tool;
use crate::transactions::{
    commit_output, install_export_output, rename_no_replace, sync_directory,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;
const MAX_ENCODE_ATTEMPTS: u32 = 3;
const FINISHED_OUTPUT_PHASES: [&str; 5] = [
    "metadata embedding",
    "output validation",
    "lossless output validation",
    "validated output install",
    "transaction commit",
];
static UNIQUE: AtomicU64 = AtomicU64::new(0);

const REJECTED_FLAGS: [(&str, &str); 3] = [
    (
        "--youtube-upload",
        "YouTube export is a separate native workflow",
    ),
    (
        "--metadata-only",
        "metadata-only MKV is a separate native workflow",
    ),
    (
        "--upload-output-dir",
        "YouTube output folders do not apply to rebuild mode",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct RebuildRunOptions {
    pub root: PathBuf,
    pub recursive: bool,
    pub minimum_size_mib: u32,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub selected_files: Vec<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub file_output_dirs: BTreeMap<PathBuf, PathBuf>,
    pub poster_overrides: BTreeMap<PathBuf, PathBuf>,
    pub metadata_overrides: BTreeMap<PathBuf, Map<String, Value>>,
    pub file_tune_overrides: BTreeMap<PathBuf, String>,
    pub file_stream_overrides: BTreeMap<PathBuf, Vec<u16>>,
    pub file_settings_overrides: BTreeMap<PathBuf, Map<String, Value>>,
    pub comment: String,
    pub text_blacklist: Vec<String>,
    pub embed_artwork: bool,
    pub crf: i64,
    pub test_crf: i64,
    pub preset: String,
    pub tune: String,
    pub detect_animation: bool,
    pub promote_8bit_to_10bit: bool,
    pub preserve_hdr10plus: bool,
    pub dolby_vision_policy: String,
    pub secondary_video: String,
    pub data_streams: String,
    pub verify_copy_hashes: bool,
    pub resource_threading: String,
    pub reserved_cpu_threads: i64,
    pub max_worker_threads: i64,
    pub frame_threads: i64,
    pub min_free_memory_gib: f64,
    pub memory_reserve_percent: f64,
    pub min_free_disk_gib: f64,
    pub preflight_output_ratio: f64,
    pub disk_projection_margin_percent: f64,
    pub calibration_mode: String,
    pub calibration_cache_hours: f64,
    pub calibration_large_file_gib: f64,
    pub calibration_sample_seconds: f64,
    pub calibration_max_candidates: usize,
    pub calibration_tolerance_percent: f64,
    pub test_mode: bool,
    pub info_only: bool,
    pub test_seconds: i64,
    pub stop_on_error: bool,
    pub keep_work: bool,
    pub reprocess: bool,
    pub force_deinterlace: bool,
    pub disable_deinterlace: bool,
    pub disable_hdr10_opt: bool,
    pub preserve_source_level: bool,
    pub work_root: String,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
}

#[derive(Clone, Debug)]
pub(crate) struct RebuildProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: String,
    pub file_percent: Option<f64>,
    pub queue_percent: f64,
    pub elapsed_seconds: f64,
    pub speed: Option<f64>,
    pub eta_seconds: Option<f64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RebuildFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
}

pub(crate) struct RebuildJobResult {
    pub files: Vec<RebuildFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
    pub succeeded: usize,
    pub skipped: usize,
}

struct Tools {
    ffmpeg: String,
    ffprobe: String,
    mediainfo: String,
    mkvmerge: String,
    mkvpropedit: String,
    mkvextract: String,
    hdr10plus: Option<String>,
    x265_pixel_formats: Vec<String>,
    x265_dolbyvision: bool,
    x265_hdr10plus: bool,
}

struct Context<'a> {
    options: &'a RebuildRunOptions,
    tools: &'a Tools,
    progress: &'a mut dyn FnMut(RebuildProgress),
    line: &'a mut dyn FnMut(String),
    child: &'a mut dyn FnMut(Option<u32>),
}

struct FileState {
    phase: &'static str,
    temp_output: Option<PathBuf>,
    final_output: Option<PathBuf>,
    row: Map<String, Value>,
    interrupted: bool,
}

struct FileFrame<'a> {
    source: &'a Path,
    number: usize,
    total: usize,
    started: Instant,
    input_bytes: Option<u64>,
}

#[derive(Clone)]
struct StreamRef {
    index: i64,
    type_ordinal: u32,
    codec_type: String,
    codec_name: String,
    width: i64,
    height: i64,
    default: bool,
    attached_pic: bool,
    disposition: Map<String, Value>,
}

impl StreamRef {
    fn pixels(&self) -> i64 {
        self.width.max(0) * self.height.max(0)
    }

    fn real_video(&self) -> bool {
        self.codec_type == "video" && !self.attached_pic && self.width > 0 && self.height > 0
    }

    fn to_plan_value(&self) -> Value {
        json!({
            "index": self.index,
            "type_ordinal": self.type_ordinal,
            "codec_type": self.codec_type,
            "codec_name": self.codec_name,
            "width": self.width,
            "height": self.height,
            "default": self.default,
            "attached_pic": self.attached_pic,
            "disposition": self.disposition,
        })
    }
}

struct ResourcePlan {
    worker_threads: i64,
    frame_threads: i64,
    per_frame: f64,
    per_worker: f64,
    budget: Option<f64>,
    memory_guard_gib: f64,
    forecast: Value,
    cpu_threads: i64,
}

impl ResourcePlan {
    fn reduced(&self) -> Option<Self> {
        let next = retry_plan(
            self.worker_threads,
            self.frame_threads,
            self.per_frame,
            self.per_worker,
            self.budget,
        );
        if next.get("reduces_parallelism").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(Self {
            worker_threads: next
                .get("planned_workers")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1),
            frame_threads: next
                .get("planned_frame_threads")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                .max(1),
            per_frame: self.per_frame,
            per_worker: self.per_worker,
            budget: self.budget,
            memory_guard_gib: self.memory_guard_gib,
            forecast: next,
            cpu_threads: self.cpu_threads,
        })
    }
}

struct EffectiveSettings {
    crf: i64,
    test_crf: i64,
    preset: String,
    tune: String,
    promote_8bit_to_10bit: bool,
    preserve_hdr10plus: bool,
    dolby_vision_policy: String,
    stream_indices: Option<Vec<u16>>,
    clear_file_tune: bool,
}

// ---------------------------------------------------------------- arguments

impl RebuildRunOptions {
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            embed_artwork: true,
            crf: 22,
            test_crf: 30,
            preset: "slow".into(),
            tune: "auto".into(),
            detect_animation: true,
            promote_8bit_to_10bit: false,
            preserve_hdr10plus: true,
            dolby_vision_policy: "safe".into(),
            secondary_video: "encode".into(),
            data_streams: "preserve".into(),
            verify_copy_hashes: true,
            resource_threading: "auto".into(),
            reserved_cpu_threads: 1,
            max_worker_threads: 0,
            frame_threads: 0,
            min_free_memory_gib: 3.0,
            memory_reserve_percent: 8.0,
            min_free_disk_gib: 0.0,
            preflight_output_ratio: 1.25,
            disk_projection_margin_percent: 15.0,
            calibration_mode: "auto".into(),
            calibration_cache_hours: 168.0,
            calibration_large_file_gib: 20.0,
            calibration_sample_seconds: 4.0,
            calibration_max_candidates: 3,
            calibration_tolerance_percent: 5.0,
            test_seconds: 5,
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut comment: Option<String> = None;
        let mut embed_artwork: Option<bool> = None;
        let mut blacklist: Vec<String> = Vec::new();
        let mut replace_blacklist = false;
        let mut output_dirs_json: Option<String> = None;
        let mut poster_json: Option<String> = None;
        let mut metadata_json: Option<String> = None;
        let mut tune_json: Option<String> = None;
        let mut stream_json: Option<String> = None;
        let mut settings_json: Option<String> = None;
        let mut index = 0;
        while index < args.len() {
            let name = args[index].as_str();
            let value = || -> Result<String, String> {
                args.get(index + 1)
                    .cloned()
                    .ok_or_else(|| format!("Option {name} requires a value."))
            };
            let mut consumed = 2;
            match name {
                "--recursive" => {
                    options.recursive = true;
                    consumed = 1;
                }
                "--stop-on-error" => {
                    options.stop_on_error = true;
                    consumed = 1;
                }
                "--keep-work" => {
                    options.keep_work = true;
                    consumed = 1;
                }
                "--reprocess" => {
                    options.reprocess = true;
                    consumed = 1;
                }
                "--test" => {
                    options.test_mode = true;
                    consumed = 1;
                }
                "--info-only" => {
                    options.info_only = true;
                    consumed = 1;
                }
                "--force-deinterlace" => {
                    options.force_deinterlace = true;
                    consumed = 1;
                }
                "--disable-deinterlace" => {
                    options.disable_deinterlace = true;
                    consumed = 1;
                }
                "--disable-hdr10-opt" => {
                    options.disable_hdr10_opt = true;
                    consumed = 1;
                }
                "--preserve-source-level" => {
                    options.preserve_source_level = true;
                    consumed = 1;
                }
                "--replace-metadata-text-blacklist" => {
                    replace_blacklist = true;
                    consumed = 1;
                }
                "--embed-artwork" => {
                    embed_artwork = Some(true);
                    consumed = 1;
                }
                "--no-embed-artwork" => {
                    embed_artwork = Some(false);
                    consumed = 1;
                }
                "--promote-8bit-to-10bit" => {
                    options.promote_8bit_to_10bit = true;
                    consumed = 1;
                }
                "--no-promote-8bit-to-10bit" => {
                    options.promote_8bit_to_10bit = false;
                    consumed = 1;
                }
                "--detect-animation" => {
                    options.detect_animation = true;
                    consumed = 1;
                }
                "--no-detect-animation" => {
                    options.detect_animation = false;
                    consumed = 1;
                }
                "--preserve-hdr10plus" => {
                    options.preserve_hdr10plus = true;
                    consumed = 1;
                }
                "--no-preserve-hdr10plus" => {
                    options.preserve_hdr10plus = false;
                    consumed = 1;
                }
                "--verify-copy-hashes" => {
                    options.verify_copy_hashes = true;
                    consumed = 1;
                }
                "--no-verify-copy-hashes" => {
                    options.verify_copy_hashes = false;
                    consumed = 1;
                }
                "--no-dependency-repair"
                | "--skip-update-check"
                | "--assume-yes"
                | "--allow-unsupported-os"
                | "--verbose" => consumed = 1,
                "--min-size-mib" => {
                    options.minimum_size_mib = value()?
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value <= 4096)
                        .ok_or("Minimum input size must be between 0 and 4096 MiB.")?;
                }
                "--include-filename-pattern" => options.include.push(value()?),
                "--exclude-filename-pattern" => options.exclude.push(value()?),
                "--selected-file" => options.selected_files.push(PathBuf::from(value()?)),
                "--output-dir" => options.output_dir = Some(PathBuf::from(value()?)),
                "--file-output-dirs-json" => output_dirs_json = Some(value()?),
                "--file-poster-overrides-json" => poster_json = Some(value()?),
                "--file-metadata-overrides-json" => metadata_json = Some(value()?),
                "--file-tune-overrides-json" => tune_json = Some(value()?),
                "--file-stream-overrides-json" => stream_json = Some(value()?),
                "--file-settings-overrides-json" => settings_json = Some(value()?),
                "--allowed-poster-cache-root" => {
                    value()?;
                }
                "--comment" => comment = Some(value()?),
                "--metadata-text-blacklist" => blacklist.push(value()?),
                "--crf" => options.crf = parse_crf(&value()?)?,
                "--test-crf" => options.test_crf = parse_crf(&value()?)?,
                "--preset" => {
                    options.preset = parse_choice(
                        &value()?,
                        &[
                            "ultrafast",
                            "superfast",
                            "veryfast",
                            "faster",
                            "fast",
                            "medium",
                            "slow",
                            "slower",
                            "veryslow",
                        ],
                        "preset",
                    )?;
                }
                "--tune" => {
                    options.tune =
                        parse_choice(&value()?, &["auto", "animation", "grain", "none"], "tune")?;
                }
                "--dolby-vision-policy" => {
                    options.dolby_vision_policy = parse_choice(
                        &value()?,
                        &["safe", "skip", "hdr10-fallback"],
                        "Dolby Vision policy",
                    )?;
                }
                "--secondary-video" => {
                    options.secondary_video =
                        parse_choice(&value()?, &["encode", "copy", "drop"], "secondary video")?;
                }
                "--data-streams" => {
                    options.data_streams =
                        parse_choice(&value()?, &["preserve", "drop"], "data streams")?;
                }
                "--resource-threading" => {
                    options.resource_threading =
                        parse_choice(&value()?, &["auto", "unlimited"], "resource threading")?;
                }
                "--reserved-cpu-threads" => {
                    options.reserved_cpu_threads =
                        parse_i64(&value()?, 0, 64, "reserved CPU threads")?;
                }
                "--max-worker-threads" => {
                    options.max_worker_threads = parse_i64(&value()?, 0, 64, "max worker threads")?;
                }
                "--frame-threads" => {
                    options.frame_threads = parse_i64(&value()?, 0, 16, "frame threads")?;
                }
                "--min-free-memory-gib" => {
                    options.min_free_memory_gib =
                        parse_f64(&value()?, 0.0, 1_000_000.0, "memory floor")?;
                }
                "--memory-reserve-percent" => {
                    options.memory_reserve_percent =
                        parse_f64(&value()?, 0.0, 50.0, "memory reserve percent")?;
                }
                "--min-free-disk-gib" => {
                    options.min_free_disk_gib =
                        parse_f64(&value()?, 0.0, 1_000_000.0, "disk reserve")?;
                }
                "--preflight-output-ratio" => {
                    options.preflight_output_ratio =
                        parse_f64(&value()?, 0.25, 4.0, "preflight output ratio")?;
                }
                "--disk-projection-margin-percent" => {
                    options.disk_projection_margin_percent =
                        parse_f64(&value()?, 0.0, 100.0, "disk projection margin")?;
                }
                "--calibration-mode" => {
                    options.calibration_mode = parse_choice(
                        &value()?,
                        &["auto", "ask", "always", "off"],
                        "calibration mode",
                    )?;
                }
                "--calibration-cache-hours" => {
                    options.calibration_cache_hours =
                        parse_f64(&value()?, 0.0, 720.0, "calibration cache hours")?;
                }
                "--calibration-large-file-gib" => {
                    options.calibration_large_file_gib =
                        parse_f64(&value()?, 0.0, 1000.0, "calibration large-file threshold")?;
                }
                "--calibration-sample-seconds" => {
                    options.calibration_sample_seconds =
                        parse_f64(&value()?, 1.0, 15.0, "calibration sample seconds")?;
                }
                "--calibration-max-candidates" => {
                    options.calibration_max_candidates =
                        parse_i64(&value()?, 1, 4, "calibration candidates")? as usize;
                }
                "--calibration-tolerance-percent" => {
                    options.calibration_tolerance_percent =
                        parse_f64(&value()?, 0.0, 20.0, "calibration tolerance")?;
                }
                "--test-seconds" => {
                    options.test_seconds = parse_i64(&value()?, 1, 600, "test seconds")?;
                }
                "--work-root" => {
                    options.work_root = value()?;
                    crate::preview_work_directory(Path::new("."), &options.work_root)?;
                }
                "--stop-request-file" => options.stop_file = Some(PathBuf::from(value()?)),
                "--ffmpeg-path" => options.tool_paths.ffmpeg = Some(value()?),
                "--ffprobe-path" => options.tool_paths.ffprobe = Some(value()?),
                "--mediainfo-path" => options.tool_paths.mediainfo = Some(value()?),
                "--mkvpropedit-path" => options.tool_paths.mkvpropedit = Some(value()?),
                "--mkvmerge-path" => options.tool_paths.mkvmerge = Some(value()?),
                "--mkvextract-path" => options.tool_paths.mkvextract = Some(value()?),
                flag if REJECTED_FLAGS.iter().any(|(rejected, _)| *rejected == flag) => {
                    let reason = REJECTED_FLAGS
                        .iter()
                        .find(|(rejected, _)| *rejected == flag)
                        .map(|(_, reason)| *reason)
                        .unwrap_or("this option is not supported");
                    return Err(format!("Rebuild runs reject this option: {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "Rebuild runs do not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("Rebuild runs accept exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        if options.info_only && options.test_mode {
            return Err("Check-only and short-test modes cannot be combined.".into());
        }
        if options.force_deinterlace && options.disable_deinterlace {
            return Err(
                "--force-deinterlace and --disable-deinterlace are mutually exclusive.".into(),
            );
        }
        let root = root.ok_or("Rebuild runs require a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;

        let settings = effective_settings_json(&options.root)?;
        options.comment = match comment {
            Some(value) => value,
            None => settings
                .pointer("/metadata/comment")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
        if options.comment.len() > 500 {
            return Err("Metadata comment is limited to 500 UTF-8 bytes.".into());
        }
        options.embed_artwork = embed_artwork.unwrap_or_else(|| {
            settings
                .pointer("/metadata/embed_artwork")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
        options.text_blacklist = merged_blacklist(&settings, blacklist, replace_blacklist);
        options.file_output_dirs = parse_path_map(output_dirs_json, "per-file output folders")?;
        options.poster_overrides = parse_path_map(poster_json, "poster selections")?;
        options.metadata_overrides = parse_metadata_overrides(metadata_json)?;
        options.file_tune_overrides = parse_tune_overrides(tune_json)?;
        options.file_stream_overrides = parse_stream_overrides(stream_json)?;
        options.file_settings_overrides = parse_settings_overrides(settings_json)?;
        Ok(options)
    }
}

fn parse_crf(raw: &str) -> Result<i64, String> {
    parse_i64(raw, 0, 51, "CRF")
}

fn parse_i64(raw: &str, min: i64, max: i64, label: &str) -> Result<i64, String> {
    let value = raw
        .parse::<i64>()
        .map_err(|_| format!("{label} must be a whole number from {min} to {max}."))?;
    if !(min..=max).contains(&value) {
        return Err(format!(
            "{label} must be a whole number from {min} to {max}."
        ));
    }
    Ok(value)
}

fn parse_f64(raw: &str, min: f64, max: f64, label: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{label} must be a finite number."))?;
    if !(min..=max).contains(&value) {
        return Err(format!("{label} must be between {min} and {max}."));
    }
    Ok(value)
}

fn parse_choice(raw: &str, allowed: &[&str], label: &str) -> Result<String, String> {
    if allowed.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(format!("{label} must be one of: {}.", allowed.join(", ")))
    }
}

fn effective_settings_json(root: &Path) -> Result<Value, String> {
    let anchor = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    let payload = serde_json::to_value(crate::settings::load_effective_settings(
        &anchor.to_string_lossy(),
    )?)
    .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    Ok(payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({})))
}

fn merged_blacklist(settings: &Value, extra: Vec<String>, replace: bool) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    let configured = if replace {
        Vec::new()
    } else {
        settings
            .pointer("/metadata/text_blacklist")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    for pattern in configured.into_iter().chain(extra) {
        if seen.insert(pattern.to_lowercase()) {
            merged.push(pattern);
        }
    }
    merged
}

fn parse_path_map(raw: Option<String>, label: &str) -> Result<BTreeMap<PathBuf, PathBuf>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("The {label} are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err(format!("The {label} are limited to 512 media files."));
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let value = value
            .as_str()
            .ok_or_else(|| format!("Each entry in the {label} must map a path to a path."))?;
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A media path in the {label} is unavailable: {error}"))?;
        result.insert(key, PathBuf::from(value));
    }
    Ok(result)
}

fn parse_metadata_overrides(
    raw: Option<String>,
) -> Result<BTreeMap<PathBuf, Map<String, Value>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Reviewed metadata is not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Reviewed metadata is limited to 512 media files per run.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let Value::Object(fields) = value else {
            return Err("Each reviewed metadata value must be an object.".into());
        };
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A reviewed metadata source is unavailable: {error}"))?;
        result.insert(key, fields);
    }
    Ok(result)
}

fn parse_tune_overrides(raw: Option<String>) -> Result<BTreeMap<PathBuf, String>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file tune overrides are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file tune overrides are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let tune = value.as_str().ok_or("Each per-file tune must be text.")?;
        if !matches!(tune, "animation" | "none") {
            return Err("Per-file tune overrides must be 'animation' or 'none'.".into());
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A per-file tune source is unavailable: {error}"))?;
        result.insert(key, tune.to_string());
    }
    Ok(result)
}

fn parse_stream_overrides(raw: Option<String>) -> Result<BTreeMap<PathBuf, Vec<u16>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file stream selections are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file stream selections are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let indices = value
            .as_array()
            .ok_or("Each per-file stream selection must be an array.")?;
        if indices.is_empty() || indices.len() > 1024 {
            return Err(
                "Each media file must select at least one stream and no more than 1024 streams."
                    .into(),
            );
        }
        let mut unique = BTreeSet::new();
        let mut values = Vec::new();
        for item in indices {
            let index = item
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or("Stream indices must be integers from 0 to 65535.")?;
            if !unique.insert(index) {
                return Err("Per-file stream selections must contain unique indices.".into());
            }
            values.push(index);
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A stream-selection source is unavailable: {error}"))?;
        result.insert(key, values);
    }
    Ok(result)
}

fn parse_settings_overrides(
    raw: Option<String>,
) -> Result<BTreeMap<PathBuf, Map<String, Value>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file run settings are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file run settings are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let Value::Object(fields) = value else {
            return Err("Each per-file run setting must be an object.".into());
        };
        if fields.is_empty() {
            return Err("Each per-file run setting must be a non-empty object.".into());
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A per-file settings source is unavailable: {error}"))?;
        result.insert(key, fields);
    }
    Ok(result)
}

fn effective_for_file(options: &RebuildRunOptions, source: &Path) -> EffectiveSettings {
    let mut effective = EffectiveSettings {
        crf: options.crf,
        test_crf: options.test_crf,
        preset: options.preset.clone(),
        tune: options.tune.clone(),
        promote_8bit_to_10bit: options.promote_8bit_to_10bit,
        preserve_hdr10plus: options.preserve_hdr10plus,
        dolby_vision_policy: options.dolby_vision_policy.clone(),
        stream_indices: options.file_stream_overrides.get(source).cloned(),
        clear_file_tune: false,
    };
    let Some(override_map) = options.file_settings_overrides.get(source) else {
        return effective;
    };
    if let Some(value) = override_map.get("crf").and_then(Value::as_i64) {
        effective.crf = value;
        if options.test_mode {
            effective.test_crf = value;
        }
    }
    if let Some(value) = override_map.get("preset").and_then(Value::as_str) {
        effective.preset = value.to_string();
    }
    if let Some(value) = override_map.get("tune").and_then(Value::as_str) {
        effective.tune = value.to_string();
        effective.clear_file_tune = true;
    }
    if let Some(value) = override_map
        .get("promote_8bit_to_10bit")
        .and_then(Value::as_bool)
    {
        effective.promote_8bit_to_10bit = value;
    }
    if let Some(value) = override_map
        .get("preserve_hdr10plus")
        .and_then(Value::as_bool)
    {
        effective.preserve_hdr10plus = value;
    }
    if let Some(value) = override_map.get("dolby_vision").and_then(Value::as_str) {
        effective.dolby_vision_policy = value.to_string();
    }
    if let Some(indices) = override_map.get("stream_indices").and_then(Value::as_array) {
        effective.stream_indices = Some(
            indices
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u16::try_from(value).ok())
                .collect(),
        );
    }
    effective
}

// ------------------------------------------------------------------ planning

fn resolved_output_path(
    source: &Path,
    options: &RebuildRunOptions,
) -> Result<(PathBuf, bool), String> {
    let per_file_dir = options.file_output_dirs.get(source);
    let Some(base) = per_file_dir.or(options.output_dir.as_ref()) else {
        return Ok((source.with_extension("mkv"), false));
    };
    let file_name = source
        .file_name()
        .map(PathBuf::from)
        .ok_or("The source has no file name.")?
        .with_extension("mkv");
    let relative = if per_file_dir.is_some() || !options.root.is_dir() {
        file_name
    } else {
        source
            .strip_prefix(&options.root)
            .map_err(|_| {
                format!(
                    "Source file is outside the selected input folder: {}",
                    source.display()
                )
            })?
            .with_extension("mkv")
    };
    let mut target = base.join(relative);
    if canonicalize_allow_missing(&target)? == source {
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        target = target.with_file_name(format!("{stem}.hevc-rebuilt.mkv"));
    }
    let resolved = canonicalize_allow_missing(&target)?;
    let base = base
        .canonicalize()
        .map_err(|error| format!("The output folder is unavailable: {error}"))?;
    if !resolved
        .parent()
        .is_some_and(|parent| parent.starts_with(&base))
    {
        return Err(format!(
            "Output folder resolves outside the selected destination: {}",
            target.parent().unwrap_or(&target).display()
        ));
    }
    Ok((resolved, true))
}

fn test_output_path(output_base: &Path) -> Result<PathBuf, String> {
    let stem = output_base
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let parent = output_base.parent().unwrap_or_else(|| Path::new("."));
    let candidate = parent.join(format!("Test_{stem}.mkv"));
    if !candidate.exists() {
        return Ok(candidate);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("Test_{stem}_{timestamp}.mkv")
        } else {
            format!("Test_{stem}_{timestamp}_{suffix}.mkv")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not find a free short-test output name for {}.",
        output_base
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ))
}

fn validate_output_destinations(
    files: &[PathBuf],
    options: &RebuildRunOptions,
) -> Result<(), String> {
    let mut planned: BTreeMap<PathBuf, &Path> = BTreeMap::new();
    for source in files {
        let (base, _) = resolved_output_path(source, options)?;
        let output = if options.test_mode {
            test_output_path(&base)?
        } else {
            base
        };
        if output.exists() && canonicalize_allow_missing(&output)? != *source {
            return Err(format!(
                "Output already exists and will not be overwritten: {}",
                output.display()
            ));
        }
        if let Some(previous) = planned.insert(output.clone(), source) {
            if previous != source.as_path() {
                return Err(format!(
                    "Two selected media files would write to the same output: {} and {} -> {}",
                    previous.display(),
                    source.display(),
                    output.display()
                ));
            }
        }
    }
    Ok(())
}

fn backup_path_for(source: &Path, reprocess: bool) -> Result<PathBuf, String> {
    let parent = source.parent().ok_or("The source has no parent folder.")?;
    let name = source.file_name().ok_or("The source has no file name.")?;
    let old_dir = parent.join("old");
    let candidate = old_dir.join(name);
    if !reprocess || !candidate.exists() {
        return Ok(candidate);
    }
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let extension = source
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    for number in 1..10_000 {
        let candidate = old_dir.join(format!("{stem}.{number}{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Could not allocate a unique backup path under old/.".into())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn storage_preflight(
    source_size: u64,
    free_bytes: u64,
    duration: f64,
    options: &RebuildRunOptions,
) -> Result<(u64, u64, bool), String> {
    let reserve = (options.min_free_disk_gib.max(0.0) * GIB as f64) as u64;
    let (estimated_temp, minimum_temp) = if options.test_mode {
        let fraction = if duration > 0.0 {
            (options.test_seconds as f64 / duration).clamp(0.0, 1.0)
        } else {
            0.05
        };
        (
            (512 * MIB).max((source_size as f64 * fraction.max(0.01) * 2.5) as u64),
            512 * MIB,
        )
    } else {
        (
            (2 * GIB).max((source_size as f64 * options.preflight_output_ratio) as u64),
            GIB.max((8 * GIB).min(source_size / 20)),
        )
    };
    let required = reserve.saturating_add(minimum_temp);
    let recommended = reserve.saturating_add(estimated_temp);
    if free_bytes < required {
        return Err(format!(
            "Insufficient free space even to start safely: {} free, need at least {} ({} reserve + {} working headroom).",
            human_bytes(free_bytes),
            human_bytes(required),
            human_bytes(reserve),
            human_bytes(minimum_temp)
        ));
    }
    Ok((estimated_temp, recommended, free_bytes >= recommended))
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:x}{:x}{:x}",
        std::process::id(),
        nanos,
        UNIQUE.fetch_add(1, Ordering::Relaxed)
    )
}

fn make_work_dir(source: &Path, configured: &str) -> Result<PathBuf, String> {
    let root = crate::media_work_directory(source, configured)?;
    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        let candidate = root.join(format!("hevc-rebuild-{}", unique_token()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create a rebuild work folder: {error}")),
        }
    }
    Err("Could not allocate a unique rebuild work folder.".into())
}

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
}

fn text_value(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => json!(text),
        Some(other) => json!(other.to_string()),
    }
}

fn flag(parent: Option<&Value>, key: &str) -> bool {
    parent
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn stream_refs(probe: &Value) -> Vec<StreamRef> {
    let mut ordinals: BTreeMap<String, u32> = BTreeMap::new();
    probe
        .get("streams")
        .and_then(Value::as_array)
        .map(|streams| {
            streams
                .iter()
                .filter_map(|stream| {
                    let codec_type = stream
                        .get("codec_type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let ordinal = *ordinals.get(&codec_type).unwrap_or(&0);
                    ordinals.insert(codec_type.clone(), ordinal + 1);
                    let disposition = stream
                        .get("disposition")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    Some(StreamRef {
                        index: stream.get("index").and_then(Value::as_i64)?,
                        type_ordinal: ordinal,
                        codec_name: stream
                            .get("codec_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        width: stream.get("width").and_then(Value::as_i64).unwrap_or(0),
                        height: stream.get("height").and_then(Value::as_i64).unwrap_or(0),
                        default: disposition
                            .get("default")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            == 1,
                        attached_pic: disposition
                            .get("attached_pic")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            == 1,
                        disposition,
                        codec_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn choose_primary_video(refs: &[StreamRef]) -> Result<(StreamRef, Vec<StreamRef>), String> {
    let videos: Vec<StreamRef> = refs.iter().filter(|s| s.real_video()).cloned().collect();
    if videos.is_empty() {
        return Err("No usable video stream was found.".into());
    }
    let selected = videos
        .iter()
        .max_by_key(|s| (s.pixels(), s.width, s.height, s.default as i64, -s.index))
        .cloned()
        .ok_or("No usable video stream was found.")?;
    let secondary = videos
        .into_iter()
        .filter(|s| s.index != selected.index)
        .collect();
    Ok((selected, secondary))
}

fn select_file_streams(
    refs: &[StreamRef],
    options: &RebuildRunOptions,
    effective: &EffectiveSettings,
) -> Result<(Vec<StreamRef>, BTreeSet<i64>), String> {
    let by_index: BTreeMap<i64, StreamRef> = refs.iter().map(|s| (s.index, s.clone())).collect();
    let (selected, explicit) = if let Some(indices) = &effective.stream_indices {
        let mut selected = Vec::new();
        for index in indices {
            let stream = by_index.get(&i64::from(*index)).ok_or_else(|| {
                format!("Selected stream index #{index} is not present in the source.")
            })?;
            selected.push(stream.clone());
        }
        (selected, true)
    } else {
        let (highest, secondary) = choose_primary_video(refs)?;
        let mut videos = vec![highest];
        if options.secondary_video != "drop" {
            videos.extend(secondary);
        }
        let mut selected = videos;
        selected.extend(
            refs.iter()
                .filter(|s| matches!(s.codec_type.as_str(), "audio" | "subtitle" | "attachment"))
                .cloned(),
        );
        if options.data_streams == "preserve" {
            selected.extend(refs.iter().filter(|s| s.codec_type == "data").cloned());
        }
        (selected, false)
    };
    let real_videos: Vec<&StreamRef> = selected.iter().filter(|s| s.real_video()).collect();
    if real_videos.is_empty() {
        return Err("Select at least one real video track for this media file.".into());
    }
    let mut ordered = selected;
    ordered.sort_by_key(|stream| {
        let kind_rank = match stream.codec_type.as_str() {
            "video" => 0,
            "audio" => 1,
            "subtitle" => 2,
            "attachment" => 3,
            "data" => 4,
            _ => 5,
        };
        (
            kind_rank,
            stream.attached_pic as i64,
            if stream.codec_type == "video" {
                -stream.pixels()
            } else {
                stream.index
            },
            stream.index,
        )
    });
    let encoded = if explicit || options.secondary_video == "encode" {
        ordered
            .iter()
            .filter(|s| s.real_video())
            .map(|s| s.index)
            .collect()
    } else {
        let primary = ordered
            .iter()
            .filter(|s| s.real_video())
            .max_by_key(|s| (s.pixels(), s.default as i64, -s.index))
            .map(|s| s.index)
            .ok_or("Select at least one real video track for this media file.")?;
        BTreeSet::from([primary])
    };
    Ok((ordered, encoded))
}

fn engine_profile_to_plan_profile(
    source: &Path,
    engine_profile: &Value,
    primary: &StreamRef,
    all_refs: &[StreamRef],
) -> Value {
    let secondary: Vec<Value> = all_refs
        .iter()
        .filter(|s| s.real_video() && s.index != primary.index)
        .map(StreamRef::to_plan_value)
        .collect();
    let group = |kind: &str| -> Vec<Value> {
        all_refs
            .iter()
            .filter(|s| s.codec_type == kind)
            .map(StreamRef::to_plan_value)
            .collect()
    };
    let hdr = engine_profile.get("hdr");
    json!({
        "path": source.to_string_lossy(),
        "primary_video": primary.to_plan_value(),
        "secondary_videos": secondary,
        "audio": group("audio"),
        "subtitles": group("subtitle"),
        "attachments": group("attachment"),
        "data": group("data"),
        "source_bit_depth": engine_profile.pointer("/video/source_bit_depth").cloned().unwrap_or(json!(8)),
        "source_chroma": engine_profile.pointer("/video/source_chroma").cloned().unwrap_or(json!("4:2:0")),
        "target_pix_fmt": engine_profile.pointer("/video/target_pixel_format").cloned().unwrap_or(Value::Null),
        "color_primaries": engine_profile.get("color_primaries").cloned().unwrap_or(Value::Null),
        "transfer": engine_profile.get("color_transfer").cloned().unwrap_or(Value::Null),
        "color_matrix": engine_profile.get("color_matrix").cloned().unwrap_or(Value::Null),
        "color_range_x265": engine_profile.get("color_range_x265").cloned().unwrap_or(Value::Null),
        "color_range_ffmpeg": engine_profile.get("color_range_ffmpeg").cloned().unwrap_or(Value::Null),
        "chroma_location": engine_profile.get("chroma_location").cloned().unwrap_or(Value::Null),
        "master_display": engine_profile.get("mastering_display").cloned().unwrap_or(Value::Null),
        "max_cll": engine_profile.get("max_cll").cloned().unwrap_or(Value::Null),
        "max_fall": engine_profile.get("max_fall").cloned().unwrap_or(Value::Null),
        "hdr10": flag(hdr, "hdr10"),
        "hdr10plus": flag(hdr, "hdr10plus"),
        "hlg": flag(hdr, "hlg"),
        "dolby_vision": flag(hdr, "dolby_vision"),
        "interlaced": engine_profile.get("interlaced").and_then(Value::as_bool).unwrap_or(false),
        "format_level": engine_profile.get("format_level").cloned().unwrap_or(Value::Null),
    })
}

fn track_profile_to_plan(
    source: &Path,
    track: &Value,
    stream: &StreamRef,
    refs: &[StreamRef],
) -> Value {
    let mut plan = engine_profile_to_plan_profile(source, track, stream, refs);
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "hdr10plus".into(),
            json!(flag(track.get("hdr"), "hdr10plus")),
        );
    }
    plan
}

fn hdr_expectation(
    profile: &Value,
    hdr10plus: bool,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
) -> Value {
    let hdr = profile.get("hdr");
    let hdr_flag_or_profile =
        |key: &str| flag(hdr, key) || profile.get(key).and_then(Value::as_bool).unwrap_or(false);
    json!({
        "color_primaries": text_value(profile.get("color_primaries")),
        "transfer": text_value(profile.get("color_transfer").or_else(|| profile.get("transfer"))),
        "color_matrix": text_value(profile.get("color_matrix")),
        "master_display": text_value(profile.get("mastering_display").or_else(|| profile.get("master_display"))),
        "max_cll": profile.get("max_cll").and_then(Value::as_i64),
        "max_fall": profile.get("max_fall").and_then(Value::as_i64),
        "hdr10": hdr_flag_or_profile("hdr10"),
        "hdr10plus": hdr10plus,
        "hlg": hdr_flag_or_profile("hlg"),
        "dolby_vision": hdr_flag_or_profile("dolby_vision"),
        "dovi_profile": hdr.and_then(|h| h.get("dolby_vision_profile")).and_then(Value::as_i64)
            .or_else(|| profile.get("dovi_profile").and_then(Value::as_i64)),
        "dovi_rpu_present": flag(hdr, "dolby_vision_rpu")
            || profile.get("dovi_rpu_present").and_then(Value::as_bool).unwrap_or(false),
        "dovi_el_present": flag(hdr, "dolby_vision_enhancement_layer")
            || profile.get("dovi_el_present").and_then(Value::as_bool).unwrap_or(false),
        "dovi_compatibility_id": hdr
            .and_then(|h| h.get("dolby_vision_compatibility_id"))
            .and_then(Value::as_i64),
        "preserve_dolbyvision": preserve_dolbyvision,
        "strip_dolbyvision": strip_dolbyvision,
    })
}

fn choose_resource_plan(
    options: &RebuildRunOptions,
    profile: &Value,
    preset: &str,
    concurrent: i64,
) -> Result<ResourcePlan, String> {
    let host = crate::resource_plan::live_host();
    let settings = json!({
        "video": { "preset": preset },
        "resources": {
            "threading": options.resource_threading,
            "reserved_cpu_threads": options.reserved_cpu_threads,
            "max_worker_threads": options.max_worker_threads,
            "frame_threads": options.frame_threads,
            "min_free_memory_gib": options.min_free_memory_gib,
            "memory_reserve_percent": options.memory_reserve_percent,
        }
    });
    let forecast = memory_forecast(MemoryForecastInput {
        profile,
        settings: &settings,
        host: &host,
        full_memory_psi: crate::resource_plan::memory_full_avg10(),
        target_dimensions: None,
        preset_override: Some(preset),
        frame_threads_override: Some(options.frame_threads),
        concurrent_video_streams: concurrent.max(1),
    });
    if forecast.get("level").and_then(Value::as_str) == Some("blocked") {
        let reason = forecast
            .pointer("/messages/0")
            .and_then(Value::as_str)
            .unwrap_or("Available memory is below the safety reserve.");
        return Err(reason.to_string());
    }
    let cpu = host
        .pointer("/hardware/logical_cpus")
        .and_then(Value::as_i64)
        .unwrap_or(1)
        .max(1);
    Ok(ResourcePlan {
        worker_threads: forecast
            .get("planned_workers")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        frame_threads: forecast
            .get("planned_frame_threads")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        per_frame: forecast
            .get("per_frame_allowance_gib")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        per_worker: forecast
            .get("per_worker_allowance_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.25),
        budget: forecast.get("planning_budget_gib").and_then(Value::as_f64),
        memory_guard_gib: forecast
            .get("reserve_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        forecast,
        cpu_threads: cpu,
    })
}

fn record_memory_forecast(row: &mut Map<String, Value>, plan: &ResourcePlan) {
    let forecast = &plan.forecast;
    for (key, field) in [
        ("memory_forecast", "level"),
        ("memory_forecast_message", "messages"),
    ] {
        if let Some(value) = forecast.get(field) {
            if field == "messages" {
                let text = value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                row.insert(key.into(), json!(text));
            } else {
                row.insert(key.into(), value.clone());
            }
        }
    }
    for (key, field) in [
        ("memory_available_gib", "available_gib"),
        ("memory_total_gib", "total_gib"),
        ("memory_swap_used_percent", "swap_used_percent"),
        ("memory_budget_gib", "planning_budget_gib"),
        ("memory_worker_cap", "memory_worker_cap"),
        ("planned_workers", "planned_workers"),
        ("planned_frame_threads", "planned_frame_threads"),
    ] {
        if let Some(value) = forecast.get(field) {
            row.insert(key.into(), value.clone());
        }
    }
}

fn resolve_tools(options: &RebuildRunOptions) -> Result<Tools, String> {
    let host = crate::environment::inspect_environment(options.tool_paths.clone(), None)?;
    let mut missing = Vec::new();
    let mut need = |key: &str, label: &str| -> String {
        optional_tool(&host, key).unwrap_or_else(|| {
            missing.push(label.to_string());
            String::new()
        })
    };
    let tools = Tools {
        ffmpeg: need("ffmpeg", "FFmpeg"),
        ffprobe: need("ffprobe", "FFprobe"),
        mediainfo: need("mediainfo", "MediaInfo"),
        mkvmerge: need("mkvmerge", "MKVToolNix mkvmerge"),
        mkvpropedit: need("mkvpropedit", "MKVToolNix mkvpropedit"),
        mkvextract: need("mkvextract", "MKVToolNix mkvextract"),
        hdr10plus: optional_tool(&host, "hdr10plus_tool"),
        x265_pixel_formats: host
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        x265_dolbyvision: host
            .get("x265_dolbyvision")
            .or_else(|| host.get("dolby_vision_x265"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        x265_hdr10plus: host
            .get("x265_hdr10plus")
            .or_else(|| host.get("hdr10plus_x265"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    if !missing.is_empty() {
        return Err(format!(
            "Rebuild needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    if tools.x265_pixel_formats.is_empty() {
        return Err("Installed FFmpeg does not advertise libx265 pixel formats.".into());
    }
    Ok(tools)
}

#[allow(clippy::too_many_arguments)]
fn emit(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    output_bytes: Option<u64>,
    speed: Option<f64>,
    eta_seconds: Option<f64>,
) {
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(RebuildProgress {
        current: frame.number,
        total: frame.total,
        file_name: frame
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_path: frame.source.to_string_lossy().into_owned(),
        phase: phase.into(),
        file_percent,
        queue_percent: ((frame.number.saturating_sub(1)) as f64 + fraction)
            / frame.total.max(1) as f64
            * 100.0,
        elapsed_seconds: frame.started.elapsed().as_secs_f64(),
        speed,
        eta_seconds,
        input_bytes: frame.input_bytes,
        output_bytes,
        estimated_output_bytes: None,
    });
}

fn failure_fraction(phase: &str) -> f64 {
    match phase {
        "metadata embedding" => 0.90,
        "lossless output validation" | "output validation" => 0.97,
        "validated output install" | "transaction commit" => 0.99,
        _ => 0.0,
    }
}

pub(crate) fn closing_lines(job: &RebuildJobResult) -> Vec<String> {
    let total = job.files.len();
    let mode = if job.files.iter().any(|f| f.status == "TEST_OK") {
        "Short test"
    } else {
        "Full rebuild"
    };
    let summary = if job.stop_requested {
        format!(
            "{mode} stopped after {total} file{}; remaining files were not changed.",
            if total == 1 { "" } else { "s" }
        )
    } else if job.failed > 0 {
        format!(
            "{mode} finished: {} succeeded, {} failed safely.",
            job.succeeded, job.failed
        )
    } else if total == 0 {
        format!("{mode} had no files to process.")
    } else {
        format!(
            "{mode} finished: {} file{} succeeded.",
            job.succeeded,
            if job.succeeded == 1 { "" } else { "s" }
        )
    };
    let skipped = if job.skipped > 0 {
        format!(
            " {} already-processed file{} skipped.",
            job.skipped,
            if job.skipped == 1 { " was" } else { "s were" }
        )
    } else {
        String::new()
    };
    vec![
        format!("Log: {}", job.log_path.display()),
        format!("Run report: {}", job.report_path.display()),
        format!("{summary}{skipped}"),
    ]
}

pub(crate) fn run_rebuild(
    options: &RebuildRunOptions,
    mut on_progress: impl FnMut(RebuildProgress),
    mut on_line: impl FnMut(String),
    mut on_child: impl FnMut(Option<u32>),
) -> Result<RebuildJobResult, String> {
    let tools = resolve_tools(options)?;
    let files = if options.selected_files.is_empty() {
        discover_media_paths(
            &options.root,
            options.recursive,
            options.minimum_size_mib,
            &options.include,
            &options.exclude,
            options.stop_file.as_deref(),
        )?
    } else {
        let mut selected = Vec::new();
        for path in &options.selected_files {
            let path = path.canonicalize().map_err(|error| {
                format!("A selected media file is no longer available: {error}")
            })?;
            if !path.is_file() {
                return Err("Every selected media path must be a file.".into());
            }
            selected.push(path);
        }
        selected
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mode_tag = if options.info_only {
        "check"
    } else if options.test_mode {
        "test"
    } else {
        "rebuild"
    };
    let log_dir = crate::state_log_directory()?;
    fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the run log folder: {error}"))?;
    let log_path = log_dir.join(format!("hevc-rebuild-{stamp}-{mode_tag}.log"));
    let report_path = log_dir.join(format!("hevc-rebuild-{stamp}-{mode_tag}.csv"));
    let mut log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create the rebuild run log: {error}"))?;
    let mut note = |text: String, line: &mut dyn FnMut(String)| {
        let _ = writeln!(log, "{text}");
        line(text);
    };

    let discovered: BTreeSet<&PathBuf> = files.iter().collect();
    for (label, keys) in [
        (
            "Per-file output folders",
            options.file_output_dirs.keys().collect::<Vec<_>>(),
        ),
        (
            "Per-file poster selections",
            options.poster_overrides.keys().collect(),
        ),
        (
            "Reviewed metadata",
            options.metadata_overrides.keys().collect(),
        ),
        (
            "Per-file tune overrides",
            options.file_tune_overrides.keys().collect(),
        ),
        (
            "Per-file stream selections",
            options.file_stream_overrides.keys().collect(),
        ),
        (
            "Per-file run settings",
            options.file_settings_overrides.keys().collect(),
        ),
    ] {
        if let Some(stray) = keys.into_iter().find(|key| !discovered.contains(key)) {
            return Err(format!(
                "{label} do not belong to the selected media input: {}",
                stray.display()
            ));
        }
    }

    let mut eligible = Vec::new();
    let mut skipped = 0usize;
    for file in files {
        let backed_up = file
            .parent()
            .zip(file.file_name())
            .is_some_and(|(parent, name)| parent.join("old").join(name).exists());
        if backed_up && !options.reprocess && !options.test_mode && !options.info_only {
            skipped += 1;
        } else {
            eligible.push(file);
        }
    }
    if skipped > 0 {
        note(
            format!("Skipped {skipped} already-processed file(s) with an existing old/ backup"),
            &mut on_line,
        );
    }
    if eligible.is_empty() {
        note(
            "No eligible media files remain for this rebuild run.".into(),
            &mut on_line,
        );
        return Ok(RebuildJobResult {
            files: Vec::new(),
            log_path,
            report_path,
            stop_requested: false,
            failed: 0,
            succeeded: 0,
            skipped,
        });
    }
    if !options.info_only {
        validate_output_destinations(&eligible, options)?;
    }
    let total = eligible.len();
    let total_bytes: u64 = eligible
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    let mode = if options.info_only {
        "CHECK ONLY"
    } else if options.test_mode {
        "SHORT TESTS"
    } else {
        "FULL REBUILD"
    };
    note(
        format!(
            "{mode} · {total} file{} · {}",
            if total == 1 { "" } else { "s" },
            human_bytes(total_bytes)
        ),
        &mut on_line,
    );

    let mut context = Context {
        options,
        tools: &tools,
        progress: &mut on_progress,
        line: &mut on_line,
        child: &mut on_child,
    };
    let mut results = Vec::with_capacity(total);
    let mut stop_requested = false;
    let mut failed = 0usize;
    let mut succeeded = 0usize;
    for (offset, source) in eligible.iter().enumerate() {
        if options
            .stop_file
            .as_deref()
            .is_some_and(crate::stop_control::requested)
        {
            stop_requested = true;
            (context.line)(format!(
                "Stopped before file {}/{total}; remaining files were not changed.",
                offset + 1
            ));
            let _ = writeln!(log, "Stopped before file {}/{total}.", offset + 1);
            break;
        }
        let result = process_one(&mut context, source, offset + 1, total, &report_path);
        let _ = writeln!(
            log,
            "{}: {} · {}",
            source.display(),
            result.status,
            result.message
        );
        match result.status.as_str() {
            "OK" | "TEST_OK" | "INFO" => succeeded += 1,
            "SKIPPED" => skipped += 1,
            "INTERRUPTED" => stop_requested = true,
            _ => failed += 1,
        }
        let halt =
            result.status == "INTERRUPTED" || (result.status == "FAILED" && options.stop_on_error);
        results.push(result);
        if halt {
            break;
        }
    }
    let _ = log.flush();
    Ok(RebuildJobResult {
        files: results,
        log_path,
        report_path,
        stop_requested,
        failed,
        succeeded,
        skipped,
    })
}

fn process_one(
    context: &mut Context<'_>,
    source: &Path,
    number: usize,
    total: usize,
    report_path: &Path,
) -> RebuildFileResult {
    let started = Instant::now();
    let input_bytes = fs::metadata(source).ok().map(|meta| meta.len());
    let frame = FileFrame {
        source,
        number,
        total,
        started,
        input_bytes,
    };
    let mut state = FileState {
        phase: "media analysis",
        temp_output: None,
        final_output: None,
        row: Map::new(),
        interrupted: false,
    };
    state
        .row
        .insert("input".into(), json!(source.to_string_lossy()));
    state.row.insert("status".into(), json!("FAILED"));
    state.row.insert("message".into(), json!(""));
    state
        .row
        .insert("metadata_text_scrub_count".into(), json!(0));
    state
        .row
        .insert("metadata_text_scrub_details".into(), json!("[]"));
    (context.line)(format!(
        "Reading media tracks · {} ({number}/{total})",
        source.file_name().unwrap_or_default().to_string_lossy()
    ));
    let work_dir = make_work_dir(source, &context.options.work_root);
    let outcome = match &work_dir {
        Ok(work_dir) => execute(context, &frame, work_dir, &mut state),
        Err(error) => Err(error.clone()),
    };

    let mut failure_detail = None;
    match outcome {
        Ok(()) => {}
        Err(message) => {
            let status = if state.interrupted {
                "INTERRUPTED"
            } else if state.row.get("status").and_then(Value::as_str) == Some("SKIPPED")
                || state.row.get("status").and_then(Value::as_str) == Some("INFO")
            {
                state
                    .row
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("FAILED")
            } else {
                "FAILED"
            };
            if status == "FAILED" || status == "INTERRUPTED" {
                state.row.insert("status".into(), json!(status));
                state.row.insert(
                    "message".into(),
                    json!(if state.interrupted {
                        "Interrupted before validated output install; source was left untouched."
                            .to_string()
                    } else {
                        message.clone()
                    }),
                );
                failure_detail = Some(message.clone());
                emit(
                    context,
                    &frame,
                    if state.interrupted {
                        "Stopped safely"
                    } else {
                        "Failed safely"
                    },
                    failure_fraction(state.phase),
                    None,
                    state.temp_output.as_deref().and_then(file_size),
                    None,
                    None,
                );
                (context.line)(format!(
                    "File {number}/{total} {} during {}: {message}",
                    if state.interrupted {
                        "was interrupted"
                    } else {
                        "failed safely"
                    },
                    state.phase
                ));
            }
        }
    }
    release_temporary_output(
        &mut state,
        source,
        failure_detail.as_deref(),
        context.options,
    );
    let elapsed = started.elapsed().as_secs_f64();
    state.row.insert(
        "elapsed_seconds".into(),
        json!((elapsed * 1000.0).round() / 1000.0),
    );
    if let Err(error) = crate::reports::append_native_row(report_path, state.row.clone()) {
        (context.line)(format!("Could not write the run report row: {error}"));
    }
    if let Ok(work_dir) = work_dir {
        if context.options.keep_work {
            (context.line)(format!(
                "Rebuild work directory retained: {}",
                work_dir.display()
            ));
        } else {
            let _ = fs::remove_dir_all(&work_dir);
        }
    }
    RebuildFileResult {
        path: source.to_string_lossy().into_owned(),
        status: state
            .row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("FAILED")
            .into(),
        message: state
            .row
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        output: state
            .row
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_owned),
        elapsed_seconds: elapsed,
    }
}

fn execute(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    work_dir: &Path,
    state: &mut FileState,
) -> Result<(), String> {
    let (source, input_bytes) = (frame.source, frame.input_bytes);
    let tools = context.tools;
    let options = context.options;
    let effective = effective_for_file(options, source);
    emit(
        context,
        frame,
        "Reading media tracks",
        0.0,
        None,
        None,
        None,
        None,
    );

    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: effective.promote_8bit_to_10bit,
    })?;
    let probe = response
        .get("probe")
        .cloned()
        .ok_or("The profiler returned no stream probe.")?;
    let mediainfo = response
        .get("mediainfo")
        .cloned()
        .ok_or("The profiler returned no MediaInfo.")?;
    let mut profile = response
        .get("profile")
        .cloned()
        .ok_or("The profiler returned no profile.")?;
    let refs = stream_refs(&probe);
    let (selected_streams, encode_video_indices) = select_file_streams(&refs, options, &effective)?;
    let real_videos: Vec<StreamRef> = selected_streams
        .iter()
        .filter(|s| s.real_video())
        .cloned()
        .collect();
    let primary = real_videos
        .first()
        .cloned()
        .ok_or("Select at least one real video track for this media file.")?;

    let mut video_profiles: BTreeMap<i64, Value> = BTreeMap::new();
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .unwrap_or(primary.index);
    if primary_index == primary.index {
        video_profiles.insert(primary.index, profile.clone());
    }
    let others: Vec<i64> = real_videos
        .iter()
        .map(|stream| stream.index)
        .filter(|index| !video_profiles.contains_key(index))
        .collect();
    if !others.is_empty() {
        let tracks = engine_track_profiles(EngineTrackProfilesRequest {
            path: source.to_string_lossy().into_owned(),
            ffprobe: tools.ffprobe.clone(),
            promote_8bit_to_10bit: effective.promote_8bit_to_10bit,
            video_stream_indices: others.clone(),
            probe: probe.clone(),
            mediainfo: mediainfo.clone(),
        })?;
        let map = tracks
            .get("video_profiles")
            .and_then(Value::as_object)
            .ok_or("The selected-track profiler returned an invalid response.")?;
        for index in others {
            let variant = map.get(&index.to_string()).ok_or_else(|| {
                format!("The selected-track profiler omitted video stream #{index}.")
            })?;
            video_profiles.insert(index, variant.clone());
            if index == primary.index {
                profile = variant.clone();
            }
        }
    }
    video_profiles
        .entry(primary.index)
        .or_insert_with(|| profile.clone());

    let mut verified_hdr10plus: BTreeSet<i64> = BTreeSet::new();
    let mut hdr10plus_verification_skipped = false;
    let mut saw_hint = false;
    for stream in &real_videos {
        if !encode_video_indices.contains(&stream.index) {
            continue;
        }
        let track = &video_profiles[&stream.index];
        if !flag(track.get("hdr"), "hdr10plus_hint") {
            continue;
        }
        saw_hint = true;
        if options.info_only
            && matches!(
                stream.codec_name.to_ascii_lowercase().as_str(),
                "hevc" | "h265"
            )
            && tools.hdr10plus.is_none()
        {
            hdr10plus_verification_skipped = true;
            continue;
        }
        emit(
            context,
            frame,
            "Checking HDR10+ metadata",
            0.02,
            None,
            None,
            None,
            None,
        );
        let verified = match &tools.hdr10plus {
            Some(helper) => crate::hdr_metadata::verify_hdr10plus_source(
                &tools.ffmpeg,
                helper,
                source,
                stream.index,
                &stream.codec_name,
            )
            .unwrap_or(false),
            None => false,
        };
        if !options.info_only && effective.preserve_hdr10plus && !verified {
            let codec = stream.codec_name.to_ascii_lowercase();
            if !matches!(codec.as_str(), "hevc" | "h265") {
                return Err(format!(
                    "HDR10+ is indicated on video stream #{}, but its {codec} codec cannot use the frame-aligned preservation path.",
                    stream.index
                ));
            }
            return Err(format!(
                "HDR10+ is indicated on video stream #{}, but hdr10plus_tool could not verify it; refusing possible metadata loss.",
                stream.index
            ));
        }
        if verified {
            verified_hdr10plus.insert(stream.index);
            if let Some(object) = video_profiles
                .get_mut(&stream.index)
                .and_then(Value::as_object_mut)
            {
                if let Some(hdr) = object.get_mut("hdr").and_then(Value::as_object_mut) {
                    hdr.insert("hdr10plus".into(), json!(true));
                }
            }
        }
    }
    if !saw_hint {
        (context.line)("No HDR10+ signaling, so the full metadata scan was skipped.".into());
    }

    let file_tune = if effective.clear_file_tune {
        None
    } else {
        options.file_tune_overrides.get(source).map(String::as_str)
    };
    let reviewed_genres = options
        .metadata_overrides
        .get(source)
        .and_then(|fields| fields.get("genres"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let content_tune = crate::content::resolve_run_tune(
        &mediainfo,
        &effective.tune,
        options.detect_animation,
        file_tune,
        &reviewed_genres,
    )?;
    state.row.insert(
        "content_tune".into(),
        json!(content_tune.clone().unwrap_or_else(|| "normal".into())),
    );

    let mut preserve_dolbyvision_indices = BTreeSet::new();
    let mut strip_dolbyvision_indices = BTreeSet::new();
    let dovi_profiles: Vec<(i64, &Value)> = video_profiles
        .iter()
        .filter(|(_, track)| flag(track.get("hdr"), "dolby_vision"))
        .map(|(index, track)| (*index, track))
        .collect();
    if !dovi_profiles.is_empty() {
        emit(
            context,
            frame,
            "Checking Dolby Vision",
            0.04,
            None,
            None,
            None,
            None,
        );
        if effective.dolby_vision_policy == "skip" {
            state.row.insert("status".into(), json!("SKIPPED"));
            state
                .row
                .insert("dolby_vision_action".into(), json!("skip"));
            state.row.insert(
                "message".into(),
                json!(
                    "A selected video track contains Dolby Vision; skipped by configured policy."
                ),
            );
            emit(
                context,
                frame,
                "Skipped by Dolby Vision policy",
                1.0,
                Some(100.0),
                None,
                None,
                None,
            );
            return Ok(());
        }
        for (index, track) in &dovi_profiles {
            if effective.dolby_vision_policy == "hdr10-fallback" {
                let reasons = crate::hdr_metadata::hdr10_fallback_reasons_for_profile(track)?;
                if !reasons.is_empty() {
                    return Err(format!(
                        "Dolby Vision HDR10 fallback is unsafe for source stream #{index}: {}",
                        reasons.join("; ")
                    ));
                }
                strip_dolbyvision_indices.insert(*index);
            } else {
                let reasons = crate::hdr_metadata::safe_preservation_reasons_for_profile(
                    track,
                    tools.x265_dolbyvision,
                    options.force_deinterlace,
                )?;
                if !reasons.is_empty() {
                    let mut message = format!(
                        "Dolby Vision safe-preservation requirements not met on source stream #{index}: {}",
                        reasons.join("; ")
                    );
                    if crate::hdr_metadata::hdr10_fallback_reasons_for_profile(track)?.is_empty() {
                        message.push_str(
                            ". A verified HDR10 fallback is available using Dolby Vision policy hdr10-fallback.",
                        );
                    }
                    state.row.insert("status".into(), json!("SKIPPED"));
                    state.row.insert(
                        "dolby_vision_action".into(),
                        json!("skip-unsafe-preservation"),
                    );
                    state.row.insert("message".into(), json!(message));
                    emit(
                        context,
                        frame,
                        "Skipped: Dolby Vision cannot be safely preserved",
                        1.0,
                        Some(100.0),
                        None,
                        None,
                        None,
                    );
                    return Ok(());
                }
                preserve_dolbyvision_indices.insert(*index);
            }
        }
        if !strip_dolbyvision_indices.is_empty() {
            state
                .row
                .insert("dolby_vision_action".into(), json!("hdr10-fallback"));
        } else if !preserve_dolbyvision_indices.is_empty() {
            state
                .row
                .insert("dolby_vision_action".into(), json!("preserve"));
        }
    }
    let preserve_dolbyvision = !preserve_dolbyvision_indices.is_empty();
    let strip_dolbyvision = !strip_dolbyvision_indices.is_empty();

    let any_hdr10plus = !verified_hdr10plus.is_empty();
    let resolution = format!(
        "{}x{}",
        profile
            .pointer("/video/width")
            .and_then(Value::as_i64)
            .unwrap_or(primary.width),
        profile
            .pointer("/video/height")
            .and_then(Value::as_i64)
            .unwrap_or(primary.height)
    );
    for (key, value) in [
        (
            "primary_video",
            profile
                .pointer("/video/codec")
                .cloned()
                .unwrap_or_else(|| json!(primary.codec_name)),
        ),
        ("resolution", json!(resolution)),
        (
            "audio_count",
            json!(selected_streams
                .iter()
                .filter(|s| s.codec_type == "audio")
                .count()),
        ),
        (
            "subtitle_count",
            json!(selected_streams
                .iter()
                .filter(|s| s.codec_type == "subtitle")
                .count()),
        ),
        (
            "selected_stream_indices",
            json!(selected_streams
                .iter()
                .map(|s| s.index.to_string())
                .collect::<Vec<_>>()
                .join(",")),
        ),
        (
            "encoded_video_indices",
            json!(encode_video_indices
                .iter()
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(",")),
        ),
        (
            "hdr10",
            json!(video_profiles.values().any(|p| flag(p.get("hdr"), "hdr10"))),
        ),
        ("hdr10plus", json!(any_hdr10plus)),
        (
            "dolby_vision",
            json!(video_profiles
                .values()
                .any(|p| flag(p.get("hdr"), "dolby_vision"))),
        ),
        ("input_bytes", json!(input_bytes)),
    ] {
        state.row.insert(key.into(), value);
    }

    let planning_profile = video_profiles
        .values()
        .max_by_key(|track| {
            let width = track
                .pointer("/video/width")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let height = track
                .pointer("/video/height")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let depth = track
                .pointer("/video/source_bit_depth")
                .and_then(Value::as_i64)
                .unwrap_or(8);
            (width * height, depth)
        })
        .cloned()
        .unwrap_or(profile.clone());
    let info_plan = choose_resource_plan(
        options,
        &planning_profile,
        &effective.preset,
        encode_video_indices.len() as i64,
    );
    match &info_plan {
        Ok(plan) => record_memory_forecast(&mut state.row, plan),
        Err(error) => {
            state.row.insert("memory_forecast".into(), json!("blocked"));
            state
                .row
                .insert("memory_forecast_message".into(), json!(error));
        }
    }

    if options.info_only {
        state.row.insert("status".into(), json!("INFO"));
        let mut message = "Analysis only; no changes made.".to_string();
        if hdr10plus_verification_skipped {
            message.push_str(
                " HDR10+ dynamic metadata was not checked because hdr10plus_tool is missing; install the helper before rebuilding HEVC tracks.",
            );
        }
        if let Err(error) = &info_plan {
            message.push_str(&format!(
                " Memory forecast blocks encoding until available RAM recovers or the configured reserve is changed ({error})."
            ));
        }
        state.row.insert("message".into(), json!(message));
        emit(
            context,
            frame,
            "Analysis complete",
            1.0,
            Some(100.0),
            None,
            None,
            None,
        );
        return Ok(());
    }

    let (output_base, separate_output) = resolved_output_path(source, options)?;
    let final_output = if options.test_mode {
        test_output_path(&output_base)?
    } else {
        output_base
    };
    if final_output != source && final_output.exists() {
        return Err(format!(
            "Output already exists and will not be overwritten: {}",
            final_output.display()
        ));
    }
    let output_parent = final_output
        .parent()
        .ok_or("The planned output has no parent folder.")?
        .to_path_buf();
    fs::create_dir_all(&output_parent)
        .map_err(|error| format!("Could not create the output folder: {error}"))?;
    let stem = final_output
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let temp_output =
        output_parent.join(format!(".{stem}.hevc-rebuild.{}.tmp.mkv", unique_token()));
    state.final_output = Some(final_output.clone());
    state.temp_output = Some(temp_output.clone());
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    if separate_output {
        (context.line)(format!(
            "Validated output will be written to {}; source stays in place",
            final_output.parent().unwrap_or(&final_output).display()
        ));
    }

    state.phase = "storage preflight";
    emit(
        context,
        frame,
        "Checking free disk space",
        0.05,
        None,
        None,
        None,
        None,
    );
    let free = disk_free_bytes(&output_parent).ok_or_else(|| {
        format!(
            "Cannot determine free space for output folder {}",
            output_parent.display()
        )
    })?;
    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let (_estimated, recommended, conservative_ok) =
        storage_preflight(input_bytes.unwrap_or(0), free, duration, options)?;
    if !conservative_ok {
        (context.line)(format!(
            "Free space is below the conservative {} budget, but above the safe start floor. Live disk projection will stop before the reserve is consumed.",
            human_bytes(recommended)
        ));
    }

    let mut hdr10plus_jsons: BTreeMap<i64, PathBuf> = BTreeMap::new();
    // Prefer encode-time dhdr10-info when libx265 has HDR10_PLUS; otherwise keep
    // static HDR10 in the encode and inject SEI afterward with hdr10plus_tool.
    let mut hdr10plus_post_inject = false;
    if any_hdr10plus && effective.preserve_hdr10plus {
        state.phase = "HDR10+ metadata extraction";
        emit(
            context,
            frame,
            "Extracting HDR10+ metadata",
            0.06,
            None,
            None,
            None,
            None,
        );
        let helper = tools
            .hdr10plus
            .as_deref()
            .ok_or("HDR10+ preservation requires hdr10plus_tool.")?;
        let extract_limit = options.test_mode.then_some(options.test_seconds as f64);
        for index in &verified_hdr10plus {
            let track = &video_profiles[index];
            let codec = track
                .pointer("/video/codec")
                .and_then(Value::as_str)
                .unwrap_or("hevc");
            let metadata_json = work_dir.join(format!("hdr10plus-stream-{index}.json"));
            crate::hdr_metadata::extract_hdr10plus(
                &tools.ffmpeg,
                helper,
                source,
                *index,
                codec,
                &metadata_json,
                extract_limit,
            )?;
            hdr10plus_jsons.insert(*index, metadata_json);
        }
        match crate::hdr_metadata::hdr10plus_preservation_mode(tools.x265_hdr10plus) {
            "encode-time" => {
                (context.line)(
                    "HDR10+ will be preserved during encode (libx265 dhdr10-info).".into(),
                );
            }
            _ => {
                hdr10plus_post_inject = true;
                (context.line)(
                    "HDR10+ will be preserved after encode with hdr10plus_tool inject (host libx265 lacks HDR10_PLUS)."
                        .into(),
                );
            }
        }
    } else if any_hdr10plus {
        (context.line)(
            "HDR10+ source detected, but preservation is disabled; selected video tracks will not carry dynamic metadata into the re-encode.".into(),
        );
    }
    let encode_hdr10plus_jsons: BTreeMap<i64, PathBuf> = if hdr10plus_post_inject {
        BTreeMap::new()
    } else {
        hdr10plus_jsons.clone()
    };

    let encode_crf = if options.test_mode {
        effective.test_crf
    } else {
        effective.crf
    };
    let expected_duration = if options.test_mode {
        duration.min(options.test_seconds as f64)
    } else {
        duration
    };
    emit(
        context,
        frame,
        "Planning encoder threads and memory",
        0.07,
        None,
        None,
        None,
        None,
    );
    let mut plan = info_plan?;
    maybe_calibrate(
        context,
        frame,
        state,
        source,
        &mut plan,
        &profile,
        &primary,
        &planning_profile,
        &effective,
        encode_crf,
        content_tune.as_deref(),
        encode_hdr10plus_jsons.get(&primary.index),
        preserve_dolbyvision,
        strip_dolbyvision,
        encode_video_indices.len(),
        input_bytes.unwrap_or(0),
    )?;
    record_memory_forecast(&mut state.row, &plan);

    state.phase = "video encode";
    emit(
        context,
        frame,
        "Starting the video encode",
        0.08,
        None,
        None,
        None,
        None,
    );
    let plan_profile = engine_profile_to_plan_profile(source, &profile, &primary, &refs);
    let mut plan_video_profiles = Map::new();
    for stream in &real_videos {
        let track = &video_profiles[&stream.index];
        plan_video_profiles.insert(
            stream.index.to_string(),
            track_profile_to_plan(source, track, stream, &refs),
        );
    }
    let mut attempt = 1u32;
    loop {
        let encode_request = json!({
            "tools": {
                "ffmpeg": tools.ffmpeg,
                "x265_pixel_formats": tools.x265_pixel_formats,
                "x265_dolbyvision": tools.x265_dolbyvision,
            },
            "profile": plan_profile,
            "video_profiles": plan_video_profiles,
            "selected_streams": selected_streams.iter().map(StreamRef::to_plan_value).collect::<Vec<_>>(),
            "encode_video_indices": encode_video_indices.iter().copied().collect::<Vec<_>>(),
            "hdr10plus_jsons": encode_hdr10plus_jsons.iter().map(|(index, path)| (index.to_string(), json!(path))).collect::<Map<_, _>>(),
            "preserve_dolbyvision_indices": preserve_dolbyvision_indices.iter().copied().collect::<Vec<_>>(),
            "strip_dolbyvision_indices": strip_dolbyvision_indices.iter().copied().collect::<Vec<_>>(),
            "resource_plan": {
                "worker_threads": plan.worker_threads,
                "frame_threads": plan.frame_threads,
            },
            "options": {
                "output": temp_output,
                "crf": encode_crf,
                "preset": effective.preset,
                "tune": content_tune.clone().unwrap_or_default(),
                "preserve_dolbyvision": preserve_dolbyvision,
                "strip_dolbyvision": strip_dolbyvision,
                "test_seconds": options.test_mode.then_some(options.test_seconds),
                "preserve_source_level": options.preserve_source_level,
                "disable_hdr10_opt": options.disable_hdr10_opt,
                "secondary_video_policy": options.secondary_video,
                "data_policy": options.data_streams,
                "force_deinterlace": options.force_deinterlace,
                "disable_deinterlace": options.disable_deinterlace,
            }
        });
        let planned = crate::encode_plan::build_plan(&encode_request)?;
        let args = planned
            .get("args")
            .and_then(Value::as_array)
            .ok_or("The encode planner returned no FFmpeg arguments.")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        (context.line)(format!(
            "[{}/{}] Encoding {} (attempt {attempt}; workers={} frame_threads={})",
            frame.number,
            frame.total,
            source.file_name().unwrap_or_default().to_string_lossy(),
            plan.worker_threads,
            plan.frame_threads
        ));
        let disk_guard_bytes = (options.min_free_disk_gib.max(0.0) * GIB as f64) as u64;
        let spec = EncodeSpec {
            args: &args,
            expected_duration,
            output_path: &temp_output,
            memory_guard_gib: plan.memory_guard_gib,
            disk_guard_bytes,
            projection_margin_percent: options.disk_projection_margin_percent,
            stop_file: options.stop_file.as_deref(),
        };
        let child_cb = &mut *context.child;
        let progress_cb = &mut *context.progress;
        let number = frame.number;
        let total = frame.total;
        let started = frame.started;
        let input_bytes = frame.input_bytes;
        let file_name = frame
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_path = frame.source.to_string_lossy().into_owned();
        match run_encode(
            &spec,
            |progress: &EncodeProgress| {
                let fraction = 0.08 + 0.80 * (progress.file_percent.unwrap_or(0.0) / 100.0);
                let clamped = fraction.clamp(0.0, 1.0);
                progress_cb(RebuildProgress {
                    current: number,
                    total,
                    file_name: file_name.clone(),
                    file_path: file_path.clone(),
                    phase: "Encoding video".into(),
                    file_percent: progress.file_percent,
                    queue_percent: ((number.saturating_sub(1)) as f64 + clamped)
                        / total.max(1) as f64
                        * 100.0,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                    speed: progress.speed,
                    eta_seconds: progress.eta_seconds,
                    input_bytes,
                    output_bytes: Some(progress.output_bytes),
                    estimated_output_bytes: None,
                });
            },
            child_cb,
        ) {
            Ok(resources) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                break;
            }
            Err(EncodeFailure::Interrupted { resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                state.interrupted = true;
                return Err("Encode interrupted.".into());
            }
            Err(EncodeFailure::Failed { message, resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                return Err(message);
            }
            Err(EncodeFailure::MemoryPressure { reason, resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                let Some(next) = plan.reduced() else {
                    return Err(format!(
                        "Memory pressure persisted after {attempt} safe encode attempt(s) ({reason}). Close memory-heavy programs or lower max worker/frame threads. The source file was not changed."
                    ));
                };
                if attempt >= MAX_ENCODE_ATTEMPTS {
                    return Err(format!(
                        "Memory pressure persisted after {attempt} safe encode attempt(s) ({reason}). The source file was not changed."
                    ));
                }
                if temp_output.exists() {
                    let _ = fs::remove_file(&temp_output);
                }
                (context.line)(format!(
                    "Memory pressure detected; retrying with {} worker / {} frame thread(s). Quality settings are unchanged.",
                    next.worker_threads, next.frame_threads
                ));
                emit(
                    context,
                    frame,
                    "Retrying the encode with fewer threads",
                    0.08,
                    None,
                    None,
                    None,
                    None,
                );
                plan = next;
                attempt += 1;
            }
        }
    }

    if hdr10plus_post_inject {
        state.phase = "HDR10+ metadata inject";
        emit(
            context,
            frame,
            "Injecting HDR10+ metadata",
            0.88,
            None,
            file_size(&temp_output),
            None,
            None,
        );
        let helper = tools
            .hdr10plus
            .as_deref()
            .ok_or("HDR10+ post-inject requires hdr10plus_tool.")?;
        // Encoded Matroska video ordinals follow encode_video_indices order.
        // Inject every track that has JSON; remux keeps non-injected videos too.
        let video_jsons: Vec<(usize, Option<&Path>)> = encode_video_indices
            .iter()
            .enumerate()
            .map(|(ordinal, stream_index)| {
                (
                    ordinal,
                    hdr10plus_jsons.get(stream_index).map(PathBuf::as_path),
                )
            })
            .collect();
        crate::hdr_metadata::apply_hdr10plus_injections_to_matroska(
            &tools.ffmpeg,
            &tools.mkvmerge,
            helper,
            &temp_output,
            &video_jsons,
            work_dir,
        )?;
        for stream_index in hdr10plus_jsons.keys() {
            if encode_video_indices.contains(stream_index) {
                (context.line)(format!(
                    "Injected HDR10+ dynamic metadata into encoded video stream #{stream_index}."
                ));
            }
        }
    }

    state.phase = "metadata embedding";
    emit(
        context,
        frame,
        "Embedding metadata and poster",
        0.90,
        None,
        file_size(&temp_output),
        None,
        None,
    );
    let applied = crate::matroska_apply::apply_metadata_value(json!({
        "media_path": source,
        "output": temp_output,
        "work_dir": work_dir,
        "ffmpeg": tools.ffmpeg,
        "tools": {
            "mkvmerge": tools.mkvmerge,
            "mkvextract": tools.mkvextract,
            "mkvpropedit": tools.mkvpropedit,
        },
        "comment": options.comment,
        "text_blacklist": options.text_blacklist,
        "metadata_override": options.metadata_overrides.get(source),
        "poster_override": options.poster_overrides.get(source),
        "embed_artwork": options.embed_artwork,
    }))?;
    let scrubbed = applied
        .get("scrubbed_text_values")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let scrub_count = scrubbed.as_array().map_or(0, Vec::len);
    state
        .row
        .insert("metadata_text_scrub_count".into(), json!(scrub_count));
    state.row.insert(
        "metadata_text_scrub_details".into(),
        json!(serde_json::to_string(&scrubbed).unwrap_or_else(|_| "[]".into())),
    );
    let scrub_note = if scrub_count > 0 {
        format!(" Metadata text cleaned in {scrub_count} field(s).")
    } else {
        String::new()
    };
    let expect_poster = applied
        .get("poster_path")
        .is_some_and(|poster| !poster.is_null());

    state.phase = "output validation";
    emit(
        context,
        frame,
        "Validating video, streams, metadata, and HDR",
        0.97,
        None,
        file_size(&temp_output),
        None,
        None,
    );
    let mut targets = Map::new();
    let mut expectations = Map::new();
    for stream in &real_videos {
        if !encode_video_indices.contains(&stream.index) {
            continue;
        }
        let track = &video_profiles[&stream.index];
        if let Some(format) = track
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
        {
            targets.insert(stream.index.to_string(), json!(format));
        }
        expectations.insert(
            stream.index.to_string(),
            hdr_expectation(
                track,
                effective.preserve_hdr10plus && verified_hdr10plus.contains(&stream.index),
                preserve_dolbyvision_indices.contains(&stream.index),
                strip_dolbyvision_indices.contains(&stream.index),
            ),
        );
    }
    let validation = crate::validation::validate_streams_value(json!({
        "ffmpeg": tools.ffmpeg,
        "ffprobe": tools.ffprobe,
        "mediainfo": tools.mediainfo,
        "hdr10plus_tool": tools.hdr10plus,
        "source_path": source,
        "output_path": temp_output,
        "source_probe": probe,
        "selected_stream_indices": selected_streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        "encode_video_indices": encode_video_indices.iter().copied().collect::<Vec<_>>(),
        "video_target_formats": targets,
        "video_hdr_expectations": expectations,
        "verify_copy_hashes": options.verify_copy_hashes,
        "test_mode": options.test_mode,
        "copy_only": false,
        "expect_poster": expect_poster,
        "source_duration": expected_duration,
        "source_chapters_count": profile.get("chapters_count").and_then(Value::as_u64).unwrap_or(0),
        "scrubbed_text_values": scrubbed,
        "chapter_text_edits": applied.get("chapter_text_edits").cloned().unwrap_or_else(|| json!([])),
    }))?;
    let _ = validation;
    let expected_tags: Vec<(String, String)> = applied
        .get("expected_global_tags")
        .and_then(Value::as_object)
        .map(|tags| {
            tags.iter()
                .filter_map(|(name, value)| value.as_str().map(|v| (name.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if expect_poster || !expected_tags.is_empty() {
        crate::matroska_verify::verify_expected_output(
            &crate::matroska_scrub::MatroskaTools {
                mkvmerge: tools.mkvmerge.clone(),
                mkvextract: tools.mkvextract.clone(),
                mkvpropedit: tools.mkvpropedit.clone(),
            },
            &temp_output,
            work_dir,
            expect_poster,
            &expected_tags,
        )
        .map_err(|error| match error {
            crate::matroska_verify::VerifyError::Mismatch(text)
            | crate::matroska_verify::VerifyError::Operational(text) => text,
        })?;
    }

    if options.test_mode {
        state.phase = "validated output install";
        install_export_output(&temp_output, &final_output)?;
        state.temp_output = None;
        let final_bytes = file_size(&final_output);
        state.row.insert("status".into(), json!("TEST_OK"));
        state
            .row
            .insert("output".into(), json!(final_output.to_string_lossy()));
        state.row.insert("output_bytes".into(), json!(final_bytes));
        state.row.insert(
            "message".into(),
            json!(format!(
                "Short test encode validated; source untouched.{scrub_note}"
            )),
        );
        emit(
            context,
            frame,
            "Short test validated",
            1.0,
            Some(100.0),
            final_bytes,
            None,
            None,
        );
        (context.line)(format!(
            "Validated test ready: {}",
            final_output
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        return Ok(());
    }

    state.phase = "transaction commit";
    if separate_output {
        install_export_output(&temp_output, &final_output)?;
        state.temp_output = None;
        let final_bytes = file_size(&final_output);
        state.row.insert("status".into(), json!("OK"));
        state
            .row
            .insert("output".into(), json!(final_output.to_string_lossy()));
        state.row.insert("output_bytes".into(), json!(final_bytes));
        state.row.insert(
            "message".into(),
            json!(format!(
                "Validated copy saved; original left in place at {}.{scrub_note}",
                source.display()
            )),
        );
        emit(
            context,
            frame,
            "Validated output installed",
            1.0,
            Some(100.0),
            final_bytes,
            None,
            None,
        );
        (context.line)(format!("Validated copy saved: {}", final_output.display()));
        return Ok(());
    }

    let backup = backup_path_for(source, options.reprocess)?;
    commit_output(source, &temp_output, &final_output, &backup)?;
    state.temp_output = None;
    let final_bytes = file_size(&final_output);
    state.row.insert("status".into(), json!("OK"));
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    state.row.insert("output_bytes".into(), json!(final_bytes));
    state.row.insert(
        "message".into(),
        json!(format!(
            "Validated and committed; original moved to {}.{scrub_note}",
            backup.display()
        )),
    );
    emit(
        context,
        frame,
        "Validated output installed",
        1.0,
        Some(100.0),
        final_bytes,
        None,
        None,
    );
    (context.line)(format!(
        "Validated rebuild complete; original preserved in {}/",
        backup
            .parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn maybe_calibrate(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    source: &Path,
    plan: &mut ResourcePlan,
    profile: &Value,
    primary: &StreamRef,
    planning_profile: &Value,
    effective: &EffectiveSettings,
    encode_crf: i64,
    content_tune: Option<&str>,
    primary_hdr_json: Option<&PathBuf>,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    encode_count: usize,
    source_size: u64,
) -> Result<(), String> {
    let options = context.options;
    let tools = context.tools;
    if options.calibration_mode == "off" || options.resource_threading != "auto" {
        return Ok(());
    }
    let width = planning_profile
        .pointer("/video/width")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let height = planning_profile
        .pointer("/video/height")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let eligible =
        crate::calibration::calibration_eligible(crate::calibration::EligibilityRequest {
            width,
            height,
            source_size_bytes: source_size,
            large_file_gib: options.calibration_large_file_gib,
        })?;
    let demanding = eligible
        .get("demanding")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let due = options.calibration_mode == "always"
        || (matches!(options.calibration_mode.as_str(), "auto" | "ask") && demanding);
    if !due {
        return Ok(());
    }
    if encode_count > 1 {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("A multi-video job is planned as a concurrent aggregate; single-stream calibration would not represent its memory use."),
        );
        (context.line)(
            "Hardware calibration skipped for multiple selected video encoders; the aggregate memory plan and live pressure guards remain active.".into(),
        );
        return Ok(());
    }
    let available = plan.forecast.get("available_gib").and_then(Value::as_f64);
    let total_memory = plan.forecast.get("total_gib").and_then(Value::as_f64);
    if available.is_none() {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("Available-memory telemetry is unavailable."),
        );
        return Ok(());
    }
    if available.unwrap_or(0.0) < plan.memory_guard_gib {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!(format!(
                "Available RAM is below the configured {:.1} GiB reserve.",
                plan.memory_guard_gib
            )),
        );
        return Ok(());
    }
    // GUI / non-interactive ask mode skips interactive approval.
    if options.calibration_mode == "ask" {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("No interactive terminal was available."),
        );
        return Ok(());
    }

    let cal_profile = crate::calibration::CalibrationProfile {
        source_path: source.to_string_lossy().into_owned(),
        duration: profile.get("duration_seconds").and_then(Value::as_f64),
        primary_video_index: u32::try_from(primary.index).unwrap_or(0),
        primary_video_width: primary.width.max(0) as u32,
        primary_video_height: primary.height.max(0) as u32,
        source_bit_depth: planning_profile
            .pointer("/video/source_bit_depth")
            .and_then(Value::as_i64)
            .unwrap_or(8),
        source_chroma: planning_profile
            .pointer("/video/source_chroma")
            .and_then(Value::as_str)
            .unwrap_or("4:2:0")
            .to_string(),
        target_pix_fmt: planning_profile
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
            .unwrap_or("yuv420p10le")
            .to_string(),
        format_level: planning_profile
            .get("format_level")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_primaries: planning_profile
            .get("color_primaries")
            .and_then(Value::as_str)
            .map(str::to_owned),
        transfer: planning_profile
            .get("color_transfer")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_matrix: planning_profile
            .get("color_matrix")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_range_x265: planning_profile
            .get("color_range_x265")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_range_ffmpeg: planning_profile
            .get("color_range_ffmpeg")
            .and_then(Value::as_str)
            .map(str::to_owned),
        chroma_location: planning_profile
            .get("chroma_location")
            .and_then(Value::as_i64),
        master_display: planning_profile
            .get("mastering_display")
            .and_then(Value::as_str)
            .map(str::to_owned),
        max_cll: planning_profile.get("max_cll").and_then(Value::as_i64),
        max_fall: planning_profile.get("max_fall").and_then(Value::as_i64),
        hdr10: flag(planning_profile.get("hdr"), "hdr10"),
        hdr10plus: primary_hdr_json.is_some(),
        hlg: flag(planning_profile.get("hdr"), "hlg"),
        dolby_vision: flag(planning_profile.get("hdr"), "dolby_vision"),
        dovi_profile: planning_profile
            .pointer("/hdr/dolby_vision_profile")
            .and_then(Value::as_i64),
        dovi_rpu_present: flag(planning_profile.get("hdr"), "dolby_vision_rpu"),
        interlaced: planning_profile
            .get("interlaced")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let toolchain = crate::calibration::CalibrationToolchain {
        ffmpeg: tools.ffmpeg.clone(),
        x265_pixel_formats: tools.x265_pixel_formats.clone(),
        x265_dolbyvision: tools.x265_dolbyvision,
    };
    let cal_options = crate::calibration::CalibrationOptions {
        preset: effective.preset.clone(),
        tune: content_tune.map(str::to_owned),
        crf: encode_crf,
        sample_seconds: options.calibration_sample_seconds,
        hdr10plus_json: primary_hdr_json.map(|path| path.to_string_lossy().into_owned()),
        preserve_dolbyvision,
        strip_dolbyvision,
        preserve_source_level: options.preserve_source_level,
        disable_hdr10_opt: options.disable_hdr10_opt,
        force_deinterlace: options.force_deinterlace,
        disable_deinterlace: options.disable_deinterlace,
    };
    let base_plan = crate::calibration::CalibrationPlanKey {
        worker_threads: Some(plan.worker_threads),
        frame_threads: Some(plan.frame_threads),
        cpu_threads: plan.cpu_threads,
        memory_guard_gib: plan.memory_guard_gib,
    };
    let key_response = crate::calibration::calibration_key(crate::calibration::KeyRequest {
        profile: cal_profile.clone(),
        toolchain: toolchain.clone(),
        options: cal_options.clone(),
        base_plan: base_plan.clone(),
        total_memory_gib: total_memory,
        max_candidates: options.calibration_max_candidates,
        throughput_tolerance_percent: options.calibration_tolerance_percent,
        hdr10plus_enabled: primary_hdr_json.is_some(),
        encoder_fingerprint: None,
    })?;
    let key = key_response
        .get("key")
        .and_then(Value::as_str)
        .ok_or("Calibration key helper returned no key.")?
        .to_string();

    let mut selection = None;
    if options.calibration_mode != "always" {
        if let Ok(cached) = crate::calibration::cache_query(crate::calibration::CacheQuery {
            key: key.clone(),
            cache_hours: options.calibration_cache_hours,
            base_worker_threads: Some(plan.worker_threads),
            base_frame_threads: Some(plan.frame_threads),
        }) {
            if cached.get("hit").and_then(Value::as_bool) == Some(true) {
                selection = Some(cached);
            }
        }
    }
    if selection.is_none() {
        state.phase = "hardware calibration";
        emit(
            context,
            frame,
            "Calibrating the encoder",
            0.08,
            None,
            None,
            None,
            None,
        );
        (context.line)(format!(
            "Calibrating x265 thread layout: up to {} candidates × {}s of representative video.",
            options.calibration_max_candidates, options.calibration_sample_seconds
        ));
        match crate::calibration::run_calibration(crate::calibration::RunRequest {
            key: key.clone(),
            max_candidates: options.calibration_max_candidates,
            throughput_tolerance_percent: options.calibration_tolerance_percent,
            memory_guard_gib: plan.memory_guard_gib,
            profile: cal_profile,
            toolchain,
            options: cal_options,
            base_plan,
        }) {
            Ok(result) => {
                if result.get("source").and_then(Value::as_str) == Some("benchmark") {
                    if let (Some(workers), Some(frames), Some(results)) = (
                        result.get("worker_threads").and_then(Value::as_i64),
                        result.get("frame_threads").and_then(Value::as_i64),
                        result.get("results").cloned(),
                    ) {
                        let parsed: Vec<crate::calibration::CalibrationResult> =
                            serde_json::from_value(results).unwrap_or_default();
                        let _ =
                            crate::calibration::store_selection(crate::calibration::StoreRequest {
                                key: key.clone(),
                                worker_threads: workers,
                                frame_threads: frames,
                                results: parsed,
                            });
                    }
                }
                selection = Some(result);
            }
            Err(error) => {
                (context.line)(format!(
                    "Hardware calibration failed; continuing with the normal resource plan: {error}"
                ));
                state
                    .row
                    .insert("calibration_source".into(), json!("fallback"));
                state.row.insert(
                    "calibration_note".into(),
                    json!("Normal resource plan retained after calibration could not run."),
                );
            }
        }
    }

    if let Some(selection) = selection {
        let workers = selection
            .get("worker_threads")
            .or_else(|| selection.pointer("/plan/worker_threads"))
            .and_then(Value::as_i64);
        let frames = selection
            .get("frame_threads")
            .or_else(|| selection.pointer("/plan/frame_threads"))
            .and_then(Value::as_i64);
        if let (Some(workers), Some(frames)) = (workers, frames) {
            plan.worker_threads = workers.max(1);
            plan.frame_threads = frames.max(1);
            let source_label = selection
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("cache");
            state
                .row
                .insert("calibration_source".into(), json!(source_label));
            state
                .row
                .insert("calibration_workers".into(), json!(plan.worker_threads));
            state.row.insert(
                "calibration_frame_threads".into(),
                json!(plan.frame_threads),
            );
            let note = selection
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Selected by hardware calibration.");
            state.row.insert("calibration_note".into(), json!(note));
            (context.line)(format!(
                "Using hardware calibration ({source_label}): {} workers / {} frame thread(s).",
                plan.worker_threads, plan.frame_threads
            ));
        }
    }
    Ok(())
}

fn release_temporary_output(
    state: &mut FileState,
    source: &Path,
    failure_detail: Option<&str>,
    options: &RebuildRunOptions,
) {
    let status = state
        .row
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("FAILED")
        .to_string();
    let succeeded = matches!(status.as_str(), "OK" | "TEST_OK" | "INFO" | "SKIPPED");
    let late = FINISHED_OUTPUT_PHASES.contains(&state.phase) && !succeeded;
    if !late {
        if let Some(temp) = state.temp_output.take() {
            if !options.keep_work && temp.exists() {
                let _ = fs::remove_file(temp);
            }
        }
        return;
    }
    let mut retained: Option<PathBuf> = None;
    if let Some(temp) = state.temp_output.clone().filter(|path| path.exists()) {
        retained = keep_finished_output(&temp, state.final_output.as_deref()).ok();
        if retained.is_none() {
            retained = Some(temp);
        }
    } else if let Some(final_output) = state.final_output.clone() {
        if fs::symlink_metadata(&final_output).is_ok() && final_output != source {
            retained = keep_finished_output(&final_output, Some(&final_output))
                .ok()
                .or(Some(final_output));
        }
    }
    if let Some(path) = &retained {
        state
            .row
            .insert("output".into(), json!(path.to_string_lossy()));
        if let Some(bytes) = file_size(path) {
            state.row.insert("output_bytes".into(), json!(bytes));
        }
    }
    let intended = state.final_output.clone().or_else(|| retained.clone());
    if let Some(intended) = intended {
        let log_target = retained.clone().unwrap_or_else(|| intended.clone());
        let log = write_retained_error_log(
            &log_target.with_extension("log"),
            source,
            &intended,
            retained.as_deref(),
            state.phase,
            &status,
            state
                .row
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(""),
            failure_detail,
        );
        let message = state
            .row
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let note = match (&retained, &log) {
            (Some(path), Some(log)) => format!(
                " The completed output was retained at {}, but remains unvalidated; diagnostics: {}.",
                path.display(),
                log.display()
            ),
            (Some(path), None) => format!(
                " The completed output was retained at {}, but remains unvalidated.",
                path.display()
            ),
            _ => String::new(),
        };
        state
            .row
            .insert("message".into(), json!(format!("{message}{note}")));
    }
    state.temp_output = None;
}

fn keep_finished_output(temp: &Path, final_output: Option<&Path>) -> Result<PathBuf, String> {
    if let Ok(file) = fs::File::open(temp) {
        let _ = file.sync_all();
    }
    let parent = final_output
        .and_then(Path::parent)
        .or_else(|| temp.parent())
        .ok_or("The retained output has no parent folder.")?
        .to_path_buf();
    let stem = final_output
        .and_then(Path::file_stem)
        .or_else(|| temp.file_stem())
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    let suffix = final_output.and_then(Path::extension).map_or_else(
        || "mkv".to_string(),
        |value| value.to_string_lossy().into_owned(),
    );
    let mut candidates = vec![parent.join(format!("{stem}.error.{suffix}"))];
    candidates
        .extend((1..10_000).map(|number| parent.join(format!("{stem}.error.{number}.{suffix}"))));
    for candidate in candidates {
        if fs::symlink_metadata(&candidate).is_ok()
            || fs::symlink_metadata(candidate.with_extension("log")).is_ok()
        {
            continue;
        }
        match rename_no_replace(temp, &candidate) {
            Ok(()) => {
                let _ = sync_directory(&parent);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not retain the finished output: {error}")),
        }
    }
    Err(format!(
        "Unable to retain completed output without overwriting an existing file: {}",
        parent.display()
    ))
}

#[allow(clippy::too_many_arguments)]
fn write_retained_error_log(
    path: &Path,
    source: &Path,
    intended: &Path,
    retained: Option<&Path>,
    phase: &str,
    status: &str,
    message: &str,
    failure_detail: Option<&str>,
) -> Option<PathBuf> {
    let contents = [
        "HEVC Rebuild retained-output diagnostic".to_string(),
        format!("Phase: {phase}"),
        format!("Status: {status}"),
        format!("Source: {}", source.display()),
        format!("Intended output: {}", intended.display()),
        format!(
            "Retained output: {}",
            retained.map_or_else(|| "not available".to_string(), |path| path.display().to_string())
        ),
        format!(
            "Error: {}",
            failure_detail
                .filter(|text| !text.is_empty())
                .unwrap_or(if message.is_empty() {
                    "unknown failure"
                } else {
                    message
                })
        ),
        format!("Message: {}", if message.is_empty() { "none" } else { message }),
        "Outcome: the completed output was retained for investigation but was not accepted as a successful rebuild.".into(),
        String::new(),
    ]
    .join("\n");
    for number in 0..10_000u32 {
        let candidate = if number == 0 {
            path.to_path_buf()
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let extension = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            path.with_file_name(format!("{stem}.{number}{extension}"))
        };
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        match open.open(&candidate) {
            Ok(mut file) => {
                return (file.write_all(contents.as_bytes()).is_ok() && file.sync_all().is_ok())
                    .then(|| {
                        if let Some(parent) = candidate.parent() {
                            let _ = sync_directory(parent);
                        }
                        candidate
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args(extra: &[&str]) -> Vec<String> {
        let mut args = vec![
            std::env::temp_dir().to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            "5".into(),
            "--no-dependency-repair".into(),
            "--skip-update-check".into(),
            "--crf".into(),
            "22".into(),
            "--preset".into(),
            "slow".into(),
            "--tune".into(),
            "auto".into(),
            "--detect-animation".into(),
            "--preserve-hdr10plus".into(),
            "--dolby-vision-policy".into(),
            "safe".into(),
            "--secondary-video".into(),
            "encode".into(),
            "--data-streams".into(),
            "preserve".into(),
            "--verify-copy-hashes".into(),
            "--resource-threading".into(),
            "auto".into(),
            "--reserved-cpu-threads".into(),
            "1".into(),
            "--max-worker-threads".into(),
            "0".into(),
            "--frame-threads".into(),
            "0".into(),
            "--min-free-memory-gib".into(),
            "3".into(),
            "--memory-reserve-percent".into(),
            "8".into(),
            "--min-free-disk-gib".into(),
            "0".into(),
            "--preflight-output-ratio".into(),
            "1.25".into(),
            "--disk-projection-margin-percent".into(),
            "15".into(),
            "--calibration-mode".into(),
            "auto".into(),
            "--calibration-cache-hours".into(),
            "168".into(),
            "--calibration-large-file-gib".into(),
            "20".into(),
            "--calibration-sample-seconds".into(),
            "4".into(),
            "--calibration-max-candidates".into(),
            "3".into(),
            "--calibration-tolerance-percent".into(),
            "5".into(),
            "--comment".into(),
            "".into(),
            "--embed-artwork".into(),
            "--replace-metadata-text-blacklist".into(),
        ];
        args.extend(extra.iter().map(|value| (*value).to_string()));
        args
    }

    #[test]
    fn parses_short_test_mode() {
        let options = RebuildRunOptions::from_engine_args(&base_args(&["--test"])).unwrap();
        assert!(options.test_mode);
        assert!(!options.info_only);
    }

    #[test]
    fn rejects_youtube_flags() {
        assert!(RebuildRunOptions::from_engine_args(&base_args(&["--youtube-upload"])).is_err());
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(RebuildRunOptions::from_engine_args(&base_args(&["--surprise"])).is_err());
    }
}
