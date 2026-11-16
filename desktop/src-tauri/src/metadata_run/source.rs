//! Source stream inventory, HDR expectations and tool resolution for a metadata-only run.

use super::*;

pub(super) struct Tools {
    pub(super) ffmpeg: String,
    pub(super) ffprobe: String,
    pub(super) mediainfo: String,
    pub(super) mkvmerge: String,
    pub(super) mkvpropedit: String,
    pub(super) mkvextract: String,
    pub(super) hdr10plus: Option<String>,
}

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

/// Expected HDR/colour state for one copied video track (the fields `validation` expects).
pub(super) fn hdr_expectation(profile: &Value, hdr10plus: bool) -> Value {
    let hdr = profile.get("hdr");
    json!({
        "color_primaries": text_value(profile.get("color_primaries")),
        "transfer": text_value(profile.get("color_transfer")),
        "color_matrix": text_value(profile.get("color_matrix")),
        "master_display": text_value(profile.get("mastering_display")),
        "max_cll": profile.get("max_cll").and_then(Value::as_i64),
        "max_fall": profile.get("max_fall").and_then(Value::as_i64),
        "hdr10": flag(hdr, "hdr10"),
        "hdr10plus": hdr10plus,
        "hlg": flag(hdr, "hlg"),
        "dolby_vision": flag(hdr, "dolby_vision"),
        "dovi_profile": hdr.and_then(|h| h.get("dolby_vision_profile")).and_then(Value::as_i64),
        "dovi_rpu_present": flag(hdr, "dolby_vision_rpu"),
        "dovi_el_present": flag(hdr, "dolby_vision_enhancement_layer"),
        "dovi_compatibility_id": hdr
            .and_then(|h| h.get("dolby_vision_compatibility_id"))
            .and_then(Value::as_i64),
        "preserve_dolbyvision": false,
        "strip_dolbyvision": false,
    })
}

pub(super) struct SourceStream {
    pub(super) index: i64,
    pub(super) codec_type: String,
    pub(super) codec_name: String,
    pub(super) real_video: bool,
}

pub(super) fn source_streams(probe: &Value) -> Vec<SourceStream> {
    probe
        .get("streams")
        .and_then(Value::as_array)
        .map(|streams| {
            streams
                .iter()
                .filter_map(|stream| {
                    let index = stream.get("index").and_then(Value::as_i64)?;
                    let codec_type = stream
                        .get("codec_type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let attached = stream
                        .pointer("/disposition/attached_pic")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                        == 1;
                    let positive =
                        |key: &str| stream.get(key).and_then(Value::as_i64).unwrap_or(0) > 0;
                    Some(SourceStream {
                        index,
                        real_video: codec_type == "video"
                            && !attached
                            && positive("width")
                            && positive("height"),
                        codec_name: stream
                            .get("codec_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        codec_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn resolve_tools(options: &MetadataRunOptions) -> Result<Tools, String> {
    let host = crate::environment::inspect_environment(options.tool_paths.clone(), None)?;
    let mut missing = Vec::new();
    let mut need = |key: &str, label: &str| -> String {
        optional_tool(&host, key).unwrap_or_else(|| {
            missing.push(label.to_string());
            String::new()
        })
    };
    let tools = Tools {
        ffmpeg: need("ffmpeg", "FFmpeg"),
        ffprobe: need("ffprobe", "FFprobe"),
        mediainfo: need("mediainfo", "MediaInfo"),
        mkvmerge: need("mkvmerge", "MKVToolNix mkvmerge"),
        mkvpropedit: need("mkvpropedit", "MKVToolNix mkvpropedit"),
        mkvextract: need("mkvextract", "MKVToolNix mkvextract"),
        hdr10plus: optional_tool(&host, "hdr10plus_tool"),
    };
    if !missing.is_empty() {
        return Err(format!(
            "Metadata-only MKV output needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    Ok(tools)
}

// ------------------------------------------------------------------- running
