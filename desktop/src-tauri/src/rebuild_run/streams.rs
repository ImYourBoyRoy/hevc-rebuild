//! Source stream inventory, primary-video choice, per-file stream selection and plan-profile conversion.

use super::*;

pub(super) fn text_value(value: Option<&Value>) -> Value {
    match value {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => json!(text),
        Some(other) => json!(other.to_string()),
    }
}

pub(super) fn flag(parent: Option<&Value>, key: &str) -> bool {
    parent
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn stream_refs(probe: &Value) -> Vec<StreamRef> {
    let mut ordinals: BTreeMap<String, u32> = BTreeMap::new();
    probe
        .get("streams")
        .and_then(Value::as_array)
        .map(|streams| {
            streams
                .iter()
                .filter_map(|stream| {
                    let codec_type = stream
                        .get("codec_type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let ordinal = *ordinals.get(&codec_type).unwrap_or(&0);
                    ordinals.insert(codec_type.clone(), ordinal + 1);
                    let disposition = stream
                        .get("disposition")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    Some(StreamRef {
                        index: stream.get("index").and_then(Value::as_i64)?,
                        type_ordinal: ordinal,
                        codec_name: stream
                            .get("codec_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        width: stream.get("width").and_then(Value::as_i64).unwrap_or(0),
                        height: stream.get("height").and_then(Value::as_i64).unwrap_or(0),
                        default: disposition
                            .get("default")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            == 1,
                        attached_pic: disposition
                            .get("attached_pic")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            == 1,
                        disposition,
                        codec_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn choose_primary_video(refs: &[StreamRef]) -> Result<(StreamRef, Vec<StreamRef>), String> {
    let videos: Vec<StreamRef> = refs.iter().filter(|s| s.real_video()).cloned().collect();
    if videos.is_empty() {
        return Err("No usable video stream was found.".into());
    }
    let selected = videos
        .iter()
        .max_by_key(|s| (s.pixels(), s.width, s.height, s.default as i64, -s.index))
        .cloned()
        .ok_or("No usable video stream was found.")?;
    let secondary = videos
        .into_iter()
        .filter(|s| s.index != selected.index)
        .collect();
    Ok((selected, secondary))
}

/// Source data streams that the default selection leaves out because Matroska
/// cannot hold them, when the settings asked for data streams to be preserved.
pub(super) fn unstorable_data_streams(refs: &[StreamRef], options: &RebuildRunOptions) -> usize {
    if options.data_streams == "preserve" {
        refs.iter().filter(|s| s.codec_type == "data").count()
    } else {
        0
    }
}

pub(super) fn select_file_streams(
    refs: &[StreamRef],
    options: &RebuildRunOptions,
    effective: &EffectiveSettings,
) -> Result<(Vec<StreamRef>, BTreeSet<i64>), String> {
    let by_index: BTreeMap<i64, StreamRef> = refs.iter().map(|s| (s.index, s.clone())).collect();
    let (selected, explicit) = if let Some(indices) = &effective.stream_indices {
        let mut selected = Vec::new();
        for index in indices {
            let stream = by_index.get(&i64::from(*index)).ok_or_else(|| {
                format!("Selected stream index #{index} is not present in the source.")
            })?;
            if stream.codec_type == "data" {
                return Err(format!(
                    "Stream #{index} is a data stream, which Matroska output cannot store. Deselect it and run again."
                ));
            }
            selected.push(stream.clone());
        }
        (selected, true)
    } else {
        let (highest, secondary) = choose_primary_video(refs)?;
        let mut videos = vec![highest];
        if options.secondary_video != "drop" {
            videos.extend(secondary);
        }
        let mut selected = videos;
        selected.extend(
            refs.iter()
                .filter(|s| matches!(s.codec_type.as_str(), "audio" | "subtitle" | "attachment"))
                .cloned(),
        );
        // Attached-picture video streams are deliberately not selected. FFmpeg 8
        // exposes a Matroska cover attachment that way, but copying it with
        // `-c:v copy` makes the Matroska muxer write a second, regular video
        // track instead of an attachment.
        // Data streams (for example an MP4 timecode track) are never selected:
        // the Matroska muxer cannot store them, so selecting one makes FFmpeg
        // fail after the encode has already started. `unstorable_data_streams`
        // reports what was left out.
        (selected, false)
    };
    let real_videos: Vec<&StreamRef> = selected.iter().filter(|s| s.real_video()).collect();
    if real_videos.is_empty() {
        return Err("Select at least one real video track for this media file.".into());
    }
    let mut ordered = selected;
    ordered.sort_by_key(|stream| {
        let kind_rank = match stream.codec_type.as_str() {
            "video" => 0,
            "audio" => 1,
            "subtitle" => 2,
            "attachment" => 3,
            "data" => 4,
            _ => 5,
        };
        (
            kind_rank,
            stream.attached_pic as i64,
            if stream.codec_type == "video" {
                -stream.pixels()
            } else {
                stream.index
            },
            stream.index,
        )
    });
    let encoded = if explicit || options.secondary_video == "encode" {
        ordered
            .iter()
            .filter(|s| s.real_video())
            .map(|s| s.index)
            .collect()
    } else {
        let primary = ordered
            .iter()
            .filter(|s| s.real_video())
            .max_by_key(|s| (s.pixels(), s.default as i64, -s.index))
            .map(|s| s.index)
            .ok_or("Select at least one real video track for this media file.")?;
        BTreeSet::from([primary])
    };
    Ok((ordered, encoded))
}

pub(super) fn engine_profile_to_plan_profile(
    source: &Path,
    engine_profile: &Value,
    primary: &StreamRef,
    all_refs: &[StreamRef],
) -> Value {
    let secondary: Vec<Value> = all_refs
        .iter()
        .filter(|s| s.real_video() && s.index != primary.index)
        .map(StreamRef::to_plan_value)
        .collect();
    let group = |kind: &str| -> Vec<Value> {
        all_refs
            .iter()
            .filter(|s| s.codec_type == kind)
            .map(StreamRef::to_plan_value)
            .collect()
    };
    let hdr = engine_profile.get("hdr");
    json!({
        "path": source.to_string_lossy(),
        "primary_video": primary.to_plan_value(),
        "secondary_videos": secondary,
        "audio": group("audio"),
        "subtitles": group("subtitle"),
        "attachments": group("attachment"),
        "data": group("data"),
        "source_bit_depth": engine_profile.pointer("/video/source_bit_depth").cloned().unwrap_or(json!(8)),
        "source_chroma": engine_profile.pointer("/video/source_chroma").cloned().unwrap_or(json!("4:2:0")),
        "target_pix_fmt": engine_profile.pointer("/video/target_pixel_format").cloned().unwrap_or(Value::Null),
        "color_primaries": engine_profile.get("color_primaries").cloned().unwrap_or(Value::Null),
        "transfer": engine_profile.get("color_transfer").cloned().unwrap_or(Value::Null),
        "color_matrix": engine_profile.get("color_matrix").cloned().unwrap_or(Value::Null),
        "color_range_x265": engine_profile.get("color_range_x265").cloned().unwrap_or(Value::Null),
        "color_range_ffmpeg": engine_profile.get("color_range_ffmpeg").cloned().unwrap_or(Value::Null),
        "chroma_location": engine_profile.get("chroma_location").cloned().unwrap_or(Value::Null),
        "master_display": engine_profile.get("mastering_display").cloned().unwrap_or(Value::Null),
        "max_cll": engine_profile.get("max_cll").cloned().unwrap_or(Value::Null),
        "max_fall": engine_profile.get("max_fall").cloned().unwrap_or(Value::Null),
        "hdr10": flag(hdr, "hdr10"),
        "hdr10plus": flag(hdr, "hdr10plus"),
        "hlg": flag(hdr, "hlg"),
        "dolby_vision": flag(hdr, "dolby_vision"),
        "interlaced": engine_profile.get("interlaced").and_then(Value::as_bool).unwrap_or(false),
        "format_level": engine_profile.get("format_level").cloned().unwrap_or(Value::Null),
    })
}

pub(super) fn track_profile_to_plan(
    source: &Path,
    track: &Value,
    stream: &StreamRef,
    refs: &[StreamRef],
) -> Value {
    let mut plan = engine_profile_to_plan_profile(source, track, stream, refs);
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "hdr10plus".into(),
            json!(flag(track.get("hdr"), "hdr10plus")),
        );
    }
    plan
}

pub(super) fn hdr_expectation(
    profile: &Value,
    hdr10plus: bool,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
) -> Value {
    let hdr = profile.get("hdr");
    let hdr_flag_or_profile =
        |key: &str| flag(hdr, key) || profile.get(key).and_then(Value::as_bool).unwrap_or(false);
    json!({
        "color_primaries": text_value(profile.get("color_primaries")),
        "transfer": text_value(profile.get("color_transfer").or_else(|| profile.get("transfer"))),
        "color_matrix": text_value(profile.get("color_matrix")),
        "master_display": text_value(profile.get("mastering_display").or_else(|| profile.get("master_display"))),
        "max_cll": profile.get("max_cll").and_then(Value::as_i64),
        "max_fall": profile.get("max_fall").and_then(Value::as_i64),
        "hdr10": hdr_flag_or_profile("hdr10"),
        "hdr10plus": hdr10plus,
        "hlg": hdr_flag_or_profile("hlg"),
        "dolby_vision": hdr_flag_or_profile("dolby_vision"),
        "dovi_profile": hdr.and_then(|h| h.get("dolby_vision_profile")).and_then(Value::as_i64)
            .or_else(|| profile.get("dovi_profile").and_then(Value::as_i64)),
        "dovi_rpu_present": flag(hdr, "dolby_vision_rpu")
            || profile.get("dovi_rpu_present").and_then(Value::as_bool).unwrap_or(false),
        "dovi_el_present": flag(hdr, "dolby_vision_enhancement_layer")
            || profile.get("dovi_el_present").and_then(Value::as_bool).unwrap_or(false),
        "dovi_compatibility_id": hdr
            .and_then(|h| h.get("dolby_vision_compatibility_id"))
            .and_then(Value::as_i64),
        "preserve_dolbyvision": preserve_dolbyvision,
        "strip_dolbyvision": strip_dolbyvision,
    })
}

/// The source's attached-picture stream (an MP4 cover, or a Matroska cover that
/// FFmpeg exposes as one) and the image type to extract it as.
pub(super) fn source_cover_stream(refs: &[StreamRef]) -> Value {
    refs.iter()
        .find(|stream| stream.codec_type == "video" && stream.attached_pic)
        .and_then(|stream| {
            let extension = match stream.codec_name.to_ascii_lowercase().as_str() {
                "mjpeg" | "jpeg" => "jpg",
                "png" => "png",
                "webp" => "webp",
                _ => return None,
            };
            Some(json!({"index": stream.index, "extension": extension}))
        })
        .unwrap_or(Value::Null)
}
