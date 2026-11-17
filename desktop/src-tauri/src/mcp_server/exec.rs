//! Job executors: run a prepared request on the native Rust engines, publish live
//! progress and per-file results into the job registry, and set the final status.

use super::*;

pub(super) fn run_integrity_job(
    jobs: Jobs,
    id: String,
    files: Vec<PathBuf>,
    level: crate::integrity::IntegrityLevel,
    tool_paths: ToolPaths,
    stop_file: PathBuf,
) {
    let result = crate::integrity::check_paths(
        &files,
        level,
        tool_paths,
        Some(&stop_file),
        |progress| {
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.get_mut(&id) {
                    job.set_progress(
                        progress.current,
                        progress.total,
                        progress.file_path,
                        ProgressDetail {
                            phase: progress.phase.to_string(),
                            file_percent: progress.file_percent,
                            queue_percent: progress.queue_percent,
                            ..ProgressDetail::default()
                        },
                    );
                }
            }
        },
        |line| record_line(&jobs, &id, line),
    );
    let _ = fs::remove_file(&stop_file);
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            match result {
                Ok(payload) => {
                    job.status = if payload.stop_requested {
                        "stopped".into()
                    } else if payload.failed > 0 {
                        "failed".into()
                    } else {
                        "completed".into()
                    };
                    job.log_path = Some(payload.log_path.to_string_lossy().into_owned());
                    job.report_path = Some(payload.report_path.to_string_lossy().into_owned());
                    job.exit_code = Some(if payload.failed > 0 { 1 } else { 0 });
                    job.current = payload.files.len();
                    job.total = files.len();
                    job.result = serde_json::to_value(&payload.files).ok();
                    if payload.stop_requested {
                        job.error =
                            Some("The integrity check stopped before every file finished.".into());
                    }
                }
                Err(error) => {
                    job.status = "failed".into();
                    job.error = Some(error);
                }
            }
        }
    }
}

pub(super) fn run_scan_job(jobs: Jobs, id: String, options: crate::media_scan::ScanOptions) {
    let result = crate::media_scan::scan_media(options, |current, total, file| {
        if let Ok(mut jobs) = jobs.lock() {
            if let Some(job) = jobs.get_mut(&id) {
                job.current = current;
                job.total = total;
                job.last_file = Some(file.to_string());
            }
        }
    });
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            match result {
                Ok(payload) => {
                    let interrupted = payload
                        .get("interrupted")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    job.status = if interrupted {
                        "interrupted".into()
                    } else {
                        "completed".into()
                    };
                    job.result = Some(payload);
                    if job.total == 0 {
                        job.total = job
                            .result
                            .as_ref()
                            .and_then(|v| v.get("candidate_count"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as usize;
                        job.current = job.total;
                    }
                }
                Err(error) => {
                    job.status = "failed".into();
                    job.error = Some(error);
                }
            }
        }
    }
}

pub(super) fn execute_native_check(
    jobs: &Jobs,
    id: &str,
    prepared: &PreparedMediaRun,
    stop_file: &Path,
) -> Result<i32, String> {
    let files = if prepared.selected_files.is_empty() {
        crate::run_coordinator::discover_check_files(
            &prepared.root,
            prepared.recursive,
            prepared.minimum_size_mib,
            &prepared.filename_include_patterns,
            &prepared.filename_exclude_patterns,
            prepared.tool_paths.clone(),
        )?
    } else {
        prepared
            .selected_files
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    };
    let result = crate::run_coordinator::analyze_paths(
        &files,
        prepared.tool_paths.clone(),
        Some(stop_file),
        |progress| {
            if let Ok(mut jobs) = jobs.lock() {
                if let Some(job) = jobs.get_mut(id) {
                    job.set_progress(
                        progress.current,
                        progress.total,
                        progress.file_path,
                        ProgressDetail {
                            phase: progress.phase.to_string(),
                            file_percent: progress.file_percent,
                            queue_percent: progress.queue_percent,
                            ..ProgressDetail::default()
                        },
                    );
                }
            }
        },
        |line| record_line(jobs, id, line),
    )?;
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.log_path = Some(result.log_path.to_string_lossy().into_owned());
            job.report_path = Some(result.report_path.to_string_lossy().into_owned());
            job.result = serde_json::to_value(&result.files).ok();
        }
    }
    Ok(if result.failed > 0 { 1 } else { 0 })
}

/// Metadata-only MKV runs execute natively in Rust and report through the same
/// job lines the native coordinators emit (`Log:` / `Run report:`).
pub(super) fn execute_native_metadata_args(
    jobs: &Jobs,
    id: &str,
    args: &[String],
    echo_to_stderr: bool,
) -> Result<i32, String> {
    let options = crate::metadata_run::MetadataRunOptions::from_engine_args(args)?;
    let disk_path = if options.root.is_file() {
        options.root.parent().map(PathBuf::from)
    } else {
        Some(options.root.clone())
    };
    let resource_jobs = jobs.clone();
    let resource_id = id.to_owned();
    let resource_monitor = crate::resource_monitor::monitor_process(
        std::process::id(),
        disk_path,
        Duration::from_secs(1),
        move |snapshot| {
            if let Ok(mut registry) = resource_jobs.lock() {
                if let Some(job) = registry.get_mut(&resource_id) {
                    job.resource_snapshot = Some(snapshot);
                }
            }
        },
    );
    let result = crate::metadata_run::run_metadata_only(
        &options,
        |progress| {
            if let Ok(mut registry) = jobs.lock() {
                if let Some(job) = registry.get_mut(id) {
                    job.set_progress(
                        progress.current,
                        progress.total,
                        progress.file_path,
                        ProgressDetail {
                            phase: progress.phase,
                            file_percent: progress.file_percent,
                            queue_percent: Some(progress.queue_percent),
                            elapsed_seconds: Some(progress.elapsed_seconds),
                            input_bytes: progress.input_bytes,
                            output_bytes: progress.output_bytes,
                            ..ProgressDetail::default()
                        },
                    );
                }
            }
        },
        |line| {
            if echo_to_stderr {
                eprintln!("{line}");
            }
            record_line(jobs, id, line);
        },
        |_| {},
    );
    resource_monitor.stop();
    let result = result?;
    for line in crate::metadata_run::closing_lines(&result) {
        if echo_to_stderr {
            eprintln!("{line}");
        }
        record_line(jobs, id, line);
    }
    if let Ok(mut registry) = jobs.lock() {
        if let Some(job) = registry.get_mut(id) {
            job.log_path = Some(result.log_path.to_string_lossy().into_owned());
            job.report_path = Some(result.report_path.to_string_lossy().into_owned());
            job.result = serde_json::to_value(&result.files).ok();
        }
    }
    Ok(i32::from(result.failed > 0))
}

/// YouTube exports execute natively in Rust (in-process FFmpeg supervision).
pub(super) fn execute_native_youtube_args(
    jobs: &Jobs,
    id: &str,
    args: &[String],
    echo_to_stderr: bool,
) -> Result<i32, String> {
    let options = crate::youtube_run::YoutubeRunOptions::from_engine_args(args)?;
    let disk_path = options.output_dir.clone().or_else(|| {
        if options.root.is_file() {
            options.root.parent().map(PathBuf::from)
        } else {
            Some(options.root.clone())
        }
    });
    let resource_jobs = jobs.clone();
    let resource_id = id.to_owned();
    let resource_monitor = crate::resource_monitor::monitor_process(
        std::process::id(),
        disk_path,
        Duration::from_secs(1),
        move |snapshot| {
            if let Ok(mut registry) = resource_jobs.lock() {
                if let Some(job) = registry.get_mut(&resource_id) {
                    job.resource_snapshot = Some(snapshot);
                }
            }
        },
    );
    let result = crate::youtube_run::run_youtube_export(
        &options,
        |progress| {
            if let Ok(mut registry) = jobs.lock() {
                if let Some(job) = registry.get_mut(id) {
                    job.set_progress(
                        progress.current,
                        progress.total,
                        progress.file_path,
                        ProgressDetail {
                            phase: progress.phase,
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
                }
            }
        },
        |line| {
            if echo_to_stderr {
                eprintln!("{line}");
            }
            record_line(jobs, id, line);
        },
        |_| {},
    );
    resource_monitor.stop();
    let result = result?;
    for line in crate::youtube_run::closing_lines(&result) {
        if echo_to_stderr {
            eprintln!("{line}");
        }
        record_line(jobs, id, line);
    }
    if let Ok(mut registry) = jobs.lock() {
        if let Some(job) = registry.get_mut(id) {
            job.log_path = Some(result.log_path.to_string_lossy().into_owned());
            job.report_path = Some(result.report_path.to_string_lossy().into_owned());
            job.result = serde_json::to_value(&result.files).ok();
        }
    }
    Ok(i32::from(result.failed > 0))
}

/// Short-test and full rebuild execute natively in Rust.
pub(super) fn execute_native_rebuild_args(
    jobs: &Jobs,
    id: &str,
    args: &[String],
    echo_to_stderr: bool,
) -> Result<i32, String> {
    let options = crate::rebuild_run::RebuildRunOptions::from_engine_args(args)?;
    let disk_path = options.output_dir.clone().or_else(|| {
        if options.root.is_file() {
            options.root.parent().map(PathBuf::from)
        } else {
            Some(options.root.clone())
        }
    });
    let resource_jobs = jobs.clone();
    let resource_id = id.to_owned();
    let resource_monitor = crate::resource_monitor::monitor_process(
        std::process::id(),
        disk_path,
        Duration::from_secs(1),
        move |snapshot| {
            if let Ok(mut registry) = resource_jobs.lock() {
                if let Some(job) = registry.get_mut(&resource_id) {
                    job.resource_snapshot = Some(snapshot);
                }
            }
        },
    );
    let live_disk_path = options.root.clone();
    let tracker = std::cell::RefCell::new(crate::batch_plan::BatchTracker::new(0));
    let publish = |tracker: &crate::batch_plan::BatchTracker| {
        let free = crate::media_scan::disk_free_bytes(&live_disk_path);
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(id) {
                job.batch = Some(tracker.to_json(free));
            }
        }
    };
    let result = crate::rebuild_run::run_rebuild_observed(
        &options,
        |progress| {
            tracker.borrow_mut().set_total(progress.total);
            if let Ok(mut registry) = jobs.lock() {
                if let Some(job) = registry.get_mut(id) {
                    job.set_progress(
                        progress.current,
                        progress.total,
                        progress.file_path,
                        ProgressDetail {
                            phase: progress.phase,
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
                }
            }
        },
        |line| {
            if echo_to_stderr {
                eprintln!("{line}");
            }
            record_line(jobs, id, line);
        },
        |_| {},
        |file| {
            let mut tracker = tracker.borrow_mut();
            tracker.record(file);
            publish(&tracker);
        },
    );
    resource_monitor.stop();
    let result = result?;
    {
        let mut tracker = tracker.borrow_mut();
        tracker.set_halted(result.halted);
        let free = crate::media_scan::disk_free_bytes(&live_disk_path);
        let stopped_by_user = result.stop_requested && result.halted.is_none();
        let report = tracker.final_report(free, stopped_by_user);
        let mut batch = tracker.to_json(free);
        batch["report"] = json!(report);
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(id) {
                job.batch = Some(batch);
            }
        }
    }
    for line in crate::rebuild_run::closing_lines(&result) {
        if echo_to_stderr {
            eprintln!("{line}");
        }
        record_line(jobs, id, line);
    }
    if let Ok(mut registry) = jobs.lock() {
        if let Some(job) = registry.get_mut(id) {
            job.log_path = Some(result.log_path.to_string_lossy().into_owned());
            job.report_path = Some(result.report_path.to_string_lossy().into_owned());
            job.result = serde_json::to_value(&result.files).ok();
        }
    }
    if result.halted.is_some() {
        return Ok(EXIT_PAUSED);
    }
    Ok(i32::from(result.failed > 0))
}

pub(super) fn execute_media_args(
    jobs: &Jobs,
    id: &str,
    args: &[String],
    echo_to_stderr: bool,
) -> Result<i32, String> {
    if args.iter().any(|argument| argument == "--youtube-upload") {
        return execute_native_youtube_args(jobs, id, args, echo_to_stderr);
    }
    if args.iter().any(|argument| argument == "--metadata-only") {
        return execute_native_metadata_args(jobs, id, args, echo_to_stderr);
    }
    execute_native_rebuild_args(jobs, id, args, echo_to_stderr)
}

pub(super) fn run_check_job(
    jobs: Jobs,
    id: String,
    prepared: PreparedMediaRun,
    stop_file: PathBuf,
    queue_sync: Option<QueueRunSync>,
) {
    let outcome = execute_native_check(&jobs, &id, &prepared, &stop_file);
    finish_media_job(jobs, id, stop_file, queue_sync, outcome);
}

pub(super) fn run_media_job(
    jobs: Jobs,
    id: String,
    args: Vec<String>,
    stop_file: PathBuf,
    queue_sync: Option<QueueRunSync>,
) {
    let outcome = execute_media_args(&jobs, &id, &args, false);
    finish_media_job(jobs, id, stop_file, queue_sync, outcome);
}

pub(super) fn finish_media_job(
    jobs: Jobs,
    id: String,
    stop_file: PathBuf,
    queue_sync: Option<QueueRunSync>,
    outcome: Result<i32, String>,
) {
    let stop_requested = crate::stop_control::requested(&stop_file);
    let _ = fs::remove_file(&stop_file);
    let exit_code = outcome.as_ref().ok().copied();
    let report_path = jobs
        .lock()
        .ok()
        .and_then(|registry| registry.get(&id).and_then(|job| job.report_path.clone()));
    let mut queue_sync_warning = None;
    if let Some(sync) = queue_sync {
        if let Err(error) =
            sync_queue_run_result(sync, report_path.as_deref(), exit_code, stop_requested)
        {
            queue_sync_warning = Some(format!(
                "The media run ended, but its per-file queue results could not be saved: {error}"
            ));
        }
    }
    if let Ok(mut jobs) = jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.stop_file = None;
            if queue_sync_warning.is_some() {
                job.queue_sync_warning = queue_sync_warning;
            }
            match outcome {
                Ok(EXIT_PAUSED) => {
                    job.exit_code = Some(EXIT_PAUSED);
                    job.status = "paused".into();
                    job.error = job
                        .batch
                        .as_ref()
                        .and_then(|batch| batch.pointer("/halted/explanation"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Ok(1) if job.kind == "library_batch" => {
                    // Per-file failures are expected in a big unattended library: every
                    // other file still finished, and each failure is listed in batch.failures.
                    job.exit_code = Some(1);
                    job.status = "completed_with_failures".into();
                }
                Ok(code) => {
                    job.exit_code = Some(code);
                    job.status = if code == 0 {
                        if stop_requested {
                            "stopped"
                        } else {
                            "completed"
                        }
                    } else {
                        "failed"
                    }
                    .into();
                    if code != 0 {
                        job.error = Some(format!(
                            "Media engine exited with code {code}; inspect recent diagnostic lines."
                        ));
                    }
                }
                Err(error) => {
                    job.status = "failed".into();
                    job.error = Some(error);
                }
            }
        }
    }
}

pub(super) fn run_media_queue_job(
    jobs: Jobs,
    id: String,
    plans: Vec<QueueRunPlan>,
    stop_file: PathBuf,
    stop_on_error: bool,
    echo_to_stderr: bool,
) {
    let total_files = plans
        .iter()
        .map(|plan| plan.run.selected_files.len())
        .sum::<usize>();
    let source_count = plans.len();
    let mut completed_files = 0usize;
    let mut failure_code = None;
    let mut first_error = None;
    let mut stop_requested = false;
    let mut paused = false;
    let mut queue_sync_warning = None;

    for (index, plan) in plans.into_iter().enumerate() {
        if crate::stop_control::requested(&stop_file) {
            stop_requested = true;
            break;
        }
        let source_label = plan.run.root.to_string_lossy().into_owned();
        if echo_to_stderr {
            eprintln!(
                "Queue source {}/{}: {}",
                index + 1,
                source_count,
                source_label
            );
        }
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(&id) {
                job.current = completed_files;
                job.total = total_files;
                job.last_file = Some(source_label.clone());
                job.report_path = None;
            }
        }
        if let Some(sync) = plan.queue_sync.as_ref() {
            let states = queue_run_states(
                &sync.media_paths,
                &sync.mode,
                "running",
                Some("This source is being processed in sequence."),
            );
            if let Err(error) = crate::media_queue::save_run_states(&sync.queue_path, states) {
                queue_sync_warning.get_or_insert_with(|| {
                    format!("Could not mark a source batch active in the saved queue: {error}")
                });
            }
        }

        let outcome = if plan.run.mode == "check" {
            execute_native_check(&jobs, &id, &plan.run, &stop_file)
        } else {
            execute_media_args(&jobs, &id, &plan.run.args, echo_to_stderr)
        };
        let exit_code = outcome.as_ref().ok().copied();
        let report_path = jobs
            .lock()
            .ok()
            .and_then(|registry| registry.get(&id).and_then(|job| job.report_path.clone()));
        let source_stop_requested = crate::stop_control::requested(&stop_file);
        if let Some(sync) = plan.queue_sync {
            if let Err(error) = sync_queue_run_result(
                sync,
                report_path.as_deref(),
                exit_code,
                source_stop_requested,
            ) {
                queue_sync_warning.get_or_insert_with(|| {
                    format!("A source finished, but its queue result could not be saved: {error}")
                });
            }
        }

        match outcome {
            Ok(0) => {
                completed_files = completed_files.saturating_add(plan.run.selected_files.len())
            }
            Ok(EXIT_PAUSED) => {
                // The engine paused itself (low disk or a failure streak): later
                // sources must not start, and this is not a plain failure.
                paused = true;
                first_error.get_or_insert_with(|| {
                    format!("Paused during {source_label} to protect the disk or the library.")
                });
            }
            Ok(code) => {
                failure_code.get_or_insert(code);
                first_error.get_or_insert_with(|| {
                    format!("Media engine exited with code {code} for {source_label}.")
                });
            }
            Err(error) => {
                failure_code.get_or_insert(1);
                first_error.get_or_insert_with(|| format!("{source_label}: {error}"));
            }
        }
        stop_requested = source_stop_requested;
        if let Ok(mut registry) = jobs.lock() {
            if let Some(job) = registry.get_mut(&id) {
                job.current = completed_files;
                job.total = total_files;
            }
        }
        if stop_requested || paused || (failure_code.is_some() && stop_on_error) {
            break;
        }
    }

    let stop_requested = stop_requested || crate::stop_control::requested(&stop_file);
    let _ = fs::remove_file(&stop_file);
    if let Ok(mut registry) = jobs.lock() {
        if let Some(job) = registry.get_mut(&id) {
            job.stop_file = None;
            job.current = completed_files;
            job.total = total_files;
            if queue_sync_warning.is_some() {
                job.queue_sync_warning = queue_sync_warning;
            }
            if paused && failure_code.is_none() {
                job.status = "paused".into();
                job.exit_code = Some(EXIT_PAUSED);
                job.error = first_error;
            } else if let Some(code) = failure_code {
                job.status = "failed".into();
                job.exit_code = Some(code);
                job.error = first_error;
            } else if stop_requested {
                job.status = "stopped".into();
                job.exit_code = Some(130);
            } else {
                job.status = "completed".into();
                job.exit_code = Some(0);
            }
        }
    }
}
