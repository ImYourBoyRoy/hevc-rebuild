//! Keeps the saved GUI/MCP media queue in step with run results: marks entries
//! running, then records outcomes from each run's own CSV report, never assuming completion.

use super::*;

#[derive(Clone)]
pub(super) struct QueueRunSync {
    pub(super) queue_path: PathBuf,
    pub(super) media_paths: Vec<String>,
    pub(super) mode: String,
}

pub(super) fn queue_run_sync(
    root: &std::path::Path,
    selected_files: &[String],
    mode: &str,
) -> Result<Option<QueueRunSync>, String> {
    let queue_path = crate::media_queue_file()?;
    queue_run_sync_at(&queue_path, root, selected_files, mode)
}

pub(super) fn queue_run_sync_at(
    queue_path: &std::path::Path,
    root: &std::path::Path,
    selected_files: &[String],
    mode: &str,
) -> Result<Option<QueueRunSync>, String> {
    let queue = crate::media_queue::read_snapshot(queue_path)?;
    let selected = selected_files
        .iter()
        .filter_map(|path| PathBuf::from(path).canonicalize().ok())
        .collect::<std::collections::BTreeSet<_>>();
    let input_has_explicit_selection = !selected_files.is_empty();
    let mut tracked = std::collections::BTreeMap::<String, ()>::new();

    for source in queue.sources {
        let source_matches = PathBuf::from(&source.root)
            .canonicalize()
            .is_ok_and(|path| path == root);
        if !input_has_explicit_selection && !source_matches {
            continue;
        }
        let Some(entries) = source
            .scan_result
            .as_ref()
            .and_then(|result| result.get("entries"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(path) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let resolved = PathBuf::from(path).canonicalize().ok();
            let Some(resolved) = resolved else {
                continue;
            };
            if input_has_explicit_selection && !selected.contains(&resolved) {
                continue;
            }
            if !input_has_explicit_selection && !source_matches {
                continue;
            }
            tracked.insert(path.to_owned(), ());
        }
    }

    let media_paths = tracked.into_keys().collect::<Vec<_>>();
    if media_paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(QueueRunSync {
        queue_path: queue_path.to_path_buf(),
        media_paths,
        mode: mode.to_owned(),
    }))
}

pub(super) fn validate_queue_recovery(
    runs: &[PreparedMediaRun],
    acknowledged_paths: &[String],
) -> Result<(), ErrorData> {
    if acknowledged_paths.len() > 25_000 {
        return Err(ErrorData::invalid_params(
            "Review acknowledgments are limited to 25,000 media paths.",
            None,
        ));
    }
    let selected = runs
        .iter()
        .flat_map(|run| &run.selected_files)
        .map(|raw_path| PathBuf::from(raw_path).canonicalize())
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("A selected media file is no longer available: {error}"),
                None,
            )
        })?;
    let acknowledged = acknowledged_paths
        .iter()
        .map(|raw_path| PathBuf::from(raw_path).canonicalize())
        .collect::<Result<std::collections::BTreeSet<_>, _>>()
        .map_err(|error| {
            ErrorData::invalid_params(
                format!("A review acknowledgment path is unavailable: {error}"),
                None,
            )
        })?;
    if acknowledged.iter().any(|path| !selected.contains(path)) {
        return Err(ErrorData::invalid_params(
            "Every acknowledged review path must also be selected for this batch.",
            None,
        ));
    }
    let queue_path =
        crate::media_queue_file().map_err(|error| ErrorData::internal_error(error, None))?;
    let queue = crate::media_queue::read_snapshot(&queue_path)
        .map_err(|error| ErrorData::internal_error(error, None))?;
    let mut review_required = std::collections::BTreeSet::new();
    for (path, state) in queue.run_states {
        if !matches!(state.status.as_str(), "interrupted" | "needs_review") {
            continue;
        }
        if let Ok(canonical) = PathBuf::from(path).canonicalize() {
            if selected.contains(&canonical) && !acknowledged.contains(&canonical) {
                review_required.insert(canonical);
            }
        }
    }
    if !review_required.is_empty() {
        return Err(ErrorData::invalid_params(
            json!({
                "message": "Review the saved report/output for every interrupted or uncertain selected file, then include those paths in acknowledge_review_paths.",
                "review_required_count": review_required.len(),
                "review_required_paths": review_required.iter().take(50).collect::<Vec<_>>(),
            })
            .to_string(),
            None,
        ));
    }
    Ok(())
}

pub(crate) fn queue_run_json_cli(args: &[String]) -> i32 {
    if args.len() != 2 || args[0] != "--request" {
        eprintln!("Usage: hevc-rebuild-studio json queue-run --request QUEUE_RUN.json");
        return 2;
    }
    let request_path = PathBuf::from(&args[1]);
    let metadata = match request_path.metadata() {
        Ok(metadata) => metadata,
        Err(_) => {
            eprintln!("Queue run request file is unavailable.");
            return 2;
        }
    };
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        eprintln!("Queue run request must be a regular file no larger than 16 MiB.");
        return 2;
    }
    let request_bytes = match fs::read(&request_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Could not read queue run request: {error}");
            return 2;
        }
    };
    let request = match serde_json::from_slice::<StartQueueRunParams>(&request_bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid queue run JSON: {error}");
            return 2;
        }
    };
    if request.runs.is_empty() || request.runs.len() > 500 {
        eprintln!("A queued run must contain between 1 and 500 source runs.");
        return 2;
    }
    let stop_on_error = request.stop_on_error.unwrap_or(true);
    let acknowledge_review_paths = request.acknowledge_review_paths.unwrap_or_default();
    let id = job_id();
    let stop_token = match crate::stop_control::StopToken::new() {
        Ok(token) => token,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let stop_file = stop_token.path().to_path_buf();
    let mut prepared = Vec::with_capacity(request.runs.len());
    for mut run in request.runs {
        if run.selected_files.as_ref().is_none_or(Vec::is_empty) {
            eprintln!(
                "Every queued source run requires an explicit non-empty selected_files list."
            );
            return 2;
        }
        run.stop_on_error = Some(stop_on_error);
        match prepare_media_run(run, &stop_file) {
            Ok(plan) => prepared.push(plan),
            Err(error) => {
                eprintln!("Could not prepare queued media run: {error}");
                return 2;
            }
        }
    }
    if let Err(error) = validate_prepared_queue_runs(&prepared) {
        eprintln!("Queued run validation failed: {error}");
        return 2;
    }
    if let Err(error) = validate_queue_recovery(&prepared, &acknowledge_review_paths) {
        eprintln!("Queued run recovery check failed: {error}");
        return 2;
    }

    let mut queue_syncs = Vec::with_capacity(prepared.len());
    let mut queue_sync_warning = None;
    for run in &prepared {
        match queue_run_sync(&run.root, &run.selected_files, &run.mode) {
            Ok(sync) => queue_syncs.push(sync),
            Err(error) => {
                queue_sync_warning = Some(format!(
                    "The media queue will run without per-file queue synchronization: {error}"
                ));
                queue_syncs = vec![None; prepared.len()];
                break;
            }
        }
    }
    let mut queue_items_tracked = 0usize;
    if queue_sync_warning.is_none() {
        let mut queued_states = BTreeMap::new();
        for sync in queue_syncs.iter().flatten() {
            queue_items_tracked = queue_items_tracked.saturating_add(sync.media_paths.len());
            queued_states.extend(queue_run_states(
                &sync.media_paths,
                &sync.mode,
                "queued",
                Some("Waiting for the preceding source batch."),
            ));
        }
        if let Some(queue_path) = queue_syncs
            .iter()
            .flatten()
            .next()
            .map(|sync| &sync.queue_path)
        {
            if let Err(error) = crate::media_queue::save_run_states(queue_path, queued_states) {
                queue_sync_warning = Some(format!(
                    "The media queue will run, but queued states could not be saved: {error}"
                ));
                queue_syncs = vec![None; prepared.len()];
                queue_items_tracked = 0;
            }
        }
    }
    let plans = prepared
        .into_iter()
        .zip(queue_syncs)
        .map(|(run, queue_sync)| QueueRunPlan { run, queue_sync })
        .collect::<Vec<_>>();
    let source_count = plans.len();
    let file_count = plans
        .iter()
        .map(|plan| plan.run.selected_files.len())
        .sum::<usize>();
    let jobs: Jobs = Arc::new(Mutex::new(HashMap::new()));
    let mut job = Job::new(id.clone(), "media_queue_run");
    job.status = "running".into();
    job.total = file_count;
    job.stop_file = Some(stop_file.clone());
    job.queue_sync_warning = queue_sync_warning;
    if let Ok(mut registry) = jobs.lock() {
        registry.insert(id.clone(), job);
    }
    run_media_queue_job(
        jobs.clone(),
        id.clone(),
        plans,
        stop_file,
        stop_on_error,
        true,
    );
    let Some(job) = jobs
        .lock()
        .ok()
        .and_then(|registry| registry.get(&id).cloned())
    else {
        eprintln!("Could not read the completed queue-run status.");
        return 1;
    };
    let exit_code = job.exit_code.unwrap_or(1);
    let payload = json!({
        "job_id": job.id,
        "status": job.status,
        "source_count": source_count,
        "file_count": file_count,
        "completed_files": job.current,
        "exit_code": exit_code,
        "last_source": job.last_file,
        "report_path": job.report_path,
        "log_path": job.log_path,
        "error": job.error,
        "queue_items_tracked": queue_items_tracked,
        "queue_sync_warning": job.queue_sync_warning,
    });
    match serde_json::to_string(&payload) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("Could not encode queued run result: {error}");
            return 1;
        }
    }
    match job.status.as_str() {
        "completed" => 0,
        "stopped" => 130,
        _ => exit_code.max(1),
    }
}

pub(super) fn queue_run_states(
    paths: &[String],
    mode: &str,
    status: &str,
    message: Option<&str>,
) -> BTreeMap<String, crate::media_queue::QueueRunState> {
    let updated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                crate::media_queue::QueueRunState {
                    status: status.into(),
                    mode: Some(mode.into()),
                    message: message.map(str::to_owned),
                    output_path: None,
                    report_path: None,
                    report_status: None,
                    updated_at_ms,
                },
            )
        })
        .collect()
}

pub(super) fn queue_state_from_report_row(
    mode: &str,
    report_path: Option<&str>,
    row: Option<&Value>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> crate::media_queue::QueueRunState {
    let report_status = row
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_uppercase);
    let status = match report_status.as_deref() {
        Some("OK" | "METADATA_OK" | "TEST_OK" | "INFO" | "INTEGRITY_OK") => "completed",
        Some("FAILED" | "CORRUPT" | "UNREADABLE") => "failed",
        Some("SKIPPED") => "skipped",
        Some("INTERRUPTED") => "interrupted",
        _ => "needs_review",
    };
    let output_path = row
        .and_then(|value| value.get("output"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let message = row
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| match (exit_code, stop_requested) {
            (_, true) => {
                Some("No matching report row was found; review the output before retrying.".into())
            }
            (Some(0), false) => {
                Some("No matching report row was found; completion was not assumed.".into())
            }
            _ => {
                Some("The run ended without a matching report row; review before retrying.".into())
            }
        });
    let updated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    crate::media_queue::QueueRunState {
        status: status.into(),
        mode: Some(mode.into()),
        message,
        output_path,
        report_path: report_path.map(str::to_owned),
        report_status,
        updated_at_ms,
    }
}

pub(super) fn sync_queue_run_result(
    sync: QueueRunSync,
    report_path: Option<&str>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> Result<(), String> {
    let directory = crate::state_log_directory()?;
    sync_queue_run_result_in(&sync, &directory, report_path, exit_code, stop_requested)
}

pub(super) fn sync_queue_run_result_in(
    sync: &QueueRunSync,
    report_directory: &std::path::Path,
    report_path: Option<&str>,
    exit_code: Option<i32>,
    stop_requested: bool,
) -> Result<(), String> {
    let report = report_path
        .map(|path| crate::reports::read_report(report_directory, &PathBuf::from(path)))
        .transpose()?;
    let rows = report
        .as_ref()
        .and_then(|value| value.get("rows"))
        .and_then(Value::as_array);
    let updated = sync
        .media_paths
        .iter()
        .map(|path| {
            let row = rows.and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("input")
                        .and_then(Value::as_str)
                        .is_some_and(|input| input == path)
                })
            });
            (
                path.clone(),
                queue_state_from_report_row(
                    &sync.mode,
                    report_path,
                    row,
                    exit_code,
                    stop_requested,
                ),
            )
        })
        .collect();
    crate::media_queue::save_run_states(&sync.queue_path, updated)
}
