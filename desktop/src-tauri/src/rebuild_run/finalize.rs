//! After the encode: metadata embedding, output validation and the durable install.

use super::*;

/// Stage 10: embed reviewed tags and the poster, and scrub unwanted text.
pub(super) fn embed_metadata(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    work_dir: &Path,
    plan: &FilePlan,
    targets: &OutputTargets,
) -> Result<EmbeddedMetadata, String> {
    let source = frame.source;
    let tools = context.tools;
    let options = context.options;
    let temp_output = &targets.temp_output;
    state.phase = Phase::MetadataEmbedding;
    emit(
        context,
        frame,
        "Embedding metadata and poster",
        0.90,
        None,
        file_size(temp_output),
        None,
        None,
    );
    let applied = crate::matroska_apply::apply_metadata_value(json!({
        "media_path": source,
        "output": temp_output,
        "work_dir": work_dir,
        "ffmpeg": tools.ffmpeg,
        "tools": {
            "mkvmerge": tools.mkvmerge,
            "mkvextract": tools.mkvextract,
            "mkvpropedit": tools.mkvpropedit,
        },
        "comment": options.comment,
        "text_blacklist": options.text_blacklist,
        "metadata_override": options.metadata_overrides.get(source),
        "poster_override": options.poster_overrides.get(source),
        "embed_artwork": options.embed_artwork,
        "carry_source_cover": true,
        "source_cover_stream": source_cover_stream(&plan.refs),
    }))?;
    let scrubbed = applied
        .get("scrubbed_text_values")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let scrub_count = scrubbed.as_array().map_or(0, Vec::len);
    state
        .row
        .insert("metadata_text_scrub_count".into(), json!(scrub_count));
    state.row.insert(
        "metadata_text_scrub_details".into(),
        json!(serde_json::to_string(&scrubbed).unwrap_or_else(|_| "[]".into())),
    );
    let notes = if scrub_count > 0 {
        format!(" Metadata text cleaned in {scrub_count} field(s).")
    } else {
        String::new()
    };
    let expect_poster = applied
        .get("poster_path")
        .is_some_and(|poster| !poster.is_null());
    Ok(EmbeddedMetadata {
        applied,
        scrubbed,
        notes,
        expect_poster,
    })
}

/// Stage 11: validate the finished output against the source before install.
pub(super) fn validate_output(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    work_dir: &Path,
    plan: &FilePlan,
    targets: &OutputTargets,
    embedded: &EmbeddedMetadata,
) -> Result<(), String> {
    let source = frame.source;
    let tools = context.tools;
    let options = context.options;
    let FilePlan {
        effective,
        probe,
        profile,
        selected_streams,
        encode_video_indices,
        real_videos,
        video_profiles,
        verified_hdr10plus,
        preserve_dolbyvision_indices,
        strip_dolbyvision_indices,
        ..
    } = plan;
    let temp_output = &targets.temp_output;
    let expected_duration = expected_duration(options, targets.duration);
    let applied = &embedded.applied;
    let scrubbed = &embedded.scrubbed;
    let expect_poster = embedded.expect_poster;
    state.phase = Phase::OutputValidation;
    emit(
        context,
        frame,
        "Validating video, streams, metadata, and HDR",
        0.97,
        None,
        file_size(temp_output),
        None,
        None,
    );
    let mut targets = Map::new();
    let mut expectations = Map::new();
    for stream in real_videos.iter() {
        if !encode_video_indices.contains(&stream.index) {
            continue;
        }
        let track = &video_profiles[&stream.index];
        if let Some(format) = track
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
        {
            targets.insert(stream.index.to_string(), json!(format));
        }
        expectations.insert(
            stream.index.to_string(),
            hdr_expectation(
                track,
                effective.preserve_hdr10plus && verified_hdr10plus.contains(&stream.index),
                preserve_dolbyvision_indices.contains(&stream.index),
                strip_dolbyvision_indices.contains(&stream.index),
            ),
        );
    }
    let validation = crate::validation::validate_streams_value(json!({
        "ffmpeg": tools.ffmpeg,
        "ffprobe": tools.ffprobe,
        "mediainfo": tools.mediainfo,
        "hdr10plus_tool": tools.hdr10plus,
        "source_path": source,
        "output_path": temp_output,
        "source_probe": probe,
        "selected_stream_indices": selected_streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        "encode_video_indices": encode_video_indices.iter().copied().collect::<Vec<_>>(),
        "video_target_formats": targets,
        "video_hdr_expectations": expectations,
        "verify_copy_hashes": options.verify_copy_hashes,
        "test_mode": options.test_mode,
        "copy_only": false,
        "expect_poster": expect_poster,
        "source_duration": expected_duration,
        "source_chapters_count": profile.get("chapters_count").and_then(Value::as_u64).unwrap_or(0),
        "scrubbed_text_values": scrubbed,
        "chapter_text_edits": applied.get("chapter_text_edits").cloned().unwrap_or_else(|| json!([])),
    }))?;
    let _ = validation;
    let expected_tags: Vec<(String, String)> = applied
        .get("expected_global_tags")
        .and_then(Value::as_object)
        .map(|tags| {
            tags.iter()
                .filter_map(|(name, value)| value.as_str().map(|v| (name.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if expect_poster || !expected_tags.is_empty() {
        crate::matroska_verify::verify_expected_output(
            &crate::matroska_scrub::MatroskaTools {
                mkvmerge: tools.mkvmerge.clone(),
                mkvextract: tools.mkvextract.clone(),
                mkvpropedit: tools.mkvpropedit.clone(),
            },
            temp_output,
            work_dir,
            expect_poster,
            &expected_tags,
        )
        .map_err(|error| match error {
            crate::matroska_verify::VerifyError::Mismatch(text)
            | crate::matroska_verify::VerifyError::Operational(text) => text,
        })?;
    }
    Ok(())
}

/// Stage 11b: compare the finished picture with the source on a few sampled
/// windows. Returns a sentence for the report (empty when the gate is off).
/// Under `enforce`, an output below the floor is refused here, before anything
/// is installed, so the original is untouched.
pub(super) fn check_quality(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &FilePlan,
    targets: &OutputTargets,
) -> Result<String, String> {
    use crate::quality_gate::{
        judge, measure_windows, rebuild_deinterlaced, rebuild_windows, Comparison, GateError,
        GateMode,
    };
    let options = context.options;
    if options.quality_gate == GateMode::Off || options.test_mode {
        return Ok(String::new());
    }
    let temp_output = &targets.temp_output;
    // Compare the primary encoded video: its position among the output's video
    // streams is its position among the selected video streams.
    let primary = &plan.primary;
    if !plan.encode_video_indices.contains(&primary.index) {
        return Ok(String::new());
    }
    let output_video_ordinal = plan
        .selected_streams
        .iter()
        .filter(|stream| stream.codec_type == "video")
        .position(|stream| stream.index == primary.index)
        .unwrap_or(0);
    let interlaced = plan
        .video_profiles
        .get(&primary.index)
        .and_then(|track| track.get("interlaced"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let comparison = Comparison {
        source: frame.source,
        source_stream_index: primary.index,
        output: temp_output,
        output_video_ordinal,
        deinterlace: rebuild_deinterlaced(
            interlaced,
            options.force_deinterlace,
            options.disable_deinterlace,
        ),
    };
    state.phase = Phase::OutputValidation;
    emit(
        context,
        frame,
        "Comparing picture quality with the source",
        0.985,
        None,
        file_size(temp_output),
        None,
        None,
    );
    let windows = rebuild_windows(expected_duration(options, targets.duration));
    let child_callback = &mut *context.child;
    let scores = match measure_windows(&context.tools.ffmpeg, &comparison, &windows, |pid| {
        child_callback(pid)
    }) {
        Ok(scores) => scores,
        Err(GateError::Interrupted) => {
            state.interrupted = true;
            return Err("Quality check interrupted.".into());
        }
        Err(GateError::Failed(message)) => {
            return if options.quality_gate == GateMode::Enforce {
                Err(format!(
                    "The quality check could not run, so the new file was not installed and the original is untouched. {message}"
                ))
            } else {
                (context.line)(format!("Quality check skipped: {message}"));
                Ok(String::new())
            };
        }
    };
    let Some(verdict) = judge(&scores, options.quality_min_ssim) else {
        return Ok(String::new());
    };
    state
        .row
        .insert("quality_gate".into(), json!(options.quality_gate.as_str()));
    state.row.insert("ssim_mean".into(), json!(verdict.mean));
    state
        .row
        .insert("ssim_lowest".into(), json!(verdict.lowest));
    state
        .row
        .insert("ssim_windows".into(), json!(verdict.windows));
    state.row.insert("ssim_floor".into(), json!(verdict.floor));
    state
        .row
        .insert("ssim_passed".into(), json!(verdict.passed));
    (context.line)(format!("Picture quality check: {}", verdict.summary()));
    if verdict.passed {
        return Ok(format!(" Picture quality: {}.", verdict.summary()));
    }
    if options.quality_gate == GateMode::Enforce {
        return Err(format!(
            "Picture quality check failed: {}. The new file was not installed and the original is untouched. Lower the minimum SSIM, or set the quality gate to report only, if this source is known to be grainy.",
            verdict.summary()
        ));
    }
    Ok(format!(
        " Picture quality is below the floor (report only, installed anyway): {}.",
        verdict.summary()
    ))
}

/// Stage 12: install the validated output (test copy, separate copy, or the
/// in-place replacement transaction) and record the result.
pub(super) fn install_output(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    targets: &OutputTargets,
    embedded: &EmbeddedMetadata,
) -> Result<(), String> {
    let source = frame.source;
    let options = context.options;
    let temp_output = &targets.temp_output;
    let final_output = &targets.final_output;
    let separate_output = targets.separate_output;
    let notes = &embedded.notes;
    if options.test_mode {
        state.phase = Phase::ValidatedOutputInstall;
        let installed = install_export_output(temp_output, final_output)?;
        state.temp_output = None;
        let final_bytes = file_size(final_output);
        state.set_status(RunStatus::TestOk);
        state
            .row
            .insert("output".into(), json!(final_output.to_string_lossy()));
        state.row.insert("output_bytes".into(), json!(final_bytes));
        state.row.insert(
            "message".into(),
            json!(installed.annotate(format!(
                "Short test encode validated; source untouched.{notes}"
            ))),
        );
        emit(
            context,
            frame,
            "Short test validated",
            1.0,
            Some(100.0),
            final_bytes,
            None,
            None,
        );
        (context.line)(format!(
            "Validated test ready: {}",
            final_output
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        return Ok(());
    }

    state.phase = Phase::TransactionCommit;
    if separate_output {
        let installed = install_export_output(temp_output, final_output)?;
        state.temp_output = None;
        let final_bytes = file_size(final_output);
        state.set_status(RunStatus::Ok);
        state
            .row
            .insert("output".into(), json!(final_output.to_string_lossy()));
        state.row.insert("output_bytes".into(), json!(final_bytes));
        state.row.insert(
            "message".into(),
            json!(installed.annotate(format!(
                "Validated copy saved; original left in place at {}.{notes}",
                source.display()
            ))),
        );
        emit(
            context,
            frame,
            "Validated output installed",
            1.0,
            Some(100.0),
            final_bytes,
            None,
            None,
        );
        (context.line)(format!("Validated copy saved: {}", final_output.display()));
        return Ok(());
    }

    let backup = backup_path_for(source, options.reprocess)?;
    let installed = commit_output(source, temp_output, final_output, &backup)?;
    state.temp_output = None;
    let final_bytes = file_size(final_output);
    state.set_status(RunStatus::Ok);
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    state.row.insert("output_bytes".into(), json!(final_bytes));
    state.row.insert(
        "message".into(),
        json!(installed.annotate(format!(
            "Validated and committed; original moved to {}.{notes}",
            backup.display()
        ))),
    );
    emit(
        context,
        frame,
        "Validated output installed",
        1.0,
        Some(100.0),
        final_bytes,
        None,
        None,
    );
    (context.line)(format!(
        "Validated rebuild complete; original preserved in {}/",
        backup
            .parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
    ));
    Ok(())
}
