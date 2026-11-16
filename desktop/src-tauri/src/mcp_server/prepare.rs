//! Turns a tool request into validated engine arguments (`prepare_media_run`) and
//! checks a queue of such runs together before anything starts.

use super::*;

pub(super) struct PreparedMediaRun {
    pub(super) root: PathBuf,
    pub(super) mode: String,
    pub(super) selected_files: Vec<String>,
    pub(super) args: Vec<String>,
    pub(super) output_dir: Option<String>,
    pub(super) file_output_dirs: BTreeMap<String, String>,
    pub(super) tool_paths: ToolPaths,
    pub(super) recursive: bool,
    pub(super) minimum_size_mib: u32,
    pub(super) filename_include_patterns: Vec<String>,
    pub(super) filename_exclude_patterns: Vec<String>,
}

pub(super) struct QueueRunPlan {
    pub(super) run: PreparedMediaRun,
    pub(super) queue_sync: Option<QueueRunSync>,
}

pub(super) fn validate_prepared_queue_runs(runs: &[PreparedMediaRun]) -> Result<(), ErrorData> {
    if runs.is_empty() || runs.len() > 500 {
        return Err(ErrorData::invalid_params(
            "A queued run must contain between 1 and 500 source runs.",
            None,
        ));
    }
    let mode = &runs[0].mode;
    let mut roots = std::collections::BTreeSet::new();
    let mut files = std::collections::BTreeSet::new();
    let mut total_files = 0usize;
    for run in runs {
        if &run.mode != mode {
            return Err(ErrorData::invalid_params(
                "All source runs in one queue batch must use the same mode.",
                None,
            ));
        }
        if !roots.insert(run.root.clone()) {
            return Err(ErrorData::invalid_params(
                "A source root can appear only once in a queued run.",
                None,
            ));
        }
        if run.selected_files.is_empty() {
            return Err(ErrorData::invalid_params(
                "Every queued source run requires an explicit non-empty selected_files list.",
                None,
            ));
        }
        total_files = total_files.saturating_add(run.selected_files.len());
        if total_files > 25_000 {
            return Err(ErrorData::invalid_params(
                "A queued run is limited to 25,000 explicitly selected files.",
                None,
            ));
        }
        for raw_path in &run.selected_files {
            let path = PathBuf::from(raw_path).canonicalize().map_err(|error| {
                ErrorData::invalid_params(
                    format!("A selected media file is no longer available: {error}"),
                    None,
                )
            })?;
            if !files.insert(path) {
                return Err(ErrorData::invalid_params(
                    "Selected media paths must be unique across the complete queue batch.",
                    None,
                ));
            }
        }
    }

    if ["metadata", "test", "full"].contains(&mode.as_str()) {
        let default_output = runs[0]
            .output_dir
            .as_deref()
            .filter(|path| !path.trim().is_empty());
        let canonical_default = default_output
            .map(Path::new)
            .map(Path::canonicalize)
            .transpose()
            .map_err(|error| {
                ErrorData::invalid_params(
                    format!("The default output folder could not be resolved: {error}"),
                    None,
                )
            })?;
        let mut per_file_output_dirs = HashMap::new();
        let mut preview_sources = Vec::with_capacity(runs.len());
        for run in runs {
            let output = run
                .output_dir
                .as_deref()
                .filter(|path| !path.trim().is_empty())
                .map(Path::new)
                .map(Path::canonicalize)
                .transpose()
                .map_err(|error| {
                    ErrorData::invalid_params(
                        format!("A source output folder could not be resolved: {error}"),
                        None,
                    )
                })?;
            if output != canonical_default {
                return Err(ErrorData::invalid_params(
                    "Use one shared default output folder for a multi-source batch; per-file output folders may still differ.",
                    None,
                ));
            }
            for (path, folder) in &run.file_output_dirs {
                if per_file_output_dirs
                    .insert(path.clone(), folder.clone())
                    .is_some()
                {
                    return Err(ErrorData::invalid_params(
                        "A selected file has conflicting per-file output folders across source runs.",
                        None,
                    ));
                }
            }
            preview_sources.push(crate::output_targets::OutputPreviewSource {
                root: run.root.to_string_lossy().into_owned(),
                files: run.selected_files.clone(),
            });
        }
        let preview = crate::output_targets::preview(crate::output_targets::OutputPreviewRequest {
            mode: mode.clone(),
            sources: preview_sources,
            output_dir: default_output.map(str::to_owned),
            file_output_dirs: per_file_output_dirs,
        })
        .map_err(|error| ErrorData::invalid_params(error, None))?;
        if !preview.eligible {
            return Err(ErrorData::invalid_params(
                json!({
                    "message": "Output preflight found occupied destinations or duplicate target names across the selected sources.",
                    "collision_count": preview.collision_count,
                    "collisions": preview.collisions,
                })
                .to_string(),
                None,
            ));
        }
    }
    Ok(())
}

pub(super) fn prepare_media_run(
    params: StartRunParams,
    stop_file: &std::path::Path,
) -> Result<PreparedMediaRun, ErrorData> {
    let preflight_output_dir = params.output_dir.clone();
    let preflight_file_output_dirs = params.file_output_dirs.clone().unwrap_or_default();
    let root = canonical_media_input(&params.root)
        .map_err(|error| ErrorData::invalid_params(error, None))?;
    let mode = params.mode.as_deref().unwrap_or("check");
    if !["check", "metadata", "test", "full", "youtube"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Mode must be check, metadata, test, full, or youtube.",
            None,
        ));
    }
    if mode == "full" && params.confirm_full_rebuild != Some(true) {
        return Err(ErrorData::invalid_params(
            "Set confirm_full_rebuild=true to authorize this full rebuild.",
            None,
        ));
    }
    if mode == "test" && params.confirm_test_output != Some(true) {
        return Err(ErrorData::invalid_params(
            "Set confirm_test_output=true to authorize writing short-test outputs.",
            None,
        ));
    }
    if mode == "youtube" && params.confirm_youtube_upload != Some(true) {
        return Err(ErrorData::invalid_params(
            "Set confirm_youtube_upload=true to authorize creating separate YouTube upload copies.",
            None,
        ));
    }
    let youtube_resolution = params.youtube_resolution.as_deref().unwrap_or("1440p");
    if !["source", "1440p", "4k"].contains(&youtube_resolution) {
        return Err(ErrorData::invalid_params(
            "youtube_resolution must be source, 1440p, or 4k.",
            None,
        ));
    }
    let youtube_quality_mode = params.youtube_quality_mode.as_deref().unwrap_or("balanced");
    if !["balanced", "recommended"].contains(&youtube_quality_mode) {
        return Err(ErrorData::invalid_params(
            "youtube_quality_mode must be balanced or recommended.",
            None,
        ));
    }
    let youtube_source_multiplier = params.youtube_source_multiplier.unwrap_or(2.0);
    if !youtube_source_multiplier.is_finite() || !(0.5..=4.0).contains(&youtube_source_multiplier) {
        return Err(ErrorData::invalid_params(
            "youtube_source_multiplier must be between 0.5 and 4.0.",
            None,
        ));
    }
    let minimum = params.minimum_size_mib.unwrap_or(5);
    if minimum > 4096 {
        return Err(ErrorData::invalid_params(
            "Minimum input size must be 4096 MiB or less.",
            None,
        ));
    }
    let mut settings = if let Some(serialized) = params.settings_json {
        serde_json::from_str::<Value>(&serialized).map_err(|error| {
            ErrorData::invalid_params(format!("settings_json is invalid: {error}"), None)
        })?
    } else {
        let effective = crate::settings::load_effective_settings(&root.to_string_lossy())
            .map_err(|error| ErrorData::internal_error(error, None))?;
        serde_json::to_value(effective)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .get("settings")
            .cloned()
            .ok_or_else(|| ErrorData::internal_error("Could not load effective settings.", None))?
    };
    if let Some(work_root) = params.work_root {
        if !settings.get("paths").is_some_and(Value::is_object) {
            settings["paths"] = json!({});
        }
        settings["paths"]["work_root"] = json!(work_root);
    }
    if mode == "full" {
        ensure_disk_floor(&mut settings, &root);
    }
    let recursive = params.recursive.unwrap_or(true);
    let filename_include_patterns = params.filename_include_patterns.clone().unwrap_or_default();
    // Backups live in the hidden originals folder, which discovery always skips; `old/` is the legacy backup folder. Left out of the walk by default so an
    // MCP client that omits the filter cannot send a finished library's backups
    // back through the encoder; passing an explicit list (even an empty one) opts out.
    let filename_exclude_patterns = params
        .filename_exclude_patterns
        .clone()
        .unwrap_or_else(|| vec![crate::backup_folder::LEGACY_BACKUP_FOLDER.to_owned()]);
    let mut native_tool_paths = params.tool_paths.clone().unwrap_or_default();
    if native_tool_paths.ffmpeg.is_none() {
        native_tool_paths.ffmpeg = params.ffmpeg_path.clone();
    }
    let mut args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        minimum.to_string(),
        "--no-dependency-repair".into(),
        "--skip-update-check".into(),
    ];
    if recursive {
        args.push("--recursive".into());
    }
    crate::append_filename_filter_arguments(
        &mut args,
        &filename_include_patterns,
        &filename_exclude_patterns,
    )
    .map_err(|error| ErrorData::invalid_params(error, None))?;
    let selected_files = params.selected_files.unwrap_or_default();
    crate::append_selected_file_arguments(&mut args, &root, &selected_files)
        .map_err(|error| ErrorData::invalid_params(error, None))?;
    let file_output_dirs = params.file_output_dirs.unwrap_or_default();
    let has_rebuild_output = params
        .output_dir
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || !file_output_dirs.is_empty();
    if has_rebuild_output && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "output_dir and file_output_dirs apply only to metadata-only output, short tests, and full rebuilds.",
            None,
        ));
    }
    if ["metadata", "test", "full"].contains(&mode) {
        let (output_arguments, _) =
            crate::rebuild_output_arguments(&root, params.output_dir.as_deref(), file_output_dirs)
                .map_err(|error| ErrorData::invalid_params(error, None))?;
        args.extend(output_arguments);
    }
    args.extend(
        tool_path_arguments(params.tool_paths, params.ffmpeg_path)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let stream_overrides = params.file_stream_overrides.unwrap_or_default();
    if (mode == "youtube" || mode == "metadata") && !stream_overrides.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file stream selections do not apply to YouTube or metadata-only output.",
            None,
        ));
    }
    args.extend(
        stream_override_arguments(&root, stream_overrides)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let file_settings_overrides = params.file_settings_overrides.unwrap_or_default();
    if !file_settings_overrides.is_empty() && ["metadata", "youtube"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Current-run encoder settings apply only to check, short-test, and full-rebuild modes.",
            None,
        ));
    }
    args.extend(
        file_settings_override_arguments(&root, file_settings_overrides)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let poster_overrides = params.file_poster_overrides.unwrap_or_default();
    if !poster_overrides.is_empty() && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Selected poster images can be attached only to metadata-only, short-test, and full rebuild outputs.",
            None,
        ));
    }
    args.extend(
        crate::poster_override_arguments(
            &root,
            poster_overrides,
            &crate::library_artwork::app_artwork_cache_root()
                .map_err(|error| ErrorData::internal_error(error, None))?,
        )
        .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let metadata_overrides = params.file_metadata_overrides.unwrap_or_default();
    if !metadata_overrides.is_empty() && !["metadata", "test", "full"].contains(&mode) {
        return Err(ErrorData::invalid_params(
            "Reviewed metadata applies only to metadata-only, short-test, and full rebuild outputs.",
            None,
        ));
    }
    args.extend(
        crate::metadata_override_arguments(&root, metadata_overrides)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    let hdr10plus_loss_paths = params
        .allow_youtube_hdr10plus_loss_paths
        .unwrap_or_default();
    if mode != "youtube" && !hdr10plus_loss_paths.is_empty() {
        return Err(ErrorData::invalid_params(
            "HDR10+ metadata-loss consent applies only to YouTube export mode.",
            None,
        ));
    }
    args.extend(
        youtube_hdr10plus_loss_arguments(&root, hdr10plus_loss_paths)
            .map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    if mode == "youtube" {
        args.push("--youtube-upload".into());
        args.extend(["--upload-resolution".into(), youtube_resolution.into()]);
        args.extend(["--upload-quality-mode".into(), youtube_quality_mode.into()]);
        args.extend([
            "--upload-source-multiplier".into(),
            youtube_source_multiplier.to_string(),
        ]);
        args.extend(
            crate::youtube_mode_arguments(
                params.youtube_mode.as_deref(),
                params.youtube_sdr_codec.as_deref(),
                params.youtube_preset.as_deref(),
                params.youtube_shrink.as_deref(),
                params.youtube_quality_check.as_deref(),
            )
            .map_err(|error| ErrorData::invalid_params(error, None))?,
        );
        if let Some(output_dir) = params
            .upload_output_dir
            .filter(|value| !value.trim().is_empty())
        {
            let output_dir = PathBuf::from(output_dir);
            if !output_dir.is_absolute() {
                return Err(ErrorData::invalid_params(
                    "upload_output_dir must be an absolute folder path.",
                    None,
                ));
            }
            args.extend([
                "--upload-output-dir".into(),
                output_dir.to_string_lossy().into_owned(),
            ]);
        }
    }
    let per_file = params.file_tune_overrides.unwrap_or_default();
    if mode == "metadata" && !per_file.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file encoding tunes do not apply to metadata-only MKV output.",
            None,
        ));
    }
    if mode == "youtube" && !per_file.is_empty() {
        return Err(ErrorData::invalid_params(
            "Per-file x265 content tunes do not apply to YouTube export mode.",
            None,
        ));
    }
    if per_file.len() > 512 {
        return Err(ErrorData::invalid_params(
            "Per-file tune overrides are limited to 512 media files.",
            None,
        ));
    }
    let mut validated_per_file = BTreeMap::new();
    for (raw_path, tune) in per_file {
        if !["animation", "none"].contains(&tune.as_str()) {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides must be 'animation' or 'none'.",
                None,
            ));
        }
        let selected = PathBuf::from(raw_path).canonicalize().map_err(|error| {
            ErrorData::invalid_params(
                format!("Per-file tune source is unavailable: {error}"),
                None,
            )
        })?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(&root))
            || (root.is_file() && selected != root)
        {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides must refer to media inside the selected input.",
                None,
            ));
        }
        validated_per_file.insert(selected.to_string_lossy().into_owned(), tune);
    }
    if !validated_per_file.is_empty() {
        let serialized = serde_json::to_string(&validated_per_file).map_err(|error| {
            ErrorData::invalid_params(
                format!("Could not encode per-file tune choices: {error}"),
                None,
            )
        })?;
        if serialized.len() > 65536 {
            return Err(ErrorData::invalid_params(
                "Per-file tune overrides exceed the 64 KiB limit.",
                None,
            ));
        }
        args.extend(["--file-tune-overrides-json".into(), serialized]);
    }
    args.extend(
        settings_arguments(&settings).map_err(|error| ErrorData::invalid_params(error, None))?,
    );
    if params.stop_on_error.unwrap_or(false) {
        args.push("--stop-on-error".into());
    }
    match mode {
        "check" => args.push("--info-only".into()),
        "metadata" => args.push("--metadata-only".into()),
        "test" => args.push("--test".into()),
        _ => {}
    }
    args.extend([
        "--stop-request-file".into(),
        stop_file.to_string_lossy().into_owned(),
    ]);
    Ok(PreparedMediaRun {
        root,
        mode: mode.to_owned(),
        selected_files,
        args,
        output_dir: preflight_output_dir,
        file_output_dirs: preflight_file_output_dirs,
        tool_paths: native_tool_paths,
        recursive,
        minimum_size_mib: minimum,
        filename_include_patterns,
        filename_exclude_patterns,
    })
}
