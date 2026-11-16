//! Builds the per-video source profile from FFprobe and MediaInfo data.

use super::*;

#[derive(serde::Serialize, Deserialize)]
pub(super) struct Profile {
    pub(super) payload: Value,
    pub(super) duration: f64,
}

pub(super) fn build_profile(
    probe: &Value,
    mediainfo: &Value,
    path: &Path,
    settings: &Value,
    ffprobe: &str,
) -> Result<Profile, String> {
    build_profile_for_video(probe, mediainfo, path, settings, ffprobe, None)
}

pub(super) fn build_profile_for_video(
    probe: &Value,
    mediainfo: &Value,
    path: &Path,
    settings: &Value,
    ffprobe: &str,
    preferred_video_ordinal: Option<usize>,
) -> Result<Profile, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or("FFprobe returned no stream list.")?;
    let mut ordinals = BTreeMap::<String, usize>::new();
    let refs = streams
        .iter()
        .map(|stream| {
            let kind = stream
                .get("codec_type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_lowercase();
            let ordinal = *ordinals
                .entry(kind.clone())
                .and_modify(|value| *value += 1)
                .or_insert(0);
            (stream, kind, ordinal)
        })
        .collect::<Vec<_>>();
    let is_usable_video = |stream: &Value, kind: &str| {
        kind == "video"
            && !disposition(stream, "attached_pic")
            && value_i64(stream.get("width")).unwrap_or(0) > 0
            && value_i64(stream.get("height")).unwrap_or(0) > 0
    };
    let primary = if let Some(video_ordinal) = preferred_video_ordinal {
        refs.iter().find(|(stream, kind, ordinal)| {
            *ordinal == video_ordinal && is_usable_video(stream, kind)
        })
    } else {
        refs.iter()
            .filter(|(stream, kind, _)| is_usable_video(stream, kind))
            .max_by_key(|(stream, _, _)| {
                let width = value_i64(stream.get("width")).unwrap_or(0);
                let height = value_i64(stream.get("height")).unwrap_or(0);
                let default = disposition(stream, "default") as i64;
                (
                    width.saturating_mul(height),
                    width,
                    height,
                    default,
                    source_depth(stream),
                    -value_i64(stream.get("index")).unwrap_or(i64::MAX),
                )
            })
    }
    .ok_or_else(|| {
        preferred_video_ordinal.map_or_else(
            || "No usable video stream was found.".to_string(),
            |ordinal| format!("No usable video stream exists at output video ordinal {ordinal}."),
        )
    })?;
    let primary_raw = primary.0;
    let primary_ordinal = primary.2;
    let primary_index = value_i64(primary_raw.get("index"))
        .ok_or("FFprobe did not return a video stream index.")?;
    let width = value_i64(primary_raw.get("width")).unwrap_or(0);
    let height = value_i64(primary_raw.get("height")).unwrap_or(0);
    let media_tracks = mediainfo
        .pointer("/media/track")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let media_primary =
        match_mediainfo_video(&media_tracks, primary_raw, primary_ordinal, width, height);
    let no_mediainfo = Value::Null;
    let media_primary = media_primary.unwrap_or(&no_mediainfo);
    let duration = value_f64(probe.pointer("/format/duration"))
        .or_else(|| value_f64(primary_raw.get("duration")))
        .or_else(|| media_primary.get("Duration").and_then(value_f64_ref))
        .filter(|duration| *duration > 0.0)
        .ok_or_else(|| format!("Unable to determine media duration for {}.", path.display()))?;
    let depth = media_primary
        .get("BitDepth")
        .and_then(parse_numeric_string)
        .filter(|value| matches!(value, 8 | 10 | 12 | 14 | 16))
        .unwrap_or_else(|| source_depth(primary_raw));
    let chroma = media_primary
        .get("ChromaSubsampling")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            chroma_from_pixel_format(
                primary_raw
                    .get("pix_fmt")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
        });
    let promoted = settings
        .pointer("/video/promote_8bit_to_10bit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target_pix_fmt = target_pixel_format(depth, &chroma, promoted)?;

    let stream_side_data = primary_raw
        .get("side_data_list")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let frame = frame_probe(ffprobe, path, primary_ordinal);
    let frame_first = frame
        .as_ref()
        .and_then(|value| value.pointer("/frames/0"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let frame_side_data = frame_side_data(frame.as_ref());
    let mut side_data = stream_side_data;
    side_data.extend(frame_side_data);

    let primaries = normalize_primaries(
        first_string(primary_raw, "color_primaries")
            .or_else(|| first_string(&frame_first, "color_primaries"))
            .or_else(|| {
                media_primary
                    .get("colour_primaries")
                    .and_then(Value::as_str)
            }),
    );
    let transfer = normalize_transfer(
        first_string(primary_raw, "color_transfer")
            .or_else(|| first_string(&frame_first, "color_transfer"))
            .or_else(|| {
                media_primary
                    .get("transfer_characteristics")
                    .and_then(Value::as_str)
            }),
    );
    let matrix = normalize_matrix(
        first_string(primary_raw, "color_space")
            .or_else(|| first_string(&frame_first, "color_space"))
            .or_else(|| {
                media_primary
                    .get("matrix_coefficients")
                    .and_then(Value::as_str)
            }),
    );
    let range = first_string(primary_raw, "color_range")
        .or_else(|| first_string(&frame_first, "color_range"))
        .or_else(|| media_primary.get("colour_range").and_then(Value::as_str))
        .map(str::to_lowercase);
    let (range_x265, range_ffmpeg) = match range.as_deref() {
        Some("tv" | "limited") => (Some("limited"), Some("tv")),
        Some("pc" | "full") => (Some("full"), Some("pc")),
        _ => (None, None),
    };
    let mastering_side = side_data
        .iter()
        .find(|value| side_kind(value).contains("mastering display metadata"));
    let (mastering_display, mastering_source) =
        if let Some(value) = mastering_side.and_then(mastering_display_from_side) {
            (Some(value), Some("ffprobe-side-data"))
        } else if let Some(value) = mastering_display_from_mediainfo(media_primary) {
            (Some(value), Some("MediaInfo-label-fallback"))
        } else {
            (None, None)
        };
    let light_side = side_data
        .iter()
        .find(|value| side_kind(value).contains("content light level metadata"));
    let max_cll = light_side
        .and_then(|value| value.get("max_content").and_then(rounded_integer))
        .or_else(|| media_primary.get("MaxCLL").and_then(rounded_integer));
    let max_fall = light_side
        .and_then(|value| value.get("max_average").and_then(rounded_integer))
        .or_else(|| media_primary.get("MaxFALL").and_then(rounded_integer));
    let side_text = side_data
        .iter()
        .map(side_kind)
        .collect::<Vec<_>>()
        .join(" ");
    let hdr_text = [
        "HDR_Format",
        "HDR_Format_Profile",
        "HDR_Format_Compatibility",
        "HDR_Format_Settings",
    ]
    .iter()
    .filter_map(|key| media_primary.get(*key).and_then(Value::as_str))
    .collect::<Vec<_>>()
    .join(" ");
    let hdr10plus_hint = contains_any(
        &format!("{hdr_text} {side_text}"),
        &["hdr10+", "smpte2094-40", "hdr dynamic metadata"],
    );
    let dovi_side = side_data.iter().find(|value| {
        let kind = side_kind(value);
        kind.contains("dovi configuration record") || kind.contains("dolby vision configuration")
    });
    let (dovi_profile, dovi_level, dovi_rpu, dovi_el, dovi_bl, dovi_compat, dovi_source) =
        if let Some(side) = dovi_side {
            (
                value_i64(side.get("dv_profile")),
                value_i64(side.get("dv_level")),
                value_i64(side.get("rpu_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("el_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("bl_present_flag")).unwrap_or(0) != 0,
                value_i64(side.get("dv_bl_signal_compatibility_id")),
                Some("FFprobe DOVI configuration record"),
            )
        } else {
            let dovi_text = format!(
                "{hdr_text} {}",
                media_primary
                    .get("CodecID")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            if contains_any(&dovi_text, &["dolby vision", "dvhe.", "dvh1."]) {
                (
                    dovi_profile_from_text(&dovi_text),
                    dovi_level_from_text(&dovi_text),
                    false,
                    false,
                    false,
                    None,
                    Some("MediaInfo Dolby Vision metadata"),
                )
            } else {
                (None, None, false, false, false, None, None)
            }
        };
    let dolby_vision = dovi_source.is_some()
        || contains_any(
            &format!("{hdr_text} {side_text}"),
            &["dolby vision", "dovi configuration"],
        );
    let hdr10 = (transfer.as_deref() == Some("smpte2084")
        && primaries.as_deref() == Some("bt2020"))
        || hdr_text.to_lowercase().contains("hdr10");
    let hlg =
        transfer.as_deref() == Some("arib-std-b67") || hdr_text.to_lowercase().contains("hlg");
    let field_order = first_string(primary_raw, "field_order")
        .unwrap_or_default()
        .to_lowercase();
    let scan_type = media_primary
        .get("ScanType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let interlaced = scan_type.to_lowercase().contains("interlaced")
        || (!field_order.is_empty() && field_order != "progressive" && field_order != "unknown")
        || frames_are_interlaced(frame.as_ref());

    let mut tracks = Vec::with_capacity(refs.len());
    let mut secondary_videos = 0;
    let mut audio_count = 0;
    let mut subtitle_count = 0;
    let mut attachment_count = 0;
    let mut data_count = 0;
    for (stream, kind, ordinal) in &refs {
        let index = value_i64(stream.get("index")).unwrap_or(-1);
        if kind == "video"
            && !disposition(stream, "attached_pic")
            && value_i64(stream.get("width")).unwrap_or(0) > 0
            && value_i64(stream.get("height")).unwrap_or(0) > 0
            && index != primary_index
        {
            secondary_videos += 1;
        }
        match kind.as_str() {
            "audio" => audio_count += 1,
            "subtitle" => subtitle_count += 1,
            "attachment" => attachment_count += 1,
            "data" => data_count += 1,
            _ => {}
        }
        let tags = stream.get("tags").cloned().unwrap_or_else(|| json!({}));
        let mi_track = match_mediainfo_track(&media_tracks, kind, index, *ordinal);
        let mi_fields = mi_track
            .map(select_mediainfo_fields)
            .unwrap_or_else(|| json!({}));
        let raw = stream.as_object().cloned().unwrap_or_default();
        tracks.push(json!({
            "index": index,
            "type": kind,
            "ordinal": ordinal,
            "codec": stream.get("codec_name").cloned().unwrap_or(Value::Null),
            "profile": stream.get("profile").cloned().unwrap_or(Value::Null),
            "width": stream.get("width").cloned().unwrap_or(Value::Null),
            "height": stream.get("height").cloned().unwrap_or(Value::Null),
            "pixel_format": stream.get("pix_fmt").cloned().unwrap_or(Value::Null),
            "bit_depth": stream.get("bits_per_raw_sample").filter(|value| !value.is_null() && value.as_str() != Some("N/A"))
                .cloned().or_else(|| stream.get("bits_per_sample").cloned())
                .unwrap_or_else(|| if kind == "video" { json!(source_depth(stream)) } else { Value::Null }),
            "color_primaries": stream.get("color_primaries").cloned().unwrap_or(Value::Null),
            "color_transfer": stream.get("color_transfer").cloned().unwrap_or(Value::Null),
            "color_matrix": stream.get("color_space").cloned().unwrap_or(Value::Null),
            "frame_rate": stream.get("avg_frame_rate").cloned().unwrap_or(Value::Null),
            "channels": stream.get("channels").cloned().unwrap_or(Value::Null),
            "channel_layout": stream.get("channel_layout").cloned().unwrap_or(Value::Null),
            "sample_rate": stream.get("sample_rate").cloned().unwrap_or(Value::Null),
            "bit_rate": stream.get("bit_rate").cloned().unwrap_or(Value::Null),
            "language": tags.get("language").cloned().unwrap_or(Value::Null),
            "title": tags.get("title").cloned().unwrap_or(Value::Null),
            "default": disposition(stream, "default"),
            "forced": disposition(stream, "forced"),
            "attached_picture": disposition(stream, "attached_pic"),
            "filename": tags.get("filename").cloned().unwrap_or(Value::Null),
            "mime_type": tags.get("mimetype").cloned().unwrap_or(Value::Null),
            "mediainfo": mi_fields,
            "ffprobe": raw
        }));
    }
    let mut general = Map::new();
    if let Some(object) = media_tracks
        .iter()
        .find(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("general"))
        })
        .and_then(Value::as_object)
    {
        for (key, value) in object {
            if ![
                "@type",
                "CompleteName",
                "FolderName",
                "FileName",
                "FileExtension",
                "FileSize",
                "Duration",
            ]
            .contains(&key.as_str())
            {
                general.insert(key.clone(), value.clone());
            }
        }
    }
    let content_profile = crate::content::profile_content(mediainfo, settings);
    let primary_stream_index = primary_index;
    let payload = json!({
        "video": {
            "codec": primary_raw.get("codec_name").cloned().unwrap_or(Value::Null),
            "width": width,
            "height": height,
            "source_bit_depth": depth,
            "source_chroma": chroma,
            "target_pixel_format": target_pix_fmt,
            "secondary_count": secondary_videos
        },
        "hdr": {
            "hdr10": hdr10,
            "hdr10plus": false,
            "hdr10plus_hint": hdr10plus_hint,
            "hlg": hlg,
            "dolby_vision": dolby_vision,
            "dolby_vision_profile": dovi_profile,
            "dolby_vision_level": dovi_level,
            "dolby_vision_rpu": dovi_rpu,
            "dolby_vision_enhancement_layer": dovi_el,
            "dolby_vision_base_layer": dovi_bl,
            "dolby_vision_compatibility_id": dovi_compat,
            "dolby_vision_evidence": dovi_source
        },
        "audio_count": audio_count,
        "subtitle_count": subtitle_count,
        "attachment_count": attachment_count,
        "data_stream_count": data_count,
        "tracks": tracks,
        "mediainfo_general": general,
        "content": content_profile,
        "chapters_count": probe.get("chapters").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "duration_seconds": duration,
        "interlaced": interlaced,
        "color_primaries": primaries,
        "color_transfer": transfer,
        "color_matrix": matrix,
        "color_range_x265": range_x265,
        "color_range_ffmpeg": range_ffmpeg,
        "mastering_display": mastering_display,
        "mastering_display_source": mastering_source,
        "max_cll": max_cll,
        "max_fall": max_fall,
        "chroma_location": chroma_location(
            first_string(primary_raw, "chroma_location"),
            media_primary.get("ChromaSubsampling_Position").and_then(Value::as_str),
        ),
        "format_level": media_primary.get("Format_Level").cloned().unwrap_or(Value::Null),
        "primary_stream_index": primary_stream_index,
        "mediainfo_available": mediainfo.pointer("/media/track").is_some(),
        "frame_metadata_available": frame.is_some(),
        "source_metadata": {
            "format": probe.get("format").cloned().unwrap_or(Value::Null),
            "chapters": probe.get("chapters").cloned().unwrap_or_else(|| json!([]))
        }
    });
    Ok(Profile { payload, duration })
}
