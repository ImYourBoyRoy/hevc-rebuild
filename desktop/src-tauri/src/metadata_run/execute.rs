//! Runs one file end to end: remux with `mkvmerge`, embed reviewed metadata and posters, validate, and verify tags.

use super::*;

pub(super) fn execute(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    work_dir: &Path,
    state: &mut FileState,
) -> Result<(), String> {
    let (source, input_bytes) = (frame.source, frame.input_bytes);
    let tools = context.tools;
    let options = context.options;
    emit(context, frame, "Reading media tracks", 0.0, None, None);

    let response = engine_profile(EngineProfileRequest {
        path: source.to_string_lossy().into_owned(),
        ffprobe: tools.ffprobe.clone(),
        mediainfo: tools.mediainfo.clone(),
        promote_8bit_to_10bit: false,
    })?;
    let probe = response
        .get("probe")
        .cloned()
        .ok_or("The profiler returned no stream probe.")?;
    let mediainfo = response
        .get("mediainfo")
        .cloned()
        .ok_or("The profiler returned no MediaInfo.")?;
    let profile = response
        .get("profile")
        .cloned()
        .ok_or("The profiler returned no profile.")?;

    let streams = source_streams(&probe);
    let mut unsupported: Vec<&str> = streams
        .iter()
        .map(|stream| stream.codec_type.as_str())
        .filter(|kind| !SUPPORTED_STREAM_TYPES.contains(kind))
        .collect();
    unsupported.sort_unstable();
    unsupported.dedup();
    if !unsupported.is_empty() {
        return Err(format!(
            "This source contains stream type(s) that cannot yet be verified for a lossless remux: {}. The original was left untouched.",
            unsupported.join(", ")
        ));
    }
    let real_videos: Vec<&SourceStream> =
        streams.iter().filter(|stream| stream.real_video).collect();
    if real_videos.is_empty() {
        return Err(
            "No real video track was found; metadata-only MKV output requires a video stream."
                .into(),
        );
    }
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .ok_or("The profiler did not select a primary video stream.")?;
    let mut video_profiles: BTreeMap<i64, Value> = BTreeMap::new();
    video_profiles.insert(primary_index, profile.clone());
    let others: Vec<i64> = real_videos
        .iter()
        .map(|stream| stream.index)
        .filter(|index| *index != primary_index)
        .collect();
    if !others.is_empty() {
        let tracks = engine_track_profiles(EngineTrackProfilesRequest {
            path: source.to_string_lossy().into_owned(),
            ffprobe: tools.ffprobe.clone(),
            promote_8bit_to_10bit: false,
            video_stream_indices: others.clone(),
            probe: probe.clone(),
            mediainfo,
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
        }
    }

    // HDR10+ indicated by metadata must be proven in the source before remux.
    let mut verified_hdr10plus: BTreeSet<i64> = BTreeSet::new();
    let hinted: Vec<&SourceStream> = real_videos
        .iter()
        .copied()
        .filter(|stream| {
            flag(
                video_profiles.get(&stream.index).and_then(|p| p.get("hdr")),
                "hdr10plus_hint",
            )
        })
        .collect();
    if !hinted.is_empty() {
        emit(context, frame, "Checking HDR10+ metadata", 0.02, None, None);
    }
    for stream in hinted {
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
        if !verified {
            return Err(format!(
                "HDR10+ is indicated on video stream #{}, but it could not be verified before remux. No output was installed.",
                stream.index
            ));
        }
        verified_hdr10plus.insert(stream.index);
    }

    let hdr_of = |key: &str| video_profiles.values().any(|p| flag(p.get("hdr"), key));
    let resolution = format!(
        "{}x{}",
        profile
            .pointer("/video/width")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        profile
            .pointer("/video/height")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    );
    let count = |kind: &str| streams.iter().filter(|s| s.codec_type == kind).count();
    for (key, value) in [
        (
            "primary_video",
            profile
                .pointer("/video/codec")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("resolution", json!(resolution)),
        ("audio_count", json!(count("audio"))),
        ("subtitle_count", json!(count("subtitle"))),
        ("hdr10", json!(hdr_of("hdr10"))),
        ("hdr10plus", json!(!verified_hdr10plus.is_empty())),
        ("dolby_vision", json!(hdr_of("dolby_vision"))),
        ("input_bytes", json!(input_bytes)),
    ] {
        state.row.insert(key.into(), value);
    }

    let final_output = metadata_output_path(source, options)?;
    if final_output == source {
        return Err(
            "Metadata-only output must be a new file; the source path cannot be the destination."
                .into(),
        );
    }
    if final_output.exists() {
        return Err(format!(
            "Output already exists and will not be overwritten: {}",
            final_output.display()
        ));
    }
    let output_parent = final_output
        .parent()
        .ok_or("The planned output has no parent folder.")?
        .to_path_buf();
    fs::create_dir_all(&output_parent)
        .map_err(|error| format!("Could not create the output folder: {error}"))?;
    let stem = final_output
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let temp_output =
        output_parent.join(format!(".{stem}.hevc-metadata.{}.tmp.mkv", unique_token()));
    state.final_output = Some(final_output.clone());
    state.temp_output = Some(temp_output.clone());
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));

    state.phase = "storage preflight";
    emit(context, frame, "Checking free disk space", 0.05, None, None);
    let free = disk_free_bytes(&output_parent).ok_or_else(|| {
        format!(
            "Cannot determine free space for output folder {}",
            output_parent.display()
        )
    })?;
    storage_preflight(input_bytes.unwrap_or(0), free, options.min_free_disk_gib)?;

    state.phase = "stream remux";
    (context.line)(format!(
        "Metadata-only output: all {} source streams will be copied to {}; no video encoder will run.",
        streams.len(),
        final_output.display()
    ));
    emit(context, frame, "Copying streams", 0.08, None, None);
    if let Err(error) = run_mkvmerge(context, frame, &temp_output) {
        state.interrupted = options
            .stop_file
            .as_deref()
            .is_some_and(crate::stop_control::requested);
        return Err(error);
    }

    state.phase = "metadata embedding";
    emit(
        context,
        frame,
        "Embedding metadata and poster",
        0.90,
        None,
        file_size(&temp_output),
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
    for warning in applied
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(text) = warning.as_str() {
            (context.line)(format!("Metadata warning: {text}"));
        }
    }
    let expect_poster = applied
        .get("poster_path")
        .is_some_and(|poster| !poster.is_null());

    state.phase = "lossless output validation";
    (context.line)(
        "Checking every copied stream, packet payload, track flag, chapter, and HDR profile".into(),
    );
    emit(
        context,
        frame,
        "Validating copied tracks and metadata",
        0.97,
        None,
        file_size(&temp_output),
    );
    let mut targets = Map::new();
    let mut expectations = Map::new();
    for stream in &real_videos {
        let track = &video_profiles[&stream.index];
        if let Some(format) = track
            .pointer("/video/target_pixel_format")
            .and_then(Value::as_str)
        {
            targets.insert(stream.index.to_string(), json!(format));
        }
        expectations.insert(
            stream.index.to_string(),
            hdr_expectation(track, verified_hdr10plus.contains(&stream.index)),
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
        "selected_stream_indices": streams.iter().map(|s| s.index).collect::<Vec<_>>(),
        "encode_video_indices": [],
        "video_target_formats": targets,
        "video_hdr_expectations": expectations,
        "verify_copy_hashes": true,
        "test_mode": false,
        "copy_only": true,
        "expect_poster": expect_poster,
        "source_duration": profile.get("duration_seconds").and_then(Value::as_f64).unwrap_or(0.0),
        "source_chapters_count": profile.get("chapters_count").and_then(Value::as_u64).unwrap_or(0),
        "scrubbed_text_values": scrubbed,
        "chapter_text_edits": applied.get("chapter_text_edits").cloned().unwrap_or_else(|| json!([])),
    }))?;
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
            &temp_output,
            work_dir,
            expect_poster,
            &expected_tags,
        )
        .map_err(|error| match error {
            crate::matroska_verify::VerifyError::Mismatch(text)
            | crate::matroska_verify::VerifyError::Operational(text) => text,
        })?;
    }
    if validation
        .get("copied_hashes_verified")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("The output was not accepted because compressed source-stream payload hashes were not verified.".into());
    }

    state.phase = "validated output install";
    let installed = install_export_output(&temp_output, &final_output)?;
    state.temp_output = None;
    let final_bytes = file_size(&final_output);
    state.row.insert("status".into(), json!("METADATA_OK"));
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    state.row.insert("output_bytes".into(), json!(final_bytes));
    state.row.insert(
        "message".into(),
        json!(installed.annotate(
            "Metadata-only MKV validated; every source stream was copied and the original stayed in place."
                .to_string()
        )),
    );
    emit(
        context,
        frame,
        "Validated output ready",
        1.0,
        Some(100.0),
        final_bytes,
    );
    (context.line)(format!(
        "Metadata-only MKV ready: {}",
        final_output
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));
    Ok(())
}

/// Stream-copy every source track with live percent progress. The child owns a
/// process group so an abort can kill it without orphaning helpers.
fn run_mkvmerge(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    output: &Path,
) -> Result<(), String> {
    let source = frame.source;
    let (number, total) = (frame.number, frame.total);
    let mut command = Command::new(&context.tools.mkvmerge);
    command
        .args(["--gui-mode", "--output"])
        .arg(output)
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_owned(&mut command)
        .map_err(|error| format!("Could not start MKVToolNix remux: {error}"))?;
    (context.child)(Some(child.id()));
    let stderr_reader = child.stderr.take().map(|stream| {
        thread::spawn(move || {
            let mut tail: Vec<String> = Vec::new();
            let mut reader = BufReader::new(stream);
            let mut buffer = Vec::new();
            while matches!(reader.read_until(b'\n', &mut buffer), Ok(read) if read > 0) {
                let line = String::from_utf8_lossy(&buffer).into_owned();
                let keep = line.chars().rev().take(2048).collect::<Vec<_>>();
                tail.push(keep.into_iter().rev().collect());
                if tail.len() > 100 {
                    tail.remove(0);
                }
                buffer.clear();
            }
            tail.concat()
        })
    });
    let mut last_percent: i64 = -1;
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut buffer = Vec::new();
        while matches!(reader.read_until(b'\n', &mut buffer), Ok(read) if read > 0) {
            let line = String::from_utf8_lossy(&buffer).into_owned();
            buffer.clear();
            let Some(rest) = line.strip_prefix("#GUI#progress ") else {
                continue;
            };
            let percent = rest
                .split('%')
                .next()
                .and_then(|digits| digits.trim().parse::<i64>().ok())
                .map(|value| value.clamp(0, 100));
            if let Some(percent) = percent.filter(|value| *value != last_percent) {
                last_percent = percent;
                emit(
                    context,
                    frame,
                    "Copying streams",
                    0.85 * percent as f64 / 100.0,
                    Some(percent as f64),
                    file_size(output),
                );
                (context.line)(format!(
                    "Copying streams without re-encoding · file {number}/{total} · {percent}%"
                ));
            }
        }
    }
    let status = child.wait();
    let stderr_tail = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    (context.child)(None);
    let status = status.map_err(|error| format!("Could not wait for MKVToolNix: {error}"))?;
    if !status.success() {
        let detail = stderr_tail.trim();
        let code = status
            .code()
            .map_or_else(|| "terminated".to_string(), |code| code.to_string());
        let tail: String = detail
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!(
            "MKVToolNix could not copy all streams (exit {code}).{}",
            if tail.is_empty() {
                String::new()
            } else {
                format!(" {tail}")
            }
        ));
    }
    if file_size(output).unwrap_or(0) == 0 {
        return Err("MKVToolNix completed without producing a non-empty output file.".into());
    }
    Ok(())
}

// ------------------------------------------------------------ failure cleanup
