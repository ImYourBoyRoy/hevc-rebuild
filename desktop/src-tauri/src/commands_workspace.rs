//! Tauri commands for environment inspection, effective settings, work directory and folder scans.

use super::*;

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn inspect_environment(tool_paths: Option<ToolPaths>) -> Result<Value, String> {
    crate::runtime::spawn_blocking(move || {
        environment::inspect_environment(tool_paths.unwrap_or_default(), None)
    })
    .await
    .map_err(|error| format!("Environment scan task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn load_settings(root: String) -> Result<settings::EffectiveSettings, String> {
    crate::runtime::spawn_blocking(move || settings::load_effective_settings(&root))
        .await
        .map_err(|error| format!("Settings load task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn resolve_work_directory_command(
    root: String,
    work_root: Option<String>,
) -> Result<WorkDirectoryPreview, String> {
    resolve_work_directory(&root, work_root.as_deref())
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) fn default_media_root() -> Result<String, String> {
    media_root_for(user_home_directory())
}

/// The folder the app starts in: the user's video folder (`Videos`, or `Movies` on macOS) when it exists,
/// otherwise the home folder. The home folder comes from `user_home_directory`, which knows `USERPROFILE`
/// on Windows, where `HOME` is normally not set.
pub(crate) fn media_root_for(home: Option<PathBuf>) -> Result<String, String> {
    let home = home.ok_or("Could not locate the user's home folder.")?;
    let chosen = ["Videos", "Movies"]
        .iter()
        .map(|name| home.join(name))
        .find(|candidate| candidate.is_dir())
        .unwrap_or(home);
    Ok(chosen.to_string_lossy().into_owned())
}

#[cfg(test)]
mod media_root_tests {
    use super::*;

    #[test]
    fn the_start_folder_is_the_video_folder_when_there_is_one() {
        let home = crate::test_dir("media-root-videos");
        std::fs::create_dir(home.join("Videos")).unwrap();
        assert_eq!(
            media_root_for(Some(home.clone())).unwrap(),
            home.join("Videos").to_string_lossy()
        );
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn macos_style_movies_folders_are_used_and_videos_wins_when_both_exist() {
        let home = crate::test_dir("media-root-movies");
        std::fs::create_dir(home.join("Movies")).unwrap();
        assert_eq!(
            media_root_for(Some(home.clone())).unwrap(),
            home.join("Movies").to_string_lossy()
        );
        std::fs::create_dir(home.join("Videos")).unwrap();
        assert_eq!(
            media_root_for(Some(home.clone())).unwrap(),
            home.join("Videos").to_string_lossy()
        );
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn without_a_video_folder_it_is_the_home_folder_and_without_a_home_it_is_an_error() {
        let home = crate::test_dir("media-root-bare");
        assert_eq!(
            media_root_for(Some(home.clone())).unwrap(),
            home.to_string_lossy()
        );
        assert_eq!(
            media_root_for(None).unwrap_err(),
            "Could not locate the user's home folder."
        );
        std::fs::remove_dir_all(home).ok();
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_the_home_folder_comes_from_userprofile_not_home() {
        // The regression: `HOME` is normally unset on Windows, so the command failed there.
        assert!(
            user_home_directory().is_some(),
            "USERPROFILE must be enough"
        );
        assert!(default_media_root().is_ok());
    }
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

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn scan_folder(
    app: AppHandle,
    state: State<'_, RunState>,
    options: ScanFolderOptions,
) -> Result<Value, String> {
    scan_folder_with(Events::from_tauri(app), state.inner(), options).await
}

/// Runtime-free body of `scan_folder`, shared with the headless server.
pub(crate) async fn scan_folder_with(
    app: Events,
    state: &RunState,
    options: ScanFolderOptions,
) -> Result<Value, String> {
    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    claim_job(
        state,
        new_active_job_of(id.clone(), Some(stop_file.clone()), "folder_analysis"),
    )?;
    let run_state = state.clone();
    let result = crate::runtime::spawn_blocking(move || {
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

/// The job holding the single slot, if any. A window that did not start it (a job started over MCP in
/// server mode, or from a second tab) uses this to follow it.
#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn active_job(state: State<'_, RunState>) -> Option<ActiveJobInfo> {
    active_job_info(&state)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn cancel_scan(state: State<'_, RunState>) -> Result<(), String> {
    cancel_scan_with(&state)
}

/// Runtime-free body of `cancel_scan`, shared with the headless server.
pub(crate) fn cancel_scan_with(state: &RunState) -> Result<(), String> {
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

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) async fn reconcile_interrupted_runs(
    state: State<'_, RunState>,
) -> Result<media_queue::InterruptedRecovery, String> {
    reconcile_interrupted_runs_with(&state).await
}

/// Runtime-free body of `reconcile_interrupted_runs`, shared with the headless server.
pub(crate) async fn reconcile_interrupted_runs_with(
    state: &RunState,
) -> Result<media_queue::InterruptedRecovery, String> {
    let job_active = state.0.lock().map(|guard| guard.is_some()).unwrap_or(true);
    let queue_file = media_queue_file()?;
    crate::runtime::spawn_blocking(move || {
        if job_active {
            return media_queue::interrupted_recovery_skipped(&queue_file);
        }
        media_queue::reconcile_interrupted(&queue_file)
    })
    .await
    .map_err(|error| format!("Interrupted-session recovery failed: {error}"))?
}
