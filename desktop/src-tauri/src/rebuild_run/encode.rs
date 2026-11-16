//! The supervised video encode with adaptive memory retries.

use super::*;

/// Stage 8: plan threads/memory, then run the encode with adaptive retries.
pub(super) fn encode_video(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &FilePlan,
    targets: &OutputTargets,
    hdr10plus: &Hdr10PlusPlan,
    info_plan: Result<ResourcePlan, String>,
) -> Result<(), String> {
    let source = frame.source;
    let input_bytes = frame.input_bytes;
    let tools = context.tools;
    let options = context.options;
    let FilePlan {
        effective,
        profile,
        refs,
        selected_streams,
        encode_video_indices,
        real_videos,
        primary,
        video_profiles,
        content_tune,
        preserve_dolbyvision_indices,
        strip_dolbyvision_indices,
        planning_profile,
        ..
    } = plan;
    let preserve_dolbyvision = plan.preserve_dolbyvision();
    let strip_dolbyvision = plan.strip_dolbyvision();
    let temp_output = &targets.temp_output;
    let encode_hdr10plus_jsons = hdr10plus.encode_jsons();
    let duration = targets.duration;
    let encode_crf = if options.test_mode {
        effective.test_crf
    } else {
        effective.crf
    };
    let expected_duration = expected_duration(options, duration);
    emit(
        context,
        frame,
        "Planning encoder threads and memory",
        0.07,
        None,
        None,
        None,
        None,
    );
    let mut plan = info_plan?;
    maybe_calibrate(
        context,
        frame,
        state,
        source,
        &mut plan,
        profile,
        primary,
        planning_profile,
        effective,
        encode_crf,
        content_tune.as_deref(),
        encode_hdr10plus_jsons.get(&primary.index),
        preserve_dolbyvision,
        strip_dolbyvision,
        encode_video_indices.len(),
        input_bytes.unwrap_or(0),
    )?;
    record_memory_forecast(&mut state.row, &plan);

    state.phase = Phase::VideoEncode;
    emit(
        context,
        frame,
        "Starting the video encode",
        0.08,
        None,
        None,
        None,
        None,
    );
    let plan_profile = engine_profile_to_plan_profile(source, profile, primary, refs);
    let mut plan_video_profiles = Map::new();
    for stream in real_videos.iter() {
        let track = &video_profiles[&stream.index];
        plan_video_profiles.insert(
            stream.index.to_string(),
            track_profile_to_plan(source, track, stream, refs),
        );
    }
    let mut attempt = 1u32;
    loop {
        let encode_request = json!({
            "tools": {
                "ffmpeg": tools.ffmpeg,
                "x265_pixel_formats": tools.x265_pixel_formats,
                "x265_dolbyvision": tools.x265_dolbyvision,
            },
            "profile": plan_profile,
            "video_profiles": plan_video_profiles,
            "selected_streams": selected_streams.iter().map(StreamRef::to_plan_value).collect::<Vec<_>>(),
            "encode_video_indices": encode_video_indices.iter().copied().collect::<Vec<_>>(),
            "hdr10plus_jsons": encode_hdr10plus_jsons.iter().map(|(index, path)| (index.to_string(), json!(path))).collect::<Map<_, _>>(),
            "preserve_dolbyvision_indices": preserve_dolbyvision_indices.iter().copied().collect::<Vec<_>>(),
            "strip_dolbyvision_indices": strip_dolbyvision_indices.iter().copied().collect::<Vec<_>>(),
            "resource_plan": {
                "worker_threads": plan.worker_threads,
                "frame_threads": plan.frame_threads,
            },
            "options": {
                "output": temp_output,
                "crf": encode_crf,
                "preset": effective.preset,
                "tune": content_tune.clone().unwrap_or_default(),
                "preserve_dolbyvision": preserve_dolbyvision,
                "strip_dolbyvision": strip_dolbyvision,
                "test_seconds": options.test_mode.then_some(options.test_seconds),
                "preserve_source_level": options.preserve_source_level,
                "disable_hdr10_opt": options.disable_hdr10_opt,
                "secondary_video_policy": options.secondary_video,
                "data_policy": options.data_streams,
                "force_deinterlace": options.force_deinterlace,
                "disable_deinterlace": options.disable_deinterlace,
            }
        });
        let planned = crate::encode_plan::build_plan(&encode_request)?;
        let args = planned
            .get("args")
            .and_then(Value::as_array)
            .ok_or("The encode planner returned no FFmpeg arguments.")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        (context.line)(format!(
            "[{}/{}] Encoding {} (attempt {attempt}; workers={} frame_threads={})",
            frame.number,
            frame.total,
            source.file_name().unwrap_or_default().to_string_lossy(),
            plan.worker_threads,
            plan.frame_threads
        ));
        let disk_guard_bytes = (options.min_free_disk_gib.max(0.0) * GIB as f64) as u64;
        let spec = EncodeSpec {
            args: &args,
            expected_duration,
            output_path: temp_output,
            memory_guard_gib: plan.memory_guard_gib,
            disk_guard_bytes,
            projection_margin_percent: options.disk_projection_margin_percent,
            stop_file: options.stop_file.as_deref(),
        };
        let child_cb = &mut *context.child;
        let progress_cb = &mut *context.progress;
        let number = frame.number;
        let total = frame.total;
        let started = frame.started;
        let input_bytes = frame.input_bytes;
        let file_name = frame
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_path = frame.source.to_string_lossy().into_owned();
        match run_encode(
            &spec,
            |progress: &EncodeProgress| {
                let fraction = 0.08 + 0.80 * (progress.file_percent.unwrap_or(0.0) / 100.0);
                let clamped = fraction.clamp(0.0, 1.0);
                progress_cb(RebuildProgress {
                    current: number,
                    total,
                    file_name: file_name.clone(),
                    file_path: file_path.clone(),
                    phase: "Encoding video".into(),
                    file_percent: progress.file_percent,
                    queue_percent: ((number.saturating_sub(1)) as f64 + clamped)
                        / total.max(1) as f64
                        * 100.0,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                    speed: progress.speed,
                    eta_seconds: progress.eta_seconds,
                    input_bytes,
                    output_bytes: Some(progress.output_bytes),
                    estimated_output_bytes: None,
                });
            },
            child_cb,
        ) {
            Ok(resources) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                break;
            }
            Err(EncodeFailure::Interrupted { resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                state.interrupted = true;
                return Err("Encode interrupted.".into());
            }
            Err(EncodeFailure::Failed { message, resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                return Err(message);
            }
            Err(EncodeFailure::MemoryPressure { reason, resources }) => {
                for (key, value) in resources {
                    state.row.insert(key, value);
                }
                let Some(next) = plan.reduced() else {
                    return Err(format!(
                        "Memory pressure persisted after {attempt} safe encode attempt(s) ({reason}). Close memory-heavy programs or lower max worker/frame threads. The source file was not changed."
                    ));
                };
                if attempt >= MAX_ENCODE_ATTEMPTS {
                    return Err(format!(
                        "Memory pressure persisted after {attempt} safe encode attempt(s) ({reason}). The source file was not changed."
                    ));
                }
                if temp_output.exists() {
                    let _ = fs::remove_file(temp_output);
                }
                (context.line)(format!(
                    "Memory pressure detected; retrying with {} worker / {} frame thread(s). Quality settings are unchanged.",
                    next.worker_threads, next.frame_threads
                ));
                emit(
                    context,
                    frame,
                    "Retrying the encode with fewer threads",
                    0.08,
                    None,
                    None,
                    None,
                    None,
                );
                plan = next;
                attempt += 1;
            }
        }
    }
    Ok(())
}
