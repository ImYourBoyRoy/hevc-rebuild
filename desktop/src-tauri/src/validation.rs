use crate::{media_scan, packet_verify};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STREAMS: usize = 1024;
const MIN_OUTPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    ffmpeg: String,
    ffprobe: String,
    #[serde(default)]
    mediainfo: Option<String>,
    #[serde(default)]
    hdr10plus_tool: Option<String>,
    source_path: PathBuf,
    output_path: PathBuf,
    source_probe: Value,
    selected_stream_indices: Vec<i64>,
    #[serde(default)]
    encode_video_indices: Vec<i64>,
    #[serde(default)]
    video_target_formats: BTreeMap<String, String>,
    #[serde(default)]
    video_hdr_expectations: BTreeMap<String, HdrTrackExpectation>,
    #[serde(default)]
    verify_copy_hashes: bool,
    #[serde(default)]
    test_mode: bool,
    #[serde(default)]
    copy_only: bool,
    #[serde(default)]
    expect_poster: bool,
    source_duration: f64,
    source_chapters_count: usize,
    #[serde(default)]
    scrubbed_text_values: Vec<Value>,
    #[serde(default)]
    chapter_text_edits: Vec<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HdrTrackExpectation {
    color_primaries: Option<String>,
    transfer: Option<String>,
    color_matrix: Option<String>,
    master_display: Option<String>,
    max_cll: Option<i64>,
    max_fall: Option<i64>,
    #[serde(default)]
    hdr10: bool,
    #[serde(default)]
    hdr10plus: bool,
    #[serde(default)]
    hlg: bool,
    #[serde(default)]
    dolby_vision: bool,
    dovi_profile: Option<i64>,
    #[serde(default)]
    dovi_rpu_present: bool,
    #[serde(default)]
    dovi_el_present: bool,
    dovi_compatibility_id: Option<i64>,
    #[serde(default)]
    preserve_dolbyvision: bool,
    #[serde(default)]
    strip_dolbyvision: bool,
}

#[derive(Debug, Clone)]
struct StreamRef {
    index: i64,
    type_ordinal: usize,
    codec_type: String,
    codec_name: Option<String>,
    width: i64,
    height: i64,
    pix_fmt: Option<String>,
    default: bool,
    attached_pic: bool,
    tags: Map<String, Value>,
    disposition: Map<String, Value>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(validate_request) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize Rust stream-validation result: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<Request, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read Rust stream-validation request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("Rust stream-validation request exceeds the 16 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Rust stream-validation request JSON is invalid: {error}"))
}

fn validate_request(request: Request) -> Result<Value, String> {
    validate_paths(&request)?;
    if !request.source_duration.is_finite() || request.source_duration < 0.0 {
        return Err("Source duration must be a finite non-negative value.".into());
    }
    if request.selected_stream_indices.len() > MAX_STREAMS {
        return Err(format!(
            "Output validation is limited to {MAX_STREAMS} selected streams."
        ));
    }
    let source_refs = stream_refs(&request.source_probe)?;
    let source_by_index: HashMap<_, _> = source_refs
        .iter()
        .map(|stream| (stream.index, stream.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut selected = Vec::with_capacity(request.selected_stream_indices.len());
    for index in &request.selected_stream_indices {
        if !seen.insert(*index) {
            return Err(format!(
                "Selected stream index #{index} occurs more than once."
            ));
        }
        selected.push(source_by_index.get(index).cloned().ok_or_else(|| {
            format!("Selected source stream #{index} is missing from FFprobe data.")
        })?);
    }
    let encoded: BTreeSet<_> = request.encode_video_indices.iter().copied().collect();
    if encoded.len() != request.encode_video_indices.len() {
        return Err("Encoded video stream indices must be unique.".into());
    }
    if encoded.iter().any(|index| {
        source_by_index
            .get(index)
            .is_none_or(|stream| stream.codec_type != "video" || stream.attached_pic)
    }) {
        return Err(
            "An encoded video stream index is absent or is not a real video stream.".into(),
        );
    }
    if encoded.iter().any(|index| !seen.contains(index)) {
        return Err("Every encoded video stream must also be selected for output.".into());
    }

    let output_probe = media_scan::probe_output(&request.ffprobe, &request.output_path)?;
    let output_refs = stream_refs(&output_probe)?;
    validate_streams(&request, &selected, &output_refs, &output_probe, &encoded)?;

    Ok(json!({
        "output_probe": output_probe,
        "copied_hashes_verified": request.verify_copy_hashes && !request.test_mode,
        "hdr_validation_complete": true,
    }))
}

fn validate_paths(request: &Request) -> Result<(), String> {
    if request.ffmpeg.trim().is_empty() || request.ffmpeg.len() > 8192 {
        return Err("FFmpeg path is empty or exceeds the supported limit.".into());
    }
    if request.ffprobe.trim().is_empty() || request.ffprobe.len() > 8192 {
        return Err("FFprobe path is empty or exceeds the supported limit.".into());
    }
    if request.source_path.as_os_str().len() > 32 * 1024
        || request.output_path.as_os_str().len() > 32 * 1024
    {
        return Err("Media paths exceed the supported validation limit.".into());
    }
    if !request.source_path.is_file() {
        return Err(format!(
            "Validation source is not a readable file: {}",
            request.source_path.display()
        ));
    }
    let metadata = request.output_path.metadata().map_err(|_| {
        format!(
            "Output is missing or implausibly small: {}",
            request.output_path.display()
        )
    })?;
    let minimum = if request.copy_only {
        1
    } else {
        MIN_OUTPUT_BYTES
    };
    if !metadata.is_file() || metadata.len() < minimum {
        return Err(format!(
            "Output is missing or implausibly small: {}",
            request.output_path.display()
        ));
    }
    Ok(())
}

fn stream_refs(probe: &Value) -> Result<Vec<StreamRef>, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| "FFprobe result has no stream list.".to_string())?;
    if streams.len() > MAX_STREAMS {
        return Err(format!("FFprobe returned more than {MAX_STREAMS} streams."));
    }
    let mut ordinals: BTreeMap<String, usize> = BTreeMap::new();
    let mut result = Vec::with_capacity(streams.len());
    let mut indexes = BTreeSet::new();
    for raw in streams {
        let codec_type = raw
            .get("codec_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let ordinal = *ordinals.get(&codec_type).unwrap_or(&0);
        ordinals.insert(codec_type.clone(), ordinal + 1);
        let index = raw
            .get("index")
            .and_then(Value::as_i64)
            .ok_or_else(|| "FFprobe returned a stream without an integer index.".to_string())?;
        if index < 0 || !indexes.insert(index) {
            return Err("FFprobe returned a negative or duplicate stream index.".into());
        }
        let disposition = raw
            .get("disposition")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let tags = raw
            .get("tags")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        result.push(StreamRef {
            index,
            type_ordinal: ordinal,
            codec_type,
            codec_name: raw
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            width: raw.get("width").and_then(Value::as_i64).unwrap_or(0),
            height: raw.get("height").and_then(Value::as_i64).unwrap_or(0),
            pix_fmt: raw
                .get("pix_fmt")
                .and_then(Value::as_str)
                .map(str::to_string),
            default: flag_value(disposition.get("default")),
            attached_pic: flag_value(disposition.get("attached_pic")),
            tags,
            disposition,
        });
    }
    Ok(result)
}

fn flag_value(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_i64() == Some(1)
            || value.as_u64() == Some(1)
            || value.as_str().is_some_and(|text| text == "1")
    })
}

fn validate_streams(
    request: &Request,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    output_probe: &Value,
    encoded: &BTreeSet<i64>,
) -> Result<(), String> {
    let selected_videos: Vec<_> = selected
        .iter()
        .filter(|stream| stream.codec_type == "video")
        .cloned()
        .collect();
    let mut real_videos: Vec<_> = selected_videos
        .iter()
        .filter(|stream| !stream.attached_pic && stream.width > 0 && stream.height > 0)
        .cloned()
        .collect();
    if !request.copy_only {
        real_videos.sort_by_key(|stream| {
            (
                Reverse(stream.width.saturating_mul(stream.height)),
                Reverse(stream.default),
                stream.index,
            )
        });
    }
    let mut selected_pictures: Vec<_> = selected_videos
        .iter()
        .filter(|stream| stream.attached_pic)
        .cloned()
        .collect();
    let out_videos: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == "video" && !stream.attached_pic)
        .cloned()
        .collect();
    let mut out_pictures: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == "video" && stream.attached_pic)
        .cloned()
        .collect();
    if request.expect_poster {
        selected_pictures.retain(|stream| !is_replaceable_cover(stream));
        out_pictures.retain(|stream| !is_replaceable_cover(stream));
    }
    if out_videos.is_empty() {
        return Err("Output contains no usable video stream.".into());
    }
    if out_videos.len() != real_videos.len() {
        return Err(format!(
            "Selected real video stream count changed: expected {} output {}",
            real_videos.len(),
            out_videos.len()
        ));
    }
    if out_pictures.len() != selected_pictures.len() {
        return Err(format!(
            "Selected attached-picture count changed: expected {} output {}",
            selected_pictures.len(),
            out_pictures.len()
        ));
    }
    if !request.copy_only && !out_videos[0].default {
        return Err(
            "Selected highest-resolution video is not marked as the output default video track."
                .into(),
        );
    }
    for (source, output) in real_videos.iter().zip(&out_videos) {
        if encoded.contains(&source.index) {
            let target = request
                .video_target_formats
                .get(&source.index.to_string())
                .ok_or_else(|| {
                    format!(
                        "No source profile was retained for selected video stream #{}.",
                        source.index
                    )
                })?;
            if !matches!(
                output
                    .codec_name
                    .as_deref()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("hevc" | "h265")
            ) {
                return Err(format!(
                    "Selected video stream #{} output codec is {:?}; expected HEVC.",
                    source.index, output.codec_name
                ));
            }
            if output.pix_fmt.as_deref().unwrap_or("") != target {
                return Err(format!(
                    "Video stream #{} pixel format {:?} != planned {target:?}.",
                    source.index, output.pix_fmt
                ));
            }
        } else if output.codec_name != source.codec_name {
            return Err(format!(
                "Copied video stream #{} codec changed: {:?} -> {:?}.",
                source.index, source.codec_name, output.codec_name
            ));
        }
        if output.width != source.width || output.height != source.height {
            return Err(format!(
                "Video stream #{} resolution changed from {}x{} to {}x{}.",
                source.index, source.width, source.height, output.width, output.height
            ));
        }
    }

    validate_hdr_video_profiles(request, &real_videos, &out_videos, output_probe, encoded)?;

    let output_duration = output_probe
        .get("format")
        .and_then(|format| float_value(format.get("duration")))
        .unwrap_or(0.0);
    if !request.test_mode {
        let tolerance = 2.0_f64.max(request.source_duration * 0.005);
        let drift = (output_duration - request.source_duration).abs();
        if drift > tolerance {
            return Err(format!(
                "Output duration differs by {drift:.3}s (tolerance {tolerance:.3}s)."
            ));
        }
    }

    if !request.test_mode {
        for kind in ["audio", "subtitle", "attachment", "data"] {
            let mut source_kind = selected
                .iter()
                .filter(|stream| stream.codec_type == kind)
                .cloned()
                .collect::<Vec<_>>();
            let mut output_kind = output_refs
                .iter()
                .filter(|stream| stream.codec_type == kind)
                .cloned()
                .collect::<Vec<_>>();
            if kind == "attachment" && request.expect_poster {
                source_kind.retain(|stream| !is_replaceable_cover(stream));
                output_kind.retain(|stream| !is_replaceable_cover(stream));
            }
            compare_copied_type(request, kind, &source_kind, &output_kind)?;
        }
        let copied_videos: Vec<_> = real_videos
            .iter()
            .filter(|stream| !encoded.contains(&stream.index))
            .cloned()
            .collect();
        let copied_output_videos: Vec<_> = real_videos
            .iter()
            .zip(&out_videos)
            .filter(|(source, _)| !encoded.contains(&source.index))
            .map(|(_, output)| output.clone())
            .collect();
        compare_copied_type(request, "video", &copied_videos, &copied_output_videos)?;
        compare_copied_type(request, "video", &selected_pictures, &out_pictures)?;
        compare_codec_parameters(
            "audio",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
        )?;
        compare_codec_parameters(
            "subtitle",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
        )?;
        validate_timing(
            "audio",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
            0.010,
        )?;
        validate_timing(
            "subtitle",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
            0.020,
        )?;
        validate_chapters(request, output_probe)?;
    }

    Ok(())
}

fn validate_hdr_video_profiles(
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
        validate_hdr_video_profile(request, source_index, output_ordinal, expected, &profile)?;
    }
    Ok(())
}

fn validate_hdr_video_profile(
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

fn compare_copied_type(
    request: &Request,
    kind: &str,
    source: &[StreamRef],
    output: &[StreamRef],
) -> Result<(), String> {
    if source.len() != output.len() {
        return Err(format!(
            "{kind} stream count changed: input={} output={}",
            source.len(),
            output.len()
        ));
    }
    let short = match kind {
        "audio" => "a",
        "subtitle" => "s",
        "video" => "v",
        "attachment" => "t",
        "data" => "d",
        _ => return Err(format!("Unsupported copied stream type {kind:?}.")),
    };
    if request.verify_copy_hashes {
        if kind == "attachment" {
            for (ordinal, (left, right)) in source.iter().zip(output).enumerate() {
                let source_hash = packet_verify::stream_copy_hash(
                    Path::new(&request.ffmpeg),
                    &request.source_path,
                    &format!("{short}:{}", left.type_ordinal),
                )?;
                let output_hash = packet_verify::stream_copy_hash(
                    Path::new(&request.ffmpeg),
                    &request.output_path,
                    &format!("{short}:{}", right.type_ordinal),
                )?;
                compare_streams(
                    request,
                    kind,
                    ordinal,
                    left,
                    right,
                    &source_hash,
                    &output_hash,
                )?;
            }
        } else {
            let source_hashes = packet_verify::packet_fingerprints(
                Path::new(&request.ffmpeg),
                &request.source_path,
                &source
                    .iter()
                    .map(|stream| format!("{short}:{}", stream.type_ordinal))
                    .collect::<Vec<_>>(),
            )?;
            let output_hashes = packet_verify::packet_fingerprints(
                Path::new(&request.ffmpeg),
                &request.output_path,
                &output
                    .iter()
                    .map(|stream| format!("{short}:{}", stream.type_ordinal))
                    .collect::<Vec<_>>(),
            )?;
            for (ordinal, ((left, right), (source_hash, output_hash))) in source
                .iter()
                .zip(output)
                .zip(source_hashes.iter().zip(&output_hashes))
                .enumerate()
            {
                compare_streams(
                    request,
                    kind,
                    ordinal,
                    left,
                    right,
                    source_hash,
                    output_hash,
                )?;
            }
        }
    } else {
        for (ordinal, (left, right)) in source.iter().zip(output).enumerate() {
            compare_stream_metadata(request, kind, ordinal, left, right)?;
        }
    }
    Ok(())
}

fn is_replaceable_cover(stream: &StreamRef) -> bool {
    let is_attachment = stream.codec_type == "attachment";
    let is_attached_picture = stream.codec_type == "video" && stream.attached_pic;
    if !is_attachment && !is_attached_picture {
        return false;
    }
    let filename = stream
        .tags
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("filename"))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    let mime = stream
        .tags
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("mimetype"))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    mime.to_ascii_lowercase().starts_with("image/")
        && ["cover", "poster", "folder", "front"]
            .iter()
            .any(|marker| filename.to_ascii_lowercase().contains(marker))
}

fn compare_streams<T: PartialEq>(
    request: &Request,
    kind: &str,
    ordinal: usize,
    source: &StreamRef,
    output: &StreamRef,
    source_hash: &T,
    output_hash: &T,
) -> Result<(), String> {
    compare_stream_metadata(request, kind, ordinal, source, output)?;
    if source_hash != output_hash {
        return Err(format!(
            "{kind} stream {ordinal} compressed packet sequence/extradata differs. The output is rejected; no audio/subtitle transcoding or packet mutation is allowed."
        ));
    }
    Ok(())
}

fn compare_stream_metadata(
    request: &Request,
    kind: &str,
    ordinal: usize,
    source: &StreamRef,
    output: &StreamRef,
) -> Result<(), String> {
    if source.codec_name != output.codec_name {
        return Err(format!(
            "{kind} stream {ordinal} codec changed: {:?} -> {:?}",
            source.codec_name, output.codec_name
        ));
    }
    if matches!(kind, "audio" | "subtitle" | "attachment" | "data") {
        for key in ["language", "title"] {
            let before = source.tags.get(key).map(value_text).unwrap_or_default();
            let after = output.tags.get(key).map(value_text).unwrap_or_default();
            if before != after
                && !(key == "title"
                    && approved_scrub_change(
                        &request.scrubbed_text_values,
                        &["track_tag", "track_header"],
                        &before,
                        &after,
                        None,
                    ))
            {
                return Err(format!("{kind} stream {ordinal} {key} metadata changed."));
            }
        }
        if kind == "attachment" {
            for key in ["filename", "mimetype"] {
                let before = source.tags.get(key).map(value_text).unwrap_or_default();
                let after = output.tags.get(key).map(value_text).unwrap_or_default();
                if before != after {
                    return Err(format!(
                        "attachment stream {ordinal} {key} metadata changed."
                    ));
                }
            }
        }
        let before = active_dispositions(&source.disposition);
        let after = active_dispositions(&output.disposition);
        if before != after {
            return Err(format!(
                "{kind} stream {ordinal} disposition flags changed: {before:?} -> {after:?}"
            ));
        }
    }
    Ok(())
}

fn active_dispositions(values: &Map<String, Value>) -> BTreeSet<String> {
    values
        .iter()
        .filter(|(_, value)| integerish(value) == Some(1))
        .map(|(name, _)| name.clone())
        .collect()
}

fn integerish(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_bool().map(i64::from))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn compare_codec_parameters(
    kind: &str,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    source_probe: &Value,
    output_probe: &Value,
) -> Result<(), String> {
    let keys: &[&str] = match kind {
        "audio" => &[
            "profile",
            "sample_rate",
            "channels",
            "channel_layout",
            "bits_per_sample",
            "bits_per_raw_sample",
            "extradata_hash",
        ],
        "subtitle" => &["profile", "width", "height", "extradata_hash"],
        _ => return Ok(()),
    };
    let source: Vec<_> = selected
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let output: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let source_raw = raw_stream_map(source_probe)?;
    let output_raw = raw_stream_map(output_probe)?;
    for (ordinal, (source_ref, output_ref)) in source.iter().zip(&output).enumerate() {
        let a = source_raw.get(&source_ref.index);
        let b = output_raw.get(&output_ref.index);
        for key in keys {
            let left = a.and_then(|stream| stream.get(*key));
            let right = b.and_then(|stream| stream.get(*key));
            if source_value_is_present(left)
                && value_text(left.unwrap()) != value_text(right.unwrap_or(&Value::Null))
            {
                return Err(format!(
                    "{kind} stream {ordinal} codec parameter {key} changed: {} -> {}",
                    value_text(left.unwrap()),
                    value_text(right.unwrap_or(&Value::Null))
                ));
            }
        }
    }
    Ok(())
}

fn validate_timing(
    kind: &str,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    source_probe: &Value,
    output_probe: &Value,
    tolerance: f64,
) -> Result<(), String> {
    let source: Vec<_> = selected
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let output: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let source_raw = raw_stream_map(source_probe)?;
    let output_raw = raw_stream_map(output_probe)?;
    for (ordinal, (source_ref, output_ref)) in source.iter().zip(&output).enumerate() {
        let a = source_raw.get(&source_ref.index);
        let b = output_raw.get(&output_ref.index);
        for key in ["start_time", "duration"] {
            let (Some(left), Some(right)) = (
                a.and_then(|stream| float_value(stream.get(key))),
                b.and_then(|stream| float_value(stream.get(key))),
            ) else {
                continue;
            };
            let drift = (left - right).abs();
            if drift > tolerance {
                return Err(format!(
                    "{kind} stream {ordinal} {key} shifted by {drift:.6}s; refusing possible A/V/subtitle sync change."
                ));
            }
        }
    }
    Ok(())
}

fn validate_chapters(request: &Request, output_probe: &Value) -> Result<(), String> {
    let source_chapters = request
        .source_probe
        .get("chapters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let output_chapters = output_probe
        .get("chapters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if output_chapters.len() != request.source_chapters_count {
        return Err(format!(
            "Chapter count changed: {} -> {}",
            request.source_chapters_count,
            output_chapters.len()
        ));
    }
    for (ordinal, (source, output)) in source_chapters.iter().zip(output_chapters).enumerate() {
        for key in ["start_time", "end_time"] {
            let (Some(before), Some(after)) = (
                source.get(key).and_then(|value| float_value(Some(value))),
                output.get(key).and_then(|value| float_value(Some(value))),
            ) else {
                continue;
            };
            let drift = (before - after).abs();
            if drift > 0.002 {
                return Err(format!("Chapter {ordinal} {key} changed by {drift:.6}s."));
            }
        }
        let source_tags = lower_key_map(source.get("tags"));
        let output_tags = lower_key_map(output.get("tags"));
        for (name, before) in source_tags {
            if matches!(name.as_str(), "encoder" | "duration") {
                continue;
            }
            let after = output_tags.get(&name).cloned().unwrap_or_default();
            if before != after
                && !(name == "title"
                    && approved_scrub_change(
                        &request.chapter_text_edits,
                        &["chapter"],
                        &before,
                        &after,
                        Some(ordinal),
                    ))
            {
                return Err(format!(
                    "Chapter {ordinal} metadata {name:?} changed: {before:?} -> {after:?}"
                ));
            }
        }
    }
    Ok(())
}

fn lower_key_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value_text(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn raw_stream_map(probe: &Value) -> Result<HashMap<i64, Value>, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| "FFprobe result has no stream list.".to_string())?;
    streams
        .iter()
        .map(|stream| {
            let index = stream
                .get("index")
                .and_then(Value::as_i64)
                .ok_or_else(|| "FFprobe returned a stream without an integer index.".to_string())?;
            Ok((index, stream.clone()))
        })
        .collect()
}

fn float_value(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if text != "N/A" => text.parse().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

fn source_value_is_present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(value)) => !value.is_empty() && value != "0",
        Some(Value::Number(value)) => value.as_f64().is_none_or(|number| number != 0.0),
        _ => true,
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn approved_scrub_change(
    edits: &[Value],
    scopes: &[&str],
    before: &str,
    after: &str,
    chapter_index: Option<usize>,
) -> bool {
    edits.iter().any(|edit| {
        let scope = edit
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let field = edit
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or("title")
            .trim()
            .to_ascii_lowercase();
        let edit_before = edit.get("before").map(value_text).unwrap_or_default();
        let edit_after = edit.get("after").map(value_text).unwrap_or_default();
        let index_matches = chapter_index.is_none_or(|index| {
            edit.get("chapter_index").and_then(Value::as_u64) == Some(index as u64)
        });
        scopes.contains(&scope)
            && matches!(field.as_str(), "title" | "name")
            && edit_before == before
            && edit_after == after
            && index_matches
    })
}

#[cfg(test)]
mod tests {
    use super::{
        approved_scrub_change, flag_value, float_value, is_replaceable_cover,
        source_value_is_present, stream_refs, validate_chapters, validate_hdr_video_profile,
        HdrTrackExpectation, Request,
    };
    use serde_json::{json, Value};

    fn empty_request() -> Request {
        Request {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            mediainfo: None,
            hdr10plus_tool: None,
            source_path: "input.mkv".into(),
            output_path: "output.mkv".into(),
            source_probe: json!({"streams": []}),
            selected_stream_indices: vec![],
            encode_video_indices: vec![],
            video_target_formats: Default::default(),
            video_hdr_expectations: Default::default(),
            verify_copy_hashes: false,
            test_mode: false,
            copy_only: false,
            expect_poster: false,
            source_duration: 0.0,
            source_chapters_count: 0,
            scrubbed_text_values: vec![],
            chapter_text_edits: vec![],
        }
    }

    #[test]
    fn parses_stream_ordinals_and_attached_picture_flags() {
        let refs = stream_refs(&json!({"streams": [
            {"index": 0, "codec_type": "video", "width": 3840, "height": 2160, "disposition": {"default": 1}},
            {"index": 1, "codec_type": "video", "width": 1920, "height": 1080, "disposition": {"attached_pic": 1}},
            {"index": 2, "codec_type": "audio", "disposition": {"default": "1"}}
        ]})).unwrap();
        assert_eq!(refs[1].type_ordinal, 1);
        assert!(refs[1].attached_pic);
        assert_eq!(refs[2].type_ordinal, 0);
        assert!(refs[2].default);
    }

    #[test]
    fn recognizes_only_cover_like_image_attachments_and_attached_pictures() {
        let refs = stream_refs(&json!({"streams": [
            {"index": 0, "codec_type": "video", "disposition": {"attached_pic": 1}, "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}},
            {"index": 1, "codec_type": "video", "disposition": {"attached_pic": 1}, "tags": {"filename": "backdrop.jpg", "mimetype": "image/jpeg"}},
            {"index": 2, "codec_type": "video", "width": 1920, "height": 1080, "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}},
            {"index": 3, "codec_type": "attachment", "tags": {"filename": "poster.png", "mimetype": "image/png"}},
            {"index": 4, "codec_type": "attachment", "tags": {"filename": "font.ttf", "mimetype": "application/x-truetype-font"}}
        ]})).unwrap();
        assert!(is_replaceable_cover(&refs[0]));
        assert!(!is_replaceable_cover(&refs[1]));
        assert!(!is_replaceable_cover(&refs[2]));
        assert!(is_replaceable_cover(&refs[3]));
        assert!(!is_replaceable_cover(&refs[4]));
    }

    #[test]
    fn parses_only_finite_numeric_timing_and_source_values() {
        assert_eq!(float_value(Some(&json!("2.5"))), Some(2.5));
        assert_eq!(float_value(Some(&json!("N/A"))), None);
        assert!(!source_value_is_present(Some(&json!(0))));
        assert!(!source_value_is_present(Some(&json!(0.0))));
        assert!(!source_value_is_present(Some(&json!("0"))));
        assert!(source_value_is_present(Some(&json!(2))));
        assert!(flag_value(Some(&json!(true))));
        assert!(!flag_value(Some(&json!(2))));
    }

    #[test]
    fn only_exact_matching_text_scrub_changes_are_approved() {
        let edits = vec![json!({
            "scope": "track_tag", "field": "title", "before": "MovieReleaseGroup",
            "after": "", "chapter_index": null
        })];
        assert!(approved_scrub_change(
            &edits,
            &["track_tag", "track_header"],
            "MovieReleaseGroup",
            "",
            None
        ));
        assert!(!approved_scrub_change(
            &edits,
            &["track_tag"],
            "MovieReleaseGroup",
            "Other",
            None
        ));
    }

    #[test]
    fn chapter_scrub_requires_matching_chapter_index() {
        let edits = vec![json!({
            "scope": "chapter", "field": "title", "before": "Scene Group",
            "after": "Scene", "chapter_index": 1
        })];
        let request = super::Request {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            mediainfo: None,
            hdr10plus_tool: None,
            source_path: "input.mkv".into(),
            output_path: "output.mkv".into(),
            source_probe: json!({"chapters": [
                {"start_time": "0", "end_time": "1", "tags": {"title": "Intro"}},
                {"start_time": "1", "end_time": "2", "tags": {"title": "Scene Group"}}
            ]}),
            selected_stream_indices: vec![],
            encode_video_indices: vec![],
            video_target_formats: Default::default(),
            video_hdr_expectations: Default::default(),
            verify_copy_hashes: false,
            test_mode: false,
            copy_only: false,
            source_duration: 2.0,
            source_chapters_count: 2,
            expect_poster: false,
            scrubbed_text_values: vec![],
            chapter_text_edits: edits,
        };
        let output = json!({"chapters": [
            {"start_time": "0", "end_time": "1", "tags": {"title": "Intro"}},
            {"start_time": "1", "end_time": "2", "tags": {"title": "Scene"}}
        ]});
        assert!(validate_chapters(&request, &output).is_ok());
    }

    #[test]
    fn hdr_profile_validation_preserves_static_hdr_and_color_per_video_track() {
        let request = empty_request();
        let expected = HdrTrackExpectation {
            color_primaries: Some("bt2020".into()),
            transfer: Some("smpte2084".into()),
            color_matrix: Some("bt2020nc".into()),
            master_display: Some("G(1,2)".into()),
            max_cll: Some(1000),
            max_fall: Some(400),
            hdr10: true,
            ..Default::default()
        };
        let actual = json!({
            "color_primaries": "bt2020",
            "color_transfer": "smpte2084",
            "color_matrix": "bt2020nc",
            "mastering_display": "G(1,2)",
            "max_cll": 1000,
            "max_fall": 400,
            "hdr": {"hdr10": true, "hlg": false, "dolby_vision": false}
        });
        assert!(validate_hdr_video_profile(&request, 4, 0, &expected, &actual).is_ok());

        let mut changed: Value = actual.clone();
        changed["max_cll"] = json!(900);
        assert!(
            validate_hdr_video_profile(&request, 4, 0, &expected, &changed)
                .unwrap_err()
                .contains("MaxCLL changed")
        );
    }

    #[test]
    fn dolby_vision_profile_validation_requires_matching_rpu_and_compatibility() {
        let request = empty_request();
        let expected = HdrTrackExpectation {
            hdr10: true,
            dolby_vision: true,
            dovi_profile: Some(8),
            dovi_rpu_present: true,
            dovi_compatibility_id: Some(1),
            preserve_dolbyvision: true,
            ..Default::default()
        };
        let actual = json!({
            "hdr": {
                "hdr10": true, "hlg": false, "dolby_vision": true,
                "dolby_vision_profile": 8, "dolby_vision_rpu": true,
                "dolby_vision_enhancement_layer": false,
                "dolby_vision_compatibility_id": 1
            }
        });
        assert!(validate_hdr_video_profile(&request, 2, 0, &expected, &actual).is_ok());

        let mut changed: Value = actual.clone();
        changed["hdr"]["dolby_vision_rpu"] = json!(false);
        assert!(
            validate_hdr_video_profile(&request, 2, 0, &expected, &changed)
                .unwrap_err()
                .contains("does not report an RPU")
        );
    }
}
