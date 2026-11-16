//! Matching MediaInfo tracks to FFprobe streams and selecting the fields the profile keeps.

use super::*;

pub(super) fn match_mediainfo_video<'a>(
    tracks: &'a [Value],
    primary: &Value,
    ordinal: usize,
    width: i64,
    height: i64,
) -> Option<&'a Value> {
    let videos = tracks
        .iter()
        .filter(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
        })
        .collect::<Vec<_>>();
    if videos.is_empty() {
        return None;
    }

    // MediaInfo StreamOrder normally matches FFprobe's global stream index.
    if let Some(track) = videos.iter().copied().find(|track| {
        track.get("StreamOrder").and_then(parse_media_intish) == value_i64(primary.get("index"))
    }) {
        return Some(track);
    }

    // If stream order is missing, accept the typed ordinal only when its
    // dimensions also agree with the selected FFprobe stream.
    if let Some(track) = videos
        .get(ordinal)
        .copied()
        .filter(|track| media_dimensions(track) == (width, height))
    {
        return Some(track);
    }

    let exact = videos
        .iter()
        .copied()
        .filter(|track| media_dimensions(track) == (width, height))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return exact.first().copied();
    }
    if !exact.is_empty() {
        let codec = primary
            .get("codec_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let compatible = exact.iter().copied().find(|track| {
            let format = track
                .get("Format")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            match codec.as_str() {
                "hevc" => ["hevc", "h.265", "h265"].contains(&format.as_str()),
                "h264" => ["avc", "h.264", "h264"].contains(&format.as_str()),
                "av1" => format == "av1",
                "vp9" => format == "vp9",
                _ => format == codec,
            }
        });
        return compatible.or_else(|| exact.first().copied());
    }

    // Fall back to the largest track when no MediaInfo track has the
    // FFprobe-selected dimensions.
    videos.into_iter().max_by_key(|track| {
        let (width, height) = media_dimensions(track);
        (width.saturating_mul(height), width, height)
    })
}

pub(super) fn media_dimensions(track: &Value) -> (i64, i64) {
    (
        track.get("Width").and_then(parse_media_intish).unwrap_or(0),
        track
            .get("Height")
            .and_then(parse_media_intish)
            .unwrap_or(0),
    )
}

fn parse_media_intish(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let compact = text.trim().replace([' ', ','], "");
    parse_first_signed_integer(&compact)
}

fn parse_first_signed_integer(value: &str) -> Option<i64> {
    let start = value
        .char_indices()
        .find(|(_, character)| character.is_ascii_digit() || *character == '-')?
        .0;
    let number = value[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect::<String>();
    number.parse().ok()
}

pub(super) fn match_mediainfo_track<'a>(
    tracks: &'a [Value],
    kind: &str,
    index: i64,
    ordinal: usize,
) -> Option<&'a Value> {
    tracks
        .iter()
        .find(|track| {
            track
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(kind))
                && track.get("StreamOrder").and_then(parse_numeric) == Some(index)
        })
        .or_else(|| {
            tracks
                .iter()
                .filter(|track| {
                    track
                        .get("@type")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(kind))
                })
                .nth(ordinal)
        })
}

const MEDIAINFO_FIELDS: &[&str] = &[
    "Format",
    "Format_Profile",
    "CodecID",
    "CodecID_Description",
    "Width",
    "Height",
    "BitDepth",
    "ChromaSubsampling",
    "FrameRate",
    "FrameRate_Mode",
    "ScanType",
    "colour_primaries",
    "transfer_characteristics",
    "matrix_coefficients",
    "colour_range",
    "HDR_Format",
    "HDR_Format_Profile",
    "HDR_Format_Compatibility",
    "HDR_Format_Settings",
    "MasteringDisplay_ColorPrimaries",
    "MasteringDisplay_Luminance",
    "MaxCLL",
    "MaxFALL",
    "Language",
    "Title",
    "Channels",
    "ChannelLayout",
    "SamplingRate",
    "BitRate",
    "Compression_Mode",
    "Default",
    "Forced",
    "StreamSize",
    "Duration",
    "Format_Settings",
    "Encoded_Library",
    "WritingLibrary",
    "InternetMediaType",
    "ChromaSubsampling_Position",
];

pub(super) fn select_mediainfo_fields(track: &Value) -> Value {
    let mut selected = Map::new();
    if let Some(object) = track.as_object() {
        for key in MEDIAINFO_FIELDS {
            if let Some(value) = object.get(*key) {
                selected.insert((*key).into(), value.clone());
            }
        }
    }
    Value::Object(selected)
}
