//! Per-file analysis: source profile, content tune, Dolby Vision/HDR10+ policy and the recorded plan.

use super::*;

/// Stage 1: probe the source, choose the streams, and profile each video track.
pub(super) fn analyze_source(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
) -> Result<FilePlan, String> {
    let source = frame.source;
    let tools = context.tools;
    let options = context.options;
    let effective = effective_for_file(options, source);
    emit(
        context,
        frame,
        "Reading media tracks",
        0.0,
        None,
        None,
        None,
        None,
    );

    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: effective.promote_8bit_to_10bit,
    })?;
    let probe = response
        .get("probe")
        .cloned()
        .ok_or("The profiler returned no stream probe.")?;
    let mediainfo = response
        .get("mediainfo")
        .cloned()
        .ok_or("The profiler returned no MediaInfo.")?;
    let mut profile = response
        .get("profile")
        .cloned()
        .ok_or("The profiler returned no profile.")?;
    let refs = stream_refs(&probe);
    let (selected_streams, encode_video_indices) = select_file_streams(&refs, options, &effective)?;
    let unstorable_data_streams = if effective.stream_indices.is_some() {
        0
    } else {
        unstorable_data_streams(&refs, options)
    };
    if unstorable_data_streams > 0 {
        (context.line)(format!(
            "{unstorable_data_streams} data stream(s) (such as a timecode track) cannot be stored in Matroska and will not be copied."
        ));
    }
    let real_videos: Vec<StreamRef> = selected_streams
        .iter()
        .filter(|s| s.real_video())
        .cloned()
        .collect();
    let primary = real_videos
        .first()
        .cloned()
        .ok_or("Select at least one real video track for this media file.")?;

    let mut video_profiles: BTreeMap<i64, Value> = BTreeMap::new();
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .unwrap_or(primary.index);
    if primary_index == primary.index {
        video_profiles.insert(primary.index, profile.clone());
    }
    let others: Vec<i64> = real_videos
        .iter()
        .map(|stream| stream.index)
        .filter(|index| !video_profiles.contains_key(index))
        .collect();
    if !others.is_empty() {
        let tracks = engine_track_profiles(EngineTrackProfilesRequest {
            path: source.to_string_lossy().into_owned(),
            ffprobe: tools.ffprobe.clone(),
            promote_8bit_to_10bit: effective.promote_8bit_to_10bit,
            video_stream_indices: others.clone(),
            probe: probe.clone(),
            mediainfo: mediainfo.clone(),
        })?;
        let map = tracks
            .get("video_profiles")
            .and_then(Value::as_object)
            .ok_or("The selected-track profiler returned an invalid response.")?;
        for index in others {
            let variant = map.get(&index.to_string()).ok_or_else(|| {
                format!("The selected-track profiler omitted video stream #{index}.")
            })?;
            video_profiles.insert(index, variant.clone());
            if index == primary.index {
                profile = variant.clone();
            }
        }
    }
    video_profiles
        .entry(primary.index)
        .or_insert_with(|| profile.clone());
    Ok(FilePlan {
        effective,
        probe,
        mediainfo,
        profile,
        refs,
        selected_streams,
        encode_video_indices,
        real_videos,
        primary,
        video_profiles,
        verified_hdr10plus: BTreeSet::new(),
        hdr10plus_verification_skipped: false,
        content_tune: None,
        preserve_dolbyvision_indices: BTreeSet::new(),
        strip_dolbyvision_indices: BTreeSet::new(),
        planning_profile: Value::Null,
        unstorable_data_streams,
    })
}

/// Stage 2: verify HDR10+ dynamic metadata on every video track being encoded.
pub(super) fn verify_hdr10plus_sources(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    plan: &mut FilePlan,
) -> Result<(), String> {
    let source = frame.source;
    let tools = context.tools;
    let options = context.options;
    let FilePlan {
        effective,
        real_videos,
        encode_video_indices,
        video_profiles,
        verified_hdr10plus,
        hdr10plus_verification_skipped,
        ..
    } = plan;
    let mut saw_hint = false;
    for stream in real_videos.iter() {
        if !encode_video_indices.contains(&stream.index) {
            continue;
        }
        let track = &video_profiles[&stream.index];
        if !flag(track.get("hdr"), "hdr10plus_hint") {
            continue;
        }
        saw_hint = true;
        if options.info_only
            && matches!(
                stream.codec_name.to_ascii_lowercase().as_str(),
                "hevc" | "h265"
            )
            && tools.hdr10plus.is_none()
        {
            *hdr10plus_verification_skipped = true;
            continue;
        }
        emit(
            context,
            frame,
            "Checking HDR10+ metadata",
            0.02,
            None,
            None,
            None,
            None,
        );
        let verified = match &tools.hdr10plus {
            Some(helper) => crate::hdr_metadata::verify_hdr10plus_source(
                &tools.ffmpeg,
                helper,
                source,
                stream.index,
                &stream.codec_name,
            )
            .unwrap_or(false),
            None => false,
        };
        if !options.info_only && effective.preserve_hdr10plus && !verified {
            let codec = stream.codec_name.to_ascii_lowercase();
            if !matches!(codec.as_str(), "hevc" | "h265") {
                return Err(format!(
                    "HDR10+ is indicated on video stream #{}, but its {codec} codec cannot use the frame-aligned preservation path.",
                    stream.index
                ));
            }
            return Err(format!(
                "HDR10+ is indicated on video stream #{}, but hdr10plus_tool could not verify it; refusing possible metadata loss.",
                stream.index
            ));
        }
        if verified {
            verified_hdr10plus.insert(stream.index);
            if let Some(object) = video_profiles
                .get_mut(&stream.index)
                .and_then(Value::as_object_mut)
            {
                if let Some(hdr) = object.get_mut("hdr").and_then(Value::as_object_mut) {
                    hdr.insert("hdr10plus".into(), json!(true));
                }
            }
        }
    }
    if !saw_hint {
        (context.line)("No HDR10+ signaling, so the full metadata scan was skipped.".into());
    }
    Ok(())
}

/// Stage 3: pick the x265 tune from reviewed genres, overrides, and detection.
pub(super) fn resolve_content_tune(
    options: &RebuildRunOptions,
    source: &Path,
    plan: &FilePlan,
    state: &mut FileState,
) -> Result<Option<String>, String> {
    let effective = &plan.effective;
    let mediainfo = &plan.mediainfo;
    let file_tune = if effective.clear_file_tune {
        None
    } else {
        options.file_tune_overrides.get(source).map(String::as_str)
    };
    let reviewed_genres = options
        .metadata_overrides
        .get(source)
        .and_then(|fields| fields.get("genres"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let content_tune = crate::content::resolve_run_tune(
        mediainfo,
        &effective.tune,
        options.detect_animation,
        file_tune,
        &reviewed_genres,
    )?;
    state.row.insert(
        "content_tune".into(),
        json!(content_tune.clone().unwrap_or_else(|| "normal".into())),
    );
    Ok(content_tune)
}

/// Stage 4: apply the Dolby Vision policy; a skip finishes the file early.
pub(super) fn resolve_dolby_vision(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &mut FilePlan,
) -> Result<Flow, String> {
    let tools = context.tools;
    let options = context.options;
    let effective = &plan.effective;
    let video_profiles = &plan.video_profiles;
    let mut preserve_dolbyvision_indices = BTreeSet::new();
    let mut strip_dolbyvision_indices = BTreeSet::new();
    let dovi_profiles: Vec<(i64, &Value)> = video_profiles
        .iter()
        .filter(|(_, track)| flag(track.get("hdr"), "dolby_vision"))
        .map(|(index, track)| (*index, track))
        .collect();
    if !dovi_profiles.is_empty() {
        emit(
            context,
            frame,
            "Checking Dolby Vision",
            0.04,
            None,
            None,
            None,
            None,
        );
        if effective.dolby_vision_policy == "skip" {
            state.set_status(RunStatus::Skipped);
            state
                .row
                .insert("dolby_vision_action".into(), json!("skip"));
            state.row.insert(
                "message".into(),
                json!(
                    "A selected video track contains Dolby Vision; skipped by configured policy."
                ),
            );
            emit(
                context,
                frame,
                "Skipped by Dolby Vision policy",
                1.0,
                Some(100.0),
                None,
                None,
                None,
            );
            return Ok(Flow::Finished);
        }
        for (index, track) in &dovi_profiles {
            if effective.dolby_vision_policy == "hdr10-fallback" {
                let reasons = crate::hdr_metadata::hdr10_fallback_reasons_for_profile(track)?;
                if !reasons.is_empty() {
                    return Err(format!(
                        "Dolby Vision HDR10 fallback is unsafe for source stream #{index}: {}",
                        reasons.join("; ")
                    ));
                }
                strip_dolbyvision_indices.insert(*index);
            } else {
                let reasons = crate::hdr_metadata::safe_preservation_reasons_for_profile(
                    track,
                    tools.x265_dolbyvision,
                    options.force_deinterlace,
                )?;
                if !reasons.is_empty() {
                    let mut message = format!(
                        "Dolby Vision safe-preservation requirements not met on source stream #{index}: {}",
                        reasons.join("; ")
                    );
                    if crate::hdr_metadata::hdr10_fallback_reasons_for_profile(track)?.is_empty() {
                        message.push_str(
                            ". A verified HDR10 fallback is available using Dolby Vision policy hdr10-fallback.",
                        );
                    }
                    state.set_status(RunStatus::Skipped);
                    state.row.insert(
                        "dolby_vision_action".into(),
                        json!("skip-unsafe-preservation"),
                    );
                    state.row.insert("message".into(), json!(message));
                    emit(
                        context,
                        frame,
                        "Skipped: Dolby Vision cannot be safely preserved",
                        1.0,
                        Some(100.0),
                        None,
                        None,
                        None,
                    );
                    return Ok(Flow::Finished);
                }
                preserve_dolbyvision_indices.insert(*index);
            }
        }
        if !strip_dolbyvision_indices.is_empty() {
            state
                .row
                .insert("dolby_vision_action".into(), json!("hdr10-fallback"));
        } else if !preserve_dolbyvision_indices.is_empty() {
            state
                .row
                .insert("dolby_vision_action".into(), json!("preserve"));
        }
    }
    plan.preserve_dolbyvision_indices = preserve_dolbyvision_indices;
    plan.strip_dolbyvision_indices = strip_dolbyvision_indices;
    Ok(Flow::Continue)
}

/// Stage 5: record the analysis in the report row and forecast resources.
pub(super) fn record_analysis_and_plan(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &mut FilePlan,
) -> Result<ResourcePlan, String> {
    let _ = &context;
    let options = context.options;
    let input_bytes = frame.input_bytes;
    let any_hdr10plus = plan.any_hdr10plus();
    let FilePlan {
        effective,
        profile,
        primary,
        selected_streams,
        encode_video_indices,
        video_profiles,
        ..
    } = &*plan;
    let resolution = format!(
        "{}x{}",
        profile
            .pointer("/video/width")
            .and_then(Value::as_i64)
            .unwrap_or(primary.width),
        profile
            .pointer("/video/height")
            .and_then(Value::as_i64)
            .unwrap_or(primary.height)
    );
    for (key, value) in [
        (
            "primary_video",
            profile
                .pointer("/video/codec")
                .cloned()
                .unwrap_or_else(|| json!(primary.codec_name)),
        ),
        ("resolution", json!(resolution)),
        (
            "audio_count",
            json!(selected_streams
                .iter()
                .filter(|s| s.codec_type == "audio")
                .count()),
        ),
        (
            "subtitle_count",
            json!(selected_streams
                .iter()
                .filter(|s| s.codec_type == "subtitle")
                .count()),
        ),
        (
            "selected_stream_indices",
            json!(selected_streams
                .iter()
                .map(|s| s.index.to_string())
                .collect::<Vec<_>>()
                .join(",")),
        ),
        (
            "encoded_video_indices",
            json!(encode_video_indices
                .iter()
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
                .join(",")),
        ),
        (
            "hdr10",
            json!(video_profiles.values().any(|p| flag(p.get("hdr"), "hdr10"))),
        ),
        ("hdr10plus", json!(any_hdr10plus)),
        (
            "dolby_vision",
            json!(video_profiles
                .values()
                .any(|p| flag(p.get("hdr"), "dolby_vision"))),
        ),
        ("input_bytes", json!(input_bytes)),
    ] {
        state.row.insert(key.into(), value);
    }
    let planning_profile = video_profiles
        .values()
        .max_by_key(|track| {
            let width = track
                .pointer("/video/width")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let height = track
                .pointer("/video/height")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let depth = track
                .pointer("/video/source_bit_depth")
                .and_then(Value::as_i64)
                .unwrap_or(8);
            (width * height, depth)
        })
        .cloned()
        .unwrap_or(profile.clone());
    let info_plan = choose_resource_plan(
        options,
        &planning_profile,
        &effective.preset,
        encode_video_indices.len() as i64,
    );
    match &info_plan {
        Ok(plan) => record_memory_forecast(&mut state.row, plan),
        Err(error) => {
            state.row.insert("memory_forecast".into(), json!("blocked"));
            state
                .row
                .insert("memory_forecast_message".into(), json!(error));
        }
    }
    plan.planning_profile = planning_profile;
    info_plan
}

/// Analysis-only runs stop here, after the plan and forecast are recorded.
pub(super) fn finish_info_only(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &FilePlan,
    info_plan: &Result<ResourcePlan, String>,
) {
    let hdr10plus_verification_skipped = plan.hdr10plus_verification_skipped;
    state.set_status(RunStatus::Info);
    let mut message = "Analysis only; no changes made.".to_string();
    if hdr10plus_verification_skipped {
        message.push_str(
                " HDR10+ dynamic metadata was not checked because hdr10plus_tool is missing; install the helper before rebuilding HEVC tracks.",
            );
    }
    if let Err(error) = &info_plan {
        message.push_str(&format!(
                " Memory forecast blocks encoding until available RAM recovers or the configured reserve is changed ({error})."
            ));
    }
    state.row.insert("message".into(), json!(message));
    emit(
        context,
        frame,
        "Analysis complete",
        1.0,
        Some(100.0),
        None,
        None,
        None,
    );
}
