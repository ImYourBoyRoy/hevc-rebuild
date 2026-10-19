//! Rust-owned, read-only media discovery and stream profiling.
//!
//! Python remains the parity reference for the encoder and transactional run.
//! GUI, native JSON CLI, and MCP scans share this module and therefore no longer
//! need Python to discover files, read FFprobe data, or calculate preflight plans.

use crate::resource_plan::{memory_forecast, memory_full_avg10};
use crate::{environment, settings, ToolPaths};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm",
];
const EXCLUDED_DIRS: &[&str] = &[".hevc-rebuild", ".git", "node_modules", "youtube uploads"];
const MAX_DISCOVERED_FILES: usize = 10_000;
const MAX_VIDEO_PROFILES: usize = 1024;
const MAX_TOOL_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENGINE_PROFILE_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES: u64 = 68 * 1024 * 1024;
const MAX_ENGINE_PROFILE_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ScanOptions {
    pub(crate) root: String,
    pub(crate) recursive: bool,
    pub(crate) minimum_size_mib: u32,
    pub(crate) filename_include_patterns: Vec<String>,
    pub(crate) filename_exclude_patterns: Vec<String>,
    pub(crate) tool_paths: ToolPaths,
}

#[derive(Default)]
struct Discovery {
    files: Vec<PathBuf>,
    warnings: Vec<String>,
    limit_reached: bool,
}

struct CapturedJson {
    value: Value,
}

#[derive(serde::Serialize, Deserialize)]
struct Profile {
    payload: Value,
    duration: f64,
}

/// Diagnostic-only timings; never used for media or resource policy decisions.
#[derive(Default, serde::Serialize)]
struct ScanTimings {
    discovery_ms: f64,
    settings_ms: f64,
    tool_poll_ms: f64,
    ffprobe_ms: f64,
    mediainfo_ms: f64,
    profile_ms: f64,
    forecast_ms: f64,
    progress_handoff_ms: f64,
    total_ms: f64,
    profile_cache_hits: usize,
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn cache_identity(path: &Path) -> Option<Value> {
    let canonical = path.canonicalize().ok()?;
    let metadata = fs::metadata(&canonical).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let mut identity = json!({"path": canonical, "size": metadata.len(), "mtime_ns": modified});
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        identity["inode"] = json!(metadata.ino());
        identity["device"] = json!(metadata.dev());
        identity["ctime"] = json!([metadata.ctime(), metadata.ctime_nsec()]);
    }
    Some(identity)
}

fn profile_cache_key(
    path: &Path,
    ffprobe: &str,
    mediainfo: Option<&str>,
    settings: &Value,
    host: &Value,
) -> Option<String> {
    let mediainfo = mediainfo?;
    serde_json::to_string(&json!({
        "source": cache_identity(path)?,
        "ffprobe": cache_identity(Path::new(ffprobe))?,
        "mediainfo": cache_identity(Path::new(mediainfo))?,
        "versions": host.get("checks")?.as_array()?.iter().filter(|check| matches!(check.get("key").and_then(Value::as_str), Some("ffprobe" | "mediainfo"))).collect::<Vec<_>>(),
        "settings": settings,
        "policy": crate::profile_cache::POLICY_REVISION,
    }))
    .ok()
}

/// Analyze a file or folder. The callback reports each file before its profile
/// is read so the GUI and MCP can expose bounded, useful progress.
pub(crate) fn scan_media(
    options: ScanOptions,
    mut progress: impl FnMut(usize, usize, &str),
) -> Result<Value, String> {
    let total_start = Instant::now();
    let mut timings = ScanTimings::default();
    if options.minimum_size_mib > 4096 {
        return Err("Minimum input size must be 4096 MiB or less.".into());
    }
    validate_filename_patterns(&options.filename_include_patterns, "include")?;
    validate_filename_patterns(&options.filename_exclude_patterns, "exclude")?;
    let root = expand_home(Path::new(&options.root))
        .canonicalize()
        .map_err(|error| format!("Media input is unavailable: {error}"))?;
    if !root.is_file() && !root.is_dir() {
        return Err("Media input must be a regular file or folder.".into());
    }

    let input_is_file = root.is_file();
    let source_base = if input_is_file {
        root.parent().unwrap_or(&root).to_path_buf()
    } else {
        root.clone()
    };
    let minimum_bytes = u64::from(options.minimum_size_mib) * 1024 * 1024;
    let discovery_start = Instant::now();
    let mut discovery = if input_is_file {
        discover_file(&root, minimum_bytes)
    } else {
        discover_folder(
            &root,
            options.recursive,
            minimum_bytes,
            &options.filename_exclude_patterns,
        )
    };
    let discovered_count = discovery.files.len();
    discovery.files.retain(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        filename_matches_filters(
            &name,
            &options.filename_include_patterns,
            &options.filename_exclude_patterns,
        )
    });
    let filtered_count = discovered_count.saturating_sub(discovery.files.len());
    if filtered_count > 0 {
        discovery.warnings.push(format!(
            "Skipped {filtered_count} file(s) using filename filters."
        ));
    }
    timings.discovery_ms = elapsed_ms(discovery_start);
    let settings_start = Instant::now();
    let settings_payload = serde_json::to_value(settings::load_effective_settings(
        &source_base.to_string_lossy(),
    )?)
    .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    let effective_settings = settings_payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let settings_sources = settings_payload
        .get("sources")
        .cloned()
        .unwrap_or_else(|| json!([]));

    timings.settings_ms = elapsed_ms(settings_start);
    let tools_start = Instant::now();
    let mut host = environment::inspect_environment(options.tool_paths, None)?;
    timings.tool_poll_ms = elapsed_ms(tools_start);
    let ffprobe = checked_tool(&host, "ffprobe");
    let mediainfo = checked_tool(&host, "mediainfo");
    let can_probe = ffprobe.is_some();
    let can_enrich = mediainfo.is_some();
    if let Some(object) = host.as_object_mut() {
        // Keep the older can_analyze contract (FFprobe + MediaInfo); these
        // fields make partial FFprobe-only analysis explicit to new clients.
        object.insert("can_probe_media".into(), json!(can_probe));
        object.insert("can_enrich_with_mediainfo".into(), json!(can_enrich));
    }
    let full_memory_psi = memory_full_avg10();
    let total = discovery.files.len();
    let mut total_input_bytes = 0_u64;
    let mut entries = Vec::with_capacity(total);

    for (offset, path) in discovery.files.iter().enumerate() {
        let current = offset + 1;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let handoff_start = Instant::now();
        progress(current, total, &name);
        timings.progress_handoff_ms += elapsed_ms(handoff_start);
        let media_metadata = fs::metadata(path).ok();
        let size_bytes = media_metadata
            .as_ref()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let modified_unix_ms = media_metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
        total_input_bytes = total_input_bytes.saturating_add(size_bytes);
        let already_processed = path
            .parent()
            .map(|parent| {
                parent
                    .join("old")
                    .join(path.file_name().unwrap_or_default())
                    .exists()
            })
            .unwrap_or(false);
        let relative_path = if input_is_file {
            name.to_string()
        } else {
            path.strip_prefix(&source_base)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        };
        let mut entry = json!({
            "path": path.to_string_lossy(),
            "relative_path": relative_path,
            "name": name,
            "size_bytes": size_bytes,
            "modified_unix_ms": modified_unix_ms,
            "status": "Tools required",
            "already_processed": already_processed,
            "profile": Value::Null,
            "resource_forecast": Value::Null,
            "youtube_estimates": Value::Null,
            "youtube_resource_forecasts": Value::Null,
            "error": Value::Null
        });

        let Some(ffprobe) = ffprobe.as_deref() else {
            entry["error"] = json!("FFprobe is unavailable. Select an FFprobe executable or install FFmpeg/FFprobe to inspect streams.");
            entries.push(entry);
            continue;
        };
        let cache_key = profile_cache_key(
            path,
            ffprobe,
            mediainfo.as_deref(),
            &effective_settings,
            &host,
        );
        let cached = cache_key
            .as_deref()
            .and_then(crate::profile_cache::get)
            .and_then(|value| serde_json::from_value::<Profile>(value).ok());
        let analyzed = if let Some(profile) = cached {
            timings.profile_cache_hits += 1;
            Ok((profile, None))
        } else {
            probe_media(
                ffprobe,
                mediainfo.as_deref(),
                path,
                &effective_settings,
                &mut timings,
            )
        };
        let analyzed = match analyzed {
            Ok((profile, error)) => {
                if let Some(key) = cache_key {
                    if Some(&key)
                        != profile_cache_key(
                            path,
                            ffprobe,
                            mediainfo.as_deref(),
                            &effective_settings,
                            &host,
                        )
                        .as_ref()
                    {
                        Err(
                            "Media or analysis tool changed during profiling; refresh the scan."
                                .into(),
                        )
                    } else {
                        if error.is_none() {
                            if let Ok(value) = serde_json::to_value(&profile) {
                                crate::profile_cache::insert(key, value);
                            }
                        }
                        Ok((profile, error))
                    }
                } else {
                    Ok((profile, error))
                }
            }
            Err(error) => Err(error),
        };
        match analyzed {
            Ok((profile, media_info_error)) => {
                let forecast_start = Instant::now();
                let primary = profile.payload.clone();
                let rebuild_forecast = memory_forecast(
                    &primary,
                    &effective_settings,
                    &host,
                    full_memory_psi,
                    None,
                    None,
                    None,
                    1,
                );
                let mut estimates = Map::new();
                let mut youtube_forecasts = Map::new();
                for resolution in ["source", "1440p", "4k"] {
                    let dimensions = crate::youtube::expected_dimensions(&primary, resolution)?;
                    estimates.insert(
                        resolution.into(),
                        json!({
                            "balanced": crate::youtube::estimate(&primary, size_bytes, profile.duration, resolution, "balanced", 2.0)?,
                            "recommended": crate::youtube::estimate(&primary, size_bytes, profile.duration, resolution, "recommended", 2.0)?
                        }),
                    );
                    youtube_forecasts.insert(
                        resolution.into(),
                        memory_forecast(
                            &primary,
                            &effective_settings,
                            &host,
                            full_memory_psi,
                            Some(dimensions),
                            Some("slow"),
                            Some(0),
                            1,
                        ),
                    );
                }
                timings.forecast_ms += elapsed_ms(forecast_start);
                entry["profile"] = profile.payload;
                entry["resource_forecast"] = rebuild_forecast;
                entry["youtube_estimates"] = Value::Object(estimates);
                entry["youtube_resource_forecasts"] = Value::Object(youtube_forecasts);
                if can_enrich && media_info_error.is_none() {
                    entry["status"] = if already_processed {
                        "Backed up"
                    } else {
                        "Ready"
                    }
                    .into();
                } else {
                    entry["status"] = "Tools required".into();
                    entry["warning"] = json!(media_info_error.unwrap_or_else(|| {
                        "MediaInfo is unavailable; this FFprobe profile is partial. MediaInfo-specific fields may be missing.".into()
                    }));
                }
            }
            Err(error) => {
                entry["status"] = "Analysis failed".into();
                entry["error"] = json!(error);
            }
        }
        entries.push(entry);
    }

    let mut warnings = discovery.warnings;
    if discovery.limit_reached {
        warnings.push(format!(
            "Discovery stopped after {MAX_DISCOVERED_FILES} media files. Narrow the selected folder to analyze the remainder."
        ));
    }
    let mut payload = json!({
        "root": root.to_string_lossy(),
        "input_kind": if input_is_file { "file" } else { "folder" },
        "recursive": options.recursive,
        "minimum_size_mib": options.minimum_size_mib,
        "candidate_count": entries.len(),
        "total_input_bytes": total_input_bytes,
        "disk_free_bytes": disk_free_bytes(&source_base).unwrap_or(0),
        "entries": entries,
        "environment": host,
        "settings": effective_settings,
        "settings_sources": settings_sources,
        "warnings": warnings,
        "discovery_limit_reached": discovery.limit_reached
    });
    timings.total_ms = elapsed_ms(total_start);
    if std::env::var_os("HEVC_REBUILD_SCAN_TIMING").is_some_and(|value| value == "1") {
        payload["scan_timings"] =
            serde_json::to_value(&timings).map_err(|error| error.to_string())?;
    }
    Ok(payload)
}

pub(crate) fn validate_filename_patterns(patterns: &[String], label: &str) -> Result<(), String> {
    if patterns.len() > 100 {
        return Err(format!("Filename {label} list is limited to 100 patterns."));
    }
    let mut seen = BTreeSet::new();
    for (index, pattern) in patterns.iter().enumerate() {
        if pattern.is_empty() || pattern.trim() != pattern {
            return Err(format!(
                "Filename {label} pattern {} must not be empty or padded with spaces.",
                index + 1
            ));
        }
        if pattern.len() > 128 {
            return Err(format!(
                "Filename {label} pattern {} is limited to 128 UTF-8 bytes.",
                index + 1
            ));
        }
        if pattern
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        {
            return Err(format!(
                "Filename {label} patterns cannot contain control characters or path separators."
            ));
        }
        if pattern
            .chars()
            .all(|character| matches!(character, '*' | '?'))
        {
            return Err(format!(
                "Filename {label} pattern {} must include at least one filename character.",
                index + 1
            ));
        }
        if !seen.insert(pattern.to_lowercase()) {
            return Err(format!(
                "Filename {label} patterns cannot contain case-insensitive duplicates."
            ));
        }
    }
    Ok(())
}

pub(crate) fn filename_matches_filters(name: &str, include: &[String], exclude: &[String]) -> bool {
    let included = include.is_empty()
        || include
            .iter()
            .any(|pattern| filename_glob_matches(name, pattern));
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    included
        && !exclude.iter().any(|pattern| {
            filename_glob_matches(name, pattern) || filename_glob_matches(stem, pattern)
        })
}

fn filename_glob_matches(name: &str, pattern: &str) -> bool {
    let name = name
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let pattern = pattern
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let (mut name_index, mut pattern_index) = (0, 0);
    let mut last_star = None;
    let mut retry_name_index = 0;
    while name_index < name.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == name[name_index])
        {
            name_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            last_star = Some(pattern_index);
            pattern_index += 1;
            retry_name_index = name_index;
        } else if let Some(star) = last_star {
            retry_name_index += 1;
            name_index = retry_name_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

/// Rust-owned discovery adapter for compatibility-engine launches. The same
/// bounded walk powers GUI/MCP/JSON analysis, so application-launched CLI runs
/// no longer maintain a second Python discovery implementation.
pub(crate) fn discover_cli_mode() -> i32 {
    const MAX_REQUEST_BYTES: u64 = 64 * 1024;
    const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)
    {
        eprintln!("Could not read the native discovery request: {error}");
        return 2;
    }
    if input.len() as u64 > MAX_REQUEST_BYTES {
        eprintln!("Native discovery request exceeds 64 KiB.");
        return 2;
    }
    let request: Value = match serde_json::from_slice(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Native discovery request is invalid JSON: {error}");
            return 2;
        }
    };
    let result = (|| {
        let raw_root = request
            .get("root")
            .and_then(Value::as_str)
            .filter(|root| !root.is_empty() && root.len() <= 8192)
            .ok_or("Discovery request must contain a path no longer than 8192 bytes.")?;
        let recursive = request
            .get("recursive")
            .and_then(Value::as_bool)
            .ok_or("Discovery request must specify whether to search recursively.")?;
        let minimum_bytes = request
            .get("minimum_size_bytes")
            .and_then(Value::as_u64)
            .ok_or("Discovery request must contain a nonnegative minimum file size.")?;
        let excluded_names = request
            .get("filename_exclude_patterns")
            .and_then(Value::as_array)
            .map(|patterns| {
                patterns
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        validate_filename_patterns(&excluded_names, "exclude")?;
        let root = expand_home(Path::new(raw_root))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !root.is_file() && !root.is_dir() {
            return Err("Media input must be a regular file or folder.".to_owned());
        }
        let discovery = if root.is_file() {
            discover_file(&root, minimum_bytes)
        } else {
            discover_folder(&root, recursive, minimum_bytes, &excluded_names)
        };
        let payload = json!({
            "files": discovery.files.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
            "warnings": discovery.warnings,
            "limit_reached": discovery.limit_reached,
        });
        let output = serde_json::to_string(&payload)
            .map_err(|error| format!("Could not format the native discovery result: {error}"))?;
        if output.len() > MAX_RESPONSE_BYTES {
            return Err("Native discovery result exceeds the 64 MiB response limit; narrow the selected folder.".to_owned());
        }
        Ok(output)
    })();
    match result {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(text.strip_prefix("~/").unwrap_or(""));
        }
    }
    path.to_path_buf()
}

fn checked_tool(environment: &Value, key: &str) -> Option<String> {
    let check = environment
        .get("checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("key").and_then(Value::as_str) == Some(key))?;
    if check.get("available").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    check.get("path")?.as_str().map(str::to_owned)
}

fn excluded_directory(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXCLUDED_DIRS.contains(&lower.as_str())
}

fn is_season_directory(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower
        .strip_prefix("season")
        .map(str::trim_start)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|character| character.is_ascii_digit())
}

fn is_candidate(path: &Path, minimum_bytes: u64) -> bool {
    if path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.starts_with("Test_") || name.contains(".hevc-rebuild.")
    }) {
        return false;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    MEDIA_EXTENSIONS.contains(&extension.as_str())
        && fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.len() >= minimum_bytes)
            .unwrap_or(false)
}

fn discover_file(path: &Path, minimum_bytes: u64) -> Discovery {
    let mut result = Discovery::default();
    if is_candidate(path, minimum_bytes) {
        result.files.push(path.to_path_buf());
    }
    result
}

fn read_directory(path: &Path, result: &mut Discovery) -> Vec<PathBuf> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry.path()),
                Err(error) => {
                    result.warnings.push(format!(
                        "Could not read an item under {}: {error}",
                        path.display()
                    ));
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            result
                .warnings
                .push(format!("Could not read folder {}: {error}", path.display()));
            Vec::new()
        }
    };
    entries.sort_by_key(|entry| {
        entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    entries
}

fn add_candidate(
    path: &Path,
    minimum_bytes: u64,
    seen: &mut BTreeSet<PathBuf>,
    result: &mut Discovery,
) {
    if !is_candidate(path, minimum_bytes) {
        return;
    }
    if result.files.len() >= MAX_DISCOVERED_FILES {
        result.limit_reached = true;
        return;
    }
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if seen.insert(key) {
        result.files.push(path.to_path_buf());
    }
}

fn discover_folder(
    root: &Path,
    recursive: bool,
    minimum_bytes: u64,
    excluded_names: &[String],
) -> Discovery {
    let mut result = Discovery::default();
    let mut seen = BTreeSet::new();
    if recursive {
        let mut stack = vec![root.to_path_buf()];
        let mut visited = BTreeSet::new();
        while let Some(directory) = stack.pop() {
            let canonical = directory
                .canonicalize()
                .unwrap_or_else(|_| directory.clone());
            if !visited.insert(canonical) {
                continue;
            }
            let mut children = Vec::new();
            for path in read_directory(&directory, &mut result) {
                let link = fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false);
                if path.is_dir() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if !link
                        && !excluded_directory(&name)
                        && !excluded_names
                            .iter()
                            .any(|pattern| filename_glob_matches(&name, pattern))
                    {
                        children.push(path);
                    }
                } else {
                    add_candidate(&path, minimum_bytes, &mut seen, &mut result);
                }
                if result.limit_reached {
                    break;
                }
            }
            children.reverse();
            stack.extend(children);
            if result.limit_reached {
                break;
            }
        }
        result
            .files
            .sort_by_key(|path| path.to_string_lossy().to_lowercase());
    } else {
        let mut seasons = Vec::new();
        for path in read_directory(root, &mut result) {
            if path.is_dir() {
                let link = fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if link
                    || excluded_directory(&name)
                    || excluded_names
                        .iter()
                        .any(|pattern| filename_glob_matches(&name, pattern))
                {
                    continue;
                }
                if is_season_directory(&name) {
                    seasons.push(path);
                } else {
                    for child in read_directory(&path, &mut result) {
                        let child_link = fs::symlink_metadata(&child)
                            .map(|metadata| metadata.file_type().is_symlink())
                            .unwrap_or(false);
                        if child.is_dir()
                            && !child_link
                            && !excluded_names.iter().any(|pattern| {
                                filename_glob_matches(
                                    &child.file_name().unwrap_or_default().to_string_lossy(),
                                    pattern,
                                )
                            })
                            && is_season_directory(
                                &child.file_name().unwrap_or_default().to_string_lossy(),
                            )
                        {
                            seasons.push(child);
                        }
                    }
                }
            } else {
                add_candidate(&path, minimum_bytes, &mut seen, &mut result);
            }
            if result.limit_reached {
                break;
            }
        }
        seasons.sort_by_key(|path| path.to_string_lossy().to_lowercase());
        seasons.dedup();
        for season in seasons {
            for path in read_directory(&season, &mut result) {
                if !path.is_dir() {
                    add_candidate(&path, minimum_bytes, &mut seen, &mut result);
                }
                if result.limit_reached {
                    break;
                }
            }
            if result.limit_reached {
                break;
            }
        }
    }
    result
}

fn probe_media(
    ffprobe: &str,
    mediainfo: Option<&str>,
    path: &Path,
    settings: &Value,
    timings: &mut ScanTimings,
) -> Result<(Profile, Option<String>), String> {
    let ffprobe_start = Instant::now();
    let probe = run_json_tool(
        ffprobe,
        &[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-show_data_hash",
            "sha256",
            "-of",
            "json",
        ],
        path,
        Duration::from_secs(120),
        MAX_TOOL_JSON_BYTES,
    )?
    .value;
    timings.ffprobe_ms += elapsed_ms(ffprobe_start);
    let mediainfo_start = Instant::now();
    let (media_info, media_info_error) = match mediainfo {
        Some(program) => match run_json_tool(
            program,
            &["--Output=JSON"],
            path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        ) {
            Ok(output) => (output.value, None),
            Err(error) => (Value::Null, Some(format!("MediaInfo enrichment failed: {error}"))),
        },
        None => (
            Value::Null,
            Some("MediaInfo is unavailable; this FFprobe profile is partial. MediaInfo-specific fields may be missing.".into()),
        ),
    };
    timings.mediainfo_ms += elapsed_ms(mediainfo_start);
    let profile_start = Instant::now();
    let profile = build_profile(&probe, &media_info, path, settings, ffprobe)?;
    timings.profile_ms += elapsed_ms(profile_start);
    Ok((profile, media_info_error))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineProfileRequest {
    path: String,
    ffprobe: String,
    mediainfo: String,
    promote_8bit_to_10bit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EngineTrackProfilesRequest {
    path: String,
    ffprobe: String,
    promote_8bit_to_10bit: bool,
    video_stream_indices: Vec<i64>,
    probe: Value,
    mediainfo: Value,
}

/// Profile the source once in Rust for app-launched engine jobs. The response
/// carries the raw probe payload for remaining compatibility stages and a full
/// profile for the selected primary stream.
pub(crate) fn engine_profile_cli_mode() -> i32 {
    let result = (|| {
        let mut input = Vec::new();
        std::io::stdin()
            .take(MAX_ENGINE_PROFILE_REQUEST_BYTES + 1)
            .read_to_end(&mut input)
            .map_err(|error| format!("Could not read the native profile request: {error}"))?;
        if input.len() as u64 > MAX_ENGINE_PROFILE_REQUEST_BYTES {
            return Err("Native profile request exceeds 64 KiB.".to_owned());
        }
        let request: EngineProfileRequest = serde_json::from_slice(&input)
            .map_err(|error| format!("Invalid native profile request: {error}"))?;
        if request.path.is_empty() || request.path.len() > 8192 {
            return Err("Profile request path must be between 1 and 8192 bytes.".into());
        }
        for (name, tool) in [
            ("FFprobe", &request.ffprobe),
            ("MediaInfo", &request.mediainfo),
        ] {
            if tool.is_empty() || tool.len() > 8192 || tool.contains('\0') {
                return Err(format!("The configured {name} path is invalid."));
            }
        }
        let path = expand_home(Path::new(&request.path))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !path.is_file() {
            return Err("Media input must be a regular file.".into());
        }
        let parent = path.parent().unwrap_or(&path);
        let settings_payload = serde_json::to_value(settings::load_effective_settings(
            &parent.to_string_lossy(),
        )?)
        .map_err(|error| format!("Could not encode effective settings: {error}"))?;
        let mut effective_settings = settings_payload
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(video) = effective_settings
            .get_mut("video")
            .and_then(Value::as_object_mut)
        {
            video.insert(
                "promote_8bit_to_10bit".into(),
                Value::Bool(request.promote_8bit_to_10bit),
            );
        }

        let probe = run_json_tool(
            &request.ffprobe,
            &[
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-show_chapters",
                "-show_data_hash",
                "sha256",
                "-of",
                "json",
            ],
            &path,
            Duration::from_secs(120),
            MAX_TOOL_JSON_BYTES,
        )?
        .value;
        let media_info = run_json_tool(
            &request.mediainfo,
            &["--Output=JSON"],
            &path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        )?
        .value;
        let primary = build_profile(
            &probe,
            &media_info,
            &path,
            &effective_settings,
            &request.ffprobe,
        )?;
        let payload = json!({
            "path": path.to_string_lossy(),
            "probe": probe,
            "mediainfo": media_info,
            "profile": primary.payload
        });
        let encoded = serde_json::to_vec(&payload)
            .map_err(|error| format!("Could not serialize the native source profile: {error}"))?;
        if encoded.len() > MAX_ENGINE_PROFILE_RESPONSE_BYTES {
            return Err("Native source profile exceeds the 64 MiB response limit.".into());
        }
        Ok(encoded)
    })();
    match result {
        Ok(encoded) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = std::io::Write::write_all(&mut stdout, &encoded)
                .and_then(|()| std::io::Write::write_all(&mut stdout, b"\n"))
            {
                eprintln!("Could not write native source profile: {error}");
                1
            } else {
                0
            }
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Build profile fields for the video streams the engine actually selected.
/// FFprobe and MediaInfo JSON are sent back from the preceding Rust call so this
/// operation only needs the per-video frame metadata probe.
pub(crate) fn engine_track_profiles_cli_mode() -> i32 {
    let result = (|| {
        let mut input = Vec::new();
        std::io::stdin()
            .take(MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES + 1)
            .read_to_end(&mut input)
            .map_err(|error| {
                format!("Could not read the native selected-track profile request: {error}")
            })?;
        if input.len() as u64 > MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES {
            return Err("Native selected-track profile request exceeds 68 MiB.".into());
        }
        let request: EngineTrackProfilesRequest = serde_json::from_slice(&input)
            .map_err(|error| format!("Invalid native selected-track profile request: {error}"))?;
        if request.path.is_empty() || request.path.len() > 8192 {
            return Err("Selected-track profile path must be between 1 and 8192 bytes.".into());
        }
        if request.ffprobe.is_empty()
            || request.ffprobe.len() > 8192
            || request.ffprobe.contains('\0')
        {
            return Err("The configured FFprobe path is invalid.".into());
        }
        if request.video_stream_indices.is_empty()
            || request.video_stream_indices.len() > MAX_VIDEO_PROFILES
        {
            return Err(format!(
                "Selected-track profile count must be between 1 and {MAX_VIDEO_PROFILES}."
            ));
        }
        let wanted = request
            .video_stream_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if wanted.len() != request.video_stream_indices.len()
            || wanted.iter().any(|index| *index < 0)
        {
            return Err("Selected video stream indices must be unique and nonnegative.".into());
        }
        if !request.probe.is_object() || !request.mediainfo.is_object() {
            return Err(
                "Selected-track profiling requires FFprobe and MediaInfo JSON objects.".into(),
            );
        }
        let path = expand_home(Path::new(&request.path))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !path.is_file() {
            return Err("Media input must be a regular file.".into());
        }
        let parent = path.parent().unwrap_or(&path);
        let settings_payload = serde_json::to_value(settings::load_effective_settings(
            &parent.to_string_lossy(),
        )?)
        .map_err(|error| format!("Could not encode effective settings: {error}"))?;
        let mut effective_settings = settings_payload
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(video) = effective_settings
            .get_mut("video")
            .and_then(Value::as_object_mut)
        {
            video.insert(
                "promote_8bit_to_10bit".into(),
                Value::Bool(request.promote_8bit_to_10bit),
            );
        }
        let streams = request
            .probe
            .get("streams")
            .and_then(Value::as_array)
            .ok_or("FFprobe JSON did not include a stream list.")?;
        let mut video_ordinal = 0_usize;
        let mut profiles = Map::new();
        for stream in streams {
            if !stream
                .get("codec_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
            {
                continue;
            }
            let ordinal = video_ordinal;
            video_ordinal += 1;
            let index = value_i64(stream.get("index"))
                .ok_or("FFprobe returned a video stream without an index.")?;
            if !wanted.contains(&index) {
                continue;
            }
            if disposition(stream, "attached_pic")
                || value_i64(stream.get("width")).unwrap_or(0) <= 0
                || value_i64(stream.get("height")).unwrap_or(0) <= 0
            {
                return Err(format!(
                    "Selected stream #{index} is not a usable video track."
                ));
            }
            let profile = build_profile_for_video(
                &request.probe,
                &request.mediainfo,
                &path,
                &effective_settings,
                &request.ffprobe,
                Some(ordinal),
            )?;
            let compact = [
                "video",
                "hdr",
                "duration_seconds",
                "interlaced",
                "color_primaries",
                "color_transfer",
                "color_matrix",
                "color_range_x265",
                "color_range_ffmpeg",
                "mastering_display",
                "mastering_display_source",
                "max_cll",
                "max_fall",
                "chroma_location",
                "format_level",
                "primary_stream_index",
            ]
            .into_iter()
            .filter_map(|key| {
                profile
                    .payload
                    .get(key)
                    .cloned()
                    .map(|value| (key.to_owned(), value))
            })
            .collect::<Map<_, _>>();
            profiles.insert(index.to_string(), Value::Object(compact));
        }
        if profiles.len() != wanted.len() {
            let missing = wanted
                .iter()
                .filter(|index| !profiles.contains_key(&index.to_string()))
                .map(|index| format!("#{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Selected video stream(s) were not found: {missing}."
            ));
        }
        Ok(json!({ "video_profiles": profiles }))
    })();
    match result {
        Ok(payload) => match serde_json::to_vec(&payload) {
            Ok(encoded) => {
                let mut stdout = std::io::stdout().lock();
                if let Err(error) = std::io::Write::write_all(&mut stdout, &encoded)
                    .and_then(|()| std::io::Write::write_all(&mut stdout, b"\n"))
                {
                    eprintln!("Could not write selected-track profiles: {error}");
                    1
                } else {
                    0
                }
            }
            Err(error) => {
                eprintln!("Could not serialize selected-track profiles: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub(crate) fn probe_output(ffprobe: &str, path: &Path) -> Result<Value, String> {
    run_json_tool(
        ffprobe,
        &[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-show_data_hash",
            "sha256",
            "-of",
            "json",
        ],
        path,
        Duration::from_secs(120),
        MAX_TOOL_JSON_BYTES,
    )
    .map(|captured| captured.value)
}

/// Build the same HDR/color profile used during scan analysis for a specific
/// video stream in an already-probed output. The stream ordinal is FFprobe's
/// zero-based video ordinal, which is also the FFmpeg `v:N` selector.
pub(crate) fn output_video_profiles(
    ffprobe: &str,
    mediainfo: Option<&str>,
    path: &Path,
    probe: &Value,
    video_ordinals: &[usize],
) -> Result<BTreeMap<usize, Value>, String> {
    if video_ordinals.len() > MAX_VIDEO_PROFILES {
        return Err(format!(
            "Output HDR profiling is limited to {MAX_VIDEO_PROFILES} video tracks."
        ));
    }
    let unique = video_ordinals.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != video_ordinals.len() {
        return Err("Output HDR profile video ordinals must be unique.".into());
    }
    let media_info = match mediainfo {
        Some(program) => run_json_tool(
            program,
            &["--Output=JSON"],
            path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        )
        .map(|output| output.value)
        .unwrap_or(Value::Null),
        None => Value::Null,
    };
    video_ordinals
        .iter()
        .map(|ordinal| {
            build_profile_for_video(
                probe,
                &media_info,
                path,
                &json!({}),
                ffprobe,
                Some(*ordinal),
            )
            .map(|profile| (*ordinal, profile.payload))
        })
        .collect()
}

fn run_json_tool(
    program: &str,
    args: &[&str],
    path: &Path,
    timeout: Duration,
    max_bytes: usize,
) -> Result<CapturedJson, String> {
    let mut child = Command::new(program)
        .args(args)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", Path::new(program).display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture media-tool output.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture media-tool diagnostics.")?;
    let stdout_reader = thread::spawn(move || read_capped(stdout, max_bytes));
    let stderr_reader = thread::spawn(move || read_capped(stderr, 1024 * 1024));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not wait for {}: {error}",
                    Path::new(program).display()
                ));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_reader.join().unwrap_or_default();
    let (stderr, _) = stderr_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{} timed out while inspecting {}.",
            Path::new(program).display(),
            path.display()
        ));
    };
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr)
            .trim()
            .chars()
            .take(2500)
            .collect::<String>();
        return Err(if detail.is_empty() {
            format!(
                "{} could not inspect {} (exit {}).",
                Path::new(program).display(),
                path.display(),
                status
            )
        } else {
            detail
        });
    }
    if stdout_truncated {
        return Err(format!(
            "{} returned more than {max_bytes} bytes for {}.",
            Path::new(program).display(),
            path.display()
        ));
    }
    let value = serde_json::from_slice(&stdout).map_err(|error| {
        format!(
            "{} returned invalid JSON for {}: {error}",
            Path::new(program).display(),
            path.display()
        )
    })?;
    Ok(CapturedJson { value })
}

fn read_capped(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let room = limit.saturating_sub(output.len());
                let retained = count.min(room);
                output.extend_from_slice(&buffer[..retained]);
                truncated |= retained != count;
            }
        }
    }
    (output, truncated)
}

fn build_profile(
    probe: &Value,
    mediainfo: &Value,
    path: &Path,
    settings: &Value,
    ffprobe: &str,
) -> Result<Profile, String> {
    build_profile_for_video(probe, mediainfo, path, settings, ffprobe, None)
}

fn build_profile_for_video(
    probe: &Value,
    mediainfo: &Value,
    path: &Path,
    settings: &Value,
    ffprobe: &str,
    preferred_video_ordinal: Option<usize>,
) -> Result<Profile, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or("FFprobe returned no stream list.")?;
    let mut ordinals = BTreeMap::<String, usize>::new();
    let refs = streams
        .iter()
        .map(|stream| {
            let kind = stream
                .get("codec_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_lowercase();
            let ordinal = *ordinals
                .entry(kind.clone())
                .and_modify(|value| *value += 1)
                .or_insert(0);
            (stream, kind, ordinal)
        })
        .collect::<Vec<_>>();
    let is_usable_video = |stream: &Value, kind: &str| {
        kind == "video"
            && !disposition(stream, "attached_pic")
            && value_i64(stream.get("width")).unwrap_or(0) > 0
            && value_i64(stream.get("height")).unwrap_or(0) > 0
    };
    let primary = if let Some(video_ordinal) = preferred_video_ordinal {
        refs.iter().find(|(stream, kind, ordinal)| {
            *ordinal == video_ordinal && is_usable_video(stream, kind)
        })
    } else {
        refs.iter()
            .filter(|(stream, kind, _)| is_usable_video(stream, kind))
            .max_by_key(|(stream, _, _)| {
                let width = value_i64(stream.get("width")).unwrap_or(0);
                let height = value_i64(stream.get("height")).unwrap_or(0);
                let default = disposition(stream, "default") as i64;
                (
                    width.saturating_mul(height),
                    width,
                    height,
                    default,
                    source_depth(stream),
                    -value_i64(stream.get("index")).unwrap_or(i64::MAX),
                )
            })
    }
    .ok_or_else(|| {
        preferred_video_ordinal.map_or_else(
            || "No usable video stream was found.".to_string(),
            |ordinal| format!("No usable video stream exists at output video ordinal {ordinal}."),
        )
    })?;
    let primary_raw = primary.0;
    let primary_ordinal = primary.2;
    let primary_index = value_i64(primary_raw.get("index"))
        .ok_or("FFprobe did not return a video stream index.")?;
    let width = value_i64(primary_raw.get("width")).unwrap_or(0);
    let height = value_i64(primary_raw.get("height")).unwrap_or(0);
    let media_tracks = mediainfo
        .pointer("/media/track")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let media_primary =
        match_mediainfo_video(&media_tracks, primary_raw, primary_ordinal, width, height);
    let no_mediainfo = Value::Null;
    let media_primary = media_primary.unwrap_or(&no_mediainfo);
    let duration = value_f64(probe.pointer("/format/duration"))
        .or_else(|| value_f64(primary_raw.get("duration")))
        .or_else(|| media_primary.get("Duration").and_then(value_f64_ref))
        .filter(|duration| *duration > 0.0)
        .ok_or_else(|| format!("Unable to determine media duration for {}.", path.display()))?;
    let depth = media_primary
        .get("BitDepth")
        .and_then(parse_numeric_string)
        .filter(|value| matches!(value, 8 | 10 | 12 | 14 | 16))
        .unwrap_or_else(|| source_depth(primary_raw));
    let chroma = media_primary
        .get("ChromaSubsampling")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            chroma_from_pixel_format(
                primary_raw
                    .get("pix_fmt")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
        });
    let promoted = settings
        .pointer("/video/promote_8bit_to_10bit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target_pix_fmt = target_pixel_format(depth, &chroma, promoted)?;

    let stream_side_data = primary_raw
        .get("side_data_list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let frame = frame_probe(ffprobe, path, primary_ordinal);
    let frame_first = frame
        .as_ref()
        .and_then(|value| value.pointer("/frames/0"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let frame_side_data = frame_side_data(frame.as_ref());
    let mut side_data = stream_side_data;
    side_data.extend(frame_side_data);

    let primaries = normalize_primaries(
        first_string(primary_raw, "color_primaries")
            .or_else(|| first_string(&frame_first, "color_primaries"))
            .or_else(|| {
                media_primary
                    .get("colour_primaries")
                    .and_then(Value::as_str)
            }),
    );
    let transfer = normalize_transfer(
        first_string(primary_raw, "color_transfer")
            .or_else(|| first_string(&frame_first, "color_transfer"))
            .or_else(|| {
                media_primary
                    .get("transfer_characteristics")
                    .and_then(Value::as_str)
            }),
    );
    let matrix = normalize_matrix(
        first_string(primary_raw, "color_space")
            .or_else(|| first_string(&frame_first, "color_space"))
            .or_else(|| {
                media_primary
                    .get("matrix_coefficients")
                    .and_then(Value::as_str)
            }),
    );
    let range = first_string(primary_raw, "color_range")
        .or_else(|| first_string(&frame_first, "color_range"))
        .or_else(|| media_primary.get("colour_range").and_then(Value::as_str))
        .map(str::to_lowercase);
    let (range_x265, range_ffmpeg) = match range.as_deref() {
        Some("tv" | "limited") => (Some("limited"), Some("tv")),
        Some("pc" | "full") => (Some("full"), Some("pc")),
        _ => (None, None),
    };
    let mastering_side = side_data
        .iter()
        .find(|value| side_kind(value).contains("mastering display metadata"));
    let (mastering_display, mastering_source) =
        if let Some(value) = mastering_side.and_then(mastering_display_from_side) {
            (Some(value), Some("ffprobe-side-data"))
        } else if let Some(value) = mastering_display_from_mediainfo(media_primary) {
            (Some(value), Some("MediaInfo-label-fallback"))
        } else {
            (None, None)
        };
    let light_side = side_data
        .iter()
        .find(|value| side_kind(value).contains("content light level metadata"));
    let max_cll = light_side
        .and_then(|value| value.get("max_content").and_then(rounded_integer))
        .or_else(|| media_primary.get("MaxCLL").and_then(rounded_integer));
    let max_fall = light_side
        .and_then(|value| value.get("max_average").and_then(rounded_integer))
        .or_else(|| media_primary.get("MaxFALL").and_then(rounded_integer));
    let side_text = side_data
        .iter()
        .map(side_kind)
        .collect::<Vec<_>>()
        .join(" ");
    let hdr_text = [
        "HDR_Format",
        "HDR_Format_Profile",
        "HDR_Format_Compatibility",
        "HDR_Format_Settings",
    ]
    .iter()
    .filter_map(|key| media_primary.get(*key).and_then(Value::as_str))
    .collect::<Vec<_>>()
    .join(" ");
    let hdr10plus_hint = contains_any(
        &format!("{hdr_text} {side_text}"),
        &["hdr10+", "smpte2094-40", "hdr dynamic metadata"],
    );
    let dovi_side = side_data.iter().find(|value| {
        let kind = side_kind(value);
        kind.contains("dovi configuration record") || kind.contains("dolby vision configuration")
    });
    let (dovi_profile, dovi_level, dovi_rpu, dovi_el, dovi_bl, dovi_compat, dovi_source) =
        if let Some(side) = dovi_side {
            (
                value_i64(side.get("dv_profile")),
                value_i64(side.get("dv_level")),
                value_i64(side.get("rpu_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("el_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("bl_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("dv_bl_signal_compatibility_id")),
                Some("FFprobe DOVI configuration record"),
            )
        } else {
            let dovi_text = format!(
                "{hdr_text} {}",
                media_primary
                    .get("CodecID")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            if contains_any(&dovi_text, &["dolby vision", "dvhe.", "dvh1."]) {
                (
                    dovi_profile_from_text(&dovi_text),
                    dovi_level_from_text(&dovi_text),
                    false,
                    false,
                    false,
                    None,
                    Some("MediaInfo Dolby Vision metadata"),
                )
            } else {
                (None, None, false, false, false, None, None)
            }
        };
    let dolby_vision = dovi_source.is_some()
        || contains_any(
            &format!("{hdr_text} {side_text}"),
            &["dolby vision", "dovi configuration"],
        );
    let hdr10 = (transfer.as_deref() == Some("smpte2084")
        && primaries.as_deref() == Some("bt2020"))
        || hdr_text.to_lowercase().contains("hdr10");
    let hlg =
        transfer.as_deref() == Some("arib-std-b67") || hdr_text.to_lowercase().contains("hlg");
    let field_order = first_string(primary_raw, "field_order")
        .unwrap_or_default()
        .to_lowercase();
    let scan_type = media_primary
        .get("ScanType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let interlaced = scan_type.to_lowercase().contains("interlaced")
        || (!field_order.is_empty() && field_order != "progressive" && field_order != "unknown")
        || frames_are_interlaced(frame.as_ref());

    let mut tracks = Vec::with_capacity(refs.len());
    let mut secondary_videos = 0;
    let mut audio_count = 0;
    let mut subtitle_count = 0;
    let mut attachment_count = 0;
    let mut data_count = 0;
    for (stream, kind, ordinal) in &refs {
        let index = value_i64(stream.get("index")).unwrap_or(-1);
        if kind == "video"
            && !disposition(stream, "attached_pic")
            && value_i64(stream.get("width")).unwrap_or(0) > 0
            && value_i64(stream.get("height")).unwrap_or(0) > 0
            && index != primary_index
        {
            secondary_videos += 1;
        }
        match kind.as_str() {
            "audio" => audio_count += 1,
            "subtitle" => subtitle_count += 1,
            "attachment" => attachment_count += 1,
            "data" => data_count += 1,
            _ => {}
        }
        let tags = stream.get("tags").cloned().unwrap_or_else(|| json!({}));
        let mi_track = match_mediainfo_track(&media_tracks, kind, index, *ordinal);
        let mi_fields = mi_track
            .map(select_mediainfo_fields)
            .unwrap_or_else(|| json!({}));
        let raw = stream.as_object().cloned().unwrap_or_default();
        tracks.push(json!({
            "index": index,
            "type": kind,
            "ordinal": ordinal,
            "codec": stream.get("codec_name").cloned().unwrap_or(Value::Null),
            "profile": stream.get("profile").cloned().unwrap_or(Value::Null),
            "width": stream.get("width").cloned().unwrap_or(Value::Null),
            "height": stream.get("height").cloned().unwrap_or(Value::Null),
            "pixel_format": stream.get("pix_fmt").cloned().unwrap_or(Value::Null),
            "bit_depth": stream.get("bits_per_raw_sample").filter(|value| !value.is_null() && value.as_str() != Some("N/A"))
                .cloned().or_else(|| stream.get("bits_per_sample").cloned())
                .unwrap_or_else(|| if kind == "video" { json!(source_depth(stream)) } else { Value::Null }),
            "color_primaries": stream.get("color_primaries").cloned().unwrap_or(Value::Null),
            "color_transfer": stream.get("color_transfer").cloned().unwrap_or(Value::Null),
            "color_matrix": stream.get("color_space").cloned().unwrap_or(Value::Null),
            "frame_rate": stream.get("avg_frame_rate").cloned().unwrap_or(Value::Null),
            "channels": stream.get("channels").cloned().unwrap_or(Value::Null),
            "channel_layout": stream.get("channel_layout").cloned().unwrap_or(Value::Null),
            "sample_rate": stream.get("sample_rate").cloned().unwrap_or(Value::Null),
            "bit_rate": stream.get("bit_rate").cloned().unwrap_or(Value::Null),
            "language": tags.get("language").cloned().unwrap_or(Value::Null),
            "title": tags.get("title").cloned().unwrap_or(Value::Null),
            "default": disposition(stream, "default"),
            "forced": disposition(stream, "forced"),
            "attached_picture": disposition(stream, "attached_pic"),
            "filename": tags.get("filename").cloned().unwrap_or(Value::Null),
            "mime_type": tags.get("mimetype").cloned().unwrap_or(Value::Null),
            "mediainfo": mi_fields,
            "ffprobe": raw
        }));
    }
    let mut general = Map::new();
    if let Some(object) = media_tracks
        .iter()
        .find(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("general"))
        })
        .and_then(Value::as_object)
    {
        for (key, value) in object {
            if ![
                "@type",
                "CompleteName",
                "FolderName",
                "FileName",
                "FileExtension",
                "FileSize",
                "Duration",
            ]
            .contains(&key.as_str())
            {
                general.insert(key.clone(), value.clone());
            }
        }
    }
    let content_profile = crate::content::profile_content(path, mediainfo, settings);
    let primary_stream_index = primary_index;
    let payload = json!({
        "video": {
            "codec": primary_raw.get("codec_name").cloned().unwrap_or(Value::Null),
            "width": width,
            "height": height,
            "source_bit_depth": depth,
            "source_chroma": chroma,
            "target_pixel_format": target_pix_fmt,
            "secondary_count": secondary_videos
        },
        "hdr": {
            "hdr10": hdr10,
            "hdr10plus": false,
            "hdr10plus_hint": hdr10plus_hint,
            "hlg": hlg,
            "dolby_vision": dolby_vision,
            "dolby_vision_profile": dovi_profile,
            "dolby_vision_level": dovi_level,
            "dolby_vision_rpu": dovi_rpu,
            "dolby_vision_enhancement_layer": dovi_el,
            "dolby_vision_base_layer": dovi_bl,
            "dolby_vision_compatibility_id": dovi_compat,
            "dolby_vision_evidence": dovi_source
        },
        "audio_count": audio_count,
        "subtitle_count": subtitle_count,
        "attachment_count": attachment_count,
        "data_stream_count": data_count,
        "tracks": tracks,
        "mediainfo_general": general,
        "content": content_profile,
        "chapters_count": probe.get("chapters").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "duration_seconds": duration,
        "interlaced": interlaced,
        "color_primaries": primaries,
        "color_transfer": transfer,
        "color_matrix": matrix,
        "color_range_x265": range_x265,
        "color_range_ffmpeg": range_ffmpeg,
        "mastering_display": mastering_display,
        "mastering_display_source": mastering_source,
        "max_cll": max_cll,
        "max_fall": max_fall,
        "chroma_location": chroma_location(
            first_string(primary_raw, "chroma_location"),
            media_primary.get("ChromaSubsampling_Position").and_then(Value::as_str),
        ),
        "format_level": media_primary.get("Format_Level").cloned().unwrap_or(Value::Null),
        "primary_stream_index": primary_stream_index,
        "mediainfo_available": mediainfo.pointer("/media/track").is_some(),
        "frame_metadata_available": frame.is_some(),
        "source_metadata": {
            "format": probe.get("format").cloned().unwrap_or(Value::Null),
            "chapters": probe.get("chapters").cloned().unwrap_or_else(|| json!([]))
        }
    });
    Ok(Profile { payload, duration })
}

fn match_mediainfo_video<'a>(
    tracks: &'a [Value],
    primary: &Value,
    ordinal: usize,
    width: i64,
    height: i64,
) -> Option<&'a Value> {
    let videos = tracks
        .iter()
        .filter(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
        })
        .collect::<Vec<_>>();
    if videos.is_empty() {
        return None;
    }

    // MediaInfo StreamOrder normally matches FFprobe's global stream index.
    if let Some(track) = videos.iter().copied().find(|track| {
        track.get("StreamOrder").and_then(parse_media_intish) == value_i64(primary.get("index"))
    }) {
        return Some(track);
    }

    // If stream order is missing, accept the typed ordinal only when its
    // dimensions also agree with the selected FFprobe stream.
    if let Some(track) = videos
        .get(ordinal)
        .copied()
        .filter(|track| media_dimensions(track) == (width, height))
    {
        return Some(track);
    }

    let exact = videos
        .iter()
        .copied()
        .filter(|track| media_dimensions(track) == (width, height))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return exact.first().copied();
    }
    if !exact.is_empty() {
        let codec = primary
            .get("codec_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let compatible = exact.iter().copied().find(|track| {
            let format = track
                .get("Format")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            match codec.as_str() {
                "hevc" => ["hevc", "h.265", "h265"].contains(&format.as_str()),
                "h264" => ["avc", "h.264", "h264"].contains(&format.as_str()),
                "av1" => format == "av1",
                "vp9" => format == "vp9",
                _ => format == codec,
            }
        });
        return compatible.or_else(|| exact.first().copied());
    }

    // Match the Python profile's final fallback when no MediaInfo track has
    // the FFprobe-selected dimensions.
    videos.into_iter().max_by_key(|track| {
        let (width, height) = media_dimensions(track);
        (width.saturating_mul(height), width, height)
    })
}

fn media_dimensions(track: &Value) -> (i64, i64) {
    (
        track.get("Width").and_then(parse_media_intish).unwrap_or(0),
        track
            .get("Height")
            .and_then(parse_media_intish)
            .unwrap_or(0),
    )
}

fn parse_media_intish(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let compact = text.trim().replace(' ', "").replace(',', "");
    parse_first_signed_integer(&compact)
}

fn parse_first_signed_integer(value: &str) -> Option<i64> {
    let start = value
        .char_indices()
        .find(|(_, character)| character.is_ascii_digit() || *character == '-')?
        .0;
    let number = value[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect::<String>();
    number.parse().ok()
}

fn match_mediainfo_track<'a>(
    tracks: &'a [Value],
    kind: &str,
    index: i64,
    ordinal: usize,
) -> Option<&'a Value> {
    tracks
        .iter()
        .find(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(kind))
                && track.get("StreamOrder").and_then(parse_numeric) == Some(index)
        })
        .or_else(|| {
            tracks
                .iter()
                .filter(|track| {
                    track
                        .get("@type")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(kind))
                })
                .nth(ordinal)
        })
}

const MEDIAINFO_FIELDS: &[&str] = &[
    "Format",
    "Format_Profile",
    "CodecID",
    "CodecID_Description",
    "Width",
    "Height",
    "BitDepth",
    "ChromaSubsampling",
    "FrameRate",
    "FrameRate_Mode",
    "ScanType",
    "colour_primaries",
    "transfer_characteristics",
    "matrix_coefficients",
    "colour_range",
    "HDR_Format",
    "HDR_Format_Profile",
    "HDR_Format_Compatibility",
    "HDR_Format_Settings",
    "MasteringDisplay_ColorPrimaries",
    "MasteringDisplay_Luminance",
    "MaxCLL",
    "MaxFALL",
    "Language",
    "Title",
    "Channels",
    "ChannelLayout",
    "SamplingRate",
    "BitRate",
    "Compression_Mode",
    "Default",
    "Forced",
    "StreamSize",
    "Duration",
    "Format_Settings",
    "Encoded_Library",
    "WritingLibrary",
    "InternetMediaType",
    "ChromaSubsampling_Position",
];

fn select_mediainfo_fields(track: &Value) -> Value {
    let mut selected = Map::new();
    if let Some(object) = track.as_object() {
        for key in MEDIAINFO_FIELDS {
            if let Some(value) = object.get(*key) {
                selected.insert((*key).into(), value.clone());
            }
        }
    }
    Value::Object(selected)
}

fn frame_probe(ffprobe: &str, path: &Path, ordinal: usize) -> Option<Value> {
    let selector = format!("v:{ordinal}");
    run_json_tool(
        ffprobe,
        &[
            "-v", "error", "-select_streams", &selector, "-read_intervals", "%+1",
            "-show_frames", "-show_entries",
            "frame=color_range,color_space,color_primaries,color_transfer,interlaced_frame:frame_side_data",
            "-of", "json",
        ],
        path,
        Duration::from_secs(30),
        8 * 1024 * 1024,
    )
    .ok()
    .map(|output| output.value)
}

fn frame_side_data(frame: Option<&Value>) -> Vec<Value> {
    let mut side_data = Vec::new();
    if let Some(frames) = frame
        .and_then(|value| value.get("frames"))
        .and_then(Value::as_array)
    {
        for entry in frames {
            if let Some(items) = entry.get("side_data_list").and_then(Value::as_array) {
                side_data.extend(items.iter().cloned());
            }
        }
    }
    side_data
}

fn frames_are_interlaced(frame: Option<&Value>) -> bool {
    frame
        .and_then(|value| value.get("frames"))
        .and_then(Value::as_array)
        .is_some_and(|frames| {
            frames
                .iter()
                .any(|entry| entry.get("interlaced_frame").and_then(value_i64_ref) == Some(1))
        })
}

fn source_depth(stream: &Value) -> i64 {
    stream
        .get("bits_per_raw_sample")
        .and_then(parse_numeric)
        .or_else(|| value_i64(stream.get("bits_per_sample")))
        .filter(|value| matches!(value, 8 | 10 | 12 | 14 | 16))
        .or_else(|| {
            let pixel = stream
                .get("pix_fmt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            [16, 14, 12, 10, 8]
                .into_iter()
                .find(|depth| pixel.contains(&depth.to_string()))
        })
        .unwrap_or(8)
}

fn chroma_from_pixel_format(pixel: &str) -> String {
    if pixel.contains("444") {
        "4:4:4".into()
    } else if pixel.contains("422") {
        "4:2:2".into()
    } else {
        "4:2:0".into()
    }
}

fn target_pixel_format(depth: i64, chroma: &str, promote_8bit: bool) -> Result<String, String> {
    if ![8, 10, 12].contains(&depth) {
        return Err(format!(
            "Source bit depth {depth} cannot be reproduced exactly by this x265 workflow."
        ));
    }
    let depth = if depth == 8 && promote_8bit {
        10
    } else {
        depth
    };
    let chroma = if chroma.contains("4:4:4") {
        "444"
    } else if chroma.contains("4:2:2") {
        "422"
    } else {
        "420"
    };
    Ok(if depth == 8 {
        format!("yuv{chroma}p")
    } else {
        format!("yuv{chroma}p{depth}le")
    })
}

fn disposition(stream: &Value, name: &str) -> bool {
    stream
        .pointer(&format!("/disposition/{name}"))
        .and_then(value_i64_ref)
        .unwrap_or(0)
        != 0
}

fn first_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| {
        !text.trim().is_empty()
            && !text.eq_ignore_ascii_case("unknown")
            && !text.eq_ignore_ascii_case("unspecified")
    })
}

fn normalize_primaries(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    Some(
        match value.as_str() {
            "bt.709" | "bt709" => "bt709",
            "bt.2020" | "bt2020" => "bt2020",
            "bt.601 ntsc" | "smpte170m" => "smpte170m",
            "bt.601 pal" | "bt470bg" => "bt470bg",
            "bt.470 system m" | "bt470m" => "bt470m",
            "generic film" | "film" => "film",
            "display p3" => "smpte432",
            "dci p3" => "smpte431",
            _ => return Some(value),
        }
        .into(),
    )
}

fn normalize_transfer(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    Some(
        match value.as_str() {
            "pq" | "smpte2084" | "smpte st 2084" => "smpte2084",
            "hlg" | "arib-std-b67" => "arib-std-b67",
            "bt.709" | "bt709" => "bt709",
            "bt.470 system b/g" | "bt470bg" => "bt470bg",
            "bt.470 system m" | "bt470m" => "bt470m",
            "srgb" => "iec61966-2-1",
            _ => return Some(value),
        }
        .into(),
    )
}

fn normalize_matrix(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    if value.contains("bt.2020") && value.contains("non-constant") {
        return Some("bt2020nc".into());
    }
    if value.contains("bt.2020") && value.contains("constant") {
        return Some("bt2020c".into());
    }
    Some(
        match value.as_str() {
            "bt2020nc" | "bt2020c" | "bt.709" | "bt709" | "fcc" | "ycgco" | "gbr" => &value,
            "bt.601" | "smpte170m" => "smpte170m",
            "bt.470 system b/g" | "bt470bg" => "bt470bg",
            "rgb" => "gbr",
            _ => return Some(value),
        }
        .into(),
    )
}

fn side_kind(value: &Value) -> String {
    value
        .get("side_data_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

fn mastering_display_from_side(side: &Value) -> Option<String> {
    let names = [
        "red_x",
        "red_y",
        "green_x",
        "green_y",
        "blue_x",
        "blue_y",
        "white_point_x",
        "white_point_y",
        "min_luminance",
        "max_luminance",
    ];
    let mut values = BTreeMap::new();
    for name in names {
        values.insert(name, value_f64(side.get(name))?);
    }
    let xy = |x: &str, y: &str| {
        format!(
            "{},{}",
            (values[x] * 50000.0).round_ties_even() as i64,
            (values[y] * 50000.0).round_ties_even() as i64
        )
    };
    Some(format!(
        "G({})B({})R({})WP({})L({},{})",
        xy("green_x", "green_y"),
        xy("blue_x", "blue_y"),
        xy("red_x", "red_y"),
        xy("white_point_x", "white_point_y"),
        (values["max_luminance"] * 10000.0).round_ties_even() as i64,
        (values["min_luminance"] * 10000.0).round_ties_even() as i64
    ))
}

fn mastering_display_from_mediainfo(video: &Value) -> Option<String> {
    let primaries = video
        .get("MasteringDisplay_ColorPrimaries")
        .and_then(Value::as_str)?
        .trim();
    if primaries.is_empty() {
        return None;
    }
    let luminance = video
        .get("MasteringDisplay_Luminance")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let min_luminance = decimal_after_label(luminance, "min:").unwrap_or(0.005);
    let max_luminance = decimal_after_label(luminance, "max:").unwrap_or(1000.0);
    let light = format!(
        "L({},{})",
        (max_luminance * 10000.0).round_ties_even() as i64,
        (min_luminance * 10000.0).round_ties_even() as i64
    );
    let normalized = primaries.to_ascii_lowercase();
    if normalized.contains("bt.2020") || normalized.contains("bt2020") {
        Some(format!(
            "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450){light}"
        ))
    } else if normalized.contains("display p3") {
        Some(format!(
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450){light}"
        ))
    } else if normalized.contains("bt.709") || normalized.contains("bt709") {
        Some(format!(
            "G(15000,30000)B(7500,3000)R(32000,16500)WP(15635,16450){light}"
        ))
    } else {
        None
    }
}

fn decimal_after_label(value: &str, label: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    let offset = lower.find(label)? + label.len();
    parse_first_decimal(&value[offset..])
}

fn parse_first_decimal(value: &str) -> Option<f64> {
    let start = value
        .char_indices()
        .find(|(_, character)| character.is_ascii_digit())?
        .0;
    let tail = &value[start..];
    let whole_digits = tail.chars().take_while(char::is_ascii_digit).count();
    let mut length = whole_digits;
    let after_whole = &tail[whole_digits..];
    if after_whole.starts_with('.')
        && after_whole
            .chars()
            .nth(1)
            .is_some_and(|character| character.is_ascii_digit())
    {
        length += 1 + after_whole[1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
    }
    tail[..length].parse().ok()
}

fn rounded_integer(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(parse_first_decimal))?;
    Some(number.round_ties_even() as i64)
}

fn chroma_location(ffprobe_value: Option<&str>, mediainfo_value: Option<&str>) -> Option<i64> {
    if let Some(value) = ffprobe_value {
        let normalized = value.to_ascii_lowercase();
        let location = match normalized.as_str() {
            "left" => Some(0),
            "center" => Some(1),
            "topleft" | "top-left" => Some(2),
            "top" => Some(3),
            "bottomleft" | "bottom-left" => Some(4),
            "bottom" => Some(5),
            _ => None,
        };
        if location.is_some() {
            return location;
        }
    }
    let value = mediainfo_value?;
    for (index, character) in value.char_indices() {
        if !matches!(character, '0'..='5') {
            continue;
        }
        let end = index + character.len_utf8();
        let word = |character: char| character.is_ascii_alphanumeric() || character == '_';
        let has_word_before = value[..index].chars().next_back().is_some_and(word);
        let has_word_after = value[end..].chars().next().is_some_and(word);
        if !has_word_before && !has_word_after {
            return character.to_digit(10).map(i64::from);
        }
    }
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles
        .iter()
        .any(|needle| lower.contains(&needle.to_lowercase()))
}

fn dovi_profile_from_text(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    for marker in ["dvhe.", "dvh1."] {
        if let Some(index) = lower.find(marker) {
            let digits = lower[index + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            return digits.trim_start_matches('0').parse().ok();
        }
    }
    lower
        .split("profile")
        .nth(1)?
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn dovi_level_from_text(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    for marker in ["dvhe.", "dvh1."] {
        if let Some(index) = lower.find(marker) {
            let tail = &lower[index + marker.len()..];
            let profile_len = tail.chars().take_while(char::is_ascii_digit).count();
            let rest = tail[profile_len..].trim_start_matches('.');
            let digits = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            return digits.trim_start_matches('0').parse().ok();
        }
    }
    None
}

fn value_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn value_i64_ref(value: &Value) -> Option<i64> {
    value_i64(Some(value))
}

fn value_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn value_f64_ref(value: &Value) -> Option<f64> {
    value_f64(Some(value))
}

fn parse_numeric(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(parse_first_integer))
}

fn parse_numeric_string(value: &Value) -> Option<i64> {
    parse_numeric(value)
}

fn parse_first_integer(value: &str) -> Option<i64> {
    value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn disk_free_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        // SAFETY: c_path is NUL-terminated and stats points to writable storage.
        let code = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
        if code != 0 {
            return None;
        }
        // SAFETY: statvfs initialized the structure on success.
        let stats = unsafe { stats.assume_init() };
        Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hevc-rebuild-scan-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create isolated scan fixture");
        path
    }

    #[test]
    fn profile_cache_keys_invalidate_for_source_settings_tool_and_version_changes() {
        let root = temp_dir("cache-key");
        let source = root.join("source.mkv");
        let tool = root.join("tool");
        fs::write(&source, b"synthetic").unwrap();
        fs::write(&tool, b"tool").unwrap();
        let host = json!({"checks": [{"key": "ffprobe", "version": "1"}, {"key": "mediainfo", "version": "1"}]});
        let key = |settings: &Value, host: &Value| {
            profile_cache_key(
                &source,
                tool.to_str().unwrap(),
                Some(tool.to_str().unwrap()),
                settings,
                host,
            )
            .unwrap()
        };
        let baseline = key(&json!({"video": {"promote": true}}), &host);
        assert_eq!(baseline, key(&json!({"video": {"promote": true}}), &host));
        assert_ne!(baseline, key(&json!({"video": {"promote": false}}), &host));
        assert_ne!(
            baseline,
            key(
                &json!({"video": {"promote": true}}),
                &json!({"checks": [{"key": "ffprobe", "version": "2"}]})
            )
        );
        fs::write(&source, b"modified synthetic").unwrap();
        assert_ne!(baseline, key(&json!({"video": {"promote": true}}), &host));
        let after_source = key(&json!({"video": {"promote": true}}), &host);
        fs::write(&tool, b"updated tool").unwrap();
        assert_ne!(
            after_source,
            key(&json!({"video": {"promote": true}}), &host)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Explicit synthetic FFmpeg benchmark; run with HEVC_REBUILD_SCAN_TIMING=1"]
    fn measure_repeated_scan_profile_cache() {
        let root = temp_dir("cache-benchmark");
        let clip = root.join("synthetic-0.mkv");
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=24",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-threads",
                "1"
            ])
            .arg(&clip)
            .status()
            .unwrap()
            .success());
        for index in 1..20 {
            fs::copy(&clip, root.join(format!("synthetic-{index}.mkv"))).unwrap();
        }
        let options = ScanOptions {
            root: root.to_string_lossy().into(),
            recursive: false,
            minimum_size_mib: 0,
            filename_include_patterns: vec![],
            filename_exclude_patterns: vec![],
            tool_paths: ToolPaths::default(),
        };
        let cold = scan_media(options.clone(), |_, _, _| {}).unwrap();
        let warm = scan_media(options.clone(), |_, _, _| {}).unwrap();
        for (cold_entry, warm_entry) in cold["entries"]
            .as_array()
            .unwrap()
            .iter()
            .zip(warm["entries"].as_array().unwrap())
        {
            assert_eq!(cold_entry["profile"], warm_entry["profile"]);
            assert_eq!(cold_entry["status"], warm_entry["status"]);
        }
        assert_eq!(warm["scan_timings"]["profile_cache_hits"], 20);
        assert!(warm["scan_timings"]["ffprobe_ms"].as_f64().unwrap() < 1.0);
        assert!(
            warm["scan_timings"]["total_ms"].as_f64().unwrap()
                < cold["scan_timings"]["total_ms"].as_f64().unwrap()
        );
        fs::write(&clip, b"changed, invalid media").unwrap();
        let changed = scan_media(options, |_, _, _| {}).unwrap();
        assert_eq!(changed["scan_timings"]["profile_cache_hits"], 19);
        assert_eq!(changed["entries"][0]["status"], "Analysis failed");
        println!(
            "CACHE_BENCHMARK {}",
            json!({"cold": cold["scan_timings"], "warm": warm["scan_timings"], "changed": changed["scan_timings"]})
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filename_filters_match_case_insensitive_wildcards_and_exclusions_win() {
        let include = vec!["*Movie*Group.mkv".into(), "Special?.mp4".into()];
        let exclude = vec!["*sample*".into()];
        assert!(filename_matches_filters(
            "GreatMovieReleaseGroup.MKV",
            &include,
            &exclude
        ));
        assert!(filename_matches_filters("Special1.mp4", &include, &exclude));
        assert!(!filename_matches_filters(
            "GreatMovieSampleGroup.mkv",
            &include,
            &exclude
        ));
        assert!(!filename_matches_filters("Other.mkv", &include, &exclude));
        assert!(filename_matches_filters("Other.mkv", &[], &[]));
    }

    #[test]
    fn filename_filter_validation_rejects_unsafe_or_ambiguous_rules() {
        for invalid in ["", "***", "?*", "../*.mkv", "bad\\name", " padded"] {
            assert!(
                validate_filename_patterns(&[invalid.into()], "include").is_err(),
                "{invalid}"
            );
        }
        assert!(validate_filename_patterns(&["Film*".into(), "film*".into()], "exclude").is_err());
        assert!(validate_filename_patterns(&["x".repeat(129)], "include").is_err());
    }

    fn profile_fixture() -> Value {
        json!({
            "streams": [
                {
                    "index": 0, "codec_type": "video", "codec_name": "hevc", "profile": "Main 10",
                    "width": 3840, "height": 2160, "pix_fmt": "yuv420p10le", "bits_per_raw_sample": "10",
                    "avg_frame_rate": "24000/1001", "field_order": "progressive",
                    "color_primaries": "bt2020", "color_transfer": "smpte2084", "color_space": "bt2020nc",
                    "color_range": "tv", "chroma_location": "left",
                    "tags": {"language": "und", "title": "4K"},
                    "disposition": {"default": 1, "attached_pic": 0},
                    "side_data_list": [
                        {"side_data_type": "DOVI configuration record", "dv_profile": 8, "dv_level": 6, "rpu_present_flag": 1, "el_present_flag": 0, "bl_present_flag": 1, "dv_bl_signal_compatibility_id": 1},
                        {"side_data_type": "HDR Dynamic Metadata SMPTE2094-40 (HDR10+)"},
                        {"side_data_type": "Mastering display metadata", "red_x": 0.68, "red_y": 0.32, "green_x": 0.265, "green_y": 0.69, "blue_x": 0.15, "blue_y": 0.06, "white_point_x": 0.3127, "white_point_y": 0.329, "min_luminance": 0.005, "max_luminance": 1000},
                        {"side_data_type": "Content light level metadata", "max_content": 1000, "max_average": 400}
                    ]
                },
                {
                    "index": 1, "codec_type": "video", "codec_name": "h264", "profile": "High",
                    "width": 1920, "height": 1080, "pix_fmt": "yuv420p", "bits_per_raw_sample": "8",
                    "avg_frame_rate": "24000/1001", "field_order": "progressive",
                    "tags": {"language": "eng", "title": "1080p"},
                    "disposition": {"default": 0, "attached_pic": 0}
                },
                {
                    "index": 2, "codec_type": "audio", "codec_name": "ac3", "channels": 6,
                    "channel_layout": "5.1(side)", "sample_rate": "48000", "bit_rate": "640000",
                    "tags": {"language": "eng", "title": "Surround"},
                    "disposition": {"default": 1, "forced": 0}
                },
                {
                    "index": 3, "codec_type": "subtitle", "codec_name": "subrip",
                    "tags": {"language": "eng", "title": "English"}, "disposition": {"default": 0, "forced": 1}
                },
                {
                    "index": 4, "codec_type": "attachment", "codec_name": "ttf",
                    "tags": {"filename": "font.ttf", "mimetype": "application/x-truetype-font"},
                    "disposition": {"default": 0}
                },
                {
                    "index": 5, "codec_type": "video", "codec_name": "mjpeg", "width": 600, "height": 600,
                    "pix_fmt": "yuvj420p", "disposition": {"attached_pic": 1}
                }
            ],
            "format": {"format_name": "matroska,webm", "duration": "3600.5", "size": "123456789", "tags": {"title": "Example"}},
            "chapters": [{"id": 0}, {"id": 1}]
        })
    }

    fn settings_fixture() -> Value {
        json!({
            "video": {"preset": "slow", "promote_8bit_to_10bit": false},
            "resources": {
                "threading": "auto", "reserved_cpu_threads": 1, "max_worker_threads": 0,
                "frame_threads": 0, "min_free_memory_gib": 2.0, "memory_reserve_percent": 8.0
            }
        })
    }

    #[test]
    fn discovery_matches_extensions_and_non_recursive_season_layout() {
        let root = temp_dir("discovery");
        let season = root.join("Show").join("Season 01");
        fs::create_dir_all(&season).unwrap();
        fs::write(root.join("Film.MKV"), b"mkv").unwrap();
        fs::write(root.join("Test_ignored.mkv"), b"test").unwrap();
        fs::write(root.join("backup.hevc-rebuild.tmp.mkv"), b"temp").unwrap();
        fs::write(season.join("Show.S01E01.mkv"), b"episode").unwrap();
        fs::write(root.join("Show").join("outside.mp4"), b"not in season").unwrap();
        let found = discover_folder(&root, false, 0, &[]).files;
        let names = found
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Film.MKV", "Show.S01E01.mkv"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recursive_discovery_applies_removable_folder_exclusions_and_is_sorted() {
        let root = temp_dir("recursive");
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::create_dir_all(root.join("old")).unwrap();
        fs::create_dir_all(root.join("Season 01")).unwrap();
        fs::write(root.join("zeta.mkv"), b"z").unwrap();
        fs::write(root.join("Alpha.mp4"), b"a").unwrap();
        fs::write(root.join(".hidden").join("private.mkv"), b"x").unwrap();
        fs::write(root.join("old").join("backup.mkv"), b"x").unwrap();
        fs::write(root.join("Season 01").join("episode.mkv"), b"e").unwrap();
        let found = discover_folder(&root, true, 0, &["old".into()]).files;
        let relative = found
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            relative,
            vec![
                ".hidden/private.mkv",
                "Alpha.mp4",
                "Season 01/episode.mkv",
                "zeta.mkv"
            ]
        );
        let without_user_exclusions = discover_folder(&root, true, 0, &[]).files;
        assert!(without_user_exclusions
            .iter()
            .any(|path| path.ends_with(".hidden/private.mkv")));
        let hidden_excluded = discover_folder(&root, true, 0, &[".*".into()]).files;
        assert!(!hidden_excluded
            .iter()
            .any(|path| path.ends_with(".hidden/private.mkv")));
        assert!(!filename_matches_filters(
            "trailer.mkv",
            &[],
            &["trailer".into()]
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_prefers_highest_resolution_and_surfaces_all_probe_stream_facts() {
        let probe = profile_fixture();
        let mediainfo = json!({
            "media": {"track": [
                {"@type": "General", "Format": "Matroska", "CompleteName": "/private/source.mkv", "FileSize": "123456789", "Duration": "3600500", "Title": "Example"},
                {"@type": "Video", "StreamOrder": "0", "Format": "HEVC", "Format_Profile": "Main 10", "CodecID": "V_MPEGH/ISO/HEVC", "Width": "3 840 pixels", "Height": "2 160 pixels", "BitDepth": "10 bits", "ChromaSubsampling": "4:2:0", "FrameRate": "23.976 FPS", "HDR_Format": "Dolby Vision, Version 1.0, Profile 8.1, dvhe.08.06", "MaxCLL": "1000 nits", "MaxFALL": "400 nits", "Language": "und", "Title": "4K"},
                {"@type": "Video", "StreamOrder": "1", "Format": "AVC", "Width": "1920 pixels", "Height": "1080 pixels", "BitDepth": "8 bits", "Language": "eng", "Title": "1080p"},
                {"@type": "Audio", "StreamOrder": "2", "Format": "AC-3", "Channels": "6", "ChannelLayout": "L R C LFE Ls Rs", "SamplingRate": "48.0 kHz", "Language": "eng"},
                {"@type": "Text", "StreamOrder": "3", "Format": "UTF-8", "Language": "eng"}
            ]}
        });
        let path = Path::new("fixture.mkv");
        let profile = build_profile(
            &probe,
            &mediainfo,
            path,
            &settings_fixture(),
            "/missing/ffprobe",
        )
        .unwrap();
        assert_eq!(profile.payload["video"]["width"], 3840);
        assert_eq!(profile.payload["video"]["secondary_count"], 1);
        assert_eq!(
            profile.payload["video"]["target_pixel_format"],
            "yuv420p10le"
        );
        assert_eq!(profile.payload["audio_count"], 1);
        assert_eq!(profile.payload["subtitle_count"], 1);
        assert_eq!(profile.payload["attachment_count"], 1);
        assert_eq!(profile.payload["chapters_count"], 2);
        assert_eq!(profile.payload["hdr"]["dolby_vision_profile"], 8);
        assert_eq!(profile.payload["hdr"]["dolby_vision_level"], 6);
        assert_eq!(profile.payload["hdr"]["dolby_vision_rpu"], true);
        assert_eq!(profile.payload["hdr"]["hdr10plus_hint"], true);
        assert_eq!(profile.payload["max_cll"], 1000);
        assert_eq!(profile.payload["max_fall"], 400);
        assert!(profile.payload["mediainfo_general"]
            .get("CompleteName")
            .is_none());
        assert_eq!(
            profile.payload["tracks"][0]["ffprobe"]["tags"]["title"],
            "4K"
        );
        assert_eq!(profile.payload["tracks"][2]["channel_layout"], "5.1(side)");
        assert_eq!(profile.payload["tracks"][4]["filename"], "font.ttf");
        assert_eq!(profile.payload["tracks"][5]["attached_picture"], true);
    }

    #[test]
    fn profile_can_target_the_second_video_stream_for_output_validation() {
        let probe = profile_fixture();
        let profile = build_profile_for_video(
            &probe,
            &json!({}),
            Path::new("fixture.mkv"),
            &settings_fixture(),
            "/missing/ffprobe",
            Some(1),
        )
        .unwrap();
        assert_eq!(profile.payload["video"]["width"], 1920);
        assert_eq!(profile.payload["color_primaries"], Value::Null);
        assert_eq!(profile.payload["hdr"]["hdr10"], false);
        assert_eq!(profile.payload["hdr"]["dolby_vision"], false);
    }

    #[test]
    fn mediainfo_video_match_handles_spaced_dimensions_and_codec_ties() {
        let tracks = vec![
            json!({"@type":"Video", "Width":"1920 pixels", "Height":"1080 pixels", "Format":"AVC"}),
            json!({"@type":"Video", "Width":"3 840 pixels", "Height":"2 160 pixels", "Format":"AVC"}),
            json!({"@type":"Video", "Width":"3840 pixels", "Height":"2160 pixels", "Format":"HEVC"}),
        ];
        let primary = json!({"index":0, "codec_name":"hevc"});
        let selected = match_mediainfo_video(&tracks, &primary, 0, 3840, 2160).unwrap();
        assert_eq!(selected["Format"], "HEVC");
        assert_eq!(media_dimensions(selected), (3840, 2160));
    }

    #[test]
    fn profile_uses_mediainfo_hdr_fallbacks_and_ignores_unknown_color_labels() {
        let mut probe = profile_fixture();
        let primary = probe["streams"][0].as_object_mut().unwrap();
        primary.remove("color_primaries");
        primary.remove("color_transfer");
        primary.remove("color_space");
        primary.remove("chroma_location");
        primary.insert("side_data_list".into(), json!([]));
        let mediainfo = json!({
            "media": {"track": [{
                "@type":"Video", "StreamOrder":"0",
                "colour_primaries":"unspecified",
                "transfer_characteristics":"unknown",
                "matrix_coefficients":"unspecified",
                "MasteringDisplay_ColorPrimaries":"BT.2020",
                "MasteringDisplay_Luminance":"min: 0.005 cd/m2, max: 1000 cd/m2",
                "ChromaSubsampling_Position":"2 (top-left)",
                "MaxCLL":"1000.8 nits", "MaxFALL":"400.5 nits"
            }]}
        });
        let profile = build_profile(
            &probe,
            &mediainfo,
            Path::new("fixture.mkv"),
            &settings_fixture(),
            "/missing/ffprobe",
        )
        .unwrap();
        assert_eq!(
            profile.payload["mastering_display_source"],
            "MediaInfo-label-fallback"
        );
        assert_eq!(
            profile.payload["mastering_display"],
            "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,50)"
        );
        assert_eq!(profile.payload["chroma_location"], 2);
        assert_eq!(profile.payload["max_cll"], 1001);
        assert_eq!(profile.payload["max_fall"], 400);
        assert!(profile.payload["color_primaries"].is_null());
        assert!(profile.payload["color_transfer"].is_null());
        assert!(profile.payload["color_matrix"].is_null());
    }

    #[test]
    fn frame_profile_helpers_include_all_sampled_frames() {
        let frame = json!({"frames": [
            {"interlaced_frame":0, "side_data_list":[]},
            {"interlaced_frame":1, "side_data_list":[{"side_data_type":"HDR Dynamic Metadata SMPTE2094-40"}]}
        ]});
        assert_eq!(frame_side_data(Some(&frame)).len(), 1);
        assert!(frames_are_interlaced(Some(&frame)));
    }

    #[test]
    fn youtube_estimates_preserve_the_source_size_budget_and_never_downscale() {
        let profile = json!({
            "video": {"width": 1280, "height": 720},
            "hdr": {"hdr10": false, "hlg": false, "dolby_vision": false},
            "audio_count": 1,
            "primary_stream_index": 0,
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "24/1", "side_data_list": []}}]
        });
        assert_eq!(
            crate::youtube::expected_dimensions(&profile, "1440p").unwrap(),
            (2560, 1440)
        );
        let estimate =
            crate::youtube::estimate(&profile, 80_000_000, 3600.0, "1440p", "balanced", 2.0)
                .unwrap();
        assert_eq!(estimate["video_bitrate_kbps"], 250);
        assert!(estimate["estimated_output_bytes"].as_u64().unwrap() < 1_000_000_000);
        let larger = json!({
            "video": {"width": 3840, "height": 2160},
            "hdr": {"hdr10": true, "hlg": false, "dolby_vision": false},
            "source_bit_depth": 10,
            "color_primaries": "bt2020",
            "color_transfer": "smpte2084",
            "color_matrix": "bt2020nc",
            "primary_stream_index": 0,
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "60/1"}}]
        });
        assert_eq!(
            crate::youtube::expected_dimensions(&larger, "1440p").unwrap(),
            (3840, 2160)
        );
        assert_eq!(
            crate::youtube::estimate(&larger, 400_000_000, 600.0, "4k", "recommended", 2.0)
                .unwrap()["recommended_video_bitrate_kbps"],
            75_500
        );
    }

    #[test]
    fn memory_forecast_uses_ram_and_pressure_but_never_counts_swap_as_headroom() {
        let settings = settings_fixture();
        let host = json!({"hardware": {
            "logical_cpus": 16, "memory_total_gib": 40.0, "memory_available_gib": 12.0,
            "swap_free_gib": 8.0, "swap_used_gib": 8.0, "swap_used_percent": 50.0
        }});
        let profile = json!({
            "video": {"width": 3840, "height": 2160, "source_bit_depth": 10, "source_chroma": "4:2:0", "target_pixel_format": "yuv420p10le"},
            "primary_stream_index": 0, "hdr": {},
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "24/1"}}]
        });
        let forecast = memory_forecast(&profile, &settings, &host, Some(0.0), None, None, None, 1);
        assert_eq!(forecast["level"], "constrained");
        assert_eq!(forecast["swap_free_gib"], 8.0);
        assert_eq!(forecast["preset_factor"], 1.0);
        assert!(forecast["planning_budget_gib"].as_f64().unwrap() < 12.0);
        assert!(forecast["planned_workers"].as_i64().unwrap() < 15);
        let pressured = memory_forecast(&profile, &settings, &host, Some(1.2), None, None, None, 1);
        assert!(
            pressured["planning_budget_gib"].as_f64().unwrap()
                < forecast["planning_budget_gib"].as_f64().unwrap()
        );
    }

    #[test]
    fn memory_forecast_blocks_below_the_dynamic_reserve_even_with_free_swap() {
        let settings = settings_fixture();
        let host = json!({"hardware": {
            "logical_cpus": 16, "memory_total_gib": 40.0, "memory_available_gib": 2.0,
            "swap_free_gib": 16.0, "swap_used_gib": 0.0, "swap_used_percent": 0.0
        }});
        let profile = json!({"video":{"width":3840,"height":2160,"source_bit_depth":10,"source_chroma":"4:2:0","target_pixel_format":"yuv420p10le"}});
        let forecast = memory_forecast(&profile, &settings, &host, Some(0.0), None, None, None, 1);
        assert_eq!(forecast["level"], "blocked");
        assert!(forecast["planned_workers"].is_null());
    }
}
