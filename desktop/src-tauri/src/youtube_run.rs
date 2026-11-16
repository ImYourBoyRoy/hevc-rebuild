//! ./desktop/src-tauri/src/youtube_run.rs
//! Native YouTube upload-copy coordinator. Rust owns the whole job:
//! discovery, filename disambiguation, source profiling,
//! HDR/Dolby Vision policy and per-file HDR10+ loss consent, bitrate and size
//! estimates, storage preflight, memory-aware encode planning with adaptive
//! retries, supervised FFmpeg encodes under the live resource guard, output
//! validation, no-overwrite atomic install, and CSV reports.
//!
//! Run: GUI `start_run`, MCP `start_run`, and the CLIs hand the same
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

mod encode;
mod execute;
mod optimize;
mod options;
mod policy;
mod progress;

use encode::*;
use execute::*;
use optimize::*;
pub(crate) use options::YoutubeRunOptions;
use policy::*;
use progress::*;
pub(crate) use progress::{closing_lines, YoutubeFileResult, YoutubeJobResult, YoutubeProgress};

const MAX_MEMORY_ATTEMPTS: u32 = 4;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const DEFAULT_EXPORT_FOLDER: &str = "YouTube Uploads";
const MIN_SOURCE_MULTIPLIER: f64 = 0.5;
const MAX_SOURCE_MULTIPLIER: f64 = 4.0;

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

struct Context<'a> {
    options: &'a YoutubeRunOptions,
    tools: &'a Tools,
    destination_root: &'a Path,
    shared_stems: &'a BTreeMap<String, usize>,
    progress: &'a mut dyn FnMut(YoutubeProgress),
    line: &'a mut dyn FnMut(String),
    child: &'a mut dyn FnMut(Option<u32>),
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

#[cfg(test)]
mod tests;
