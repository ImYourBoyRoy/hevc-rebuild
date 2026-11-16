//! Optimizes one file for YouTube: copy a healthy stream or shrink an oversized one, behind the SSIM gate.

use super::*;

/// Lossless-first Optimize flow for one source that is already 1440p or larger:
/// stream-copy into a faststart MP4 with YouTube tags, or re-encode only when the
/// plan says so and the sampled SSIM check accepts the result.
pub(super) fn execute_optimize(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    destination: &Path,
    row: &mut Map<String, Value>,
    temporary: &mut Option<PathBuf>,
    live: &mut Live,
    profile: &Value,
) -> Result<(), Halt> {
    use youtube_optimize::{Action, AudioAction, PlanInput, QualityCheck, Shrink};
    let source = frame.source;
    let options = context.options;
    let tools = context.tools;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let quality = QualityCheck::parse(&options.quality_check)?;
    let plan = youtube_optimize::plan(
        profile,
        &PlanInput {
            shrink: Shrink::parse(&options.shrink)?,
            sdr_codec: &options.sdr_codec,
            hdr10plus_loss_accepted: options.hdr10plus_loss.contains(source),
            hdr10plus_can_inject: tools.hdr10plus.is_some(),
            x265_pixel_formats: &tools.x265_pixel_formats,
            file_bytes: frame.input_bytes,
            duration,
        },
    )
    .map_err(Halt::Skip)?;
    let (width, height) = youtube::expected_dimensions(profile, "source")?;
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .ok_or_else(|| String::from("The source profile has no selected video stream."))?;
    row.insert("upload_mode".into(), json!("optimize"));
    row.insert("upload_resolution".into(), json!("source"));
    row.insert("upload_color_mode".into(), json!(plan.color_mode));
    row.insert("optimize_reason".into(), json!(plan.reason));
    row.insert(
        "youtube_recommended_video_bitrate_kbps".into(),
        json!(plan.recommended_kbps),
    );
    if let Some(rate) = plan.source_video_kbps {
        row.insert(
            "source_video_bitrate_kbps".into(),
            json!(rate.round() as i64),
        );
    }
    if !plan.notes.is_empty() {
        row.insert("optimize_note".into(), json!(plan.notes.join(" ")));
    }
    (context.line)(format!("{name}: {}", plan.reason));
    for note_text in &plan.notes {
        (context.line)(format!("{name}: {note_text}"));
    }

    let audio_rate: i64 = plan
        .audio
        .iter()
        .map(|action| match action {
            AudioAction::Copy => 384,
            AudioAction::Transcode { kbps } => *kbps,
        })
        .sum();
    let encode_estimate =
        (duration * (plan.encode_bitrate_kbps.saturating_add(audio_rate)) as f64 * 1000.0 / 8.0
            * 1.12)
            .max(0.0) as u64;
    let estimated_bytes = match plan.action {
        Action::Copy => frame.input_bytes,
        Action::Encode => encode_estimate,
    };
    row.insert("estimated_output_bytes".into(), json!(estimated_bytes));
    live.estimated_output_bytes = Some(estimated_bytes);
    let reserve = options.disk_reserve_bytes();
    let output_parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(free) = disk_free_bytes(output_parent) {
        let needed = frame.input_bytes.max(estimated_bytes);
        if free < needed.saturating_add(reserve) {
            return Err(Halt::Fail(format!(
                "Not enough free space for the output ({} plus {:.1} GiB reserve).",
                human_bytes(needed),
                reserve as f64 / GIB
            )));
        }
    }

    let temp_path = temporary_name(destination);
    *temporary = Some(temp_path.clone());
    let mut action = plan.action;
    let mut decision_note: Option<String> = None;
    let mut quality_value: Option<f64> = None;
    let audio_plan = youtube_optimize::audio_plan_json(&plan.audio);
    let hdr10plus_expected = tools.hdr10plus.is_some()
        && profile
            .pointer("/hdr/hdr10plus_hint")
            .and_then(Value::as_bool)
            == Some(true);
    let inject_hdr10plus = action == Action::Encode && hdr10plus_expected;
    let optimize_work_dir = if inject_hdr10plus {
        Some(
            crate::media_work_directory(source, &context.options.work_root)?
                .join(format!("youtube-opt-hdr10plus-{}", std::process::id())),
        )
    } else {
        None
    };
    let hdr10plus_json = if inject_hdr10plus {
        let dir = optimize_work_dir.as_ref().expect("inject work dir");
        (context.line)(format!("{name}: extracting HDR10+ for post-encode inject"));
        Some(extract_youtube_hdr10plus_json(tools, source, profile, dir)?)
    } else {
        None
    };

    if action == Action::Encode {
        let resources =
            plan_resources(&options.settings, profile, (width, height), &options.preset)?;
        row.insert("planned_workers".into(), json!(resources.workers));
        row.insert("planned_frame_threads".into(), json!(resources.frames));
        let job = EncodeJob {
            duration,
            estimated_bytes,
            reserve,
            phase: "Re-encoding",
            temp_path: &temp_path,
        };
        encode_with_retries(context, frame, live, row, &job, resources, &|resources| {
            let built = youtube::build_args(&json!({
                "profile": profile,
                "tools": {
                    "ffmpeg": tools.ffmpeg,
                    "x265_pixel_formats": tools.x265_pixel_formats,
                },
                "source_path": source.to_string_lossy(),
                "output": temp_path.to_string_lossy(),
                "resolution": "source",
                "video_bitrate_kbps": plan.encode_bitrate_kbps,
                "worker_threads": resources.workers,
                "frame_threads": resources.frames,
                "sdr_codec": options.sdr_codec,
                "preset": options.preset,
                "audio_plan": audio_plan,
            }))?;
            string_args(&built)
        })?;
        if let (Some(json), Some(dir)) = (&hdr10plus_json, &optimize_work_dir) {
            emit(
                context,
                frame,
                "Injecting HDR10+ metadata",
                0.91,
                Some(100.0),
                live,
            );
            (context.line)(format!("{name}: injecting HDR10+ into the YouTube MP4"));
            inject_youtube_hdr10plus(tools, &temp_path, json, dir)?;
            row.insert(
                "hdr10plus_note".into(),
                json!("HDR10+ dynamic metadata was preserved after encode with hdr10plus_tool inject."),
            );
            let _ = fs::remove_file(json);
            let _ = fs::remove_dir(dir);
        }
        emit(context, frame, "Validating output", 0.92, Some(100.0), live);
        youtube::validate_request(&json!({
            "profile": profile,
            "tools": {
                "ffprobe": tools.ffprobe,
                "ffmpeg": tools.ffmpeg,
                "hdr10plus_tool": tools.hdr10plus,
            },
            "output": temp_path.to_string_lossy(),
            "resolution": "source",
            "sdr_codec": options.sdr_codec,
            "allow_copied_audio": true,
            "hdr10plus_expected": inject_hdr10plus,
        }))?;
        let output_bytes = fs::metadata(&temp_path).map(|meta| meta.len()).unwrap_or(0);
        let mut rejection: Option<String> = None;
        let saved = 1.0 - output_bytes as f64 / frame.input_bytes.max(1) as f64;
        if plan.copy_fallback && saved < youtube_optimize::MIN_SAVING_FRACTION {
            rejection = Some(format!(
                "the re-encode saved only {:.0}% ({} versus {})",
                saved.max(0.0) * 100.0,
                human_bytes(output_bytes),
                human_bytes(frame.input_bytes)
            ));
        }
        if rejection.is_none() {
            if let Some(floor) = quality.floor() {
                emit(context, frame, "Checking quality", 0.94, None, live);
                (context.line)(format!(
                    "Comparing {name} with the source on sampled windows (SSIM floor {floor})"
                ));
                let child = &mut *context.child;
                match youtube_optimize::ssim_gate(
                    &tools.ffmpeg,
                    source,
                    &temp_path,
                    primary_index,
                    duration,
                    child,
                ) {
                    Ok(value) => {
                        quality_value = Some(value);
                        row.insert(
                            "quality_ssim_min".into(),
                            json!((value * 10_000.0).round() / 10_000.0),
                        );
                        if value < floor {
                            rejection = Some(format!(
                                "sampled SSIM {value:.4} is below the {floor} floor"
                            ));
                        }
                    }
                    Err(youtube_optimize::GateError::Interrupted) => return Err(Halt::Interrupted),
                    Err(youtube_optimize::GateError::Failed(message)) => {
                        rejection = Some(format!("the quality check could not run ({message})"));
                    }
                }
            }
        }
        if let Some(why) = rejection {
            let _ = fs::remove_file(&temp_path);
            row.remove("hdr10plus_note");
            if let Some(json) = &hdr10plus_json {
                let _ = fs::remove_file(json);
            }
            if let Some(dir) = &optimize_work_dir {
                let _ = fs::remove_dir_all(dir);
            }
            if plan.copy_fallback {
                let text =
                    format!("Kept the original video stream instead of the re-encode: {why}.");
                (context.line)(format!("{name}: {text}"));
                decision_note = Some(text);
                action = Action::Copy;
            } else {
                return Err(Halt::Skip(format!(
                    "The re-encode was rejected ({why}) and this source cannot be stream-copied, so nothing was written. Relax the quality check or re-encode it with Upscale mode."
                )));
            }
        }
    }

    let mut payload_identical: Option<bool> = None;
    if action == Action::Copy {
        let resources = EncodePlan {
            workers: 1,
            frames: 1,
            per_frame: 0.0,
            per_worker: 0.0,
            budget: None,
            memory_guard_gib: 0.0,
        };
        let job = EncodeJob {
            duration,
            estimated_bytes: frame.input_bytes,
            reserve,
            phase: "Copying streams",
            temp_path: &temp_path,
        };
        encode_with_retries(context, frame, live, row, &job, resources, &|_| {
            youtube_optimize::copy_args(
                &tools.ffmpeg,
                &source.to_string_lossy(),
                &temp_path.to_string_lossy(),
                profile,
                &plan,
            )
        })?;
        emit(context, frame, "Validating output", 0.92, Some(100.0), live);
        youtube::validate_request(&json!({
            "profile": profile,
            "tools": {
                "ffprobe": tools.ffprobe,
                "ffmpeg": tools.ffmpeg,
                "hdr10plus_tool": tools.hdr10plus,
            },
            "output": temp_path.to_string_lossy(),
            "resolution": "source",
            "expected_codec": plan.source_codec,
            "allow_copied_audio": true,
            "hdr10plus_expected": hdr10plus_expected,
        }))?;
        emit(context, frame, "Verifying streams", 0.95, None, live);
        let ordinal = profile
            .get("tracks")
            .and_then(Value::as_array)
            .map(|tracks| {
                tracks
                    .iter()
                    .filter(|track| {
                        track.get("type").and_then(Value::as_str) == Some("video")
                            && track.get("index").and_then(Value::as_i64) < Some(primary_index)
                    })
                    .count()
            })
            .unwrap_or(0);
        let ffmpeg = Path::new(&tools.ffmpeg);
        let before = packet_verify::packet_fingerprints(ffmpeg, source, &[format!("v:{ordinal}")])?;
        let after = packet_verify::packet_fingerprints(ffmpeg, &temp_path, &["v:0".to_string()])?;
        if let (Some(left), Some(right)) = (before.first(), after.first()) {
            if left.packet_count != right.packet_count {
                return Err(Halt::Fail(format!(
                    "The stream copy changed the video packet count ({} in the source, {} in the copy); nothing was installed.",
                    left.packet_count, right.packet_count
                )));
            }
            payload_identical = Some(left == right);
        }
    }

    emit(context, frame, "Installing output", 0.98, Some(100.0), live);
    let installed = transactions::install_export_output(&temp_path, destination)?;
    *temporary = None;
    let output_bytes = fs::metadata(destination)
        .map(|meta| meta.len())
        .unwrap_or(0);
    live.output_bytes = Some(output_bytes);
    let mode_label = match plan.color_mode {
        "hdr10" => "HDR10/PQ",
        "hlg" => "HLG",
        _ => "SDR",
    };
    let saved_text = if output_bytes < frame.input_bytes {
        format!(
            ", {:.0}% smaller",
            (1.0 - output_bytes as f64 / frame.input_bytes.max(1) as f64) * 100.0
        )
    } else {
        String::new()
    };
    let how = match action {
        Action::Copy => format!(
            "video stream copied untouched ({}{}), YouTube tags and faststart applied",
            plan.source_codec.to_uppercase(),
            match payload_identical {
                Some(true) => ", packets verified identical",
                Some(false) => ", packet count verified",
                None => "",
            }
        ),
        Action::Encode => format!(
            "re-encoded at {} kb/s{}",
            plan.encode_bitrate_kbps,
            quality_value
                .map(|value| format!(", sampled SSIM {value:.4}"))
                .unwrap_or_default()
        ),
    };
    row.insert(
        "optimize_action".into(),
        json!(match action {
            Action::Copy => "copy",
            Action::Encode => "encode",
        }),
    );
    if let Some(identical) = payload_identical {
        row.insert("video_payload_identical".into(), json!(identical));
    }
    row.insert("status".into(), json!("OK"));
    row.insert("output_bytes".into(), json!(output_bytes));
    row.insert(
        "output_resolution".into(),
        json!(format!("{width}x{height}")),
    );
    let mut message = format!(
        "Validated YouTube {mode_label} MP4 ({width}x{height}): {how}{saved_text}; source unchanged."
    );
    if let Some(extra) = decision_note {
        message.push(' ');
        message.push_str(&extra);
    }
    row.insert("message".into(), json!(installed.annotate(message)));
    Ok(())
}
