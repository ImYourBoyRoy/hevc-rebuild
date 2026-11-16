//! Tauri commands for environment inspection, effective settings, work directory and folder scans.

use super::*;

#[tauri::command]
pub(crate) async fn inspect_environment(tool_paths: Option<ToolPaths>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        environment::inspect_environment(tool_paths.unwrap_or_default(), None)
    })
    .await
    .map_err(|error| format!("Environment scan task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn load_settings(root: String) -> Result<settings::EffectiveSettings, String> {
    tauri::async_runtime::spawn_blocking(move || settings::load_effective_settings(&root))
        .await
        .map_err(|error| format!("Settings load task failed: {error}"))?
}

#[tauri::command]
pub(crate) fn resolve_work_directory_command(
    root: String,
    work_root: Option<String>,
) -> Result<WorkDirectoryPreview, String> {
    resolve_work_directory(&root, work_root.as_deref())
}

#[tauri::command]
pub(crate) fn default_media_root() -> Result<String, String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Could not locate the user's home folder.")?;
    let videos = home.join("Videos");
    Ok(if videos.is_dir() { videos } else { home }
        .to_string_lossy()
        .into_owned())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ScanFolderOptions {
    root: String,
    recursive: bool,
    minimum_size_mib: u32,
    filename_include_patterns: Vec<String>,
    filename_exclude_patterns: Vec<String>,
    tool_paths: Option<ToolPaths>,
}

#[tauri::command]
pub(crate) async fn scan_folder(
    app: AppHandle,
    state: State<'_, RunState>,
    options: ScanFolderOptions,
) -> Result<Value, String> {
    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    claim_job(&state, new_active_job(id.clone(), Some(stop_file.clone())))?;
    let run_state = (*state).clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        media_scan::scan_media(
            media_scan::ScanOptions {
                root: options.root,
                recursive: options.recursive,
                minimum_size_mib: options.minimum_size_mib,
                filename_include_patterns: options.filename_include_patterns,
                filename_exclude_patterns: options.filename_exclude_patterns,
                tool_paths: options.tool_paths.unwrap_or_default(),
                stop_file: Some(stop_file),
            },
            |current, total, file| {
                let _ = app.emit(
                    "scan-progress",
                    json!({ "current": current, "total": total, "file": file }),
                );
            },
        )
    })
    .await
    .map_err(|error| format!("Folder scan task failed: {error}"));
    release_job(&run_state, &id);
    drop(stop_token);
    result?
}

#[tauri::command]
pub(crate) fn cancel_scan(state: State<'_, RunState>) -> Result<(), String> {
    let path = {
        let current = state
            .0
            .lock()
            .map_err(|_| "Could not lock the desktop job state.")?;
        let job = current.as_ref().ok_or("There is no active scan.")?;
        job.stop_file
            .clone()
            .ok_or("This job cannot be cancelled from the scan controls.")?
    };
    stop_control::request(&path)
}

#[tauri::command]
pub(crate) async fn reconcile_interrupted_runs(
    state: State<'_, RunState>,
) -> Result<media_queue::InterruptedRecovery, String> {
    let job_active = state.0.lock().map(|guard| guard.is_some()).unwrap_or(true);
    let queue_file = media_queue_file()?;
    tauri::async_runtime::spawn_blocking(move || {
        if job_active {
            return media_queue::interrupted_recovery_skipped(&queue_file);
        }
        media_queue::reconcile_interrupted(&queue_file)
    })
    .await
    .map_err(|error| format!("Interrupted-session recovery failed: {error}"))?
}
