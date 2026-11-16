//! Optional hardware calibration that picks encoder settings before the encode.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn maybe_calibrate(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    source: &Path,
    plan: &mut ResourcePlan,
    profile: &Value,
    primary: &StreamRef,
    planning_profile: &Value,
    effective: &EffectiveSettings,
    encode_crf: i64,
    content_tune: Option<&str>,
    primary_hdr_json: Option<&PathBuf>,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    encode_count: usize,
    source_size: u64,
) -> Result<(), String> {
    let options = context.options;
    let tools = context.tools;
    if options.calibration_mode == "off" || options.resource_threading != "auto" {
        return Ok(());
    }
    let width = planning_profile
        .pointer("/video/width")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let height = planning_profile
        .pointer("/video/height")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let eligible =
        crate::calibration::calibration_eligible(crate::calibration::EligibilityRequest {
            width,
            height,
            source_size_bytes: source_size,
            large_file_gib: options.calibration_large_file_gib,
        })?;
    let demanding = eligible
        .get("demanding")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let due = options.calibration_mode == "always"
        || (matches!(options.calibration_mode.as_str(), "auto" | "ask") && demanding);
    if !due {
        return Ok(());
    }
    if encode_count > 1 {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("A multi-video job is planned as a concurrent aggregate; single-stream calibration would not represent its memory use."),
        );
        (context.line)(
            "Hardware calibration skipped for multiple selected video encoders; the aggregate memory plan and live pressure guards remain active.".into(),
        );
        return Ok(());
    }
    let available = plan.forecast.get("available_gib").and_then(Value::as_f64);
    let total_memory = plan.forecast.get("total_gib").and_then(Value::as_f64);
    if available.is_none() {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("Available-memory telemetry is unavailable."),
        );
        return Ok(());
    }
    if available.unwrap_or(0.0) < plan.memory_guard_gib {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!(format!(
                "Available RAM is below the configured {:.1} GiB reserve.",
                plan.memory_guard_gib
            )),
        );
        return Ok(());
    }
    // GUI / non-interactive ask mode skips interactive approval.
    if options.calibration_mode == "ask" {
        state
            .row
            .insert("calibration_source".into(), json!("skipped"));
        state.row.insert(
            "calibration_note".into(),
            json!("No interactive terminal was available."),
        );
        return Ok(());
    }

    let cal_profile = crate::calibration::CalibrationProfile {
        source_path: source.to_string_lossy().into_owned(),
        duration: profile.get("duration_seconds").and_then(Value::as_f64),
        primary_video_index: u32::try_from(primary.index).unwrap_or(0),
        primary_video_width: primary.width.max(0) as u32,
        primary_video_height: primary.height.max(0) as u32,
        source_bit_depth: planning_profile
            .pointer("/video/source_bit_depth")
            .and_then(Value::as_i64)
            .unwrap_or(8),
        source_chroma: planning_profile
            .pointer("/video/source_chroma")
            .and_then(Value::as_str)
            .unwrap_or("4:2:0")
            .to_string(),
        target_pix_fmt: planning_profile
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
            .unwrap_or("yuv420p10le")
            .to_string(),
        format_level: planning_profile
            .get("format_level")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_primaries: planning_profile
            .get("color_primaries")
            .and_then(Value::as_str)
            .map(str::to_owned),
        transfer: planning_profile
            .get("color_transfer")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_matrix: planning_profile
            .get("color_matrix")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_range_x265: planning_profile
            .get("color_range_x265")
            .and_then(Value::as_str)
            .map(str::to_owned),
        color_range_ffmpeg: planning_profile
            .get("color_range_ffmpeg")
            .and_then(Value::as_str)
            .map(str::to_owned),
        chroma_location: planning_profile
            .get("chroma_location")
            .and_then(Value::as_i64),
        master_display: planning_profile
            .get("mastering_display")
            .and_then(Value::as_str)
            .map(str::to_owned),
        max_cll: planning_profile.get("max_cll").and_then(Value::as_i64),
        max_fall: planning_profile.get("max_fall").and_then(Value::as_i64),
        hdr10: flag(planning_profile.get("hdr"), "hdr10"),
        hdr10plus: primary_hdr_json.is_some(),
        hlg: flag(planning_profile.get("hdr"), "hlg"),
        dolby_vision: flag(planning_profile.get("hdr"), "dolby_vision"),
        dovi_profile: planning_profile
            .pointer("/hdr/dolby_vision_profile")
            .and_then(Value::as_i64),
        dovi_rpu_present: flag(planning_profile.get("hdr"), "dolby_vision_rpu"),
        interlaced: planning_profile
            .get("interlaced")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let toolchain = crate::calibration::CalibrationToolchain {
        ffmpeg: tools.ffmpeg.clone(),
        x265_pixel_formats: tools.x265_pixel_formats.clone(),
        x265_dolbyvision: tools.x265_dolbyvision,
    };
    let cal_options = crate::calibration::CalibrationOptions {
        preset: effective.preset.clone(),
        tune: content_tune.map(str::to_owned),
        crf: encode_crf,
        sample_seconds: options.calibration_sample_seconds,
        hdr10plus_json: primary_hdr_json.map(|path| path.to_string_lossy().into_owned()),
        preserve_dolbyvision,
        strip_dolbyvision,
        preserve_source_level: options.preserve_source_level,
        disable_hdr10_opt: options.disable_hdr10_opt,
        force_deinterlace: options.force_deinterlace,
        disable_deinterlace: options.disable_deinterlace,
    };
    let base_plan = crate::calibration::CalibrationPlanKey {
        worker_threads: Some(plan.worker_threads),
        frame_threads: Some(plan.frame_threads),
        cpu_threads: plan.cpu_threads,
        memory_guard_gib: plan.memory_guard_gib,
    };
    let key_response = crate::calibration::calibration_key(crate::calibration::KeyRequest {
        profile: cal_profile.clone(),
        toolchain: toolchain.clone(),
        options: cal_options.clone(),
        base_plan: base_plan.clone(),
        total_memory_gib: total_memory,
        max_candidates: options.calibration_max_candidates,
        throughput_tolerance_percent: options.calibration_tolerance_percent,
        hdr10plus_enabled: primary_hdr_json.is_some(),
        encoder_fingerprint: None,
    })?;
    let key = key_response
        .get("key")
        .and_then(Value::as_str)
        .ok_or("Calibration key helper returned no key.")?
        .to_string();

    let mut selection = None;
    if options.calibration_mode != "always" {
        if let Ok(cached) = crate::calibration::cache_query(crate::calibration::CacheQuery {
            key: key.clone(),
            cache_hours: options.calibration_cache_hours,
            base_worker_threads: Some(plan.worker_threads),
            base_frame_threads: Some(plan.frame_threads),
        }) {
            if cached.get("hit").and_then(Value::as_bool) == Some(true) {
                selection = Some(cached);
            }
        }
    }
    if selection.is_none() {
        state.phase = Phase::HardwareCalibration;
        emit(
            context,
            frame,
            "Calibrating the encoder",
            0.08,
            None,
            None,
            None,
            None,
        );
        (context.line)(format!(
            "Calibrating x265 thread layout: up to {} candidates × {}s of representative video.",
            options.calibration_max_candidates, options.calibration_sample_seconds
        ));
        match crate::calibration::run_calibration(crate::calibration::RunRequest {
            key: key.clone(),
            max_candidates: options.calibration_max_candidates,
            throughput_tolerance_percent: options.calibration_tolerance_percent,
            memory_guard_gib: plan.memory_guard_gib,
            profile: cal_profile,
            toolchain,
            options: cal_options,
            base_plan,
        }) {
            Ok(result) => {
                if result.get("source").and_then(Value::as_str) == Some("benchmark") {
                    if let (Some(workers), Some(frames), Some(results)) = (
                        result.get("worker_threads").and_then(Value::as_i64),
                        result.get("frame_threads").and_then(Value::as_i64),
                        result.get("results").cloned(),
                    ) {
                        let parsed: Vec<crate::calibration::CalibrationResult> =
                            serde_json::from_value(results).unwrap_or_default();
                        let _ =
                            crate::calibration::store_selection(crate::calibration::StoreRequest {
                                key: key.clone(),
                                worker_threads: workers,
                                frame_threads: frames,
                                results: parsed,
                            });
                    }
                }
                selection = Some(result);
            }
            Err(error) => {
                (context.line)(format!(
                    "Hardware calibration failed; continuing with the normal resource plan: {error}"
                ));
                state
                    .row
                    .insert("calibration_source".into(), json!("fallback"));
                state.row.insert(
                    "calibration_note".into(),
                    json!("Normal resource plan retained after calibration could not run."),
                );
            }
        }
    }

    if let Some(selection) = selection {
        let workers = selection
            .get("worker_threads")
            .or_else(|| selection.pointer("/plan/worker_threads"))
            .and_then(Value::as_i64);
        let frames = selection
            .get("frame_threads")
            .or_else(|| selection.pointer("/plan/frame_threads"))
            .and_then(Value::as_i64);
        if let (Some(workers), Some(frames)) = (workers, frames) {
            plan.worker_threads = workers.max(1);
            plan.frame_threads = frames.max(1);
            let source_label = selection
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("cache");
            state
                .row
                .insert("calibration_source".into(), json!(source_label));
            state
                .row
                .insert("calibration_workers".into(), json!(plan.worker_threads));
            state.row.insert(
                "calibration_frame_threads".into(),
                json!(plan.frame_threads),
            );
            let note = selection
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("Selected by hardware calibration.");
            state.row.insert("calibration_note".into(), json!(note));
            (context.line)(format!(
                "Using hardware calibration ({source_label}): {} workers / {} frame thread(s).",
                plan.worker_threads, plan.frame_threads
            ));
        }
    }
    Ok(())
}
