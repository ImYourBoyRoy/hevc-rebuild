//! Integrity and tool-check runs started from the desktop run controls.

use super::*;

pub(crate) fn start_integrity_run(
    app: AppHandle,
    state: State<'_, RunState>,
    request: RunRequest,
    root: PathBuf,
) -> Result<String, String> {
    let level =
        integrity::IntegrityLevel::parse(request.integrity_level.as_deref().unwrap_or("quick"))?;
    if request.selected_files.is_empty() {
        return Err("Select at least one media file to check.".into());
    }
    let mut selected = Vec::new();
    append_selected_file_arguments(&mut selected, &root, &request.selected_files)?;
    let files = selected
        .chunks(2)
        .filter_map(|pair| pair.get(1).map(PathBuf::from))
        .collect::<Vec<_>>();
    let mut tool_paths = request.tool_paths.unwrap_or_default();
    if tool_paths.ffmpeg.is_none() {
        tool_paths.ffmpeg = request.ffmpeg_path;
    }
    let disk_path = if root.is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root.clone()
    };
    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    claim_job(
        state.inner(),
        new_active_job(id.clone(), Some(stop_file.clone())),
    )?;
    let app_handle = app.clone();
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let _stop_token = stop_token;
        let resource_monitor = resource_monitor::monitor_process(
            std::process::id(),
            Some(disk_path),
            std::time::Duration::from_secs(1),
            {
                let app = app_handle.clone();
                let run_id = worker_id.clone();
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
        let mut log_path = None;
        let mut report_path = None;
        let mut error = None;
        let mut exit_code = Some(0);
        match integrity::check_paths(
            &files,
            level,
            tool_paths,
            Some(&stop_file),
            |progress| {
                let _ = app_handle.emit(
                    "run-progress",
                    RunProgress {
                        run_id: worker_id.clone(),
                        file_name: progress.file_name,
                        file_path: Some(progress.file_path),
                        phase: progress.phase.into(),
                        batch_number: progress.current as u32,
                        batch_total: progress.total as u32,
                        file_percent: progress.file_percent,
                        queue_percent: progress.queue_percent,
                        speed: None,
                        eta_seconds: None,
                        elapsed_seconds: None,
                        input_bytes: None,
                        output_bytes: None,
                        estimated_output_bytes: None,
                    },
                );
            },
            |line| {
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line,
                    },
                );
            },
        ) {
            Ok(job) => {
                log_path = Some(job.log_path.to_string_lossy().into_owned());
                report_path = Some(job.report_path.to_string_lossy().into_owned());
                let summary = if job.stop_requested {
                    format!(
                        "Integrity check stopped after {} file{}.",
                        job.files.len(),
                        if job.files.len() == 1 { "" } else { "s" }
                    )
                } else if job.failed == 0 {
                    format!(
                        "Integrity check passed for {} file{}.",
                        job.files.len(),
                        if job.files.len() == 1 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "Integrity check found {} damaged or unreadable file{}.",
                        job.failed,
                        if job.failed == 1 { "" } else { "s" }
                    )
                };
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: format!("Log: {}", job.log_path.display()),
                    },
                );
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: format!("Run report: {}", job.report_path.display()),
                    },
                );
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: summary,
                    },
                );
                if job.failed > 0 {
                    exit_code = Some(1);
                }
            }
            Err(start_error) => {
                error = Some(start_error);
                exit_code = Some(1);
            }
        }
        resource_monitor.stop();
        let stop_requested = stop_control::requested(&stop_file);
        let _ = fs::remove_file(&stop_file);
        let _ = app_handle.emit(
            "run-finished",
            RunFinished {
                run_id: worker_id.clone(),
                exit_code,
                stop_requested,
                log_path,
                report_path,
                error,
            },
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

pub(crate) fn start_check_run(
    app: AppHandle,
    state: State<'_, RunState>,
    request: RunRequest,
    root: PathBuf,
) -> Result<String, String> {
    let mut tool_paths = request.tool_paths.unwrap_or_default();
    if tool_paths.ffmpeg.is_none() {
        tool_paths.ffmpeg = request.ffmpeg_path;
    }
    let files = if request.selected_files.is_empty() {
        run_coordinator::discover_check_files(
            &root,
            request.recursive,
            request.minimum_size_mib,
            &request.filename_include_patterns,
            &request.filename_exclude_patterns,
            tool_paths.clone(),
        )?
    } else {
        let mut selected = Vec::new();
        append_selected_file_arguments(&mut selected, &root, &request.selected_files)?;
        selected
            .chunks(2)
            .filter_map(|pair| pair.get(1).map(PathBuf::from))
            .collect::<Vec<_>>()
    };
    let disk_path = if root.is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root.clone()
    };
    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    claim_job(
        state.inner(),
        new_active_job(id.clone(), Some(stop_file.clone())),
    )?;
    let app_handle = app.clone();
    let run_state = state.inner().clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let _stop_token = stop_token;
        let resource_monitor = resource_monitor::monitor_process(
            std::process::id(),
            Some(disk_path),
            std::time::Duration::from_secs(1),
            {
                let app = app_handle.clone();
                let run_id = worker_id.clone();
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
        let mut log_path = None;
        let mut report_path = None;
        let mut error = None;
        let mut exit_code = Some(0);
        match run_coordinator::analyze_paths(
            &files,
            tool_paths,
            Some(&stop_file),
            |progress| {
                let _ = app_handle.emit(
                    "run-progress",
                    RunProgress {
                        run_id: worker_id.clone(),
                        file_name: progress.file_name,
                        file_path: Some(progress.file_path),
                        phase: progress.phase.into(),
                        batch_number: progress.current as u32,
                        batch_total: progress.total as u32,
                        file_percent: progress.file_percent,
                        queue_percent: progress.queue_percent,
                        speed: None,
                        eta_seconds: None,
                        elapsed_seconds: None,
                        input_bytes: None,
                        output_bytes: None,
                        estimated_output_bytes: None,
                    },
                );
            },
            |line| {
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line,
                    },
                );
            },
        ) {
            Ok(job) => {
                log_path = Some(job.log_path.to_string_lossy().into_owned());
                report_path = Some(job.report_path.to_string_lossy().into_owned());
                let summary = if job.stop_requested {
                    format!(
                        "Check stopped after {} file{}.",
                        job.files.len(),
                        if job.files.len() == 1 { "" } else { "s" }
                    )
                } else if job.failed == 0 {
                    format!(
                        "Check finished for {} file{}. Nothing was written.",
                        job.files.len(),
                        if job.files.len() == 1 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "Check finished with {} unreadable file{}.",
                        job.failed,
                        if job.failed == 1 { "" } else { "s" }
                    )
                };
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: format!("Log: {}", job.log_path.display()),
                    },
                );
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: format!("Run report: {}", job.report_path.display()),
                    },
                );
                let _ = app_handle.emit(
                    "run-line",
                    RunLine {
                        run_id: worker_id.clone(),
                        line: summary,
                    },
                );
                if job.failed > 0 {
                    exit_code = Some(1);
                }
            }
            Err(start_error) => {
                error = Some(start_error);
                exit_code = Some(1);
            }
        }
        resource_monitor.stop();
        let stop_requested = stop_control::requested(&stop_file);
        let _ = fs::remove_file(&stop_file);
        let _ = app_handle.emit(
            "run-finished",
            RunFinished {
                run_id: worker_id.clone(),
                exit_code,
                stop_requested,
                log_path,
                report_path,
                error,
            },
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}
