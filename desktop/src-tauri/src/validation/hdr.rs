//! HDR video profile checks on the output: color description, mastering display, light level and dynamic metadata.

use super::*;

pub(super) fn validate_hdr_video_profiles(
    request: &Request,
    source_videos: &[StreamRef],
    output_videos: &[StreamRef],
    output_probe: &Value,
    encoded: &BTreeSet<i64>,
) -> Result<(), String> {
    let mut tracks = Vec::new();
    for (source, output) in source_videos.iter().zip(output_videos) {
        if !encoded.contains(&source.index) && !request.copy_only {
            continue;
        }
        let Some(expected) = request
            .video_hdr_expectations
            .get(&source.index.to_string())
        else {
            return Err(format!(
                "No source HDR profile was retained for encoded video stream #{}.",
                source.index
            ));
        };
        if expected.preserve_dolbyvision && expected.strip_dolbyvision {
            return Err(format!(
                "Video stream #{} cannot preserve and strip Dolby Vision simultaneously.",
                source.index
            ));
        }
        tracks.push((source.index, output.type_ordinal, expected));
    }
    if tracks.is_empty() {
        return Ok(());
    }
    let output_ordinals = tracks
        .iter()
        .map(|(_, ordinal, _)| *ordinal)
        .collect::<Vec<_>>();
    let profiles = media_scan::output_video_profiles(
        &request.ffprobe,
        request.mediainfo.as_deref(),
        &request.output_path,
        output_probe,
        &output_ordinals,
    )?;
    for (source_index, output_ordinal, expected) in tracks {
        let profile = profiles.get(&output_ordinal).ok_or_else(|| {
            format!("Output HDR profile for video ordinal {output_ordinal} is missing.")
        })?;
        validate_hdr_video_profile(request, source_index, output_ordinal, expected, profile)?;
    }
    Ok(())
}

pub(super) fn validate_hdr_video_profile(
    request: &Request,
    source_index: i64,
    output_ordinal: usize,
    expected: &HdrTrackExpectation,
    actual: &Value,
) -> Result<(), String> {
    for (label, expected_value, actual_value) in [
        (
            "color primaries",
            expected.color_primaries.as_deref(),
            actual.get("color_primaries").and_then(Value::as_str),
        ),
        (
            "transfer characteristic",
            expected.transfer.as_deref(),
            actual.get("color_transfer").and_then(Value::as_str),
        ),
        (
            "color matrix",
            expected.color_matrix.as_deref(),
            actual.get("color_matrix").and_then(Value::as_str),
        ),
    ] {
        if let (Some(before), Some(after)) = (expected_value, actual_value) {
            if before != after {
                return Err(format!(
                    "Video stream #{source_index} {label} changed: expected {before}, output {after}."
                ));
            }
        }
        if request.copy_only && expected_value.is_some() && actual_value != expected_value {
            return Err(format!(
                "Copied video stream #{source_index} {label} is missing or changed: expected {:?}, output {:?}.",
                expected_value, actual_value
            ));
        }
    }
    let hdr = actual
        .get("hdr")
        .ok_or_else(|| format!("Video stream #{source_index} output HDR profile is missing."))?;
    if request.copy_only {
        for (label, expected_value, key) in [
            ("HDR10", expected.hdr10, "hdr10"),
            ("HDR10+", expected.hdr10plus, "hdr10plus"),
            ("HLG", expected.hlg, "hlg"),
            ("Dolby Vision", expected.dolby_vision, "dolby_vision"),
        ] {
            if hdr.get(key).and_then(Value::as_bool) != Some(expected_value) {
                return Err(format!(
                    "Copied video stream #{source_index} {label} state changed from {expected_value}."
                ));
            }
        }
        for (label, expected_value, key) in [
            (
                "Dolby Vision profile",
                expected.dovi_profile,
                "dolby_vision_profile",
            ),
            (
                "Dolby Vision compatibility ID",
                expected.dovi_compatibility_id,
                "dolby_vision_compatibility_id",
            ),
        ] {
            if expected_value.is_some() && hdr.get(key).and_then(integerish) != expected_value {
                return Err(format!(
                    "Copied video stream #{source_index} {label} changed from {:?}.",
                    expected_value
                ));
            }
        }
        for (label, expected_value, key) in [
            (
                "Dolby Vision RPU",
                expected.dovi_rpu_present,
                "dolby_vision_rpu",
            ),
            (
                "Dolby Vision enhancement layer",
                expected.dovi_el_present,
                "dolby_vision_enhancement_layer",
            ),
        ] {
            if hdr.get(key).and_then(Value::as_bool) != Some(expected_value) {
                return Err(format!(
                    "Copied video stream #{source_index} {label} state changed from {expected_value}."
                ));
            }
        }
    }
    if expected.hdr10 && hdr.get("hdr10").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "Video stream #{source_index} HDR10 input no longer identifies as HDR10 after encode."
        ));
    }
    if expected.hlg && hdr.get("hlg").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "Video stream #{source_index} HLG input no longer identifies as HLG after encode."
        ));
    }
    let actual_master_display = actual.get("mastering_display").and_then(Value::as_str);
    if expected.master_display.is_some() && actual_master_display.is_none() {
        return Err(format!(
            "Video stream #{source_index} carried mastering-display metadata but output does not."
        ));
    }
    if let (Some(before), Some(after)) = (expected.master_display.as_deref(), actual_master_display)
    {
        if before != after {
            return Err(format!(
                "Video stream #{source_index} mastering-display metadata changed: input={before:?} output={after:?}."
            ));
        }
    }
    for (label, before, key) in [
        ("MaxCLL", expected.max_cll, "max_cll"),
        ("MaxFALL", expected.max_fall, "max_fall"),
    ] {
        if let Some(before) = before {
            let after = actual.get(key).and_then(integerish);
            if after != Some(before) {
                return Err(format!(
                    "Video stream #{source_index} {label} changed: input={before} output={after:?}."
                ));
            }
        }
    }
    if expected.hdr10plus {
        let parser = request.hdr10plus_tool.as_deref().ok_or_else(|| {
            format!(
                "HDR10+ preservation was requested for video stream #{source_index}, but hdr10plus_tool is unavailable."
            )
        })?;
        if !crate::hdr_metadata::verify_hdr10plus_output(
            &request.ffmpeg,
            parser,
            &request.output_path,
            output_ordinal,
        )? {
            return Err(format!(
                "HDR10+ source on video stream #{source_index} was encoded but dynamic HDR10+ metadata was not verified in the output."
            ));
        }
    }
    if expected.preserve_dolbyvision {
        if !expected.dolby_vision || !expected.dovi_rpu_present || expected.dovi_el_present {
            return Err(format!(
                "Dolby Vision preservation was requested for video stream #{source_index} without a verified single-layer source RPU."
            ));
        }
        if hdr.get("dolby_vision").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "Dolby Vision preservation was requested for video stream #{source_index}, but output no longer identifies as Dolby Vision."
            ));
        }
        let actual_profile = hdr.get("dolby_vision_profile").and_then(integerish);
        if actual_profile != expected.dovi_profile {
            return Err(format!(
                "Dolby Vision profile changed on video stream #{source_index}: input={:?} output={actual_profile:?}.",
                expected.dovi_profile
            ));
        }
        if hdr.get("dolby_vision_rpu").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "Dolby Vision output on video stream #{source_index} does not report an RPU; refusing metadata loss."
            ));
        }
        if hdr
            .get("dolby_vision_enhancement_layer")
            .and_then(Value::as_bool)
            != Some(false)
        {
            return Err(format!(
                "Dolby Vision output on video stream #{source_index} unexpectedly reports an enhancement layer."
            ));
        }
        if let (Some(before), Some(after)) = (
            expected.dovi_compatibility_id,
            hdr.get("dolby_vision_compatibility_id")
                .and_then(integerish),
        ) {
            if before != after {
                return Err(format!(
                    "Dolby Vision base-layer compatibility ID changed on video stream #{source_index}: input={before} output={after}."
                ));
            }
        }
    }
    if expected.strip_dolbyvision {
        if !expected.dolby_vision || !expected.hdr10 {
            return Err(format!(
                "Dolby Vision HDR10 fallback was requested on video stream #{source_index} without verified Dolby Vision and HDR10-compatible input."
            ));
        }
        if hdr.get("hdr10").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "Dolby Vision HDR10 fallback output on video stream #{source_index} no longer identifies as HDR10."
            ));
        }
        if hdr.get("dolby_vision").and_then(Value::as_bool) == Some(true) {
            return Err(format!(
                "Dolby Vision HDR10 fallback on video stream #{source_index} still carries Dolby Vision signaling."
            ));
        }
    }
    Ok(())
}
