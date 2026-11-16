//! ./desktop/src-tauri/src/rebuild_run.rs
//! Native Short-Test and Full Rebuild coordinator. Rust owns the whole job:
//! discovery, profiling, stream selection, content tune,
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

mod analysis;
mod artifacts;
mod calibrate;
mod encode;
mod finalize;
mod hdr10plus;
mod model;
mod options;
mod output;
mod progress;
mod resources;
mod status;
mod streams;

use analysis::*;
use artifacts::*;
use calibrate::*;
use encode::*;
use finalize::*;
use hdr10plus::*;
use model::*;
pub(crate) use options::RebuildRunOptions;
use options::*;
use output::*;
pub(crate) use progress::closing_lines;
use progress::*;
use resources::*;
use status::*;
pub(crate) use status::{
    BatchHalt, RebuildFileResult, RebuildJobResult, RebuildProgress, RunStatus,
};
use streams::*;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const MAX_ENCODE_ATTEMPTS: u32 = 3;

pub(crate) fn run_rebuild(
    options: &RebuildRunOptions,
    on_progress: impl FnMut(RebuildProgress),
    on_line: impl FnMut(String),
    on_child: impl FnMut(Option<u32>),
) -> Result<RebuildJobResult, String> {
    run_rebuild_observed(options, on_progress, on_line, on_child, |_| {})
}

/// Same run, but `on_file` is called as soon as each file finishes, so a
/// caller can report live results instead of waiting for the whole batch.
pub(crate) fn run_rebuild_observed(
    options: &RebuildRunOptions,
    mut on_progress: impl FnMut(RebuildProgress),
    mut on_line: impl FnMut(String),
    mut on_child: impl FnMut(Option<u32>),
    mut on_file: impl FnMut(&RebuildFileResult),
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
            .is_some_and(|(parent, name)| crate::backup_folder::has_backup(parent, name));
        if backed_up && !options.reprocess && !options.test_mode && !options.info_only {
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
            halted: None,
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
    let mut policy = HaltPolicy::new(options.max_consecutive_failures);
    let mut halted = None;
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
        let outcome = RunStatus::parse(&result.status).unwrap_or(RunStatus::Failed);
        match outcome {
            RunStatus::Ok | RunStatus::TestOk | RunStatus::Info => succeeded += 1,
            RunStatus::Skipped => skipped += 1,
            RunStatus::Interrupted => stop_requested = true,
            RunStatus::Failed => failed += 1,
        }
        let batch_halt = policy.observe(outcome, &result.message);
        let halt = outcome == RunStatus::Interrupted
            || (outcome == RunStatus::Failed && options.stop_on_error)
            || batch_halt.is_some();
        on_file(&result);
        results.push(result);
        if let Some(reason) = batch_halt {
            let remaining = total - (offset + 1);
            let text = format!(
                "{} ({remaining} file(s) not started.)",
                reason.explanation()
            );
            let _ = writeln!(log, "{text}");
            (context.line)(text);
            halted = Some(reason);
        }
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
        halted,
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
        phase: Phase::MediaAnalysis,
        temp_output: None,
        final_output: None,
        row: Map::new(),
        interrupted: false,
    };
    state
        .row
        .insert("input".into(), json!(source.to_string_lossy()));
    state.set_status(RunStatus::Failed);
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
                RunStatus::Interrupted
            } else if matches!(state.status(), RunStatus::Skipped | RunStatus::Info) {
                state.status()
            } else {
                RunStatus::Failed
            };
            if matches!(status, RunStatus::Failed | RunStatus::Interrupted) {
                state.set_status(status);
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
                    state.phase.failure_fraction(),
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
                    state.phase.label()
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
            remove_empty_auto_work_root(&work_dir);
        }
    }
    RebuildFileResult {
        path: source.to_string_lossy().into_owned(),
        status: state.status().as_str().into(),
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
        input_bytes,
        output_bytes: if state.status() == RunStatus::Ok {
            state
                .row
                .get("output")
                .and_then(Value::as_str)
                .and_then(|path| file_size(Path::new(path)))
        } else {
            None
        },
    }
}

fn expected_duration(options: &RebuildRunOptions, duration: f64) -> f64 {
    if options.test_mode {
        duration.min(options.test_seconds as f64)
    } else {
        duration
    }
}

/// Run one file through every stage. Each stage either advances the shared
/// `FilePlan`/`state` or returns an error that `process_one` records; the phase
/// recorded in `state` decides how a late failure is retained.
fn execute(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    work_dir: &Path,
    state: &mut FileState,
) -> Result<(), String> {
    let options = context.options;
    let mut plan = analyze_source(context, frame)?;
    verify_hdr10plus_sources(context, frame, &mut plan)?;
    plan.content_tune = resolve_content_tune(options, frame.source, &plan, state)?;
    if let Flow::Finished = resolve_dolby_vision(context, frame, state, &mut plan)? {
        return Ok(());
    }
    let info_plan = record_analysis_and_plan(context, frame, state, &mut plan);
    if options.info_only {
        finish_info_only(context, frame, state, &plan, &info_plan);
        return Ok(());
    }
    let targets = prepare_output(context, frame, state, &plan)?;
    let hdr10plus = extract_hdr10plus(context, frame, state, work_dir, &plan)?;
    encode_video(
        context, frame, state, &plan, &targets, &hdr10plus, info_plan,
    )?;
    inject_hdr10plus(context, frame, state, work_dir, &plan, &targets, &hdr10plus)?;
    let mut embedded = embed_metadata(context, frame, state, work_dir, &plan, &targets)?;
    if plan.unstorable_data_streams > 0 {
        embedded.notes.push_str(&format!(
            " {} data stream(s) cannot be stored in Matroska and were not copied.",
            plan.unstorable_data_streams
        ));
    }
    validate_output(context, frame, state, work_dir, &plan, &targets, &embedded)?;
    let quality_note = check_quality(context, frame, state, &plan, &targets)?;
    embedded.notes.push_str(&quality_note);
    install_output(context, frame, state, &targets, &embedded)
}

#[cfg(test)]
mod tests;
