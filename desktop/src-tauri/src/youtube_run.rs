//! ./desktop/src-tauri/src/youtube_run.rs
//! Native YouTube upload-copy coordinator (T4.5). Rust owns the whole job with
//! no Python process: discovery, filename disambiguation, source profiling,
//! HDR/Dolby Vision policy and per-file HDR10+ loss consent, bitrate and size
//! estimates, storage preflight, memory-aware encode planning with adaptive
//! retries, supervised FFmpeg encodes under the live resource guard, output
//! validation, no-overwrite atomic install, and CSV reports.
//!
//! Run: GUI `start_run`, MCP `start_run`, and Python-free CLIs hand the same
//! engine-style argument vector to `YoutubeRunOptions::from_engine_args`, then
//! call `run_youtube_export`. Sizing, bitrate, FFmpeg arguments, and acceptance
//! checks come from `youtube.rs`; encoding comes from `encode_run.rs`.
//! Inputs: media root or file plus the flags parsed below. Outputs: separate
//! `.mp4` copies under the export folder (never the source path), a run log, and
//! a CSV report under the app log folder. Sources are only read.
//! Stop: safe-stop is honoured between files; an abort that kills FFmpeg is
//! reported as INTERRUPTED, its temporary file is removed, and the source stays
//! untouched.

use crate::encode_run::{run_encode, EncodeFailure, EncodeProgress, EncodeSpec};
use crate::media_scan::{
    discover_media_paths, disk_free_bytes, engine_profile, EngineProfileRequest,
};
use crate::output_targets::canonicalize_allow_missing;
use crate::resource_plan::{self, MemoryForecastInput};
use crate::run_arguments::ToolPaths;
use crate::run_coordinator::optional_tool;
use crate::{packet_verify, stop_control, transactions, youtube, youtube_optimize};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_MEMORY_ATTEMPTS: u32 = 4;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const DEFAULT_EXPORT_FOLDER: &str = "YouTube Uploads";
const MIN_SOURCE_MULTIPLIER: f64 = 0.5;
const MAX_SOURCE_MULTIPLIER: f64 = 4.0;

const RESOURCE_FLAGS: [(&str, &str); 6] = [
    ("--resource-threading", "threading"),
    ("--reserved-cpu-threads", "reserved_cpu_threads"),
    ("--max-worker-threads", "max_worker_threads"),
    ("--min-free-memory-gib", "min_free_memory_gib"),
    ("--memory-reserve-percent", "memory_reserve_percent"),
    (
        "--disk-projection-margin-percent",
        "disk_projection_margin_percent",
    ),
];
const IGNORED_VALUE_FLAGS: [&str; 20] = [
    "--crf",
    "--preset",
    "--tune",
    "--dolby-vision-policy",
    "--secondary-video",
    "--data-streams",
    "--frame-threads",
    "--calibration-mode",
    "--calibration-cache-hours",
    "--calibration-large-file-gib",
    "--calibration-sample-seconds",
    "--calibration-max-candidates",
    "--calibration-tolerance-percent",
    "--preflight-output-ratio",
    "--comment",
    "--metadata-text-blacklist",
    "--allowed-poster-cache-root",
    "--file-output-dirs-json",
    "--file-poster-overrides-json",
    "--file-metadata-overrides-json",
];
const IGNORED_BOOLEAN_FLAGS: [&str; 21] = [
    "--youtube-upload",
    "--no-dependency-repair",
    "--skip-update-check",
    "--assume-yes",
    "--allow-unsupported-os",
    "--verbose",
    "--promote-8bit-to-10bit",
    "--no-promote-8bit-to-10bit",
    "--detect-animation",
    "--no-detect-animation",
    "--preserve-hdr10plus",
    "--no-preserve-hdr10plus",
    "--verify-copy-hashes",
    "--no-verify-copy-hashes",
    "--force-deinterlace",
    "--disable-deinterlace",
    "--disable-hdr10-opt",
    "--preserve-source-level",
    "--embed-artwork",
    "--no-embed-artwork",
    "--replace-metadata-text-blacklist",
];
const REJECTED_FLAGS: [(&str, &str); 4] = [
    (
        "--file-stream-overrides-json",
        "per-file stream selections do not apply to YouTube export mode",
    ),
    (
        "--file-settings-overrides-json",
        "current-run encoder settings do not apply to YouTube export mode",
    ),
    (
        "--file-tune-overrides-json",
        "per-file x265 content tunes do not apply to YouTube export mode",
    ),
    (
        "--output-dir",
        "rebuild output folders do not apply to YouTube export mode; use the YouTube output folder",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct YoutubeRunOptions {
    pub root: PathBuf,
    pub recursive: bool,
    pub minimum_size_mib: u32,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub selected_files: Vec<PathBuf>,
    pub resolution: String,
    pub quality_mode: String,
    pub source_multiplier: f64,
    pub output_dir: Option<PathBuf>,
    pub hdr10plus_loss: BTreeSet<PathBuf>,
    pub stop_on_error: bool,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
    pub settings: Value,
    /// `true` for Optimize mode (lossless-first, never resizes).
    pub optimize: bool,
    pub sdr_codec: String,
    pub preset: String,
    pub shrink: String,
    pub quality_check: String,
    pub work_root: String,
}

#[derive(Clone, Debug)]
pub(crate) struct YoutubeProgress {
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
pub(crate) struct YoutubeFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
}

pub(crate) struct YoutubeJobResult {
    pub files: Vec<YoutubeFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub destination_root: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
    pub succeeded: usize,
    pub skipped: usize,
}

struct Tools {
    ffmpeg: String,
    ffprobe: String,
    mediainfo: String,
    x265_pixel_formats: Vec<String>,
    /// Quietvoid helper path when available; enables post-encode HDR10+ inject.
    hdr10plus: Option<String>,
}

/// Why a file did not produce an output.
enum Halt {
    Skip(String),
    Fail(String),
    Interrupted,
}

impl From<String> for Halt {
    fn from(message: String) -> Self {
        Self::Fail(message)
    }
}

// ---------------------------------------------------------------- arguments

impl YoutubeRunOptions {
    /// Parse the engine-style vector built by the GUI and MCP. Unknown flags are
    /// rejected so a future flag can never be silently dropped.
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            resolution: "1440p".into(),
            quality_mode: "balanced".into(),
            source_multiplier: 2.0,
            sdr_codec: "h264".into(),
            preset: "slow".into(),
            shrink: "auto".into(),
            quality_check: "standard".into(),
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut resource_overrides: Vec<(&'static str, String)> = Vec::new();
        let mut min_free_disk_gib: Option<String> = None;
        let mut loss_paths: Vec<PathBuf> = Vec::new();
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
                "--upload-resolution" => options.resolution = value()?,
                "--upload-quality-mode" => options.quality_mode = value()?,
                "--upload-source-multiplier" => {
                    options.source_multiplier = value()?
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .ok_or("The YouTube source-size multiplier must be a number.")?;
                }
                "--youtube-mode" => {
                    options.optimize = match value()?.as_str() {
                        "upscale" => false,
                        "optimize" => true,
                        other => {
                            return Err(format!(
                                "YouTube mode must be upscale or optimize, not {other:?}."
                            ))
                        }
                    };
                }
                "--youtube-sdr-codec" => options.sdr_codec = value()?,
                "--youtube-preset" => options.preset = value()?,
                "--youtube-shrink" => options.shrink = value()?,
                "--youtube-quality-check" => options.quality_check = value()?,
                "--upload-output-dir" => options.output_dir = Some(PathBuf::from(value()?)),
                "--allow-youtube-hdr10plus-loss" => loss_paths.push(PathBuf::from(value()?)),
                "--work-root" => {
                    options.work_root = value()?;
                    crate::preview_work_directory(Path::new("."), &options.work_root)?;
                }
                "--min-free-disk-gib" => min_free_disk_gib = Some(value()?),
                "--stop-request-file" => options.stop_file = Some(PathBuf::from(value()?)),
                "--ffmpeg-path" => options.tool_paths.ffmpeg = Some(value()?),
                "--ffprobe-path" => options.tool_paths.ffprobe = Some(value()?),
                "--mediainfo-path" => options.tool_paths.mediainfo = Some(value()?),
                "--mkvpropedit-path" => options.tool_paths.mkvpropedit = Some(value()?),
                "--mkvmerge-path" => options.tool_paths.mkvmerge = Some(value()?),
                "--mkvextract-path" => options.tool_paths.mkvextract = Some(value()?),
                flag if RESOURCE_FLAGS.iter().any(|(known, _)| *known == flag) => {
                    let key = RESOURCE_FLAGS
                        .iter()
                        .find(|(known, _)| *known == flag)
                        .map(|(_, key)| *key)
                        .unwrap_or_default();
                    resource_overrides.push((key, value()?));
                }
                flag if IGNORED_VALUE_FLAGS.contains(&flag) => {
                    value()?;
                }
                flag if IGNORED_BOOLEAN_FLAGS.contains(&flag) => consumed = 1,
                flag if REJECTED_FLAGS.iter().any(|(rejected, _)| *rejected == flag) => {
                    let reason = REJECTED_FLAGS
                        .iter()
                        .find(|(rejected, _)| *rejected == flag)
                        .map(|(_, reason)| *reason)
                        .unwrap_or("this option is not supported");
                    return Err(format!("YouTube export: {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "YouTube export does not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("YouTube export accepts exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        if !matches!(options.sdr_codec.as_str(), "h264" | "hevc") {
            return Err("The YouTube SDR codec must be h264 or hevc.".into());
        }
        if !matches!(options.preset.as_str(), "medium" | "slow" | "slower") {
            return Err("The YouTube encoder preset must be medium, slow, or slower.".into());
        }
        youtube_optimize::Shrink::parse(&options.shrink)?;
        youtube_optimize::QualityCheck::parse(&options.quality_check)?;
        if options.optimize {
            // Optimize never resizes; any selected upscale tier is ignored.
            options.resolution = "source".into();
        }
        if !matches!(options.resolution.as_str(), "source" | "1440p" | "4k") {
            return Err("Upload resolution must be source, 1440p, or 4k.".into());
        }
        if !matches!(options.quality_mode.as_str(), "balanced" | "recommended") {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        if !(MIN_SOURCE_MULTIPLIER..=MAX_SOURCE_MULTIPLIER).contains(&options.source_multiplier) {
            return Err(format!(
                "YouTube source-size multiplier must be between {MIN_SOURCE_MULTIPLIER} and {MAX_SOURCE_MULTIPLIER}."
            ));
        }
        let root = root.ok_or("YouTube export requires a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;
        for path in loss_paths {
            let canonical = path.canonicalize().map_err(|error| {
                format!("An HDR10+ consent path is no longer available: {error}")
            })?;
            options.hdr10plus_loss.insert(canonical);
        }
        let mut settings = effective_settings(&options.root)?;
        for (key, raw) in resource_overrides {
            let parsed = if key == "threading" {
                json!(raw)
            } else if let Ok(number) = raw.parse::<i64>() {
                json!(number)
            } else {
                json!(raw
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("The resource setting {key} must be a number."))?)
            };
            settings["resources"][key] = parsed;
        }
        if let Some(raw) = min_free_disk_gib {
            settings["resources"]["min_free_disk_gib"] = json!(raw
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or("The disk reserve must be a non-negative number of GiB.")?);
        }
        options.settings = settings;
        Ok(options)
    }

    fn disk_reserve_bytes(&self) -> u64 {
        let gib = self
            .settings
            .pointer("/resources/min_free_disk_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        (gib * GIB) as u64
    }

    fn projection_margin_percent(&self) -> f64 {
        self.settings
            .pointer("/resources/disk_projection_margin_percent")
            .and_then(Value::as_f64)
            .unwrap_or(15.0)
            .clamp(0.0, 100.0)
    }
}

fn effective_settings(root: &Path) -> Result<Value, String> {
    let anchor = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    let payload = serde_json::to_value(crate::settings::load_effective_settings(
        &anchor.to_string_lossy(),
    )?)
    .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    let mut settings = payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !settings.get("resources").is_some_and(Value::is_object) {
        settings["resources"] = json!({});
    }
    Ok(settings)
}

// -------------------------------------------------------------- pure policy

/// Names an output, disambiguating sources that share a stem but differ in
/// extension (`Movie.mkv` + `Movie.mp4` become `Movie [mkv].mp4` / `Movie [mp4].mp4`).
fn stem_key(relative: &Path) -> String {
    relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

fn output_relative_path(relative: &Path, shared_stem: bool) -> PathBuf {
    if shared_stem {
        let suffix = relative
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "media".into());
        let stem = relative
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        relative.with_file_name(format!("{stem} [{suffix}].mp4"))
    } else {
        relative.with_extension("mp4")
    }
}

struct SourcePolicy {
    color_mode: &'static str,
    note: Option<String>,
}

/// Decide whether a source can be exported and how. `Err` carries the SKIPPED
/// message. When `hdr10plus_can_inject` is true, HDR10+ is preserved after
/// encode via `hdr10plus_tool`; otherwise per-file loss consent is required.
fn source_policy(
    profile: &Value,
    resolution: &str,
    x265_pixel_formats: &[String],
    hdr10plus_loss_accepted: bool,
    hdr10plus_can_inject: bool,
) -> Result<SourcePolicy, String> {
    let color_mode = youtube::upload_color_mode(profile);
    let unsupported = youtube::unsupported_color_message(color_mode);
    if let Some(message) = unsupported {
        return Err(message.into());
    }
    if color_mode == "sdr" {
        return Ok(SourcePolicy {
            color_mode: "sdr",
            note: None,
        });
    }
    let (width, height) = youtube::expected_dimensions(profile, resolution)?;
    if width.min(height) < 720 {
        return Err("YouTube's HDR upload guidance lists 720p and higher; this selected output would be below 720p.".into());
    }
    if !x265_pixel_formats.iter().any(|name| name == "yuv420p10le") {
        return Err("This FFmpeg does not expose 10-bit libx265 (yuv420p10le), which the HDR export requires.".into());
    }
    let mut note = None;
    if profile
        .pointer("/hdr/hdr10plus_hint")
        .and_then(Value::as_bool)
        == Some(true)
    {
        if hdr10plus_can_inject {
            note = Some(
                "HDR10+ will be preserved after encode with hdr10plus_tool inject.".to_string(),
            );
        } else if !hdr10plus_loss_accepted {
            return Err("HDR10+ dynamic metadata cannot be retained without hdr10plus_tool. No copy was written; install the helper, or explicitly accept this loss for the individual file to export static HDR10/PQ only.".into());
        } else {
            note = Some("HDR10+ dynamic metadata was not retained after explicit per-file approval; static HDR10/PQ signaling is retained.".to_string());
        }
    }
    Ok(SourcePolicy { color_mode, note })
}

/// Extract source HDR10+ JSON into the app work tree for a later MP4 inject.
fn extract_youtube_hdr10plus_json(
    tools: &Tools,
    source: &Path,
    profile: &Value,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    let helper = tools
        .hdr10plus
        .as_deref()
        .ok_or("HDR10+ inject requires hdr10plus_tool.")?;
    let stream_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .or_else(|| profile.pointer("/video/index").and_then(Value::as_i64))
        .unwrap_or(0);
    let codec = profile
        .pointer("/video/codec")
        .and_then(Value::as_str)
        .unwrap_or("hevc");
    fs::create_dir_all(work_dir)
        .map_err(|error| format!("Could not create HDR10+ work folder: {error}"))?;
    let metadata_json = work_dir.join(format!("youtube-hdr10plus-{}.json", std::process::id()));
    if metadata_json.exists() {
        return Err(format!(
            "HDR10+ extract refused to overwrite {}",
            metadata_json.display()
        ));
    }
    crate::hdr_metadata::extract_hdr10plus(
        &tools.ffmpeg,
        helper,
        source,
        stream_index,
        codec,
        &metadata_json,
        None,
    )?;
    Ok(metadata_json)
}

fn inject_youtube_hdr10plus(
    tools: &Tools,
    mp4: &Path,
    metadata_json: &Path,
    work_dir: &Path,
) -> Result<(), String> {
    let helper = tools
        .hdr10plus
        .as_deref()
        .ok_or("HDR10+ inject requires hdr10plus_tool.")?;
    crate::hdr_metadata::inject_hdr10plus_into_mp4(
        &tools.ffmpeg,
        helper,
        mp4,
        0,
        metadata_json,
        work_dir,
    )
}

struct EncodePlan {
    workers: i64,
    frames: i64,
    per_frame: f64,
    per_worker: f64,
    budget: Option<f64>,
    memory_guard_gib: f64,
}

fn plan_resources(
    settings: &Value,
    profile: &Value,
    dimensions: (i64, i64),
    preset: &str,
) -> Result<EncodePlan, String> {
    let host = resource_plan::live_host();
    let forecast = resource_plan::memory_forecast(MemoryForecastInput {
        profile,
        settings,
        host: &host,
        full_memory_psi: resource_plan::memory_full_avg10(),
        target_dimensions: Some(dimensions),
        preset_override: Some(preset),
        frame_threads_override: Some(0),
        concurrent_video_streams: 1,
    });
    if forecast.get("level").and_then(Value::as_str) == Some("blocked") {
        let reason = forecast
            .pointer("/messages/0")
            .and_then(Value::as_str)
            .unwrap_or("Available memory is below the safety reserve.");
        return Err(format!(
            "{reason} Free memory or lower the reserve in Settings, then retry; the source remains untouched."
        ));
    }
    let number = |key: &str| forecast.get(key).and_then(Value::as_f64);
    Ok(EncodePlan {
        workers: forecast
            .get("planned_workers")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        frames: forecast
            .get("planned_frame_threads")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        per_frame: number("per_frame_allowance_gib").unwrap_or(1.0),
        per_worker: number("per_worker_allowance_gib").unwrap_or(0.25),
        budget: number("planning_budget_gib"),
        memory_guard_gib: number("reserve_gib").unwrap_or(0.0),
    })
}

impl EncodePlan {
    /// Lower-parallelism plan for a memory-pressure retry, or `None` when no
    /// further reduction is possible.
    fn reduced(&self) -> Option<Self> {
        let next = resource_plan::retry_plan(
            self.workers,
            self.frames,
            self.per_frame,
            self.per_worker,
            self.budget,
        );
        if next.get("reduces_parallelism").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(Self {
            workers: next.get("planned_workers").and_then(Value::as_i64)?.max(1),
            frames: next
                .get("planned_frame_threads")
                .and_then(Value::as_i64)?
                .max(1),
            per_frame: self.per_frame,
            per_worker: self.per_worker,
            budget: self.budget,
            memory_guard_gib: self.memory_guard_gib,
        })
    }
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

fn temporary_name(destination: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = destination
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    destination.with_file_name(format!(
        ".{stem}.upload-{:x}-{:x}-{:x}.tmp.mp4",
        std::process::id(),
        nonce,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn resolve_tools(options: &YoutubeRunOptions) -> Result<Tools, String> {
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
        x265_pixel_formats: host
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        hdr10plus: optional_tool(&host, "hdr10plus_tool"),
    };
    for (key, label) in [
        ("libx264", "the libx264 encoder"),
        ("aac", "the AAC encoder"),
    ] {
        let available = host
            .get("checks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|check| {
                check.get("key").and_then(Value::as_str) == Some(key)
                    && check.get("available").and_then(Value::as_bool) == Some(true)
            });
        if !available {
            missing.push(label.to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "YouTube export needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    Ok(tools)
}

// ------------------------------------------------------------------- running

struct Context<'a> {
    options: &'a YoutubeRunOptions,
    tools: &'a Tools,
    destination_root: &'a Path,
    shared_stems: &'a BTreeMap<String, usize>,
    progress: &'a mut dyn FnMut(YoutubeProgress),
    line: &'a mut dyn FnMut(String),
    child: &'a mut dyn FnMut(Option<u32>),
}

struct Frame<'a> {
    source: &'a Path,
    number: usize,
    total: usize,
    started: Instant,
    input_bytes: u64,
}

#[derive(Default)]
struct Live {
    estimated_output_bytes: Option<u64>,
    output_bytes: Option<u64>,
    speed: Option<f64>,
    eta_seconds: Option<f64>,
}

fn emit(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    live: &Live,
) {
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(YoutubeProgress {
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
        queue_percent: (frame.number.saturating_sub(1) as f64 + fraction)
            / frame.total.max(1) as f64
            * 100.0,
        elapsed_seconds: frame.started.elapsed().as_secs_f64(),
        speed: live.speed,
        eta_seconds: live.eta_seconds,
        input_bytes: Some(frame.input_bytes),
        output_bytes: live.output_bytes,
        estimated_output_bytes: live.estimated_output_bytes,
    });
}

fn note(log: &mut fs::File, line: &mut dyn FnMut(String), text: String) {
    let _ = writeln!(log, "{text}");
    line(text);
}

/// Lines the desktop and MCP consumers read from engine output after a run.
pub(crate) fn closing_lines(job: &YoutubeJobResult) -> Vec<String> {
    let total = job.files.len();
    let summary = if job.stop_requested {
        format!(
            "YouTube export stopped after {total} file{}; remaining files were not started.",
            if total == 1 { "" } else { "s" }
        )
    } else if total == 0 {
        "YouTube export had no files to process.".to_string()
    } else {
        format!(
            "YouTube export finished: {} validated cop{}, {} skipped, {} failed safely. Originals stayed in place.",
            job.succeeded,
            if job.succeeded == 1 { "y" } else { "ies" },
            job.skipped,
            job.failed
        )
    };
    vec![
        format!("Log: {}", job.log_path.display()),
        format!("Run report: {}", job.report_path.display()),
        summary,
        format!("Output folder: {}", job.destination_root.display()),
    ]
}

pub(crate) fn run_youtube_export(
    options: &YoutubeRunOptions,
    mut on_progress: impl FnMut(YoutubeProgress),
    mut on_line: impl FnMut(String),
    mut on_child: impl FnMut(Option<u32>),
) -> Result<YoutubeJobResult, String> {
    let tools = resolve_tools(options)?;
    let root = &options.root;
    let mut files = discover_media_paths(
        root,
        options.recursive,
        options.minimum_size_mib,
        &options.include,
        &options.exclude,
        options.stop_file.as_deref(),
    )?;
    if root.is_file() && files.is_empty() {
        return Err(format!(
            "This file is not a supported media input or is below the size limit: {}",
            root.display()
        ));
    }
    if !options.selected_files.is_empty() {
        let selected: BTreeSet<PathBuf> = options
            .selected_files
            .iter()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .collect();
        files.retain(|path| selected.contains(path));
    }
    let discovered: BTreeSet<&PathBuf> = files.iter().collect();
    let unknown: Vec<String> = options
        .hdr10plus_loss
        .iter()
        .filter(|path| !discovered.contains(path))
        .take(5)
        .map(|path| path.display().to_string())
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "YouTube HDR10+ loss consent does not belong to the selected media input: {}",
            unknown.join(", ")
        ));
    }
    let source_base: PathBuf = if root.is_file() {
        root.parent().unwrap_or(root).to_path_buf()
    } else {
        root.clone()
    };
    let requested_destination = options
        .output_dir
        .clone()
        .unwrap_or_else(|| source_base.join(DEFAULT_EXPORT_FOLDER));
    if !requested_destination.is_absolute() {
        return Err("Choose an absolute YouTube output folder path.".into());
    }
    let destination_root = canonicalize_allow_missing(&requested_destination)?;
    if destination_root == *root {
        return Err("The upload output folder cannot be the selected source path; choose a separate YouTube Uploads folder.".into());
    }
    if root.is_dir()
        && destination_root.starts_with(root)
        && destination_root
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .as_deref()
            != Some("youtube uploads")
    {
        return Err("When exporting inside the source folder, use a folder named 'YouTube Uploads' so later scans can safely exclude it.".into());
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_dir = crate::state_log_directory()?;
    fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the run log folder: {error}"))?;
    let log_path = log_dir.join(format!("hevc-rebuild-{stamp}-youtube.log"));
    let report_path = log_dir.join(format!("hevc-rebuild-{stamp}-youtube.csv"));
    let mut log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create the YouTube run log: {error}"))?;

    if files.is_empty() {
        note(
            &mut log,
            &mut on_line,
            "No supported media files were found in the selected input.".into(),
        );
        return Ok(YoutubeJobResult {
            files: Vec::new(),
            log_path,
            report_path,
            destination_root,
            stop_requested: false,
            failed: 0,
            succeeded: 0,
            skipped: 0,
        });
    }
    fs::create_dir_all(&destination_root)
        .map_err(|error| format!("Could not create the YouTube output folder: {error}"))?;

    let relative_of = |path: &Path| -> PathBuf {
        if root.is_file() {
            path.file_name().map(PathBuf::from).unwrap_or_default()
        } else {
            path.strip_prefix(&source_base)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.to_path_buf())
        }
    };
    let mut shared_stems: BTreeMap<String, usize> = BTreeMap::new();
    for path in &files {
        *shared_stems
            .entry(stem_key(&relative_of(path)))
            .or_default() += 1;
    }
    let total = files.len();
    let total_bytes: u64 = files
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    note(
        &mut log,
        &mut on_line,
        format!(
            "YouTube export · {} · {total} file{} · {}",
            options.resolution.to_uppercase(),
            if total == 1 { "" } else { "s" },
            human_bytes(total_bytes)
        ),
    );
    note(
        &mut log,
        &mut on_line,
        format!(
            "Output copies will be written to {}; source files stay in place",
            destination_root.display()
        ),
    );

    let mut context = Context {
        options,
        tools: &tools,
        destination_root: &destination_root,
        shared_stems: &shared_stems,
        progress: &mut on_progress,
        line: &mut on_line,
        child: &mut on_child,
    };
    let mut results = Vec::with_capacity(total);
    let mut stop_requested = false;
    let (mut succeeded, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    for (offset, source) in files.iter().enumerate() {
        if options
            .stop_file
            .as_deref()
            .is_some_and(stop_control::requested)
        {
            stop_requested = true;
            (context.line)(format!(
                "Safe stop requested; file {}/{total} and the remaining files were not started.",
                offset + 1
            ));
            let _ = writeln!(log, "Stopped before file {}/{total}.", offset + 1);
            break;
        }
        let relative = relative_of(source);
        let shared = context
            .shared_stems
            .get(&stem_key(&relative))
            .is_some_and(|count| *count > 1);
        let result = process_one(
            &mut context,
            source,
            &relative,
            shared,
            offset + 1,
            total,
            &report_path,
        );
        let _ = writeln!(
            log,
            "{}: {} · {}",
            source.display(),
            result.status,
            result.message
        );
        match result.status.as_str() {
            "OK" => succeeded += 1,
            "SKIPPED" => skipped += 1,
            "INTERRUPTED" => stop_requested = true,
            _ => failed += 1,
        }
        let halt =
            result.status == "INTERRUPTED" || (result.status == "FAILED" && options.stop_on_error);
        if result.status == "FAILED" && options.stop_on_error {
            (context.line)(
                "Stop-on-error is enabled; remaining files were not started.".to_string(),
            );
        }
        results.push(result);
        if halt {
            break;
        }
    }
    let _ = log.flush();
    Ok(YoutubeJobResult {
        files: results,
        log_path,
        report_path,
        destination_root,
        stop_requested,
        failed,
        succeeded,
        skipped,
    })
}

fn process_one(
    context: &mut Context<'_>,
    source: &Path,
    relative: &Path,
    shared_stem: bool,
    number: usize,
    total: usize,
    report_path: &Path,
) -> YoutubeFileResult {
    let started = Instant::now();
    let input_bytes = fs::metadata(source).map(|meta| meta.len()).unwrap_or(0);
    let frame = Frame {
        source,
        number,
        total,
        started,
        input_bytes,
    };
    let options = context.options;
    let mut row = Map::new();
    row.insert("input".into(), json!(source.to_string_lossy()));
    row.insert("status".into(), json!("FAILED"));
    row.insert("message".into(), json!(""));
    row.insert("input_bytes".into(), json!(input_bytes));
    row.insert("upload_resolution".into(), json!(options.resolution));
    row.insert("upload_quality_mode".into(), json!(options.quality_mode));
    row.insert(
        "upload_source_multiplier".into(),
        json!(options.source_multiplier),
    );
    let destination = context
        .destination_root
        .join(output_relative_path(relative, shared_stem));
    row.insert("output".into(), json!(destination.to_string_lossy()));
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    (context.line)(format!("Preparing {name} ({number}/{total})"));
    let mut temporary: Option<PathBuf> = None;
    let mut live = Live::default();
    let outcome = execute(
        context,
        &frame,
        &destination,
        &mut row,
        &mut temporary,
        &mut live,
    );
    if let Some(path) = temporary.take() {
        let _ = fs::remove_file(path);
    }
    match outcome {
        Ok(()) => {
            emit(context, &frame, "Export complete", 1.0, Some(100.0), &live);
        }
        Err(Halt::Skip(message)) => {
            row.insert("status".into(), json!("SKIPPED"));
            row.insert("message".into(), json!(message));
            (context.line)(format!("{name}: skipped · {message}"));
            emit(context, &frame, "Skipped", 1.0, None, &live);
        }
        Err(Halt::Interrupted) => {
            row.insert("status".into(), json!("INTERRUPTED"));
            row.insert(
                "message".into(),
                json!("Interrupted before a validated copy was installed; the source was left untouched."),
            );
            (context.line)(format!("File {number}/{total} was interrupted: {name}"));
            emit(context, &frame, "Stopped safely", 0.0, None, &live);
        }
        Err(Halt::Fail(message)) => {
            row.insert("status".into(), json!("FAILED"));
            row.insert("message".into(), json!(message));
            (context.line)(format!("{name}: failed safely · {message}"));
            emit(context, &frame, "Failed safely", 0.0, None, &live);
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    row.insert(
        "elapsed_seconds".into(),
        json!((elapsed * 1000.0).round() / 1000.0),
    );
    if let Err(error) = crate::reports::append_native_row(report_path, row.clone()) {
        (context.line)(format!("Could not write the run report row: {error}"));
    }
    let text = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    YoutubeFileResult {
        path: source.to_string_lossy().into_owned(),
        status: text("status"),
        message: text("message"),
        output: (text("status") == "OK").then(|| destination.to_string_lossy().into_owned()),
        elapsed_seconds: elapsed,
    }
}

struct EncodeJob<'a> {
    duration: f64,
    estimated_bytes: u64,
    reserve: u64,
    phase: &'a str,
    temp_path: &'a Path,
}

fn string_args(built: &Value) -> Result<Vec<String>, String> {
    Ok(built
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("The YouTube planner returned no FFmpeg arguments."))?
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect())
}

/// Run one supervised FFmpeg job, retrying with lower parallelism on memory
/// pressure. `make_args` rebuilds the argument vector for each attempt's plan.
fn encode_with_retries(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    live: &mut Live,
    row: &mut Map<String, Value>,
    job: &EncodeJob<'_>,
    mut plan: EncodePlan,
    make_args: &dyn Fn(&EncodePlan) -> Result<Vec<String>, String>,
) -> Result<(), Halt> {
    let (source, number, total) = (frame.source, frame.number, frame.total);
    let options = context.options;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut merged: Map<String, Value> = Map::new();
    let mut attempt = 1;
    loop {
        let args = make_args(&plan)?;
        emit(context, frame, job.phase, 0.05, Some(0.0), live);
        let progress = &mut *context.progress;
        let child = &mut *context.child;
        let spec = EncodeSpec {
            args: &args,
            expected_duration: job.duration,
            output_path: job.temp_path,
            memory_guard_gib: plan.memory_guard_gib,
            disk_guard_bytes: job.reserve,
            projection_margin_percent: options.projection_margin_percent(),
            stop_file: options.stop_file.as_deref(),
        };
        let result = run_encode(
            &spec,
            |block: &EncodeProgress| {
                live.speed = block.speed;
                live.eta_seconds = block.eta_seconds;
                live.output_bytes = Some(block.output_bytes);
                let percent = block.file_percent;
                progress(YoutubeProgress {
                    current: number,
                    total,
                    file_name: name.clone(),
                    file_path: source.to_string_lossy().into_owned(),
                    phase: job.phase.into(),
                    file_percent: percent,
                    queue_percent: (number.saturating_sub(1) as f64
                        + 0.05
                        + 0.90 * percent.unwrap_or(0.0) / 100.0)
                        / total.max(1) as f64
                        * 100.0,
                    elapsed_seconds: frame.started.elapsed().as_secs_f64(),
                    speed: block.speed,
                    eta_seconds: block.eta_seconds,
                    input_bytes: Some(frame.input_bytes),
                    output_bytes: Some(block.output_bytes),
                    estimated_output_bytes: Some(job.estimated_bytes),
                });
            },
            child,
        );
        match result {
            Ok(metrics) => {
                merged.extend(metrics);
                break;
            }
            Err(EncodeFailure::MemoryPressure { reason, resources }) => {
                merged.extend(resources);
                let next = if attempt >= MAX_MEMORY_ATTEMPTS {
                    None
                } else {
                    plan.reduced()
                };
                let Some(next) = next else {
                    row.extend(merged);
                    return Err(Halt::Fail(
                        "Memory pressure persisted after four safe attempts; the source remains untouched."
                            .into(),
                    ));
                };
                let _ = fs::remove_file(job.temp_path);
                (context.line)(format!(
                    "Memory pressure detected ({reason}); retrying {name} with {} encoder thread(s)",
                    next.workers
                ));
                plan = next;
                attempt += 1;
            }
            Err(EncodeFailure::Interrupted { resources }) => {
                row.extend(resources);
                return Err(Halt::Interrupted);
            }
            Err(EncodeFailure::Failed { message, resources }) => {
                row.extend(resources);
                return Err(Halt::Fail(message));
            }
        }
    }
    row.extend(merged);
    Ok(())
}

fn execute(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    destination: &Path,
    row: &mut Map<String, Value>,
    temporary: &mut Option<PathBuf>,
    live: &mut Live,
) -> Result<(), Halt> {
    let source = frame.source;
    let options = context.options;
    let tools = context.tools;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let planned = canonicalize_allow_missing(destination)?;
    if planned == source {
        return Err(Halt::Fail(
            "The output path matches the source file; choose a separate export folder.".into(),
        ));
    }
    if destination.exists() {
        return Err(Halt::Skip(
            "An output with this name already exists; it was left unchanged.".into(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the output folder: {error}"))?;
    }
    emit(context, frame, "Analyzing source", 0.0, Some(0.0), live);
    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: false,
    })?;
    let profile = response
        .get("profile")
        .cloned()
        .ok_or_else(|| String::from("The profiler returned no source profile."))?;
    if let Some(codec) = profile.pointer("/video/codec") {
        row.insert("primary_video".into(), codec.clone());
    }
    if let (Some(width), Some(height)) = (
        profile.pointer("/video/width").and_then(Value::as_i64),
        profile.pointer("/video/height").and_then(Value::as_i64),
    ) {
        row.insert("resolution".into(), json!(format!("{width}x{height}")));
    }
    for key in ["audio_count", "subtitle_count"] {
        if let Some(count) = profile.get(key) {
            row.insert(key.into(), count.clone());
        }
    }
    if profile.get("subtitle_count").and_then(Value::as_i64) > Some(0) {
        row.insert(
            "subtitle_note".into(),
            json!("Subtitle streams are not included in the YouTube MP4; upload supported caption files separately."),
        );
    }
    if options.optimize {
        return execute_optimize(context, frame, destination, row, temporary, live, &profile);
    }
    let accepted = options.hdr10plus_loss.contains(source);
    let can_inject = tools.hdr10plus.is_some();
    let policy = source_policy(
        &profile,
        &options.resolution,
        &tools.x265_pixel_formats,
        accepted,
        can_inject,
    )
    .map_err(Halt::Skip)?;
    let color_mode = policy.color_mode;
    let mut hdr10plus_note = policy.note;
    row.insert("upload_color_mode".into(), json!(color_mode));

    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let estimate = youtube::estimate(
        &profile,
        frame.input_bytes,
        duration,
        &options.resolution,
        &options.quality_mode,
        options.source_multiplier,
    )?;
    let integer = |key: &str| estimate.get(key).and_then(Value::as_i64).unwrap_or(0);
    let (out_width, out_height) = (integer("output_width"), integer("output_height"));
    let (source_width, source_height) = (integer("source_width"), integer("source_height"));
    let bitrate = integer("video_bitrate_kbps");
    let estimated_bytes = integer("estimated_output_bytes").max(0) as u64;
    if (out_width, out_height) != (source_width, source_height) {
        (context.line)(format!(
            "{name}: resizing to {out_width}x{out_height}; aspect ratio is preserved"
        ));
    }
    row.insert("upload_video_bitrate_kbps".into(), json!(bitrate));
    row.insert(
        "youtube_recommended_video_bitrate_kbps".into(),
        json!(integer("recommended_video_bitrate_kbps")),
    );
    row.insert("estimated_output_bytes".into(), json!(estimated_bytes));
    live.estimated_output_bytes = Some(estimated_bytes);
    (context.line)(format!(
        "{name}: estimated upload copy {} at {bitrate} kb/s video; YouTube guidance is {} kb/s.",
        human_bytes(estimated_bytes),
        integer("recommended_video_bitrate_kbps")
    ));
    let reserve = options.disk_reserve_bytes();
    let output_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(free) = disk_free_bytes(output_parent) {
        if free < estimated_bytes.saturating_add(reserve) {
            return Err(Halt::Fail(format!(
                "Not enough free space for the conservative upload estimate ({} output plus {:.1} GiB reserve).",
                human_bytes(estimated_bytes),
                reserve as f64 / GIB
            )));
        }
    }

    let inject_hdr10plus = can_inject
        && profile
            .pointer("/hdr/hdr10plus_hint")
            .and_then(Value::as_bool)
            == Some(true);
    let work_dir = if inject_hdr10plus {
        Some(
            crate::media_work_directory(source, &context.options.work_root)?
                .join(format!("youtube-hdr10plus-{}", std::process::id())),
        )
    } else {
        None
    };
    let hdr10plus_json = if inject_hdr10plus {
        let dir = work_dir.as_ref().expect("inject work dir");
        (context.line)(format!("{name}: extracting HDR10+ for post-encode inject"));
        Some(extract_youtube_hdr10plus_json(
            tools, source, &profile, dir,
        )?)
    } else {
        None
    };

    let plan = plan_resources(
        &options.settings,
        &profile,
        (out_width, out_height),
        &options.preset,
    )?;
    row.insert("planned_workers".into(), json!(plan.workers));
    row.insert("planned_frame_threads".into(), json!(plan.frames));
    let temp_path = temporary_name(destination);
    *temporary = Some(temp_path.clone());
    let job = EncodeJob {
        duration,
        estimated_bytes,
        reserve,
        phase: "Encoding",
        temp_path: &temp_path,
    };
    encode_with_retries(context, frame, live, row, &job, plan, &|plan| {
        let built = youtube::build_args(&json!({
            "profile": profile,
            "tools": {
                "ffmpeg": tools.ffmpeg,
                "x265_pixel_formats": tools.x265_pixel_formats,
            },
            "source_path": source.to_string_lossy(),
            "output": temp_path.to_string_lossy(),
            "resolution": options.resolution,
            "video_bitrate_kbps": bitrate,
            "worker_threads": plan.workers,
            "frame_threads": plan.frames,
            "sdr_codec": options.sdr_codec,
            "preset": options.preset,
        }))?;
        string_args(&built)
    })?;

    if let (Some(json), Some(dir)) = (&hdr10plus_json, &work_dir) {
        emit(
            context,
            frame,
            "Injecting HDR10+ metadata",
            0.93,
            Some(100.0),
            live,
        );
        (context.line)(format!("{name}: injecting HDR10+ into the YouTube MP4"));
        inject_youtube_hdr10plus(tools, &temp_path, json, dir)?;
        hdr10plus_note = Some(
            "HDR10+ dynamic metadata was preserved after encode with hdr10plus_tool inject.".into(),
        );
        let _ = fs::remove_file(json);
        let _ = fs::remove_dir(dir);
    }
    if let Some(note_text) = hdr10plus_note {
        row.insert("hdr10plus_note".into(), json!(note_text));
    }

    emit(context, frame, "Validating output", 0.95, Some(100.0), live);
    let codec_label = if matches!(color_mode, "hdr10" | "hlg") {
        "HEVC Main 10/AAC"
    } else if options.sdr_codec == "hevc" {
        "HEVC/AAC"
    } else {
        "H.264/AAC"
    };
    (context.line)(format!(
        "Validating {codec_label} streams, color tags, dimensions, and duration for {name}"
    ));
    let validated = youtube::validate_request(&json!({
        "profile": profile,
        "tools": {
            "ffprobe": tools.ffprobe,
            "ffmpeg": tools.ffmpeg,
            "hdr10plus_tool": tools.hdr10plus,
        },
        "output": temp_path.to_string_lossy(),
        "resolution": options.resolution,
        "sdr_codec": options.sdr_codec,
        "hdr10plus_expected": inject_hdr10plus,
    }))?;
    let (output_width, output_height) = (
        validated.get("width").and_then(Value::as_i64).unwrap_or(0),
        validated.get("height").and_then(Value::as_i64).unwrap_or(0),
    );
    emit(context, frame, "Installing output", 0.98, Some(100.0), live);
    transactions::install_export_output(&temp_path, destination)?;
    *temporary = None;
    let output_bytes = fs::metadata(destination)
        .map(|meta| meta.len())
        .unwrap_or(0);
    live.output_bytes = Some(output_bytes);
    let mode_label = match color_mode {
        "hdr10" => "HDR10/PQ",
        "hlg" => "HLG",
        _ => "SDR",
    };
    row.insert("status".into(), json!("OK"));
    row.insert("output_bytes".into(), json!(output_bytes));
    row.insert(
        "output_resolution".into(),
        json!(format!("{output_width}x{output_height}")),
    );
    row.insert(
        "message".into(),
        json!(format!(
            "Validated YouTube {mode_label} MP4 copy ({output_width}x{output_height}, {bitrate} kb/s video target, {} size profile, {}x source multiplier); source unchanged.",
            options.quality_mode, options.source_multiplier
        )),
    );
    Ok(())
}

/// Lossless-first Optimize flow for one source that is already 1440p or larger:
/// stream-copy into a faststart MP4 with YouTube tags, or re-encode only when the
/// plan says so and the sampled SSIM check accepts the result.
fn execute_optimize(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    destination: &Path,
    row: &mut Map<String, Value>,
    temporary: &mut Option<PathBuf>,
    live: &mut Live,
    profile: &Value,
) -> Result<(), Halt> {
    use youtube_optimize::{Action, AudioAction, PlanInput, QualityCheck, Shrink};
    let source = frame.source;
    let options = context.options;
    let tools = context.tools;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let quality = QualityCheck::parse(&options.quality_check)?;
    let plan = youtube_optimize::plan(
        profile,
        &PlanInput {
            shrink: Shrink::parse(&options.shrink)?,
            sdr_codec: &options.sdr_codec,
            hdr10plus_loss_accepted: options.hdr10plus_loss.contains(source),
            hdr10plus_can_inject: tools.hdr10plus.is_some(),
            x265_pixel_formats: &tools.x265_pixel_formats,
            file_bytes: frame.input_bytes,
            duration,
        },
    )
    .map_err(Halt::Skip)?;
    let (width, height) = youtube::expected_dimensions(profile, "source")?;
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .ok_or_else(|| String::from("The source profile has no selected video stream."))?;
    row.insert("upload_mode".into(), json!("optimize"));
    row.insert("upload_resolution".into(), json!("source"));
    row.insert("upload_color_mode".into(), json!(plan.color_mode));
    row.insert("optimize_reason".into(), json!(plan.reason));
    row.insert(
        "youtube_recommended_video_bitrate_kbps".into(),
        json!(plan.recommended_kbps),
    );
    if let Some(rate) = plan.source_video_kbps {
        row.insert(
            "source_video_bitrate_kbps".into(),
            json!(rate.round() as i64),
        );
    }
    if !plan.notes.is_empty() {
        row.insert("optimize_note".into(), json!(plan.notes.join(" ")));
    }
    (context.line)(format!("{name}: {}", plan.reason));
    for note_text in &plan.notes {
        (context.line)(format!("{name}: {note_text}"));
    }

    let audio_rate: i64 = plan
        .audio
        .iter()
        .map(|action| match action {
            AudioAction::Copy => 384,
            AudioAction::Transcode { kbps } => *kbps,
        })
        .sum();
    let encode_estimate =
        (duration * (plan.encode_bitrate_kbps.saturating_add(audio_rate)) as f64 * 1000.0 / 8.0
            * 1.12)
            .max(0.0) as u64;
    let estimated_bytes = match plan.action {
        Action::Copy => frame.input_bytes,
        Action::Encode => encode_estimate,
    };
    row.insert("estimated_output_bytes".into(), json!(estimated_bytes));
    live.estimated_output_bytes = Some(estimated_bytes);
    let reserve = options.disk_reserve_bytes();
    let output_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(free) = disk_free_bytes(output_parent) {
        let needed = frame.input_bytes.max(estimated_bytes);
        if free < needed.saturating_add(reserve) {
            return Err(Halt::Fail(format!(
                "Not enough free space for the output ({} plus {:.1} GiB reserve).",
                human_bytes(needed),
                reserve as f64 / GIB
            )));
        }
    }

    let temp_path = temporary_name(destination);
    *temporary = Some(temp_path.clone());
    let mut action = plan.action;
    let mut decision_note: Option<String> = None;
    let mut quality_value: Option<f64> = None;
    let audio_plan = youtube_optimize::audio_plan_json(&plan.audio);
    let hdr10plus_expected = tools.hdr10plus.is_some()
        && profile
            .pointer("/hdr/hdr10plus_hint")
            .and_then(Value::as_bool)
            == Some(true);
    let inject_hdr10plus = action == Action::Encode && hdr10plus_expected;
    let optimize_work_dir = if inject_hdr10plus {
        Some(
            crate::media_work_directory(source, &context.options.work_root)?
                .join(format!("youtube-opt-hdr10plus-{}", std::process::id())),
        )
    } else {
        None
    };
    let hdr10plus_json = if inject_hdr10plus {
        let dir = optimize_work_dir.as_ref().expect("inject work dir");
        (context.line)(format!("{name}: extracting HDR10+ for post-encode inject"));
        Some(extract_youtube_hdr10plus_json(tools, source, profile, dir)?)
    } else {
        None
    };

    if action == Action::Encode {
        let resources =
            plan_resources(&options.settings, profile, (width, height), &options.preset)?;
        row.insert("planned_workers".into(), json!(resources.workers));
        row.insert("planned_frame_threads".into(), json!(resources.frames));
        let job = EncodeJob {
            duration,
            estimated_bytes,
            reserve,
            phase: "Re-encoding",
            temp_path: &temp_path,
        };
        encode_with_retries(context, frame, live, row, &job, resources, &|resources| {
            let built = youtube::build_args(&json!({
                "profile": profile,
                "tools": {
                    "ffmpeg": tools.ffmpeg,
                    "x265_pixel_formats": tools.x265_pixel_formats,
                },
                "source_path": source.to_string_lossy(),
                "output": temp_path.to_string_lossy(),
                "resolution": "source",
                "video_bitrate_kbps": plan.encode_bitrate_kbps,
                "worker_threads": resources.workers,
                "frame_threads": resources.frames,
                "sdr_codec": options.sdr_codec,
                "preset": options.preset,
                "audio_plan": audio_plan,
            }))?;
            string_args(&built)
        })?;
        if let (Some(json), Some(dir)) = (&hdr10plus_json, &optimize_work_dir) {
            emit(
                context,
                frame,
                "Injecting HDR10+ metadata",
                0.91,
                Some(100.0),
                live,
            );
            (context.line)(format!("{name}: injecting HDR10+ into the YouTube MP4"));
            inject_youtube_hdr10plus(tools, &temp_path, json, dir)?;
            row.insert(
                "hdr10plus_note".into(),
                json!("HDR10+ dynamic metadata was preserved after encode with hdr10plus_tool inject."),
            );
            let _ = fs::remove_file(json);
            let _ = fs::remove_dir(dir);
        }
        emit(context, frame, "Validating output", 0.92, Some(100.0), live);
        youtube::validate_request(&json!({
            "profile": profile,
            "tools": {
                "ffprobe": tools.ffprobe,
                "ffmpeg": tools.ffmpeg,
                "hdr10plus_tool": tools.hdr10plus,
            },
            "output": temp_path.to_string_lossy(),
            "resolution": "source",
            "sdr_codec": options.sdr_codec,
            "allow_copied_audio": true,
            "hdr10plus_expected": inject_hdr10plus,
        }))?;
        let output_bytes = fs::metadata(&temp_path).map(|meta| meta.len()).unwrap_or(0);
        let mut rejection: Option<String> = None;
        let saved = 1.0 - output_bytes as f64 / frame.input_bytes.max(1) as f64;
        if plan.copy_fallback && saved < youtube_optimize::MIN_SAVING_FRACTION {
            rejection = Some(format!(
                "the re-encode saved only {:.0}% ({} versus {})",
                saved.max(0.0) * 100.0,
                human_bytes(output_bytes),
                human_bytes(frame.input_bytes)
            ));
        }
        if rejection.is_none() {
            if let Some(floor) = quality.floor() {
                emit(context, frame, "Checking quality", 0.94, None, live);
                (context.line)(format!(
                    "Comparing {name} with the source on sampled windows (SSIM floor {floor})"
                ));
                let child = &mut *context.child;
                match youtube_optimize::ssim_gate(
                    &tools.ffmpeg,
                    source,
                    &temp_path,
                    primary_index,
                    duration,
                    child,
                ) {
                    Ok(value) => {
                        quality_value = Some(value);
                        row.insert(
                            "quality_ssim_min".into(),
                            json!((value * 10_000.0).round() / 10_000.0),
                        );
                        if value < floor {
                            rejection = Some(format!(
                                "sampled SSIM {value:.4} is below the {floor} floor"
                            ));
                        }
                    }
                    Err(youtube_optimize::GateError::Interrupted) => return Err(Halt::Interrupted),
                    Err(youtube_optimize::GateError::Failed(message)) => {
                        rejection = Some(format!("the quality check could not run ({message})"));
                    }
                }
            }
        }
        if let Some(why) = rejection {
            let _ = fs::remove_file(&temp_path);
            row.remove("hdr10plus_note");
            if let Some(json) = &hdr10plus_json {
                let _ = fs::remove_file(json);
            }
            if let Some(dir) = &optimize_work_dir {
                let _ = fs::remove_dir_all(dir);
            }
            if plan.copy_fallback {
                let text =
                    format!("Kept the original video stream instead of the re-encode: {why}.");
                (context.line)(format!("{name}: {text}"));
                decision_note = Some(text);
                action = Action::Copy;
            } else {
                return Err(Halt::Skip(format!(
                    "The re-encode was rejected ({why}) and this source cannot be stream-copied, so nothing was written. Relax the quality check or re-encode it with Upscale mode."
                )));
            }
        }
    }

    let mut payload_identical: Option<bool> = None;
    if action == Action::Copy {
        let resources = EncodePlan {
            workers: 1,
            frames: 1,
            per_frame: 0.0,
            per_worker: 0.0,
            budget: None,
            memory_guard_gib: 0.0,
        };
        let job = EncodeJob {
            duration,
            estimated_bytes: frame.input_bytes,
            reserve,
            phase: "Copying streams",
            temp_path: &temp_path,
        };
        encode_with_retries(context, frame, live, row, &job, resources, &|_| {
            youtube_optimize::copy_args(
                &tools.ffmpeg,
                &source.to_string_lossy(),
                &temp_path.to_string_lossy(),
                profile,
                &plan,
            )
        })?;
        emit(context, frame, "Validating output", 0.92, Some(100.0), live);
        youtube::validate_request(&json!({
            "profile": profile,
            "tools": {
                "ffprobe": tools.ffprobe,
                "ffmpeg": tools.ffmpeg,
                "hdr10plus_tool": tools.hdr10plus,
            },
            "output": temp_path.to_string_lossy(),
            "resolution": "source",
            "expected_codec": plan.source_codec,
            "allow_copied_audio": true,
            "hdr10plus_expected": hdr10plus_expected,
        }))?;
        emit(context, frame, "Verifying streams", 0.95, None, live);
        let ordinal = profile
            .get("tracks")
            .and_then(Value::as_array)
            .map(|tracks| {
                tracks
                    .iter()
                    .filter(|track| {
                        track.get("type").and_then(Value::as_str) == Some("video")
                            && track.get("index").and_then(Value::as_i64) < Some(primary_index)
                    })
                    .count()
            })
            .unwrap_or(0);
        let ffmpeg = Path::new(&tools.ffmpeg);
        let before = packet_verify::packet_fingerprints(ffmpeg, source, &[format!("v:{ordinal}")])?;
        let after = packet_verify::packet_fingerprints(ffmpeg, &temp_path, &["v:0".to_string()])?;
        if let (Some(left), Some(right)) = (before.first(), after.first()) {
            if left.packet_count != right.packet_count {
                return Err(Halt::Fail(format!(
                    "The stream copy changed the video packet count ({} in the source, {} in the copy); nothing was installed.",
                    left.packet_count, right.packet_count
                )));
            }
            payload_identical = Some(left == right);
        }
    }

    emit(context, frame, "Installing output", 0.98, Some(100.0), live);
    transactions::install_export_output(&temp_path, destination)?;
    *temporary = None;
    let output_bytes = fs::metadata(destination)
        .map(|meta| meta.len())
        .unwrap_or(0);
    live.output_bytes = Some(output_bytes);
    let mode_label = match plan.color_mode {
        "hdr10" => "HDR10/PQ",
        "hlg" => "HLG",
        _ => "SDR",
    };
    let saved_text = if output_bytes < frame.input_bytes {
        format!(
            ", {:.0}% smaller",
            (1.0 - output_bytes as f64 / frame.input_bytes.max(1) as f64) * 100.0
        )
    } else {
        String::new()
    };
    let how = match action {
        Action::Copy => format!(
            "video stream copied untouched ({}{}), YouTube tags and faststart applied",
            plan.source_codec.to_uppercase(),
            match payload_identical {
                Some(true) => ", packets verified identical",
                Some(false) => ", packet count verified",
                None => "",
            }
        ),
        Action::Encode => format!(
            "re-encoded at {} kb/s{}",
            plan.encode_bitrate_kbps,
            quality_value
                .map(|value| format!(", sampled SSIM {value:.4}"))
                .unwrap_or_default()
        ),
    };
    row.insert(
        "optimize_action".into(),
        json!(match action {
            Action::Copy => "copy",
            Action::Encode => "encode",
        }),
    );
    if let Some(identical) = payload_identical {
        row.insert("video_payload_identical".into(), json!(identical));
    }
    row.insert("status".into(), json!("OK"));
    row.insert("output_bytes".into(), json!(output_bytes));
    row.insert(
        "output_resolution".into(),
        json!(format!("{width}x{height}")),
    );
    let mut message = format!(
        "Validated YouTube {mode_label} MP4 ({width}x{height}): {how}{saved_text}; source unchanged."
    );
    if let Some(extra) = decision_note {
        message.push(' ');
        message.push_str(&extra);
    }
    row.insert("message".into(), json!(message));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdr_profile(width: i64, height: i64) -> Value {
        json!({
            "video": {"codec": "h264", "width": width, "height": height},
            "hdr": {"hdr10": false, "hdr10plus_hint": false, "hlg": false, "dolby_vision": false},
            "audio_count": 1, "subtitle_count": 0, "duration_seconds": 60.0
        })
    }

    #[test]
    fn colliding_stems_get_extension_suffixes_and_unique_ones_do_not() {
        let mkv = Path::new("Show/Movie.mkv");
        let mp4 = Path::new("Show/Movie.MP4");
        assert_eq!(stem_key(mkv), stem_key(mp4));
        assert_eq!(
            output_relative_path(mkv, true),
            PathBuf::from("Show/Movie [mkv].mp4")
        );
        assert_eq!(
            output_relative_path(mp4, true),
            PathBuf::from("Show/Movie [mp4].mp4")
        );
        assert_eq!(
            output_relative_path(Path::new("Movie.2020.mkv"), false),
            PathBuf::from("Movie.2020.mp4")
        );
    }

    #[test]
    fn sdr_sources_are_always_exportable_without_consent() {
        let policy = source_policy(&sdr_profile(1920, 1080), "1440p", &[], false, false).unwrap();
        assert_eq!(policy.color_mode, "sdr");
        assert!(policy.note.is_none());
    }

    fn hdr_profile() -> Value {
        json!({
            "video": {"codec": "hevc", "width": 3840, "height": 2160, "source_bit_depth": 10},
            "hdr": {"hdr10": true, "hdr10plus_hint": true, "hlg": false, "dolby_vision": false},
            "audio_count": 1, "duration_seconds": 600.0, "source_bit_depth": 10,
            "color_primaries": "bt2020", "transfer": "smpte2084", "color_matrix": "bt2020nc"
        })
    }

    #[test]
    fn dolby_vision_skips_even_with_consent() {
        let mut dovi = hdr_profile();
        dovi["hdr"]["dolby_vision"] = json!(true);
        let message = source_policy(&dovi, "4k", &["yuv420p10le".into()], true, false)
            .err()
            .unwrap();
        assert!(message.contains("Dolby Vision"));
    }

    #[test]
    fn hdr10plus_loss_requires_per_file_consent_and_leaves_a_note() {
        let formats = vec!["yuv420p10le".to_string()];
        let declined = source_policy(&hdr_profile(), "4k", &formats, false, false)
            .err()
            .unwrap();
        assert!(declined.contains("HDR10+ dynamic metadata cannot be retained"));
        let accepted = source_policy(&hdr_profile(), "4k", &formats, true, false).unwrap();
        assert_eq!(accepted.color_mode, "hdr10");
        assert!(accepted
            .note
            .unwrap()
            .contains("explicit per-file approval"));
        let mut plain_hdr = hdr_profile();
        plain_hdr["hdr"]["hdr10plus_hint"] = json!(false);
        let policy = source_policy(&plain_hdr, "4k", &formats, false, false).unwrap();
        assert!(policy.note.is_none());
    }

    #[test]
    fn hdr10plus_inject_skips_loss_consent() {
        let formats = vec!["yuv420p10le".to_string()];
        let policy = source_policy(&hdr_profile(), "4k", &formats, false, true).unwrap();
        assert_eq!(policy.color_mode, "hdr10");
        assert!(policy.note.unwrap().contains("hdr10plus_tool inject"));
    }

    #[test]
    fn hdr_requires_ten_bit_x265_and_at_least_720p() {
        let message = source_policy(&hdr_profile(), "4k", &[], true, false)
            .err()
            .unwrap();
        assert!(message.contains("10-bit libx265"));
        let mut small = hdr_profile();
        small["video"]["width"] = json!(640);
        small["video"]["height"] = json!(360);
        let message = source_policy(&small, "source", &["yuv420p10le".into()], true, false)
            .err()
            .unwrap();
        assert!(message.contains("below 720p"));
        let mut untagged = hdr_profile();
        untagged["transfer"] = json!("unknown");
        untagged["hdr"]["hdr10"] = json!(true);
        let message = source_policy(&untagged, "4k", &["yuv420p10le".into()], true, false)
            .err()
            .unwrap();
        assert!(message.contains("PQ or HLG"));
    }

    #[test]
    fn engine_arguments_parse_options_and_ignore_settings_noise() {
        let dir = std::env::temp_dir().join(format!(
            "hevc-youtube-args-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mkv");
        fs::write(&file, b"x").unwrap();
        let root = dir.to_string_lossy().into_owned();
        let arguments = |extra: &[&str]| -> Vec<String> {
            let mut args = vec![
                root.clone(),
                "--min-size-mib".into(),
                "0".into(),
                "--no-dependency-repair".into(),
                "--skip-update-check".into(),
                "--recursive".into(),
                "--youtube-upload".into(),
                "--upload-resolution".into(),
                "4k".into(),
                "--upload-quality-mode".into(),
                "recommended".into(),
                "--upload-source-multiplier".into(),
                "1.5".into(),
                "--crf".into(),
                "20".into(),
                "--no-embed-artwork".into(),
                "--min-free-disk-gib".into(),
                "2".into(),
                "--reserved-cpu-threads".into(),
                "2".into(),
                "--allow-youtube-hdr10plus-loss".into(),
                file.to_string_lossy().into_owned(),
            ];
            args.extend(extra.iter().map(|value| value.to_string()));
            args
        };
        let options = YoutubeRunOptions::from_engine_args(&arguments(&[])).unwrap();
        assert_eq!(options.resolution, "4k");
        assert_eq!(options.quality_mode, "recommended");
        assert_eq!(options.source_multiplier, 1.5);
        assert!(options.recursive);
        assert_eq!(options.hdr10plus_loss.len(), 1);
        assert_eq!(options.disk_reserve_bytes(), (2.0 * GIB) as u64);
        assert_eq!(
            options.settings.pointer("/resources/reserved_cpu_threads"),
            Some(&json!(2))
        );
        assert!(YoutubeRunOptions::from_engine_args(&arguments(&["--surprise"])).is_err());
        assert!(YoutubeRunOptions::from_engine_args(&arguments(&[
            "--file-stream-overrides-json",
            "{}"
        ]))
        .is_err());
        assert!(YoutubeRunOptions::from_engine_args(&arguments(&[
            "--upload-source-multiplier",
            "9"
        ]))
        .is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn closing_lines_expose_log_report_and_output_folder() {
        let job = YoutubeJobResult {
            files: vec![YoutubeFileResult {
                path: "/m/a.mkv".into(),
                status: "OK".into(),
                message: String::new(),
                output: None,
                elapsed_seconds: 1.0,
            }],
            log_path: "/logs/a.log".into(),
            report_path: "/logs/a.csv".into(),
            destination_root: "/m/YouTube Uploads".into(),
            stop_requested: false,
            failed: 0,
            succeeded: 1,
            skipped: 0,
        };
        let lines = closing_lines(&job);
        assert_eq!(lines[0], "Log: /logs/a.log");
        assert_eq!(lines[1], "Run report: /logs/a.csv");
        assert!(lines[2].contains("1 validated copy"));
        assert_eq!(lines[3], "Output folder: /m/YouTube Uploads");
    }

    #[test]
    fn temporary_names_are_hidden_unique_mp4_siblings() {
        let destination = Path::new("/out/Movie.mp4");
        let first = temporary_name(destination);
        let second = temporary_name(destination);
        assert_ne!(first, second);
        assert_eq!(first.parent(), destination.parent());
        let text = first.file_name().unwrap().to_string_lossy();
        assert!(text.starts_with(".Movie.upload-") && text.ends_with(".tmp.mp4"));
    }

    fn tool_available(name: &str) -> bool {
        std::process::Command::new(name)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    /// Fresh canonical media root and state directory; also points the engine's state there.
    fn scratch_dirs() -> (PathBuf, PathBuf) {
        let token = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        );
        let root = std::env::temp_dir().join(format!("hevc-youtube-run-{token}"));
        let state_dir = std::env::temp_dir().join(format!("hevc-youtube-state-{token}"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
        (root.canonicalize().unwrap(), state_dir)
    }

    fn tools_present() -> bool {
        ["ffmpeg", "ffprobe", "mediainfo"].iter().all(|tool| {
            let present = tool_available(tool);
            if !present {
                eprintln!("skipping native YouTube integration: {tool} is unavailable");
            }
            present
        })
    }

    /// 2560x1440 H.264 + AAC fixture; `kbps` forces a constant (padded) video bitrate.
    fn make_1440p_clip(path: &Path, kbps: u32) -> bool {
        let rate = format!("{kbps}k");
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=2560x1440:rate=24:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-b:v",
                &rate,
                "-minrate",
                &rate,
                "-maxrate",
                &rate,
                "-bufsize",
                &rate,
                "-x264-params",
                "nal-hrd=cbr:force-cfr=1",
                "-c:a",
                "aac",
                "-shortest",
                "-y",
            ])
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    /// End to end on a synthetic SDR clip: needs ffmpeg (libx264 + aac), ffprobe, and
    /// MediaInfo; skipped with a note on machines without them.
    #[test]
    fn synthetic_sdr_clip_exports_a_validated_youtube_copy() {
        if !tools_present() {
            return;
        }
        let _guard = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (root, state_dir) = scratch_dirs();
        let source = root.join("Clip One (2024).mkv");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x360:rate=24:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
                "-shortest",
                "-y",
            ])
            .arg(&source)
            .status();
        if !made.is_ok_and(|status| status.success()) {
            eprintln!("skipping native YouTube integration: could not build the fixture");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let before = fs::read(&source).unwrap();
        let args = vec![
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            "0".into(),
            "--youtube-upload".into(),
            "--upload-resolution".into(),
            "source".into(),
        ];
        let options = YoutubeRunOptions::from_engine_args(&args).unwrap();
        let mut phases = Vec::new();
        let mut saw_progress_percent = false;
        let result = run_youtube_export(
            &options,
            |progress| {
                saw_progress_percent |= progress.file_percent.is_some();
                phases.push(progress.phase);
            },
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(result.failed, 0, "{:?}", result.files);
        assert_eq!(result.succeeded, 1, "{:?}", result.files);
        let output = PathBuf::from(result.files[0].output.clone().unwrap());
        assert!(output.is_file(), "missing {}", output.display());
        assert_eq!(
            output.extension().and_then(|value| value.to_str()),
            Some("mp4")
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            before,
            "source must stay untouched"
        );
        assert!(!phases.is_empty());
        assert!(saw_progress_percent, "encode progress was never reported");
        let leftovers: Vec<_> = fs::read_dir(output.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files left behind: {leftovers:?}"
        );
        let report = fs::read_to_string(&result.report_path).unwrap();
        assert!(report.contains("Clip One (2024).mkv"), "{report}");
        assert!(report.contains("OK"), "{report}");
        for dir in [root, state_dir] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    /// Optimize mode on two 1440p clips: a healthy one must be stream-copied with
    /// YouTube tags (video packets untouched), an oversized one re-encoded smaller.
    #[test]
    fn optimize_copies_healthy_clips_and_shrinks_oversized_ones() {
        if !tools_present() {
            return;
        }
        let _guard = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (root, state_dir) = scratch_dirs();
        let healthy = root.join("Healthy.mkv");
        let huge = root.join("Huge.mkv");
        if !make_1440p_clip(&healthy, 6_000) || !make_1440p_clip(&huge, 60_000) {
            eprintln!("skipping native YouTube optimize integration: could not build fixtures");
            let _ = fs::remove_dir_all(&root);
            let _ = fs::remove_dir_all(&state_dir);
            return;
        }
        let before = (fs::read(&healthy).unwrap(), fs::read(&huge).unwrap());
        let args = vec![
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            "0".into(),
            "--youtube-upload".into(),
            "--youtube-mode".into(),
            "optimize".into(),
            "--youtube-preset".into(),
            "medium".into(),
            "--max-worker-threads".into(),
            "1".into(),
            "--min-free-memory-gib".into(),
            "0".into(),
            "--memory-reserve-percent".into(),
            "0".into(),
            "--disk-projection-margin-percent".into(),
            "0".into(),
        ];
        let options = YoutubeRunOptions::from_engine_args(&args).unwrap();
        let result = run_youtube_export(&options, |_| {}, |_| {}, |_| {}).unwrap();
        assert_eq!(result.failed, 0, "{:?}", result.files);
        assert_eq!(result.succeeded, 2, "{:?}", result.files);
        let find = |needle: &str| {
            result
                .files
                .iter()
                .find(|file| file.path.contains(needle))
                .unwrap_or_else(|| panic!("no result for {needle}: {:?}", result.files))
        };
        let copied = find("Healthy");
        assert!(
            copied.message.contains("copied untouched"),
            "{}",
            copied.message
        );
        assert!(
            copied.message.contains("packets verified identical"),
            "{}",
            copied.message
        );
        let shrunk = find("Huge");
        assert!(shrunk.message.contains("re-encoded"), "{}", shrunk.message);
        // The saved CSV must carry the Optimize decision for each file.
        let saved = crate::reports::read_report(
            &crate::state_log_directory().unwrap(),
            Path::new(&result.report_path),
        )
        .unwrap();
        let saved_rows = saved["rows"].as_array().unwrap();
        let saved_row = |needle: &str| {
            saved_rows
                .iter()
                .find(|row| {
                    row["input"]
                        .as_str()
                        .is_some_and(|path| path.contains(needle))
                })
                .unwrap_or_else(|| panic!("no report row for {needle}: {saved_rows:?}"))
        };
        let healthy_row = saved_row("Healthy");
        assert_eq!(healthy_row["upload_mode"], "optimize");
        assert_eq!(healthy_row["optimize_action"], "copy");
        // Booleans keep the report schema's established "True"/"False" spelling.
        assert_eq!(healthy_row["video_payload_identical"], "True");
        assert!(
            !healthy_row["optimize_reason"]
                .as_str()
                .unwrap_or("")
                .is_empty(),
            "copy rows must say why the stream was kept: {healthy_row:?}"
        );
        let huge_row = saved_row("Huge");
        assert_eq!(huge_row["optimize_action"], "encode");
        let ssim: f64 = huge_row["quality_ssim_min"]
            .as_str()
            .unwrap_or_else(|| panic!("encode rows must record the SSIM value: {huge_row:?}"))
            .parse()
            .unwrap();
        assert!((0.0..=1.0).contains(&ssim), "SSIM out of range: {ssim}");
        let huge_out = PathBuf::from(shrunk.output.clone().unwrap());
        assert!(huge_out.is_file());
        assert!(
            fs::metadata(&huge_out).unwrap().len() < before.1.len() as u64 / 2,
            "the oversized clip should shrink by more than half"
        );
        assert_eq!(
            fs::read(&healthy).unwrap(),
            before.0,
            "source must stay untouched"
        );
        assert_eq!(
            fs::read(&huge).unwrap(),
            before.1,
            "source must stay untouched"
        );
        for dir in [root, state_dir] {
            let _ = fs::remove_dir_all(dir);
        }
    }
}
