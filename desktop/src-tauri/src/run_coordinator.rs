//! ./desktop/src-tauri/src/run_coordinator.rs
//! Native media-run coordinator. Check-only analysis is fully Rust-owned:
//! FFprobe/MediaInfo profiling, memory forecasts, CSV reports, and stop
//! handling with no Python process. Encode, metadata-only, short-test, and
//! YouTube modes still launch the compatibility engine until those slices
//! reach parity. GUI, JSON CLI, and MCP share this module.

use crate::media_scan::{analyze_file, scan_media, ScanOptions};
use crate::stop_control;
use crate::ToolPaths;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CheckFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub elapsed_seconds: f64,
}

pub(crate) struct CheckJobResult {
    pub files: Vec<CheckFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
}

#[derive(Clone)]
pub(crate) struct CheckProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: &'static str,
    pub file_percent: Option<f64>,
    pub queue_percent: Option<f64>,
}

pub(crate) fn analyze_paths(
    paths: &[PathBuf],
    tool_paths: ToolPaths,
    stop_file: Option<&Path>,
    mut on_progress: impl FnMut(CheckProgress),
    mut on_line: impl FnMut(String),
) -> Result<CheckJobResult, String> {
    let files = unique_media_files(paths)?;
    let host = crate::environment::inspect_environment(tool_paths, None)?;
    let ffprobe = optional_tool(&host, "ffprobe");
    let mediainfo = optional_tool(&host, "mediainfo");
    if ffprobe.is_none() {
        return Err(
            "Install FFprobe or choose its executable path before running Check only.".into(),
        );
    }
    let settings_root = files[0]
        .parent()
        .unwrap_or(Path::new("."))
        .to_string_lossy()
        .into_owned();
    let settings_payload =
        serde_json::to_value(crate::settings::load_effective_settings(&settings_root)?)
            .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    let settings = settings_payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_dir = crate::state_log_directory()?;
    fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the run log folder: {error}"))?;
    let log_path = log_dir.join(format!("hevc-rebuild-{stamp}-check.log"));
    let report_path = log_dir.join(format!("hevc-rebuild-{stamp}-check.csv"));
    let mut log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create the check log: {error}"))?;
    writeln!(
        log,
        "Check only · {} file{}",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    )
    .ok();

    let total = files.len();
    let mut results = Vec::with_capacity(total);
    let mut failed = 0;
    let mut stop_requested = false;
    for (index, path) in files.iter().enumerate() {
        if stop_file.is_some_and(stop_control::requested) {
            stop_requested = true;
            break;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        on_progress(CheckProgress {
            current: index + 1,
            total,
            file_name: name.clone(),
            file_path: path.to_string_lossy().into_owned(),
            phase: "Analyzing streams",
            file_percent: Some(0.0),
            queue_percent: Some((index as f64 / total as f64) * 100.0),
        });
        let line = format!("Checking {} ({}/{})", name, index + 1, total);
        on_line(line.clone());
        let _ = writeln!(log, "{line}");

        let started = Instant::now();
        let analysis = analyze_file(
            path,
            ffprobe.as_deref(),
            mediainfo.as_deref(),
            &settings,
            &host,
        );
        let result = result_from_analysis(path, &analysis, started);
        if result.status == "INTERRUPTED" {
            stop_requested = true;
        }
        if result.status == "FAILED" {
            failed += 1;
        }
        append_check_row(&report_path, &result, &analysis)?;
        let summary = format!("{}: {} · {}", name, result.status, result.message);
        on_line(summary.clone());
        let _ = writeln!(log, "{summary}");
        on_progress(CheckProgress {
            current: index + 1,
            total,
            file_name: name,
            file_path: path.to_string_lossy().into_owned(),
            phase: "Analysis complete",
            file_percent: Some(100.0),
            queue_percent: Some(((index + 1) as f64 / total as f64) * 100.0),
        });
        results.push(result);
        if stop_requested {
            break;
        }
    }
    let _ = log.flush();
    Ok(CheckJobResult {
        files: results,
        log_path,
        report_path,
        stop_requested,
        failed,
    })
}

/// Discover files for a Check-only run when the caller did not pass an explicit selection.
pub(crate) fn discover_check_files(
    root: &Path,
    recursive: bool,
    minimum_size_mib: u32,
    include: &[String],
    exclude: &[String],
    tool_paths: ToolPaths,
) -> Result<Vec<PathBuf>, String> {
    let scan = scan_media(
        ScanOptions {
            root: root.to_string_lossy().into_owned(),
            recursive,
            minimum_size_mib,
            filename_include_patterns: include.to_vec(),
            filename_exclude_patterns: exclude.to_vec(),
            tool_paths,
            stop_file: None,
        },
        |_, _, _| {},
    )?;
    let mut files = Vec::new();
    if let Some(entries) = scan.get("entries").and_then(Value::as_array) {
        for entry in entries {
            if let Some(path) = entry.get("path").and_then(Value::as_str) {
                files.push(PathBuf::from(path));
            }
        }
    }
    if files.is_empty() {
        return Err("No eligible media files were found for Check only.".into());
    }
    Ok(files)
}

pub(crate) fn cli_mode(args: &[String]) -> i32 {
    let mut files = Vec::new();
    let mut tool_paths = ToolPaths::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--file" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Check option {} requires a path.", args[index]);
                    return 2;
                };
                files.push(PathBuf::from(value));
                index += 2;
            }
            "--ffmpeg-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Check option --ffmpeg-path requires a path.");
                    return 2;
                };
                if tool_paths.ffmpeg.replace(value.clone()).is_some() {
                    eprintln!("Check option --ffmpeg-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--ffprobe-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Check option --ffprobe-path requires a path.");
                    return 2;
                };
                if tool_paths.ffprobe.replace(value.clone()).is_some() {
                    eprintln!("Check option --ffprobe-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--mediainfo-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Check option --mediainfo-path requires a path.");
                    return 2;
                };
                if tool_paths.mediainfo.replace(value.clone()).is_some() {
                    eprintln!("Check option --mediainfo-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            name => {
                eprintln!("Unknown check option {name}.");
                return 2;
            }
        }
    }
    if files.is_empty() {
        eprintln!("Check only requires at least one --file PATH.");
        return 2;
    }
    match analyze_paths(
        &files,
        tool_paths,
        None,
        |progress| {
            eprintln!(
                "check\t{}\t{}\t{}\t{:.1}",
                progress.current,
                progress.total,
                progress.file_path,
                progress.file_percent.unwrap_or(0.0)
            );
        },
        |_| {},
    ) {
        Ok(job) => match serde_json::to_string(&json!({
            "log_path": job.log_path,
            "report_path": job.report_path,
            "failed": job.failed,
            "stop_requested": job.stop_requested,
            "files": job.files,
        })) {
            Ok(payload) => {
                println!("{payload}");
                if job.failed > 0 {
                    1
                } else {
                    0
                }
            }
            Err(error) => {
                eprintln!("Could not serialize the check result: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn unique_media_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("Select at least one media file to check.".into());
    }
    if paths.len() > 10_000 {
        return Err("Check only accepts at most 10,000 files.".into());
    }
    let mut unique = BTreeSet::new();
    let mut files = Vec::new();
    for path in paths {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        if !canonical.is_file() {
            return Err(format!("{} is not a file.", canonical.display()));
        }
        if !unique.insert(canonical.clone()) {
            return Err("Selected media paths must be unique.".into());
        }
        files.push(canonical);
    }
    Ok(files)
}

fn result_from_analysis(
    path: &Path,
    analysis: &crate::media_scan::AnalyzedFile,
    started: Instant,
) -> CheckFileResult {
    let path_text = path.to_string_lossy().into_owned();
    if let Some(error) = analysis.error.as_deref() {
        return CheckFileResult {
            path: path_text,
            status: "FAILED".into(),
            message: error.to_string(),
            elapsed_seconds: elapsed(started),
        };
    }
    let mut message = "Analysis only; no changes made.".to_string();
    if let Some(warning) = analysis.warning.as_deref() {
        message.push(' ');
        message.push_str(warning);
    }
    if analysis
        .forecast
        .as_ref()
        .and_then(|value| value.get("level"))
        .and_then(Value::as_str)
        == Some("blocked")
    {
        message.push_str(
            " Memory forecast blocks encoding until available RAM recovers or the configured reserve is changed.",
        );
    }
    CheckFileResult {
        path: path_text,
        status: "INFO".into(),
        message,
        elapsed_seconds: elapsed(started),
    }
}

fn append_check_row(
    path: &Path,
    result: &CheckFileResult,
    analysis: &crate::media_scan::AnalyzedFile,
) -> Result<(), String> {
    crate::reports::append_native_row(path, report_row(result, analysis))
}

pub(crate) fn report_row(
    result: &CheckFileResult,
    analysis: &crate::media_scan::AnalyzedFile,
) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert("input".into(), json!(result.path));
    row.insert("output".into(), json!(""));
    row.insert("status".into(), json!(result.status));
    row.insert("message".into(), json!(result.message));
    row.insert("input_bytes".into(), json!(analysis.size_bytes));
    row.insert(
        "elapsed_seconds".into(),
        json!((result.elapsed_seconds * 10.0).round() / 10.0),
    );
    if let Some(profile) = analysis.profile.as_ref() {
        if let Some(codec) = profile.pointer("/video/codec") {
            row.insert("primary_video".into(), codec.clone());
        }
        let width = profile.pointer("/video/width").and_then(Value::as_i64);
        let height = profile.pointer("/video/height").and_then(Value::as_i64);
        if let (Some(width), Some(height)) = (width, height) {
            row.insert("resolution".into(), json!(format!("{width}x{height}")));
        }
        if let Some(count) = profile.get("audio_count") {
            row.insert("audio_count".into(), count.clone());
        }
        if let Some(count) = profile.get("subtitle_count") {
            row.insert("subtitle_count".into(), count.clone());
        }
        if let Some(hdr10) = profile.pointer("/hdr/hdr10") {
            row.insert("hdr10".into(), hdr10.clone());
        }
        if let Some(hdr10plus) = profile.pointer("/hdr/hdr10plus") {
            row.insert("hdr10plus".into(), hdr10plus.clone());
        }
        if let Some(dovi) = profile.pointer("/hdr/dolby_vision") {
            row.insert("dolby_vision".into(), dovi.clone());
        }
        if let Some(tune) = profile
            .pointer("/content/resolved_tune")
            .and_then(Value::as_str)
        {
            row.insert(
                "message".into(),
                json!(format!(
                    "{} Content tune: {}.",
                    result.message.trim_end_matches('.'),
                    if tune.is_empty() || tune == "none" {
                        "normal"
                    } else {
                        tune
                    }
                )),
            );
        }
    }
    if let Some(forecast) = analysis.forecast.as_ref() {
        if let Some(level) = forecast.get("level") {
            row.insert("memory_forecast".into(), level.clone());
        }
        if let Some(value) = forecast.get("available_gib") {
            row.insert("memory_available_gib".into(), value.clone());
        }
        if let Some(value) = forecast.get("total_gib") {
            row.insert("memory_total_gib".into(), value.clone());
        }
        if let Some(value) = forecast.get("swap_used_percent") {
            row.insert("memory_swap_used_percent".into(), value.clone());
        }
        if let Some(value) = forecast.get("planning_budget_gib") {
            row.insert("memory_budget_gib".into(), value.clone());
        }
        if let Some(value) = forecast.get("memory_worker_cap") {
            row.insert("memory_worker_cap".into(), value.clone());
        }
        if let Some(value) = forecast.get("planned_workers") {
            row.insert("planned_workers".into(), value.clone());
        }
        if let Some(value) = forecast.get("planned_frame_threads") {
            row.insert("planned_frame_threads".into(), value.clone());
        }
        if let Some(messages) = forecast.get("messages").and_then(Value::as_array) {
            let text = messages
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() {
                row.insert("memory_forecast_message".into(), json!(text));
            }
        }
    }
    row
}

pub(crate) fn optional_tool(host: &Value, key: &str) -> Option<String> {
    host.get("checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("key").and_then(Value::as_str) == Some(key))
        .filter(|check| check.get("available").and_then(Value::as_bool) == Some(true))
        .and_then(|check| check.get("path")?.as_str().map(str::to_owned))
}

fn elapsed(started: Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_scan::AnalyzedFile;

    #[test]
    fn unique_media_files_reject_empty_and_duplicates() {
        assert!(unique_media_files(&[]).is_err());
        let dir = std::env::temp_dir().join(format!(
            "hevc-rebuild-check-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clip.mkv");
        fs::write(&file, b"fixture").unwrap();
        let duplicate = unique_media_files(&[file.clone(), file.clone()]);
        let _ = fs::remove_file(&file);
        let _ = fs::remove_dir(&dir);
        assert!(duplicate.is_err());
    }

    #[test]
    fn report_row_maps_profile_and_blocked_forecast() {
        let analysis = AnalyzedFile {
            profile: Some(json!({
                "video": {"codec": "hevc", "width": 1920, "height": 1080},
                "audio_count": 2,
                "subtitle_count": 1,
                "hdr": {"hdr10": true, "hdr10plus": false, "dolby_vision": false},
                "content": {"resolved_tune": "animation"}
            })),
            forecast: Some(json!({
                "level": "blocked",
                "available_gib": 1.2,
                "total_gib": 16.0,
                "messages": ["Only 1.2 GiB RAM is available."]
            })),
            warning: None,
            error: None,
            size_bytes: 4096,
        };
        let result = CheckFileResult {
            path: "/media/Film.mkv".into(),
            status: "INFO".into(),
            message: "Analysis only; no changes made.".into(),
            elapsed_seconds: 1.25,
        };
        let row = report_row(&result, &analysis);
        assert_eq!(row.get("status").unwrap(), "INFO");
        assert_eq!(row.get("primary_video").unwrap(), "hevc");
        assert_eq!(row.get("resolution").unwrap(), "1920x1080");
        assert_eq!(row.get("audio_count").unwrap(), 2);
        assert_eq!(row.get("memory_forecast").unwrap(), "blocked");
        assert!(row
            .get("message")
            .and_then(Value::as_str)
            .unwrap()
            .contains("Content tune: animation"));
    }

    #[test]
    fn failed_analysis_keeps_the_probe_error() {
        let analysis = AnalyzedFile {
            profile: None,
            forecast: None,
            warning: None,
            error: Some("FFprobe could not read the file.".into()),
            size_bytes: 12,
        };
        let result =
            result_from_analysis(Path::new("/media/broken.mkv"), &analysis, Instant::now());
        assert_eq!(result.status, "FAILED");
        assert!(result.message.contains("FFprobe"));
    }
}
