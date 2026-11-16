//! ./desktop/src-tauri/src/metadata_run.rs
//! Native Metadata-only MKV run coordinator. Rust owns the whole job:
//! discovery, source profiling, HDR10+ source
//! verification, output planning, storage preflight, `mkvmerge` lossless remux
//! with live progress, reviewed metadata/poster embedding, packet-hash and HDR
//! validation, expected-tag verification, durable no-overwrite install, retained
//! `.error.mkv`/`.error.log` review artifacts, and CSV reports.
//!
//! Run: GUI `start_run`, MCP `start_run`, and the JSON CLI all hand the same
//! engine-style argument vector to `MetadataRunOptions::from_engine_args`, then
//! call `run_metadata_only`. Unspecified comment/blacklist/artwork/disk-reserve
//! values fall back to the effective settings.
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

mod artifacts;
mod execute;
mod options;
mod output;
mod progress;
mod source;

use artifacts::*;
use execute::*;
pub(crate) use options::MetadataRunOptions;
#[cfg(test)]
use options::*;
use output::*;
use progress::*;
pub(crate) use progress::{closing_lines, MetadataFileResult, MetadataJobResult, MetadataProgress};
use source::*;

const MIB: u64 = 1024 * 1024;
const SUPPORTED_STREAM_TYPES: [&str; 5] = ["video", "audio", "subtitle", "attachment", "data"];
const FINISHED_OUTPUT_PHASES: [&str; 5] = [
    "metadata embedding",
    "output validation",
    "lossless output validation",
    "validated output install",
    "transaction commit",
];

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
            .is_some_and(|(parent, name)| crate::backup_folder::has_backup(parent, name));
        if backed_up && !options.reprocess {
            skipped += 1;
        } else {
            eligible.push(file);
        }
    }
    if skipped > 0 {
        note(
            format!(
                "Skipped {skipped} already-processed file(s) with an existing originals backup"
            ),
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

#[cfg(test)]
mod tests;
