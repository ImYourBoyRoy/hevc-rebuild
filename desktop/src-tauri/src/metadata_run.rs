//! ./desktop/src-tauri/src/metadata_run.rs
//! Native Metadata-only MKV run coordinator (T4.2 / T4.4). Rust owns the whole
//! job with no Python process: discovery, source profiling, HDR10+ source
//! verification, output planning, storage preflight, `mkvmerge` lossless remux
//! with live progress, reviewed metadata/poster embedding, packet-hash and HDR
//! validation, expected-tag verification, durable no-overwrite install, retained
//! `.error.mkv`/`.error.log` review artifacts, and CSV reports.
//!
//! Run: GUI `start_run`, MCP `start_run`, and the JSON CLI all hand the same
//! engine-style argument vector to `MetadataRunOptions::from_engine_args`, then
//! call `run_metadata_only`. Unspecified comment/blacklist/artwork/disk-reserve
//! values fall back to the effective settings, matching the Python engine.
//! Inputs: media root or file plus the flags parsed below. Outputs: a new MKV per
//! source (never the source path), a run log, and a CSV report under the app log
//! folder. Sources are only read. Stop: safe-stop is honoured between files; an
//! abort that kills `mkvmerge` is reported as INTERRUPTED and leaves the source
//! untouched.

use crate::media_scan::{
    discover_media_paths, disk_free_bytes, engine_profile, engine_track_profiles,
    EngineProfileRequest, EngineTrackProfilesRequest,
};
use crate::output_targets::canonicalize_allow_missing;
use crate::process_output::spawn_owned;
use crate::run_arguments::ToolPaths;
use crate::run_coordinator::optional_tool;
use crate::transactions::{install_export_output, rename_no_replace, sync_directory};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MIB: u64 = 1024 * 1024;
const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;
const SUPPORTED_STREAM_TYPES: [&str; 5] = ["video", "audio", "subtitle", "attachment", "data"];
const FINISHED_OUTPUT_PHASES: [&str; 5] = [
    "metadata embedding",
    "output validation",
    "lossless output validation",
    "validated output install",
    "transaction commit",
];
static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// Engine flags the native metadata run accepts and deliberately ignores because
/// they only tune encoding, resources, or dependency repair.
const IGNORED_VALUE_FLAGS: [&str; 20] = [
    "--crf",
    "--preset",
    "--tune",
    "--dolby-vision-policy",
    "--secondary-video",
    "--data-streams",
    "--resource-threading",
    "--reserved-cpu-threads",
    "--max-worker-threads",
    "--frame-threads",
    "--min-free-memory-gib",
    "--memory-reserve-percent",
    "--disk-projection-margin-percent",
    "--calibration-mode",
    "--calibration-cache-hours",
    "--calibration-large-file-gib",
    "--calibration-sample-seconds",
    "--calibration-max-candidates",
    "--calibration-tolerance-percent",
    "--preflight-output-ratio",
];
const IGNORED_BOOLEAN_FLAGS: [&str; 18] = [
    "--metadata-only",
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
];
const REJECTED_FLAGS: [(&str, &str); 3] = [
    (
        "--file-stream-overrides-json",
        "metadata-only output always preserves every source stream; per-file stream selection is not allowed",
    ),
    (
        "--file-settings-overrides-json",
        "current-run encoder settings do not apply to metadata-only output",
    ),
    (
        "--file-tune-overrides-json",
        "per-file encoding tunes do not apply to metadata-only output",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct MetadataRunOptions {
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
    pub comment: String,
    pub text_blacklist: Vec<String>,
    pub embed_artwork: bool,
    pub min_free_disk_gib: f64,
    pub stop_on_error: bool,
    pub keep_work: bool,
    pub reprocess: bool,
    pub work_root: String,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
}

#[derive(Clone, Debug)]
pub(crate) struct MetadataProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: String,
    pub file_percent: Option<f64>,
    pub queue_percent: f64,
    pub elapsed_seconds: f64,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetadataFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
}

pub(crate) struct MetadataJobResult {
    pub files: Vec<MetadataFileResult>,
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
}

struct Context<'a> {
    options: &'a MetadataRunOptions,
    tools: &'a Tools,
    progress: &'a mut dyn FnMut(MetadataProgress),
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

// ---------------------------------------------------------------- arguments

impl MetadataRunOptions {
    /// Parse the engine-style vector built by the GUI and MCP. Unknown flags are
    /// rejected so a future flag can never be silently dropped.
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            embed_artwork: true,
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut comment: Option<String> = None;
        let mut embed_artwork: Option<bool> = None;
        let mut blacklist: Vec<String> = Vec::new();
        let mut replace_blacklist = false;
        let mut min_free_disk_gib: Option<f64> = None;
        let mut output_dirs_json: Option<String> = None;
        let mut poster_json: Option<String> = None;
        let mut metadata_json: Option<String> = None;
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
                "--allowed-poster-cache-root" => {
                    value()?;
                }
                "--comment" => comment = Some(value()?),
                "--metadata-text-blacklist" => blacklist.push(value()?),
                "--min-free-disk-gib" => {
                    min_free_disk_gib = Some(
                        value()?
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite() && *value >= 0.0)
                            .ok_or("The disk reserve must be a non-negative number of GiB.")?,
                    );
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
                    return Err(format!("Metadata-only {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "Metadata-only runs do not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("Metadata-only runs accept exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        let root = root.ok_or("Metadata-only runs require a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;

        let settings = effective_settings(&options.root)?;
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
        options.min_free_disk_gib = min_free_disk_gib.unwrap_or_else(|| {
            settings
                .pointer("/resources/min_free_disk_gib")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        });
        options.text_blacklist = merged_blacklist(&settings, blacklist, replace_blacklist);
        options.file_output_dirs = parse_path_map(output_dirs_json, "per-file output folders")?
            .into_iter()
            .collect();
        options.poster_overrides = parse_path_map(poster_json, "poster selections")?
            .into_iter()
            .collect();
        options.metadata_overrides = parse_metadata_overrides(metadata_json)?;
        Ok(options)
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
    Ok(payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({})))
}

/// Configured patterns plus run patterns, or only run patterns when replaced.
fn merged_blacklist(settings: &Value, extra: Vec<String>, replace: bool) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();
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

// ------------------------------------------------------------------ planning

/// Pick the destination, mirroring Python `resolved_output_path`.
fn resolved_output_path(
    source: &Path,
    options: &MetadataRunOptions,
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

/// A new MKV path for metadata-only work; never resolves to the source.
fn metadata_output_path(source: &Path, options: &MetadataRunOptions) -> Result<PathBuf, String> {
    let (base, _) = resolved_output_path(source, options)?;
    if base == source {
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        return Ok(source.with_file_name(format!("{stem}.metadata.mkv")));
    }
    Ok(base)
}

fn validate_output_destinations(
    files: &[PathBuf],
    options: &MetadataRunOptions,
) -> Result<(), String> {
    let mut planned: BTreeMap<PathBuf, &Path> = BTreeMap::new();
    for source in files {
        let output = metadata_output_path(source, options)?;
        if output.exists() {
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

/// Copy-only budget: enough room to start plus the configured reserve.
fn storage_preflight(
    source_size: u64,
    free_bytes: u64,
    min_free_disk_gib: f64,
) -> Result<(), String> {
    let reserve = (min_free_disk_gib.max(0.0) * 1024.0 * 1024.0 * 1024.0) as u64;
    let minimum_temp = (16 * MIB).max(source_size / 20);
    let required = reserve.saturating_add(minimum_temp);
    if free_bytes < required {
        return Err(format!(
            "Insufficient free space even to start safely: {} free, need at least {} ({} reserve + {} working headroom).",
            human_bytes(free_bytes),
            human_bytes(required),
            human_bytes(reserve),
            human_bytes(minimum_temp)
        ));
    }
    Ok(())
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
        let candidate = root.join(format!("hevc-metadata-only-{}", unique_token()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create a metadata work folder: {error}")),
        }
    }
    Err("Could not allocate a unique metadata work folder.".into())
}

// ----------------------------------------------------------------- profiling

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

/// Expected HDR/colour state for one copied video track (same fields Python sent).
fn hdr_expectation(profile: &Value, hdr10plus: bool) -> Value {
    let hdr = profile.get("hdr");
    json!({
        "color_primaries": text_value(profile.get("color_primaries")),
        "transfer": text_value(profile.get("color_transfer")),
        "color_matrix": text_value(profile.get("color_matrix")),
        "master_display": text_value(profile.get("mastering_display")),
        "max_cll": profile.get("max_cll").and_then(Value::as_i64),
        "max_fall": profile.get("max_fall").and_then(Value::as_i64),
        "hdr10": flag(hdr, "hdr10"),
        "hdr10plus": hdr10plus,
        "hlg": flag(hdr, "hlg"),
        "dolby_vision": flag(hdr, "dolby_vision"),
        "dovi_profile": hdr.and_then(|h| h.get("dolby_vision_profile")).and_then(Value::as_i64),
        "dovi_rpu_present": flag(hdr, "dolby_vision_rpu"),
        "dovi_el_present": flag(hdr, "dolby_vision_enhancement_layer"),
        "dovi_compatibility_id": hdr
            .and_then(|h| h.get("dolby_vision_compatibility_id"))
            .and_then(Value::as_i64),
        "preserve_dolbyvision": false,
        "strip_dolbyvision": false,
    })
}

struct SourceStream {
    index: i64,
    codec_type: String,
    codec_name: String,
    real_video: bool,
}

fn source_streams(probe: &Value) -> Vec<SourceStream> {
    probe
        .get("streams")
        .and_then(Value::as_array)
        .map(|streams| {
            streams
                .iter()
                .filter_map(|stream| {
                    let index = stream.get("index").and_then(Value::as_i64)?;
                    let codec_type = stream
                        .get("codec_type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let attached = stream
                        .pointer("/disposition/attached_pic")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        == 1;
                    let positive =
                        |key: &str| stream.get(key).and_then(Value::as_i64).unwrap_or(0) > 0;
                    Some(SourceStream {
                        index,
                        real_video: codec_type == "video"
                            && !attached
                            && positive("width")
                            && positive("height"),
                        codec_name: stream
                            .get("codec_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        codec_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn resolve_tools(options: &MetadataRunOptions) -> Result<Tools, String> {
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
    };
    if !missing.is_empty() {
        return Err(format!(
            "Metadata-only MKV output needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    Ok(tools)
}

// ------------------------------------------------------------------- running

/// Per-file facts shared by every progress event.
struct FileFrame<'a> {
    source: &'a Path,
    number: usize,
    total: usize,
    started: Instant,
    input_bytes: Option<u64>,
}

fn emit(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    output_bytes: Option<u64>,
) {
    let FileFrame {
        source,
        number,
        total,
        started,
        input_bytes,
    } = *frame;
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(MetadataProgress {
        current: number,
        total,
        file_name: source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_path: source.to_string_lossy().into_owned(),
        phase: phase.into(),
        file_percent,
        queue_percent: ((number.saturating_sub(1)) as f64 + fraction) / total.max(1) as f64 * 100.0,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        input_bytes,
        output_bytes,
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

/// Lines the desktop and MCP consumers read from engine output after a run.
pub(crate) fn closing_lines(job: &MetadataJobResult) -> Vec<String> {
    let total = job.files.len();
    let summary = if job.stop_requested {
        format!(
            "Metadata-only MKV stopped after {total} file{}; remaining files were not changed.",
            if total == 1 { "" } else { "s" }
        )
    } else if job.failed > 0 {
        format!(
            "Metadata-only MKV finished: {} validated, {} failed safely.",
            job.succeeded, job.failed
        )
    } else if total == 0 {
        "Metadata-only MKV had no files to process.".to_string()
    } else {
        format!(
            "Metadata-only MKV finished: {} validated output{}. Originals stayed in place.",
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

pub(crate) fn run_metadata_only(
    options: &MetadataRunOptions,
    mut on_progress: impl FnMut(MetadataProgress),
    mut on_line: impl FnMut(String),
    mut on_child: impl FnMut(Option<u32>),
) -> Result<MetadataJobResult, String> {
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
    let log_dir = crate::state_log_directory()?;
    fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the run log folder: {error}"))?;
    let log_path = log_dir.join(format!("hevc-rebuild-{stamp}-metadata.log"));
    let report_path = log_dir.join(format!("hevc-rebuild-{stamp}-metadata.csv"));
    let mut log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create the metadata run log: {error}"))?;
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
        if backed_up && !options.reprocess {
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
            "No eligible media files remain for Metadata-only MKV.".into(),
            &mut on_line,
        );
        return Ok(MetadataJobResult {
            files: Vec::new(),
            log_path,
            report_path,
            stop_requested: false,
            failed: 0,
            succeeded: 0,
            skipped,
        });
    }
    validate_output_destinations(&eligible, options)?;
    let total = eligible.len();
    let total_bytes: u64 = eligible
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    note(
        format!(
            "Metadata-only MKV · {total} file{} · {} · no video encoder will run",
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
            "METADATA_OK" => succeeded += 1,
            "INTERRUPTED" => {
                stop_requested = true;
            }
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
    Ok(MetadataJobResult {
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
) -> MetadataFileResult {
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
            } else {
                "FAILED"
            };
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
                "Metadata work directory retained: {}",
                work_dir.display()
            ));
        } else {
            let _ = fs::remove_dir_all(&work_dir);
        }
    }
    MetadataFileResult {
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

fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
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
    emit(context, frame, "Reading media tracks", 0.0, None, None);

    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: false,
    })?;
    let probe = response
        .get("probe")
        .cloned()
        .ok_or("The profiler returned no stream probe.")?;
    let mediainfo = response
        .get("mediainfo")
        .cloned()
        .ok_or("The profiler returned no MediaInfo.")?;
    let profile = response
        .get("profile")
        .cloned()
        .ok_or("The profiler returned no profile.")?;

    let streams = source_streams(&probe);
    let mut unsupported: Vec<&str> = streams
        .iter()
        .map(|stream| stream.codec_type.as_str())
        .filter(|kind| !SUPPORTED_STREAM_TYPES.contains(kind))
        .collect();
    unsupported.sort_unstable();
    unsupported.dedup();
    if !unsupported.is_empty() {
        return Err(format!(
            "This source contains stream type(s) that cannot yet be verified for a lossless remux: {}. The original was left untouched.",
            unsupported.join(", ")
        ));
    }
    let real_videos: Vec<&SourceStream> =
        streams.iter().filter(|stream| stream.real_video).collect();
    if real_videos.is_empty() {
        return Err(
            "No real video track was found; metadata-only MKV output requires a video stream."
                .into(),
        );
    }
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .ok_or("The profiler did not select a primary video stream.")?;
    let mut video_profiles: BTreeMap<i64, Value> = BTreeMap::new();
    video_profiles.insert(primary_index, profile.clone());
    let others: Vec<i64> = real_videos
        .iter()
        .map(|stream| stream.index)
        .filter(|index| *index != primary_index)
        .collect();
    if !others.is_empty() {
        let tracks = engine_track_profiles(EngineTrackProfilesRequest {
            path: source.to_string_lossy().into_owned(),
            ffprobe: tools.ffprobe.clone(),
            promote_8bit_to_10bit: false,
            video_stream_indices: others.clone(),
            probe: probe.clone(),
            mediainfo,
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
        }
    }

    // HDR10+ indicated by metadata must be proven in the source before remux.
    let mut verified_hdr10plus: BTreeSet<i64> = BTreeSet::new();
    let hinted: Vec<&SourceStream> = real_videos
        .iter()
        .copied()
        .filter(|stream| {
            flag(
                video_profiles.get(&stream.index).and_then(|p| p.get("hdr")),
                "hdr10plus_hint",
            )
        })
        .collect();
    if !hinted.is_empty() {
        emit(context, frame, "Checking HDR10+ metadata", 0.02, None, None);
    }
    for stream in hinted {
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
        if !verified {
            return Err(format!(
                "HDR10+ is indicated on video stream #{}, but it could not be verified before remux. No output was installed.",
                stream.index
            ));
        }
        verified_hdr10plus.insert(stream.index);
    }

    let hdr_of = |key: &str| video_profiles.values().any(|p| flag(p.get("hdr"), key));
    let resolution = format!(
        "{}x{}",
        profile
            .pointer("/video/width")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        profile
            .pointer("/video/height")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    );
    let count = |kind: &str| streams.iter().filter(|s| s.codec_type == kind).count();
    for (key, value) in [
        (
            "primary_video",
            profile
                .pointer("/video/codec")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("resolution", json!(resolution)),
        ("audio_count", json!(count("audio"))),
        ("subtitle_count", json!(count("subtitle"))),
        ("hdr10", json!(hdr_of("hdr10"))),
        ("hdr10plus", json!(!verified_hdr10plus.is_empty())),
        ("dolby_vision", json!(hdr_of("dolby_vision"))),
        ("input_bytes", json!(input_bytes)),
    ] {
        state.row.insert(key.into(), value);
    }

    let final_output = metadata_output_path(source, options)?;
    if final_output == source {
        return Err(
            "Metadata-only output must be a new file; the source path cannot be the destination."
                .into(),
        );
    }
    if final_output.exists() {
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
        output_parent.join(format!(".{stem}.hevc-metadata.{}.tmp.mkv", unique_token()));
    state.final_output = Some(final_output.clone());
    state.temp_output = Some(temp_output.clone());
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));

    state.phase = "storage preflight";
    emit(context, frame, "Checking free disk space", 0.05, None, None);
    let free = disk_free_bytes(&output_parent).ok_or_else(|| {
        format!(
            "Cannot determine free space for output folder {}",
            output_parent.display()
        )
    })?;
    storage_preflight(input_bytes.unwrap_or(0), free, options.min_free_disk_gib)?;

    state.phase = "stream remux";
    (context.line)(format!(
        "Metadata-only output: all {} source streams will be copied to {}; no video encoder will run.",
        streams.len(),
        final_output.display()
    ));
    emit(context, frame, "Copying streams", 0.08, None, None);
    if let Err(error) = run_mkvmerge(context, frame, &temp_output) {
        state.interrupted = options
            .stop_file
            .as_deref()
            .is_some_and(crate::stop_control::requested);
        return Err(error);
    }

    state.phase = "metadata embedding";
    emit(
        context,
        frame,
        "Embedding metadata and poster",
        0.90,
        None,
        file_size(&temp_output),
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
    for warning in applied
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(text) = warning.as_str() {
            (context.line)(format!("Metadata warning: {text}"));
        }
    }
    let expect_poster = applied
        .get("poster_path")
        .is_some_and(|poster| !poster.is_null());

    state.phase = "lossless output validation";
    (context.line)(
        "Checking every copied stream, packet payload, track flag, chapter, and HDR profile".into(),
    );
    emit(
        context,
        frame,
        "Validating copied tracks and metadata",
        0.97,
        None,
        file_size(&temp_output),
    );
    let mut targets = Map::new();
    let mut expectations = Map::new();
    for stream in &real_videos {
        let track = &video_profiles[&stream.index];
        if let Some(format) = track
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
        {
            targets.insert(stream.index.to_string(), json!(format));
        }
        expectations.insert(
            stream.index.to_string(),
            hdr_expectation(track, verified_hdr10plus.contains(&stream.index)),
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
        "selected_stream_indices": streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        "encode_video_indices": [],
        "video_target_formats": targets,
        "video_hdr_expectations": expectations,
        "verify_copy_hashes": true,
        "test_mode": false,
        "copy_only": true,
        "expect_poster": expect_poster,
        "source_duration": profile.get("duration_seconds").and_then(Value::as_f64).unwrap_or(0.0),
        "source_chapters_count": profile.get("chapters_count").and_then(Value::as_u64).unwrap_or(0),
        "scrubbed_text_values": scrubbed,
        "chapter_text_edits": applied.get("chapter_text_edits").cloned().unwrap_or_else(|| json!([])),
    }))?;
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
    if validation
        .get("copied_hashes_verified")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("The output was not accepted because compressed source-stream payload hashes were not verified.".into());
    }

    state.phase = "validated output install";
    install_export_output(&temp_output, &final_output)?;
    state.temp_output = None;
    let final_bytes = file_size(&final_output);
    state.row.insert("status".into(), json!("METADATA_OK"));
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    state.row.insert("output_bytes".into(), json!(final_bytes));
    state.row.insert(
        "message".into(),
        json!("Metadata-only MKV validated; every source stream was copied and the original stayed in place."),
    );
    emit(
        context,
        frame,
        "Validated output ready",
        1.0,
        Some(100.0),
        final_bytes,
    );
    (context.line)(format!(
        "Metadata-only MKV ready: {}",
        final_output
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    Ok(())
}

/// Stream-copy every source track with live percent progress. The child owns a
/// process group so an abort can kill it without orphaning helpers.
fn run_mkvmerge(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    output: &Path,
) -> Result<(), String> {
    let source = frame.source;
    let (number, total) = (frame.number, frame.total);
    let mut command = Command::new(&context.tools.mkvmerge);
    command
        .args(["--gui-mode", "--output"])
        .arg(output)
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_owned(&mut command)
        .map_err(|error| format!("Could not start MKVToolNix remux: {error}"))?;
    (context.child)(Some(child.id()));
    let stderr_reader = child.stderr.take().map(|stream| {
        thread::spawn(move || {
            let mut tail: Vec<String> = Vec::new();
            let mut reader = BufReader::new(stream);
            let mut buffer = Vec::new();
            while matches!(reader.read_until(b'\n', &mut buffer), Ok(read) if read > 0) {
                let line = String::from_utf8_lossy(&buffer).into_owned();
                let keep = line.chars().rev().take(2048).collect::<Vec<_>>();
                tail.push(keep.into_iter().rev().collect());
                if tail.len() > 100 {
                    tail.remove(0);
                }
                buffer.clear();
            }
            tail.concat()
        })
    });
    let mut last_percent: i64 = -1;
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();
        while matches!(reader.read_until(b'\n', &mut buffer), Ok(read) if read > 0) {
            let line = String::from_utf8_lossy(&buffer).into_owned();
            buffer.clear();
            let Some(rest) = line.strip_prefix("#GUI#progress ") else {
                continue;
            };
            let percent = rest
                .split('%')
                .next()
                .and_then(|digits| digits.trim().parse::<i64>().ok())
                .map(|value| value.clamp(0, 100));
            if let Some(percent) = percent.filter(|value| *value != last_percent) {
                last_percent = percent;
                emit(
                    context,
                    frame,
                    "Copying streams",
                    0.85 * percent as f64 / 100.0,
                    Some(percent as f64),
                    file_size(output),
                );
                (context.line)(format!(
                    "Copying streams without re-encoding · file {number}/{total} · {percent}%"
                ));
            }
        }
    }
    let status = child.wait();
    let stderr_tail = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    (context.child)(None);
    let status = status.map_err(|error| format!("Could not wait for MKVToolNix: {error}"))?;
    if !status.success() {
        let detail = stderr_tail.trim();
        let code = status
            .code()
            .map_or_else(|| "terminated".to_string(), |code| code.to_string());
        let tail: String = detail
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "MKVToolNix could not copy all streams (exit {code}).{}",
            if tail.is_empty() {
                String::new()
            } else {
                format!(" {tail}")
            }
        ));
    }
    if file_size(output).unwrap_or(0) == 0 {
        return Err("MKVToolNix completed without producing a non-empty output file.".into());
    }
    Ok(())
}

// ------------------------------------------------------------ failure cleanup

/// Remove unfinished work; retain a completed late-failure output for review.
fn release_temporary_output(
    state: &mut FileState,
    source: &Path,
    failure_detail: Option<&str>,
    options: &MetadataRunOptions,
) {
    let status = state
        .row
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("FAILED")
        .to_string();
    let succeeded = status == "METADATA_OK";
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
        // Install can finish its rename and then fail on a directory sync.
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

/// Move a finished output to a visible `.error.mkv` name without overwriting.
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
    let original_note = if [
        "metadata embedding",
        "output validation",
        "lossless output validation",
    ]
    .contains(&phase)
    {
        "The original source was still protected because validation had not accepted this output."
    } else {
        "Review the original and any old/backup path before retrying; this run did not report a successful replacement."
    };
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
        format!("Error: {}", failure_detail.filter(|text| !text.is_empty()).unwrap_or(if message.is_empty() { "unknown failure" } else { message })),
        format!("Message: {}", if message.is_empty() { "none" } else { message }),
        "Outcome: the completed output was retained for investigation but was not accepted as a successful rebuild.".into(),
        original_note.into(),
        "Do not delete or retry over these files until this diagnostic and the media have been reviewed.".into(),
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

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hevc-metadata-run-{label}-{}", unique_token()));
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    fn options_for(root: &Path) -> MetadataRunOptions {
        MetadataRunOptions {
            root: root.to_path_buf(),
            minimum_size_mib: 0,
            embed_artwork: true,
            ..MetadataRunOptions::default()
        }
    }

    #[test]
    fn engine_arguments_cover_the_gui_vocabulary() {
        let root = scratch("args");
        let args: Vec<String> = [
            root.to_str().unwrap(),
            "--min-size-mib",
            "0",
            "--no-dependency-repair",
            "--skip-update-check",
            "--recursive",
            "--include-filename-pattern",
            "*.mkv",
            "--exclude-filename-pattern",
            "*sample*",
            "--output-dir",
            root.to_str().unwrap(),
            "--crf",
            "20",
            "--preset",
            "slow",
            "--tune",
            "auto",
            "--detect-animation",
            "--no-promote-8bit-to-10bit",
            "--preserve-hdr10plus",
            "--verify-copy-hashes",
            "--comment",
            "Hello",
            "--no-embed-artwork",
            "--replace-metadata-text-blacklist",
            "--metadata-text-blacklist",
            "Movie*Group",
            "--metadata-text-blacklist",
            "movie*group",
            "--min-free-disk-gib",
            "2.5",
            "--preflight-output-ratio",
            "0.5",
            "--stop-on-error",
            "--metadata-only",
            "--ffmpeg-path",
            "/usr/bin/ffmpeg",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let options = MetadataRunOptions::from_engine_args(&args).unwrap();
        assert!(options.recursive && options.stop_on_error);
        assert_eq!(options.minimum_size_mib, 0);
        assert_eq!(options.comment, "Hello");
        assert!(!options.embed_artwork);
        assert_eq!(options.text_blacklist, vec!["Movie*Group".to_string()]);
        assert_eq!(options.min_free_disk_gib, 2.5);
        assert_eq!(options.include, vec!["*.mkv".to_string()]);
        assert_eq!(
            options.tool_paths.ffmpeg.as_deref(),
            Some("/usr/bin/ffmpeg")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn every_settings_argument_the_app_sends_is_understood() {
        let root = scratch("settings-args");
        let settings = effective_settings(&root).unwrap();
        let mut args = vec![root.to_string_lossy().into_owned()];
        args.extend(crate::settings_arguments::settings_arguments(&settings).unwrap());
        args.extend(
            crate::settings_arguments::metadata_text_blacklist_arguments(&settings).unwrap(),
        );
        args.extend([
            "--metadata-only".into(),
            "--stop-request-file".into(),
            "/tmp/stop".into(),
        ]);
        let options = MetadataRunOptions::from_engine_args(&args).unwrap();
        assert_eq!(options.root, root);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_and_encode_only_flags_are_rejected() {
        let root = scratch("reject");
        let base = root.to_str().unwrap().to_string();
        for (extra, needle) in [
            (vec!["--mystery-flag"], "do not recognise"),
            (
                vec!["--file-stream-overrides-json", "{}"],
                "preserves every source stream",
            ),
            (vec!["--file-settings-overrides-json", "{}"], "do not apply"),
        ] {
            let mut args = vec![base.clone()];
            args.extend(extra.into_iter().map(str::to_owned));
            let error = MetadataRunOptions::from_engine_args(&args).err().unwrap();
            assert!(error.contains(needle), "{error}");
        }
        assert!(MetadataRunOptions::from_engine_args(&[]).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn output_paths_never_resolve_to_the_source() {
        let root = scratch("paths");
        let season = root.join("Show").join("Season 1");
        fs::create_dir_all(&season).unwrap();
        let source = season.join("Episode.One.mkv");
        fs::write(&source, b"x").unwrap();
        let mut options = options_for(&root);
        assert_eq!(
            metadata_output_path(&source, &options).unwrap(),
            season.join("Episode.One.metadata.mkv")
        );
        // A separate folder mirrors the relative layout for folder inputs.
        let out = scratch("paths-out");
        options.output_dir = Some(out.clone());
        assert_eq!(
            metadata_output_path(&source, &options).unwrap(),
            out.join("Show").join("Season 1").join("Episode.One.mkv")
        );
        // A per-file folder stores the file directly in that folder.
        let direct = scratch("paths-direct");
        options
            .file_output_dirs
            .insert(source.clone(), direct.clone());
        assert_eq!(
            metadata_output_path(&source, &options).unwrap(),
            direct.join("Episode.One.mkv")
        );
        // Pointing the destination back at the source folder avoids the source name.
        let mut same = options_for(&season);
        same.output_dir = Some(season.clone());
        assert_eq!(
            metadata_output_path(&source, &same).unwrap(),
            season.join("Episode.One.hevc-rebuilt.mkv")
        );
        for dir in [root, out, direct] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn destination_validation_blocks_existing_and_duplicate_outputs() {
        let root = scratch("collide");
        let first = root.join("a.mp4");
        let second = root.join("a.mkv");
        fs::write(&first, b"x").unwrap();
        fs::write(&second, b"y").unwrap();
        let options = options_for(&root);
        // a.mp4 -> a.mkv exists already (the second source), so it must be refused.
        let error = validate_output_destinations(std::slice::from_ref(&first), &options)
            .err()
            .unwrap();
        assert!(error.contains("will not be overwritten"), "{error}");
        // Two sources sharing one planned output in a separate folder collide.
        let out = scratch("collide-out");
        let mut separate = options_for(&root);
        separate.output_dir = Some(out.clone());
        let error = validate_output_destinations(&[first, second], &separate)
            .err()
            .unwrap();
        assert!(error.contains("same output"), "{error}");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn storage_preflight_matches_the_copy_budget() {
        let one_gib = 1024 * MIB;
        assert!(storage_preflight(10 * one_gib, one_gib, 0.0).is_ok());
        let error = storage_preflight(10 * one_gib, 400 * MIB, 0.0).unwrap_err();
        assert!(error.contains("Insufficient free space"), "{error}");
        assert!(storage_preflight(MIB, 100 * MIB, 1.0).is_err());
        assert!(storage_preflight(MIB, 2 * one_gib, 1.0).is_ok());
    }

    #[test]
    fn hdr_expectation_reads_both_profile_shapes() {
        let profile = json!({
            "color_primaries": "bt2020",
            "color_transfer": "smpte2084",
            "color_matrix": "bt2020nc",
            "mastering_display": "G(1,2)",
            "max_cll": 1000,
            "max_fall": 400,
            "hdr": {"hdr10": true, "hlg": false, "dolby_vision": true,
                    "dolby_vision_profile": 8, "dolby_vision_rpu": true,
                    "dolby_vision_enhancement_layer": false,
                    "dolby_vision_compatibility_id": 1}
        });
        let value = hdr_expectation(&profile, true);
        assert_eq!(value["transfer"], "smpte2084");
        assert_eq!(value["hdr10plus"], true);
        assert_eq!(value["dovi_profile"], 8);
        assert_eq!(value["dovi_compatibility_id"], 1);
        assert_eq!(value["max_cll"], 1000);
        assert_eq!(value["preserve_dolbyvision"], false);
    }

    #[test]
    fn retained_output_gets_error_names_and_a_private_log() {
        let dir = scratch("retain");
        let source = dir.join("Film.mkv");
        let final_output = dir.join("Film.metadata.mkv");
        fs::write(&source, b"source").unwrap();
        let temp = dir.join(".Film.metadata.hevc-metadata.abc.tmp.mkv");
        fs::write(&temp, b"finished but unvalidated").unwrap();
        let mut state = FileState {
            phase: "lossless output validation",
            temp_output: Some(temp.clone()),
            final_output: Some(final_output.clone()),
            row: Map::new(),
            interrupted: false,
        };
        state.row.insert("status".into(), json!("FAILED"));
        state.row.insert("message".into(), json!("hash mismatch"));
        release_temporary_output(
            &mut state,
            &source,
            Some("hash mismatch"),
            &options_for(&dir),
        );
        let retained = dir.join("Film.metadata.error.mkv");
        assert!(retained.is_file());
        assert!(!temp.exists());
        assert!(!final_output.exists());
        let log = fs::read_to_string(dir.join("Film.metadata.error.log")).unwrap();
        assert!(log.contains("Phase: lossless output validation"));
        assert!(log.contains("hash mismatch"));
        assert!(state.row["message"]
            .as_str()
            .unwrap()
            .contains("remains unvalidated"));
        // A second failure never overwrites the first review artifacts.
        let again = dir.join(".again.tmp.mkv");
        fs::write(&again, b"second").unwrap();
        let second = keep_finished_output(&again, Some(&final_output)).unwrap();
        assert_eq!(second, dir.join("Film.metadata.error.1.mkv"));
        assert_eq!(fs::read(&retained).unwrap(), b"finished but unvalidated");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn early_failures_discard_the_temporary_file_only() {
        let dir = scratch("discard");
        let source = dir.join("Film.mkv");
        fs::write(&source, b"source").unwrap();
        let temp = dir.join(".Film.tmp.mkv");
        fs::write(&temp, b"partial").unwrap();
        let mut state = FileState {
            phase: "stream remux",
            temp_output: Some(temp.clone()),
            final_output: Some(dir.join("Film.metadata.mkv")),
            row: Map::new(),
            interrupted: false,
        };
        state.row.insert("status".into(), json!("FAILED"));
        release_temporary_output(&mut state, &source, None, &options_for(&dir));
        assert!(!temp.exists());
        assert!(source.is_file());
        let _ = fs::remove_dir_all(dir);
    }

    fn tool_available(name: &str) -> bool {
        Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    fn make_normalized_synthetic_clip(root: &Path, file_name: &str) -> Option<PathBuf> {
        let source = root.join(file_name);
        let raw = root.join("raw-fixture.mkv");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=24:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx265",
                "-pix_fmt",
                "yuv420p10le",
                "-preset",
                "ultrafast",
                "-x265-params",
                "log-level=none",
                "-c:a",
                "aac",
                "-shortest",
                "-y",
            ])
            .arg(&raw)
            .status();
        let normalised = made.is_ok_and(|status| status.success())
            && Command::new("mkvmerge")
                .arg("-o")
                .arg(&source)
                .arg(&raw)
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|status| status.code().is_some_and(|code| code <= 1));
        let _ = fs::remove_file(&raw);
        if normalised {
            Some(source)
        } else {
            let _ = fs::remove_file(&source);
            None
        }
    }

    /// End to end on a synthetic clip: needs ffmpeg + MKVToolNix + MediaInfo and is
    /// skipped (with a note) on machines without them.
    #[test]
    fn synthetic_clip_round_trips_through_native_metadata_only() {
        for tool in [
            "ffmpeg",
            "ffprobe",
            "mkvmerge",
            "mkvpropedit",
            "mkvextract",
            "mediainfo",
        ] {
            if !tool_available(tool) {
                eprintln!("skipping native metadata-only integration: {tool} is unavailable");
                return;
            }
        }
        let _state_guard = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state_dir = scratch("state");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
        let root = scratch("clip");
        let Some(source) = make_normalized_synthetic_clip(&root, "Clip.One (2024 PG).mkv") else {
            eprintln!("skipping native metadata-only integration: could not build the fixture");
            let _ = fs::remove_dir_all(root);
            return;
        };
        let before = fs::read(&source).unwrap();
        let mut overrides = Map::new();
        overrides.insert(
            source.to_string_lossy().into_owned(),
            json!({"title": "Clip One", "year": "2024", "plot": "A synthetic test pattern."}),
        );
        let args = vec![
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            "0".into(),
            "--comment".into(),
            "native test".into(),
            "--file-metadata-overrides-json".into(),
            Value::Object(overrides).to_string(),
        ];
        let options = MetadataRunOptions::from_engine_args(&args).unwrap();
        let mut phases = Vec::new();
        let result = run_metadata_only(
            &options,
            |progress| phases.push(progress.phase),
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(result.failed, 0, "{:?}", result.files);
        assert_eq!(result.succeeded, 1);
        let output = root.join("Clip.One (2024 PG).metadata.mkv");
        assert!(output.is_file(), "missing {}", output.display());
        assert_eq!(
            fs::read(&source).unwrap(),
            before,
            "the source must stay byte-identical"
        );
        assert!(phases
            .iter()
            .any(|phase| phase == "Validating copied tracks and metadata"));
        assert_eq!(
            phases.last().map(String::as_str),
            Some("Validated output ready")
        );
        let tags = Command::new("mkvextract")
            .arg(&output)
            .args(["tags", "-"])
            .output()
            .unwrap();
        let xml = String::from_utf8_lossy(&tags.stdout);
        assert!(xml.contains("native test"), "comment tag missing: {xml}");
        assert!(
            xml.contains("A synthetic test pattern."),
            "plot tag missing: {xml}"
        );
        let report = fs::read_to_string(&result.report_path).unwrap();
        assert!(report.contains("METADATA_OK"));
        // A rerun refuses to overwrite the validated output and leaves it intact.
        let again = MetadataRunOptions::from_engine_args(&args).unwrap();
        let error = run_metadata_only(&again, |_| {}, |_| {}, |_| {})
            .err()
            .unwrap();
        assert!(error.contains("will not be overwritten"), "{error}");
        assert!(output.is_file());
        for dir in [root, state_dir] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    /// Default metadata-only behavior must validate a normalized native MKV
    /// without requiring an environment-selected real-media fixture.
    #[test]
    fn synthetic_default_media_sample_validates_natively() {
        for tool in [
            "ffmpeg",
            "ffprobe",
            "mkvmerge",
            "mkvpropedit",
            "mkvextract",
            "mediainfo",
        ] {
            if !tool_available(tool) {
                eprintln!("skipping native metadata-only integration: {tool} is unavailable");
                return;
            }
        }
        let _state_guard = crate::TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state_dir = scratch("default-state");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
        let root = scratch("default-clip");
        let Some(source) = make_normalized_synthetic_clip(&root, "Default Clip.mkv") else {
            eprintln!("skipping native metadata-only integration: could not build the fixture");
            let _ = fs::remove_dir_all(&root);
            let _ = fs::remove_dir_all(&state_dir);
            return;
        };
        let before = fs::read(&source).unwrap();
        let args = vec![
            root.to_string_lossy().into_owned(),
            "--min-size-mib".into(),
            "0".into(),
            "--no-embed-artwork".into(),
        ];
        let options = MetadataRunOptions::from_engine_args(&args).unwrap();
        let result = run_metadata_only(&options, |_| {}, |_| {}, |_| {}).unwrap();
        assert_eq!(result.failed, 0, "{:?}", result.files);
        assert_eq!(result.succeeded, 1);
        assert!(root.join("Default Clip.metadata.mkv").is_file());
        assert_eq!(fs::read(&source).unwrap(), before);
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(state_dir);
    }
}
