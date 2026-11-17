//! Tauri commands that start, stop and abort a desktop run.

use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunRequest {
    pub(crate) root: String,
    pub(crate) recursive: bool,
    pub(crate) minimum_size_mib: u32,
    pub(crate) ffmpeg_path: Option<String>,
    #[serde(default)]
    pub(crate) tool_paths: Option<ToolPaths>,
    pub(crate) mode: String,
    pub(crate) youtube_resolution: Option<String>,
    pub(crate) youtube_quality_mode: Option<String>,
    pub(crate) youtube_source_multiplier: Option<f64>,
    pub(crate) youtube_mode: Option<String>,
    pub(crate) youtube_sdr_codec: Option<String>,
    pub(crate) youtube_preset: Option<String>,
    pub(crate) youtube_shrink: Option<String>,
    pub(crate) youtube_quality_check: Option<String>,
    pub(crate) youtube_output_dir: Option<String>,
    pub(crate) rebuild_output_dir: Option<String>,
    pub(crate) settings: Value,
    pub(crate) stop_on_error: bool,
    pub(crate) full_rebuild_confirmed: bool,
    #[serde(default)]
    pub(crate) filename_include_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) filename_exclude_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) selected_files: Vec<String>,
    #[serde(default)]
    pub(crate) file_tune_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) file_stream_overrides: BTreeMap<String, Vec<u16>>,
    #[serde(default)]
    pub(crate) file_settings_overrides: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) file_poster_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) file_metadata_overrides: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) file_output_dirs: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) allow_youtube_hdr10plus_loss_paths: Vec<String>,
    #[serde(default)]
    pub(crate) integrity_level: Option<String>,
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn start_run(
    app: AppHandle,
    state: State<'_, RunState>,
    request: RunRequest,
) -> Result<String, String> {
    start_run_with(Events::from_tauri(app), state.inner(), request)
}

/// Runtime-free body of `start_run`, shared with the headless server: events go to `app`.
pub(crate) fn start_run_with(
    app: Events,
    state: &RunState,
    request: RunRequest,
) -> Result<String, String> {
    if request.minimum_size_mib > 4096 {
        return Err("Minimum input size must be 4096 MiB or less.".into());
    }
    let root = PathBuf::from(&request.root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media path: {error}"))?;
    if !root.is_dir() && !root.is_file() {
        return Err("The selected media path is not a file or folder.".into());
    }
    if !["check", "metadata", "test", "full", "youtube", "integrity"]
        .contains(&request.mode.as_str())
    {
        return Err("Choose Check only, Integrity check, Metadata-only MKV, Short test, Full rebuild, or YouTube upload prep.".into());
    }
    if request.mode == "full" && !request.full_rebuild_confirmed {
        return Err("Confirm the full rebuild before starting it.".into());
    }
    let has_rebuild_output = request
        .rebuild_output_dir
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || !request.file_output_dirs.is_empty();
    if has_rebuild_output && !["metadata", "test", "full"].contains(&request.mode.as_str()) {
        return Err(
            "Output folders apply only to metadata-only output, short tests, and full rebuilds."
                .into(),
        );
    }
    if request.mode == "integrity" {
        return start_integrity_run(app, state, request, root);
    }
    if request.mode == "check" {
        return start_check_run(app, state, request, root);
    }
    let (rebuild_output_args, rebuild_resource_path) =
        if ["metadata", "test", "full"].contains(&request.mode.as_str()) {
            rebuild_output_arguments(
                &root,
                request.rebuild_output_dir.as_deref(),
                request.file_output_dirs.clone(),
            )?
        } else {
            (Vec::new(), None)
        };
    let selected_disk_path = if root.is_file() {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        root.clone()
    };
    let resource_disk_path = if request.mode == "youtube" {
        request
            .youtube_output_dir
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or(selected_disk_path)
    } else {
        rebuild_resource_path.unwrap_or(selected_disk_path)
    };
    let mut args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        request.minimum_size_mib.to_string(),
        "--no-dependency-repair".into(),
        "--skip-update-check".into(),
    ];
    if request.recursive {
        args.push("--recursive".into());
    }
    append_filename_filter_arguments(
        &mut args,
        &request.filename_include_patterns,
        &request.filename_exclude_patterns,
    )?;
    append_selected_file_arguments(&mut args, &root, &request.selected_files)?;
    args.extend(rebuild_output_args);
    if request.mode == "youtube" {
        let resolution = request.youtube_resolution.as_deref().unwrap_or("1440p");
        if !["source", "1440p", "4k"].contains(&resolution) {
            return Err("YouTube output size must be source, 1440p, or 4K.".into());
        }
        let quality_mode = request
            .youtube_quality_mode
            .as_deref()
            .unwrap_or("balanced");
        if !["balanced", "recommended"].contains(&quality_mode) {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        let source_multiplier = request.youtube_source_multiplier.unwrap_or(2.0);
        if !source_multiplier.is_finite() || !(0.5..=4.0).contains(&source_multiplier) {
            return Err("YouTube source-size multiplier must be between 0.5 and 4.0.".into());
        }
        args.push("--youtube-upload".into());
        args.extend(["--upload-resolution".into(), resolution.into()]);
        args.extend(["--upload-quality-mode".into(), quality_mode.into()]);
        args.extend([
            "--upload-source-multiplier".into(),
            source_multiplier.to_string(),
        ]);
        args.extend(youtube_mode_arguments(
            request.youtube_mode.as_deref(),
            request.youtube_sdr_codec.as_deref(),
            request.youtube_preset.as_deref(),
            request.youtube_shrink.as_deref(),
            request.youtube_quality_check.as_deref(),
        )?);
        if let Some(output_dir) = request
            .youtube_output_dir
            .filter(|value| !value.trim().is_empty())
        {
            let output_dir = PathBuf::from(output_dir);
            if !output_dir.is_absolute() {
                return Err("Choose an absolute YouTube output folder path.".into());
            }
            args.extend([
                "--upload-output-dir".into(),
                output_dir.to_string_lossy().into_owned(),
            ]);
        }
    }
    append_tool_path_arguments(
        &mut args,
        request.tool_paths.unwrap_or_default(),
        request.ffmpeg_path,
    )?;
    if request.mode == "youtube" && !request.file_stream_overrides.is_empty() {
        return Err("Per-file stream selections do not apply to YouTube export mode.".into());
    }
    if request.mode == "metadata" && !request.file_stream_overrides.is_empty() {
        return Err("Metadata-only MKV always preserves every source stream; per-file stream selection is not allowed.".into());
    }
    if request.mode != "youtube" && !request.allow_youtube_hdr10plus_loss_paths.is_empty() {
        return Err("HDR10+ metadata-loss consent applies only to YouTube export mode.".into());
    }
    args.extend(youtube_hdr10plus_loss_arguments(
        &root,
        request.allow_youtube_hdr10plus_loss_paths,
    )?);
    args.extend(stream_override_arguments(
        &root,
        request.file_stream_overrides,
    )?);
    if !request.file_settings_overrides.is_empty()
        && ["metadata", "youtube"].contains(&request.mode.as_str())
    {
        return Err(
            "Current-run encoder settings apply only to check, short-test, and full-rebuild modes."
                .into(),
        );
    }
    args.extend(file_settings_override_arguments(
        &root,
        request.file_settings_overrides,
    )?);
    if !request.file_poster_overrides.is_empty()
        && !["metadata", "test", "full"].contains(&request.mode.as_str())
    {
        return Err(
            "Selected poster images can be attached only to metadata-only, short-test, and full rebuild outputs."
                .into(),
        );
    }
    args.extend(poster_override_arguments(
        &root,
        request.file_poster_overrides,
        &library_artwork::app_artwork_cache_root()?,
    )?);
    if !request.file_metadata_overrides.is_empty()
        && !["metadata", "test", "full"].contains(&request.mode.as_str())
    {
        return Err("Reviewed metadata applies only to metadata-only, short-test, and full rebuild outputs.".into());
    }
    args.extend(metadata_override_arguments(
        &root,
        request.file_metadata_overrides,
    )?);
    if request.file_tune_overrides.len() > 512 {
        return Err("Per-file tune overrides are limited to 512 media files.".into());
    }
    if request.mode == "metadata" && !request.file_tune_overrides.is_empty() {
        return Err("Per-file encoding tunes do not apply to metadata-only MKV output.".into());
    }
    let mut validated_overrides = BTreeMap::new();
    for (path, tune) in request.file_tune_overrides {
        if !["animation", "none"].contains(&tune.as_str()) {
            return Err("Per-file tune overrides must be 'animation' or 'none'.".into());
        }
        let selected = PathBuf::from(path)
            .canonicalize()
            .map_err(|error| format!("Per-file tune source is unavailable: {error}"))?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(&root))
            || (root.is_file() && selected != root)
        {
            return Err(
                "Per-file tune overrides must refer to media inside the selected input.".into(),
            );
        }
        validated_overrides.insert(selected.to_string_lossy().into_owned(), tune);
    }
    if !validated_overrides.is_empty() {
        let serialized = serde_json::to_string(&validated_overrides)
            .map_err(|error| format!("Could not encode per-file tune choices: {error}"))?;
        if serialized.len() > 65536 {
            return Err("Per-file tune overrides exceed the 64 KiB limit.".into());
        }
        args.extend(["--file-tune-overrides-json".into(), serialized]);
    }
    args.extend(settings_arguments(&request.settings)?);
    if request.stop_on_error {
        args.push("--stop-on-error".into());
    }
    match request.mode.as_str() {
        "check" => args.push("--info-only".into()),
        "metadata" => args.push("--metadata-only".into()),
        "test" => args.push("--test".into()),
        _ => {}
    }

    let id = new_job_id();
    let stop_token = stop_control::StopToken::new()?;
    let stop_file = stop_token.path().to_path_buf();
    args.extend([
        "--stop-request-file".into(),
        stop_file.to_string_lossy().into_owned(),
    ]);
    // Parse before claiming the job so unsupported arguments fail immediately.
    let native_metadata = if request.mode == "metadata" {
        Some(metadata_run::MetadataRunOptions::from_engine_args(&args)?)
    } else {
        None
    };
    let native_youtube = if request.mode == "youtube" {
        Some(youtube_run::YoutubeRunOptions::from_engine_args(&args)?)
    } else {
        None
    };
    let native_rebuild = if matches!(request.mode.as_str(), "test" | "full") {
        Some(rebuild_run::RebuildRunOptions::from_engine_args(&args)?)
    } else {
        None
    };
    claim_job(
        state,
        new_active_job_of(
            id.clone(),
            Some(stop_file.clone()),
            kind_for_run_mode(&request.mode),
        ),
    )?;

    let app_handle = app.clone();
    let run_state = state.clone();
    let worker_id = id.clone();
    thread::spawn(move || {
        let _stop_token = stop_token;
        if let Some(options) = native_youtube {
            run_native_youtube_job(
                &app_handle,
                &run_state,
                &worker_id,
                options,
                resource_disk_path,
                &stop_file,
            );
            return;
        }
        if let Some(options) = native_metadata {
            run_native_metadata_job(
                &app_handle,
                &run_state,
                &worker_id,
                options,
                resource_disk_path,
                &stop_file,
            );
            return;
        }
        if let Some(options) = native_rebuild {
            run_native_rebuild_job(
                &app_handle,
                &run_state,
                &worker_id,
                options,
                resource_disk_path,
                &stop_file,
            );
            return;
        }
        let _ = app_handle.emit(
            "run-finished",
            RunFinished {
                run_id: worker_id.clone(),
                exit_code: Some(1),
                stop_requested: false,
                log_path: None,
                report_path: None,
                error: Some("Unsupported run mode for the native engine.".into()),
            },
        );
        release_job(&run_state, &worker_id);
    });
    Ok(id)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn request_stop(state: State<'_, RunState>, run_id: String) -> Result<(), String> {
    request_stop_with(&state, run_id)
}

/// Runtime-free body of `request_stop`, shared with the headless server.
pub(crate) fn request_stop_with(state: &RunState, run_id: String) -> Result<(), String> {
    let path = {
        let current = state
            .0
            .lock()
            .map_err(|_| "Could not lock the desktop job state.")?;
        let job = current.as_ref().ok_or("There is no active run.")?;
        if job.id != run_id {
            return Err("That run is no longer active.".into());
        }
        job.stop_file
            .clone()
            .ok_or("This system repair cannot be stopped from the run controls.")?
    };
    stop_control::request(&path)
}

#[cfg(feature = "desktop")]
#[tauri::command]
pub(crate) fn request_abort(state: State<'_, RunState>, run_id: String) -> Result<(), String> {
    request_abort_with(&state, run_id)
}

/// Runtime-free body of `request_abort`, shared with the headless server.
pub(crate) fn request_abort_with(state: &RunState, run_id: String) -> Result<(), String> {
    let (stop_file, child_pid) = {
        let current = state
            .0
            .lock()
            .map_err(|_| "Could not lock the desktop job state.")?;
        let job = current.as_ref().ok_or("There is no active run.")?;
        if job.id != run_id {
            return Err("That run is no longer active.".into());
        }
        let child_pid = *job
            .child_pid
            .lock()
            .map_err(|_| "Could not lock the engine process id.")?;
        (job.stop_file.clone(), child_pid)
    };
    if let Some(path) = stop_file.as_ref() {
        stop_control::request(path)?;
    }
    if let Some(process_id) = child_pid {
        crate::process_output::kill_owned_group(process_id)?;
    } else if stop_file.is_none() {
        return Err("This job cannot be stopped from the run controls.".into());
    }
    Ok(())
}
