//! Native job runners that drive metadata, YouTube and rebuild runs for the desktop window.

use super::*;

/// Final facts every native run reports back to the shell.
pub(crate) struct NativeOutcome {
    pub(crate) log_path: PathBuf,
    pub(crate) report_path: PathBuf,
    pub(crate) failed: usize,
}

/// Shared shell for runs that execute fully in Rust: resource monitoring,
/// line forwarding, and the `run-finished` event.
fn run_native_job(
    app: &Events,
    state: &RunState,
    run_id: &str,
    disk_path: PathBuf,
    stop_file: &Path,
    work: impl FnOnce(&dyn Fn(String)) -> Result<NativeOutcome, String>,
) {
    let resource_monitor = resource_monitor::monitor_process(
        std::process::id(),
        Some(disk_path),
        std::time::Duration::from_secs(1),
        {
            let app = app.clone();
            let run_id = run_id.to_string();
            move |snapshot| {
                let _ = app.emit(
                    "run-resources",
                    RunResourceEvent {
                        run_id: run_id.clone(),
                        snapshot,
                    },
                );
            }
        },
    );
    let emit_line = |line: String| {
        let _ = app.emit(
            "run-line",
            RunLine {
                run_id: run_id.to_string(),
                line,
            },
        );
    };
    let mut log_path = None;
    let mut report_path = None;
    let mut error = None;
    let mut exit_code = Some(0);
    match work(&emit_line) {
        Ok(outcome) => {
            log_path = Some(outcome.log_path.to_string_lossy().into_owned());
            report_path = Some(outcome.report_path.to_string_lossy().into_owned());
            if outcome.failed > 0 {
                exit_code = Some(1);
            }
        }
        Err(start_error) => {
            error = Some(start_error);
            exit_code = Some(1);
        }
    }
    resource_monitor.stop();
    let stop_requested = stop_control::requested(stop_file);
    let _ = fs::remove_file(stop_file);
    let _ = app.emit(
        "run-finished",
        RunFinished {
            run_id: run_id.to_string(),
            exit_code,
            stop_requested,
            log_path,
            report_path,
            error,
        },
    );
    release_job(state, run_id);
}

/// Metadata-only MKV runs execute fully in Rust.
pub(crate) fn run_native_metadata_job(
    app: &Events,
    state: &RunState,
    run_id: &str,
    options: metadata_run::MetadataRunOptions,
    disk_path: PathBuf,
    stop_file: &Path,
) {
    run_native_job(app, state, run_id, disk_path, stop_file, |emit_line| {
        let job = metadata_run::run_metadata_only(
            &options,
            |progress| {
                let _ = app.emit(
                    "run-progress",
                    RunProgress {
                        run_id: run_id.to_string(),
                        file_name: progress.file_name,
                        file_path: Some(progress.file_path),
                        phase: progress.phase,
                        batch_number: progress.current as u32,
                        batch_total: progress.total as u32,
                        file_percent: progress.file_percent,
                        queue_percent: Some(progress.queue_percent),
                        speed: None,
                        eta_seconds: None,
                        elapsed_seconds: Some(progress.elapsed_seconds),
                        input_bytes: progress.input_bytes,
                        output_bytes: progress.output_bytes,
                        estimated_output_bytes: None,
                    },
                );
            },
            emit_line,
            child_tracker(state, run_id),
        )?;
        for line in metadata_run::closing_lines(&job) {
            emit_line(line);
        }
        Ok(NativeOutcome {
            log_path: job.log_path,
            report_path: job.report_path,
            failed: job.failed,
        })
    });
}

/// YouTube exports execute fully in Rust: FFmpeg is spawned and supervised
/// in-process.
pub(crate) fn run_native_youtube_job(
    app: &Events,
    state: &RunState,
    run_id: &str,
    options: youtube_run::YoutubeRunOptions,
    disk_path: PathBuf,
    stop_file: &Path,
) {
    run_native_job(app, state, run_id, disk_path, stop_file, |emit_line| {
        let job = youtube_run::run_youtube_export(
            &options,
            |progress| {
                let _ = app.emit(
                    "run-progress",
                    RunProgress {
                        run_id: run_id.to_string(),
                        file_name: progress.file_name,
                        file_path: Some(progress.file_path),
                        phase: progress.phase,
                        batch_number: progress.current as u32,
                        batch_total: progress.total as u32,
                        file_percent: progress.file_percent,
                        queue_percent: Some(progress.queue_percent),
                        speed: progress.speed,
                        eta_seconds: progress.eta_seconds,
                        elapsed_seconds: Some(progress.elapsed_seconds),
                        input_bytes: progress.input_bytes,
                        output_bytes: progress.output_bytes,
                        estimated_output_bytes: progress.estimated_output_bytes,
                    },
                );
            },
            emit_line,
            child_tracker(state, run_id),
        )?;
        for line in youtube_run::closing_lines(&job) {
            emit_line(line);
        }
        Ok(NativeOutcome {
            log_path: job.log_path,
            report_path: job.report_path,
            failed: job.failed,
        })
    });
}

/// Short-test and full rebuilds execute fully in Rust.
pub(crate) fn run_native_rebuild_job(
    app: &Events,
    state: &RunState,
    run_id: &str,
    options: rebuild_run::RebuildRunOptions,
    disk_path: PathBuf,
    stop_file: &Path,
) {
    run_native_job(app, state, run_id, disk_path, stop_file, |emit_line| {
        let job = rebuild_run::run_rebuild(
            &options,
            |progress| {
                let _ = app.emit(
                    "run-progress",
                    RunProgress {
                        run_id: run_id.to_string(),
                        file_name: progress.file_name,
                        file_path: Some(progress.file_path),
                        phase: progress.phase,
                        batch_number: progress.current as u32,
                        batch_total: progress.total as u32,
                        file_percent: progress.file_percent,
                        queue_percent: Some(progress.queue_percent),
                        speed: progress.speed,
                        eta_seconds: progress.eta_seconds,
                        elapsed_seconds: Some(progress.elapsed_seconds),
                        input_bytes: progress.input_bytes,
                        output_bytes: progress.output_bytes,
                        estimated_output_bytes: progress.estimated_output_bytes,
                    },
                );
            },
            emit_line,
            child_tracker(state, run_id),
        )?;
        for line in rebuild_run::closing_lines(&job) {
            emit_line(line);
        }
        Ok(NativeOutcome {
            log_path: job.log_path,
            report_path: job.report_path,
            failed: job.failed,
        })
    });
}
