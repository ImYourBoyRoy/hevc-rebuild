//! Rust-owned, read-only media discovery and stream profiling.
//!
//! GUI, native JSON CLI, and MCP scans share this module to discover files, read
//! FFprobe data, and calculate preflight plans.

use crate::resource_plan::{memory_forecast, memory_full_avg10, MemoryForecastInput};
use crate::{environment, settings, ToolPaths};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) mod disk_space;

mod color;
mod discovery;
mod engine_profile;
mod mediainfo;
mod profile;
mod tool_json;
mod values;

use color::*;
use discovery::*;
pub(crate) use discovery::{
    discover_cli_mode, discover_media_paths, filename_matches_filters, validate_filename_patterns,
};
pub(crate) use engine_profile::{
    engine_profile, engine_profile_cli_mode, engine_track_profiles, engine_track_profiles_cli_mode,
    EngineProfileRequest, EngineTrackProfilesRequest,
};
use mediainfo::*;
use profile::*;
use tool_json::*;
pub(crate) use tool_json::{count_stream_packets, output_video_profiles, probe_output};
use values::*;

const MAX_VIDEO_PROFILES: usize = 1024;

#[derive(Clone)]
pub(crate) struct ScanOptions {
    pub(crate) root: String,
    pub(crate) recursive: bool,
    pub(crate) minimum_size_mib: u32,
    pub(crate) filename_include_patterns: Vec<String>,
    pub(crate) filename_exclude_patterns: Vec<String>,
    pub(crate) tool_paths: ToolPaths,
    pub(crate) stop_file: Option<PathBuf>,
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

fn scan_stop_requested(stop_file: Option<&Path>) -> bool {
    stop_file.is_some_and(crate::stop_control::requested)
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
    #[cfg_attr(not(unix), allow(unused_mut))]
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
            options.stop_file.as_deref(),
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
    let work_directory = settings_payload
        .get("work_directory")
        .cloned()
        .unwrap_or_else(|| json!({}));

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
    let mut interrupted = false;

    for (offset, path) in discovery.files.iter().enumerate() {
        if scan_stop_requested(options.stop_file.as_deref()) {
            interrupted = true;
            break;
        }
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
                crate::backup_folder::has_backup(parent, path.file_name().unwrap_or_default())
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
                let rebuild_forecast = memory_forecast(MemoryForecastInput {
                    profile: &primary,
                    settings: &effective_settings,
                    host: &host,
                    full_memory_psi,
                    target_dimensions: None,
                    preset_override: None,
                    frame_threads_override: None,
                    concurrent_video_streams: 1,
                });
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
                        memory_forecast(MemoryForecastInput {
                            profile: &primary,
                            settings: &effective_settings,
                            host: &host,
                            full_memory_psi,
                            target_dimensions: Some(dimensions),
                            preset_override: Some("slow"),
                            frame_threads_override: Some(0),
                            concurrent_video_streams: 1,
                        }),
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
    if interrupted {
        warnings.push(if entries.is_empty() {
            "Scan stopped before files were analyzed.".into()
        } else {
            format!(
                "Scan stopped after analyzing {} of {total} files. Remaining files were not read.",
                entries.len()
            )
        });
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
        "work_directory": work_directory,
        "warnings": warnings,
        "discovery_limit_reached": discovery.limit_reached,
        "interrupted": interrupted
    });
    timings.total_ms = elapsed_ms(total_start);
    if std::env::var_os("HEVC_REBUILD_SCAN_TIMING").is_some_and(|value| value == "1") {
        payload["scan_timings"] =
            serde_json::to_value(&timings).map_err(|error| error.to_string())?;
    }
    Ok(payload)
}

pub(crate) struct AnalyzedFile {
    pub profile: Option<Value>,
    pub forecast: Option<Value>,
    pub warning: Option<String>,
    pub error: Option<String>,
    pub size_bytes: u64,
}

/// Profile one already-selected media file for native Check-only runs.
/// This reuses the scan profiler so GUI/CLI/MCP analysis share one code path.
pub(crate) fn analyze_file(
    path: &Path,
    ffprobe: Option<&str>,
    mediainfo: Option<&str>,
    settings: &Value,
    host: &Value,
) -> AnalyzedFile {
    let size_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let Some(ffprobe) = ffprobe else {
        return AnalyzedFile {
            profile: None,
            forecast: None,
            warning: None,
            error: Some(
                "FFprobe is unavailable. Select an FFprobe executable or install FFmpeg/FFprobe to inspect streams."
                    .into(),
            ),
            size_bytes,
        };
    };
    let mut timings = ScanTimings::default();
    match probe_media(ffprobe, mediainfo, path, settings, &mut timings) {
        Ok((profile, media_info_error)) => {
            let forecast = memory_forecast(MemoryForecastInput {
                profile: &profile.payload,
                settings,
                host,
                full_memory_psi: memory_full_avg10(),
                target_dimensions: None,
                preset_override: None,
                frame_threads_override: None,
                concurrent_video_streams: 1,
            });
            AnalyzedFile {
                profile: Some(profile.payload),
                forecast: Some(forecast),
                warning: media_info_error,
                error: None,
                size_bytes,
            }
        }
        Err(error) => AnalyzedFile {
            profile: None,
            forecast: None,
            warning: None,
            error: Some(error),
            size_bytes,
        },
    }
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

/// Total capacity of the volume holding `path` (Linux, macOS and Windows).
pub(crate) fn disk_total_bytes(path: &Path) -> Option<u64> {
    disk_space::disk_space(path).map(|space| space.total_bytes)
}

/// Space this user can still write on the volume holding `path`.
pub(crate) fn disk_free_bytes(path: &Path) -> Option<u64> {
    disk_space::disk_space(path).map(|space| space.available_bytes)
}

#[cfg(test)]
mod tests;
