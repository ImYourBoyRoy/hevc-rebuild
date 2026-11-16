//! Exports one file as a YouTube upscale copy: plan, encode, validate and install.

use super::*;

pub(super) fn execute(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    destination: &Path,
    row: &mut Map<String, Value>,
    temporary: &mut Option<PathBuf>,
    live: &mut Live,
) -> Result<(), Halt> {
    let source = frame.source;
    let options = context.options;
    let tools = context.tools;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let planned = canonicalize_allow_missing(destination)?;
    if planned == source {
        return Err(Halt::Fail(
            "The output path matches the source file; choose a separate export folder.".into(),
        ));
    }
    if destination.exists() {
        return Err(Halt::Skip(
            "An output with this name already exists; it was left unchanged.".into(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create the output folder: {error}"))?;
    }
    emit(context, frame, "Analyzing source", 0.0, Some(0.0), live);
    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: false,
    })?;
    let profile = response
        .get("profile")
        .cloned()
        .ok_or_else(|| String::from("The profiler returned no source profile."))?;
    if let Some(codec) = profile.pointer("/video/codec") {
        row.insert("primary_video".into(), codec.clone());
    }
    if let (Some(width), Some(height)) = (
        profile.pointer("/video/width").and_then(Value::as_i64),
        profile.pointer("/video/height").and_then(Value::as_i64),
    ) {
        row.insert("resolution".into(), json!(format!("{width}x{height}")));
    }
    for key in ["audio_count", "subtitle_count"] {
        if let Some(count) = profile.get(key) {
            row.insert(key.into(), count.clone());
        }
    }
    if profile.get("subtitle_count").and_then(Value::as_i64) > Some(0) {
        row.insert(
            "subtitle_note".into(),
            json!("Subtitle streams are not included in the YouTube MP4; upload supported caption files separately."),
        );
    }
    if options.optimize {
        return execute_optimize(context, frame, destination, row, temporary, live, &profile);
    }
    let accepted = options.hdr10plus_loss.contains(source);
    let can_inject = tools.hdr10plus.is_some();
    let policy = source_policy(
        &profile,
        &options.resolution,
        &tools.x265_pixel_formats,
        accepted,
        can_inject,
    )
    .map_err(Halt::Skip)?;
    let color_mode = policy.color_mode;
    let mut hdr10plus_note = policy.note;
    row.insert("upload_color_mode".into(), json!(color_mode));

    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let estimate = youtube::estimate(
        &profile,
        frame.input_bytes,
        duration,
        &options.resolution,
        &options.quality_mode,
        options.source_multiplier,
    )?;
    let integer = |key: &str| estimate.get(key).and_then(Value::as_i64).unwrap_or(0);
    let (out_width, out_height) = (integer("output_width"), integer("output_height"));
    let (source_width, source_height) = (integer("source_width"), integer("source_height"));
    let bitrate = integer("video_bitrate_kbps");
    let estimated_bytes = integer("estimated_output_bytes").max(0) as u64;
    if (out_width, out_height) != (source_width, source_height) {
        (context.line)(format!(
            "{name}: resizing to {out_width}x{out_height}; aspect ratio is preserved"
        ));
    }
    row.insert("upload_video_bitrate_kbps".into(), json!(bitrate));
    row.insert(
        "youtube_recommended_video_bitrate_kbps".into(),
        json!(integer("recommended_video_bitrate_kbps")),
    );
    row.insert("estimated_output_bytes".into(), json!(estimated_bytes));
    live.estimated_output_bytes = Some(estimated_bytes);
    (context.line)(format!(
        "{name}: estimated upload copy {} at {bitrate} kb/s video; YouTube guidance is {} kb/s.",
        human_bytes(estimated_bytes),
        integer("recommended_video_bitrate_kbps")
    ));
    let reserve = options.disk_reserve_bytes();
    let output_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(free) = disk_free_bytes(output_parent) {
        if free < estimated_bytes.saturating_add(reserve) {
            return Err(Halt::Fail(format!(
                "Not enough free space for the conservative upload estimate ({} output plus {:.1} GiB reserve).",
                human_bytes(estimated_bytes),
                reserve as f64 / GIB
            )));
        }
    }

    let inject_hdr10plus = can_inject
        && profile
            .pointer("/hdr/hdr10plus_hint")
            .and_then(Value::as_bool)
            == Some(true);
    let work_dir = if inject_hdr10plus {
        Some(
            crate::media_work_directory(source, &context.options.work_root)?
                .join(format!("youtube-hdr10plus-{}", std::process::id())),
        )
    } else {
        None
    };
    let hdr10plus_json = if inject_hdr10plus {
        let dir = work_dir.as_ref().expect("inject work dir");
        (context.line)(format!("{name}: extracting HDR10+ for post-encode inject"));
        Some(extract_youtube_hdr10plus_json(
            tools, source, &profile, dir,
        )?)
    } else {
        None
    };

    let plan = plan_resources(
        &options.settings,
        &profile,
        (out_width, out_height),
        &options.preset,
    )?;
    row.insert("planned_workers".into(), json!(plan.workers));
    row.insert("planned_frame_threads".into(), json!(plan.frames));
    let temp_path = temporary_name(destination);
    *temporary = Some(temp_path.clone());
    let job = EncodeJob {
        duration,
        estimated_bytes,
        reserve,
        phase: "Encoding",
        temp_path: &temp_path,
    };
    encode_with_retries(context, frame, live, row, &job, plan, &|plan| {
        let built = youtube::build_args(&json!({
            "profile": profile,
            "tools": {
                "ffmpeg": tools.ffmpeg,
                "x265_pixel_formats": tools.x265_pixel_formats,
            },
            "source_path": source.to_string_lossy(),
            "output": temp_path.to_string_lossy(),
            "resolution": options.resolution,
            "video_bitrate_kbps": bitrate,
            "worker_threads": plan.workers,
            "frame_threads": plan.frames,
            "sdr_codec": options.sdr_codec,
            "preset": options.preset,
        }))?;
        string_args(&built)
    })?;

    if let (Some(json), Some(dir)) = (&hdr10plus_json, &work_dir) {
        emit(
            context,
            frame,
            "Injecting HDR10+ metadata",
            0.93,
            Some(100.0),
            live,
        );
        (context.line)(format!("{name}: injecting HDR10+ into the YouTube MP4"));
        inject_youtube_hdr10plus(tools, &temp_path, json, dir)?;
        hdr10plus_note = Some(
            "HDR10+ dynamic metadata was preserved after encode with hdr10plus_tool inject.".into(),
        );
        let _ = fs::remove_file(json);
        let _ = fs::remove_dir(dir);
    }
    if let Some(note_text) = hdr10plus_note {
        row.insert("hdr10plus_note".into(), json!(note_text));
    }

    emit(context, frame, "Validating output", 0.95, Some(100.0), live);
    let codec_label = if matches!(color_mode, "hdr10" | "hlg") {
        "HEVC Main 10/AAC"
    } else if options.sdr_codec == "hevc" {
        "HEVC/AAC"
    } else {
        "H.264/AAC"
    };
    (context.line)(format!(
        "Validating {codec_label} streams, color tags, dimensions, and duration for {name}"
    ));
    let validated = youtube::validate_request(&json!({
        "profile": profile,
        "tools": {
            "ffprobe": tools.ffprobe,
            "ffmpeg": tools.ffmpeg,
            "hdr10plus_tool": tools.hdr10plus,
        },
        "output": temp_path.to_string_lossy(),
        "resolution": options.resolution,
        "sdr_codec": options.sdr_codec,
        "hdr10plus_expected": inject_hdr10plus,
    }))?;
    let (output_width, output_height) = (
        validated.get("width").and_then(Value::as_i64).unwrap_or(0),
        validated.get("height").and_then(Value::as_i64).unwrap_or(0),
    );
    emit(context, frame, "Installing output", 0.98, Some(100.0), live);
    let installed = transactions::install_export_output(&temp_path, destination)?;
    *temporary = None;
    let output_bytes = fs::metadata(destination)
        .map(|meta| meta.len())
        .unwrap_or(0);
    live.output_bytes = Some(output_bytes);
    let mode_label = match color_mode {
        "hdr10" => "HDR10/PQ",
        "hlg" => "HLG",
        _ => "SDR",
    };
    row.insert("status".into(), json!("OK"));
    row.insert("output_bytes".into(), json!(output_bytes));
    row.insert(
        "output_resolution".into(),
        json!(format!("{output_width}x{output_height}")),
    );
    row.insert(
        "message".into(),
        json!(installed.annotate(format!(
            "Validated YouTube {mode_label} MP4 copy ({output_width}x{output_height}, {bitrate} kb/s video target, {} size profile, {}x source multiplier); source unchanged.",
            options.quality_mode, options.source_multiplier
        ))),
    );
    Ok(())
}
