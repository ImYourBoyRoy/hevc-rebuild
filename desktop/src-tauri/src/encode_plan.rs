//! Rust-owned FFmpeg/x265 argument planning for the compatibility engine.
//!
//! The Python compatibility adapter still owns run orchestration, output
//! validation, transactions, reports, and HDR10+ extraction/reinsertion. Rust
//! also constructs and supervises app-launched calibration benchmarks from a
//! typed profile/options payload supplied by Python. Runs started by the Rust app
//! ask this module to construct their exact encode argument vector; direct
//! Python runs keep the reference planner until parity is proven.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{Read, Write};

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const DISPOSITION_ORDER: &[&str] = &[
    "default",
    "dub",
    "original",
    "comment",
    "lyrics",
    "karaoke",
    "forced",
    "hearing_impaired",
    "visual_impaired",
    "clean_effects",
    "attached_pic",
    "timed_thumbnails",
    "captions",
    "descriptions",
    "metadata",
    "dependent",
    "still_image",
    "non_diegetic",
];

#[derive(Clone, Debug)]
struct Stream {
    index: u32,
    ordinal: u32,
    kind: String,
    codec: Option<String>,
    width: u32,
    height: u32,
    default: bool,
    attached_pic: bool,
    disposition: BTreeMap<String, Value>,
}

impl Stream {
    fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    fn label(&self) -> String {
        let codec = self.codec.as_deref().unwrap_or("?");
        if self.kind == "video" {
            format!(
                "#{} v:{} {}x{} {}",
                self.index, self.ordinal, self.width, self.height, codec
            )
        } else {
            format!("#{} {}:{} {}", self.index, self.kind, self.ordinal, codec)
        }
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    value
        .get(name)
        .ok_or_else(|| format!("Encoder plan is missing {name:?}."))
}

fn string(value: &Value, name: &str) -> Result<String, String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("Encoder plan field {name:?} must be text."))
}

fn optional_string(value: &Value, name: &str) -> Result<Option<String>, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .ok_or_else(|| format!("Encoder plan field {name:?} must be text or null.")),
    }
}

fn boolean(value: &Value, name: &str, default: bool) -> Result<bool, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("Encoder plan field {name:?} must be boolean.")),
    }
}

fn optional_integer(value: &Value, name: &str) -> Result<Option<i64>, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("Encoder plan field {name:?} must be an integer.")),
        Some(_) => Err(format!(
            "Encoder plan field {name:?} must be an integer or null."
        )),
    }
}

fn unsigned(value: &Value, name: &str, default: u32) -> Result<u32, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .ok_or_else(|| format!("Encoder plan field {name:?} must be a non-negative integer.")),
        Some(_) => Err(format!("Encoder plan field {name:?} must be an integer.")),
    }
}

fn parse_stream(value: &Value) -> Result<Stream, String> {
    let disposition = value
        .get("disposition")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok(Stream {
        index: field(value, "index")?
            .as_u64()
            .and_then(|index| u32::try_from(index).ok())
            .ok_or("Encoder plan stream index must be a non-negative 32-bit integer.")?,
        ordinal: unsigned(value, "type_ordinal", 0)?,
        kind: string(field(value, "codec_type")?, "codec_type")?,
        codec: optional_string(value, "codec_name")?,
        width: unsigned(value, "width", 0)?,
        height: unsigned(value, "height", 0)?,
        default: boolean(value, "default", false)?,
        attached_pic: boolean(value, "attached_pic", false)?,
        disposition,
    })
}

fn parse_streams(value: &Value, name: &str) -> Result<Vec<Stream>, String> {
    let values = value
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Encoder plan field {name:?} must be an array."))?;
    values.iter().map(parse_stream).collect()
}

fn integer_truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|number| number.trunc() as i64))
                .unwrap_or_default()
                == 1
        }
        Some(Value::String(value)) => value.trim().parse::<i64>().ok() == Some(1),
        _ => false,
    }
}

fn disposition_value(stream: &Stream, force_default: Option<bool>) -> String {
    let mut flags = DISPOSITION_ORDER
        .iter()
        .filter(|name| **name != "attached_pic")
        .filter(|name| integer_truthy(stream.disposition.get(**name)))
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    match force_default {
        Some(true) if !flags.iter().any(|name| name == "default") => {
            flags.insert(0, "default".into());
        }
        Some(false) => flags.retain(|name| name != "default"),
        _ => {}
    }
    if flags.is_empty() {
        "0".into()
    } else {
        flags.join("+")
    }
}

fn option_string<'a>(options: &'a Value, name: &str, fallback: &'a str) -> Result<&'a str, String> {
    match options.get(name) {
        None | Some(Value::Null) => Ok(fallback),
        Some(value) => value
            .as_str()
            .ok_or_else(|| format!("Encoder plan option {name:?} must be text.")),
    }
}

fn option_bool(options: &Value, name: &str, fallback: bool) -> Result<bool, String> {
    boolean(options, name, fallback)
}

fn int_set(options: &Value, name: &str) -> Result<BTreeSet<u32>, String> {
    let Some(value) = options.get(name) else {
        return Ok(BTreeSet::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("Encoder plan option {name:?} must be an array."))?;
    values
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| format!("Encoder plan option {name:?} contains an invalid index."))
        })
        .collect()
}

fn optional_video_profile<'a>(
    video_profiles: &'a Value,
    stream_index: u32,
    fallback: &'a Value,
) -> &'a Value {
    video_profiles
        .get(stream_index.to_string())
        .unwrap_or(fallback)
}

fn parse_level(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index < bytes.len() && bytes[index] == b'.' {
            let decimal = index;
            index += 1;
            let fraction_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if fraction_start == index {
                index = decimal;
            }
        }
        let token = &value[start..index];
        if token.parse::<f64>().ok().is_some_and(|level| level >= 5.0) {
            return Some(token.to_owned());
        }
    }
    None
}

fn x265_params(
    profile: &Value,
    hdr10plus_path: Option<&str>,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
) -> Result<Vec<String>, String> {
    let mut params = Vec::new();
    if preserve_source_level {
        if let Some(level) = optional_string(profile, "format_level")? {
            if let Some(level) = parse_level(&level) {
                params.push(format!("level-idc={level}"));
            }
        }
    }
    for (field_name, parameter) in [
        ("color_primaries", "colorprim"),
        ("transfer", "transfer"),
        ("color_matrix", "colormatrix"),
        ("color_range_x265", "range"),
    ] {
        if let Some(value) = optional_string(profile, field_name)? {
            params.push(format!("{parameter}={value}"));
        }
    }
    let target_pix_fmt = optional_string(profile, "target_pix_fmt")?.unwrap_or_default();
    if target_pix_fmt.contains("420") {
        if let Some(location) = optional_integer(profile, "chroma_location")? {
            params.push(format!("chromaloc={location}"));
        }
    }
    if let Some(master_display) = optional_string(profile, "master_display")? {
        params.push(format!("master-display={master_display}"));
    }
    if let Some(max_cll) = optional_integer(profile, "max_cll")? {
        let max_fall = optional_integer(profile, "max_fall")?.unwrap_or(0);
        params.push(format!("max-cll={max_cll},{max_fall}"));
    }
    let hdr10 = boolean(profile, "hdr10", false)?;
    if hdr10 {
        params.push("hdr10=1".into());
        if !disable_hdr10_opt
            && target_pix_fmt == "yuv420p10le"
            && optional_string(profile, "color_primaries")?.as_deref() == Some("bt2020")
            && optional_string(profile, "transfer")?.as_deref() == Some("smpte2084")
        {
            params.push("hdr10-opt=1".into());
        }
    }
    if boolean(profile, "hlg", false)? {
        params.extend(["atc-sei=18".into(), "pic-struct=0".into()]);
    }
    if let Some(path) = hdr10plus_path {
        if path.contains(':') {
            return Err(format!(
                "HDR10+ JSON path contains ':' and cannot be safely embedded in x265-params: {path}"
            ));
        }
        params.extend([format!("dhdr10-info={path}"), "dhdr10-opt=1".into()]);
    }
    Ok(params)
}

fn stream_groups(profile: &Value) -> Result<Vec<Stream>, String> {
    let mut streams = Vec::new();
    for group in [
        "primary_video",
        "secondary_videos",
        "audio",
        "subtitles",
        "attachments",
        "data",
    ] {
        if group == "primary_video" {
            streams.push(parse_stream(field(profile, group)?)?);
        } else {
            streams.extend(parse_streams(profile, group)?);
        }
    }
    Ok(streams)
}

fn build_plan(request: &Value) -> Result<Value, String> {
    let tools = field(request, "tools")?;
    let profile = field(request, "profile")?;
    let options = field(request, "options")?;
    let ffmpeg = string(field(tools, "ffmpeg")?, "tools.ffmpeg")?;
    let path = string(field(profile, "path")?, "profile.path")?;
    let video_profiles = request
        .get("video_profiles")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let hdr10plus_jsons = request
        .get("hdr10plus_jsons")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let preserve_dv_indices = int_set(request, "preserve_dolbyvision_indices")?;
    let strip_dv_indices = int_set(request, "strip_dolbyvision_indices")?;
    let preserve_dv = option_bool(options, "preserve_dolbyvision", false)?;
    let strip_dv = option_bool(options, "strip_dolbyvision", false)?;
    let secondary_policy = option_string(options, "secondary_video_policy", "drop")?;
    let data_policy = option_string(options, "data_policy", "preserve")?;

    let selected_streams =
        if let Some(values) = request.get("selected_streams").and_then(Value::as_array) {
            values
                .iter()
                .map(parse_stream)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let all = stream_groups(profile)?;
            let primary_index = parse_stream(field(profile, "primary_video")?)?.index;
            let mut streams = all
                .iter()
                .filter(|stream| {
                    stream.kind == "video"
                        && (stream.index == primary_index || secondary_policy != "drop")
                })
                .cloned()
                .collect::<Vec<_>>();
            streams.extend(
                all.iter()
                    .filter(|stream| {
                        matches!(stream.kind.as_str(), "audio" | "subtitle" | "attachment")
                    })
                    .cloned(),
            );
            if data_policy == "preserve" {
                streams.extend(all.iter().filter(|stream| stream.kind == "data").cloned());
            }
            streams
        };

    let mut selected_videos = selected_streams
        .iter()
        .filter(|stream| {
            stream.kind == "video" && !stream.attached_pic && stream.width > 0 && stream.height > 0
        })
        .cloned()
        .collect::<Vec<_>>();
    if selected_videos.is_empty() {
        return Err("Select at least one usable video track to rebuild this file.".into());
    }
    selected_videos.sort_by(|left, right| {
        right
            .pixels()
            .cmp(&left.pixels())
            .then_with(|| right.default.cmp(&left.default))
            .then_with(|| left.index.cmp(&right.index))
    });
    let mut videos = selected_videos.clone();
    videos.extend(
        selected_streams
            .iter()
            .filter(|stream| stream.kind == "video" && stream.attached_pic)
            .cloned(),
    );

    let encode_video_indices = if let Some(indices) = request.get("encode_video_indices") {
        indices
            .as_array()
            .ok_or("Encoder plan field \"encode_video_indices\" must be an array.")?
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|number| u32::try_from(number).ok())
                    .ok_or("Encoder plan contains an invalid encode index.")
            })
            .collect::<Result<BTreeSet<_>, _>>()?
    } else {
        let primary = parse_stream(field(profile, "primary_video")?)?.index;
        let mut indices = BTreeSet::from([primary]);
        if secondary_policy == "encode" {
            indices.extend(selected_videos.iter().map(|stream| stream.index));
        }
        indices
    };
    let real_video_indices = selected_videos
        .iter()
        .map(|stream| stream.index)
        .collect::<BTreeSet<_>>();
    if !encode_video_indices.is_subset(&real_video_indices) {
        return Err("An encode selection refers to a video track that is not selected.".into());
    }
    let mut unique_indices = BTreeSet::new();
    if selected_streams
        .iter()
        .any(|stream| !unique_indices.insert(stream.index))
    {
        return Err("A source stream was selected more than once.".into());
    }

    let pixel_formats = field(tools, "x265_pixel_formats")?
        .as_array()
        .ok_or("Encoder plan field \"x265_pixel_formats\" must be an array.")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for stream in &selected_videos {
        if encode_video_indices.contains(&stream.index) {
            let track_profile = optional_video_profile(&video_profiles, stream.index, profile);
            let pix_fmt = string(field(track_profile, "target_pix_fmt")?, "target_pix_fmt")?;
            if !pixel_formats.contains(pix_fmt.as_str()) {
                return Err(format!(
                    "Installed FFmpeg/libx265 does not advertise required pixel format {pix_fmt:?} for {}; refusing a silent chroma/bit-depth downgrade.",
                    stream.label()
                ));
            }
        }
    }

    let test_seconds = optional_integer(options, "test_seconds")?;
    let mut args = vec![
        ffmpeg,
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-stats_period".into(),
        "0.5".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-i".into(),
        path,
    ];
    if let Some(seconds) = test_seconds.filter(|seconds| *seconds != 0) {
        args.extend(["-t".into(), seconds.to_string()]);
    }

    let mut output_video_map = BTreeMap::new();
    for (output_ordinal, stream) in videos.iter().enumerate() {
        args.extend(["-map".into(), format!("0:{}", stream.index)]);
        output_video_map.insert(stream.index, output_ordinal);
    }
    let audio = selected_streams
        .iter()
        .filter(|stream| stream.kind == "audio")
        .collect::<Vec<_>>();
    let subtitles = selected_streams
        .iter()
        .filter(|stream| stream.kind == "subtitle")
        .collect::<Vec<_>>();
    let attachments = selected_streams
        .iter()
        .filter(|stream| stream.kind == "attachment")
        .collect::<Vec<_>>();
    let data = selected_streams
        .iter()
        .filter(|stream| stream.kind == "data")
        .collect::<Vec<_>>();
    if test_seconds.is_none() {
        for stream in audio
            .iter()
            .chain(&subtitles)
            .chain(&attachments)
            .chain(&data)
        {
            args.extend(["-map".into(), format!("0:{}", stream.index)]);
        }
        args.extend([
            "-map_metadata".into(),
            "0".into(),
            "-map_chapters".into(),
            "0".into(),
        ]);
    } else {
        args.extend([
            "-map_metadata".into(),
            "-1".into(),
            "-map_chapters".into(),
            "-1".into(),
        ]);
    }
    args.extend(["-c".into(), "copy".into()]);

    let plan = request.get("resource_plan").unwrap_or(&Value::Null);
    let worker_threads = optional_integer(plan, "worker_threads")?.filter(|value| *value > 0);
    let frame_threads = optional_integer(plan, "frame_threads")?.filter(|value| *value > 0);
    let crf = optional_integer(options, "crf")?.unwrap_or(22);
    let preset = option_string(options, "preset", "slow")?;
    let tune = option_string(options, "tune", "")?;
    let force_deinterlace = option_bool(options, "force_deinterlace", false)?;
    let disable_deinterlace = option_bool(options, "disable_deinterlace", false)?;
    let preserve_source_level = option_bool(options, "preserve_source_level", true)?;
    let disable_hdr10_opt = option_bool(options, "disable_hdr10_opt", false)?;
    let x265_dolbyvision = boolean(tools, "x265_dolbyvision", false)?;
    let mut video_map_json = Map::new();

    for (output_ordinal, stream) in videos.iter().enumerate() {
        let output_ordinal_index = output_ordinal;
        let output_ordinal = output_ordinal_index.to_string();
        if test_seconds.is_none() {
            args.extend([
                format!("-map_metadata:s:v:{output_ordinal}"),
                format!("0:s:{}", stream.index),
            ]);
        }
        video_map_json.insert(stream.index.to_string(), json!(output_ordinal_index));
        if stream.attached_pic || !encode_video_indices.contains(&stream.index) {
            args.extend([format!("-c:v:{output_ordinal}"), "copy".into()]);
            continue;
        }
        let track_profile = optional_video_profile(&video_profiles, stream.index, profile);
        let pix_fmt = string(field(track_profile, "target_pix_fmt")?, "target_pix_fmt")?;
        args.extend([
            format!("-c:v:{output_ordinal}"),
            "libx265".into(),
            format!("-pix_fmt:v:{output_ordinal}"),
            pix_fmt,
            format!("-preset:v:{output_ordinal}"),
            preset.into(),
            format!("-crf:v:{output_ordinal}"),
            crf.to_string(),
        ]);
        if !tune.is_empty() {
            args.extend([format!("-tune:v:{output_ordinal}"), tune.into()]);
        }
        let keep_dv = preserve_dv_indices.contains(&stream.index)
            || (preserve_dv && boolean(track_profile, "dolby_vision", false)?);
        let drop_dv = strip_dv_indices.contains(&stream.index)
            || (strip_dv && boolean(track_profile, "dolby_vision", false)?);
        if keep_dv && drop_dv {
            return Err("Internal error: Dolby Vision cannot be preserved and stripped on the same video stream.".into());
        }
        if keep_dv {
            args.extend([format!("-dolbyvision:v:{output_ordinal}"), "1".into()]);
        } else if drop_dv && x265_dolbyvision {
            args.extend([format!("-dolbyvision:v:{output_ordinal}"), "0".into()]);
        }
        let interlaced = boolean(track_profile, "interlaced", false)?;
        let hlg = boolean(track_profile, "hlg", false)?;
        if hlg && interlaced && disable_deinterlace && !force_deinterlace {
            return Err(
                "Interlaced HLG with deinterlacing disabled cannot be signaled safely.".into(),
            );
        }
        if force_deinterlace || (interlaced && !disable_deinterlace) {
            args.extend([
                format!("-filter:v:{output_ordinal}"),
                "bwdif=mode=send_frame:parity=auto:deint=interlaced".into(),
            ]);
        }
        args.extend([
            format!("-fps_mode:v:{output_ordinal}"),
            "passthrough".into(),
        ]);
        for (option, field_name) in [
            ("color_primaries", "color_primaries"),
            ("color_trc", "transfer"),
            ("colorspace", "color_matrix"),
            ("color_range", "color_range_ffmpeg"),
        ] {
            if let Some(value) = optional_string(track_profile, field_name)? {
                args.extend([format!("-{option}:v:{output_ordinal}"), value]);
            }
        }
        let hdr10plus_path = match hdr10plus_jsons.get(stream.index.to_string()) {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_str().ok_or("HDR10+ path must be text.")?),
        };
        let mut params = x265_params(
            track_profile,
            hdr10plus_path,
            preserve_source_level,
            disable_hdr10_opt,
        )?;
        if let Some(workers) = worker_threads {
            params.push(format!("pools={workers}"));
        }
        if let Some(frames) = frame_threads {
            params.push(format!("frame-threads={frames}"));
        }
        if !params.is_empty() {
            args.extend([format!("-x265-params:v:{output_ordinal}"), params.join(":")]);
        }
        args.extend([
            format!("-disposition:v:{output_ordinal}"),
            disposition_value(stream, Some(output_ordinal == "0")),
        ]);
    }

    for (kind, short, streams) in [
        ("audio", "a", audio),
        ("subtitle", "s", subtitles),
        ("attachment", "t", attachments),
        ("data", "d", data),
    ] {
        for (output_ordinal, stream) in streams.iter().enumerate() {
            if test_seconds.is_none() {
                args.extend([
                    format!("-map_metadata:s:{short}:{output_ordinal}"),
                    format!("0:s:{}", stream.index),
                ]);
            }
            if kind == "audio" || kind == "subtitle" {
                args.extend([
                    format!("-disposition:{short}:{output_ordinal}"),
                    disposition_value(stream, None),
                ]);
            }
        }
    }
    let output = string(field(options, "output")?, "options.output")?;
    args.extend(["-f".into(), "matroska".into(), output]);
    Ok(json!({"args": args, "output_video_map": video_map_json}))
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    if !args.is_empty() {
        eprintln!("The encode-plan helper does not accept command-line arguments.");
        return 2;
    }
    let mut input = String::new();
    if let Err(error) = std::io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_string(&mut input)
    {
        eprintln!("Could not read encoder plan request: {error}");
        return 2;
    }
    if input.len() > MAX_REQUEST_BYTES {
        eprintln!("Encoder plan request exceeds the 8 MiB limit.");
        return 2;
    }
    let request: Value = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Encoder plan request is invalid JSON: {error}");
            return 2;
        }
    };
    match build_plan(&request) {
        Ok(response) => match serde_json::to_vec(&response) {
            Ok(json) => {
                let mut stdout = std::io::stdout().lock();
                if stdout
                    .write_all(&json)
                    .and_then(|_| stdout.write_all(b"\n"))
                    .is_ok()
                {
                    0
                } else {
                    eprintln!("Could not write encoder plan response.");
                    2
                }
            }
            Err(error) => {
                eprintln!("Could not serialize encoder plan: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(index: u32, kind: &str, width: u32, height: u32) -> Value {
        json!({
            "index": index, "type_ordinal": index, "codec_type": kind,
            "codec_name": "fixture", "width": width, "height": height,
            "default": index == 0, "attached_pic": false, "disposition": {}
        })
    }

    #[test]
    fn rejects_a_plan_without_a_usable_real_video() {
        let request = json!({
            "tools":{"ffmpeg":"ffmpeg","x265_pixel_formats":[],"x265_dolbyvision":false},
            "profile":{"path":"source.mkv","primary_video":stream(0,"video",0,0),
                "secondary_videos":[],"audio":[],"subtitles":[],"attachments":[],"data":[]},
            "options":{"output":"output.mkv"},"selected_streams":[stream(0,"video",0,0)]
        });
        assert!(build_plan(&request)
            .unwrap_err()
            .contains("usable video track"));
    }

    #[test]
    fn refuses_missing_source_pixel_format_support() {
        let profile = json!({
            "path":"source.mkv","primary_video":stream(0,"video",3840,2160),
            "secondary_videos":[],"audio":[],"subtitles":[],"attachments":[],"data":[],
            "target_pix_fmt":"yuv420p10le"
        });
        let request = json!({
            "tools":{"ffmpeg":"ffmpeg","x265_pixel_formats":[],"x265_dolbyvision":false},
            "profile":profile,"options":{"output":"output.mkv"},
            "selected_streams":[stream(0,"video",3840,2160)],"encode_video_indices":[0]
        });
        let error = build_plan(&request).unwrap_err();
        assert!(error.contains("yuv420p10le"));
        assert!(error.contains("3840x2160"));
    }

    #[test]
    fn rejects_hdr10plus_paths_that_cannot_be_encoded_in_x265_params() {
        let stream = stream(0, "video", 3840, 2160);
        let request = json!({
            "tools":{"ffmpeg":"ffmpeg","x265_pixel_formats":["yuv420p10le"],"x265_dolbyvision":false},
            "profile":{"path":"source.mkv","primary_video":stream,"secondary_videos":[],
                "audio":[],"subtitles":[],"attachments":[],"data":[],"target_pix_fmt":"yuv420p10le"},
            "video_profiles":{"0":{"target_pix_fmt":"yuv420p10le","hdr10":true}},
            "hdr10plus_jsons":{"0":"/tmp/data:part.json"},
            "selected_streams":[stream],"encode_video_indices":[0],
            "options":{"output":"output.mkv"},"resource_plan":{}
        });
        assert!(build_plan(&request).unwrap_err().contains("contains ':'"));
    }

    #[test]
    fn rejects_interlaced_hlg_when_deinterlacing_is_explicitly_disabled() {
        let stream = stream(0, "video", 1920, 1080);
        let request = json!({
            "tools":{"ffmpeg":"ffmpeg","x265_pixel_formats":["yuv420p10le"],"x265_dolbyvision":false},
            "profile":{"path":"source.mkv","primary_video":stream,"secondary_videos":[],
                "audio":[],"subtitles":[],"attachments":[],"data":[],"target_pix_fmt":"yuv420p10le"},
            "video_profiles":{"0":{"target_pix_fmt":"yuv420p10le","hlg":true,"interlaced":true}},
            "selected_streams":[stream],"encode_video_indices":[0],
            "options":{"output":"output.mkv","disable_deinterlace":true},"resource_plan":{}
        });
        assert!(build_plan(&request).unwrap_err().contains("Interlaced HLG"));
    }

    #[test]
    fn plans_two_video_encodes_and_preserves_selected_non_video_streams() {
        let video_4k = stream(0, "video", 3840, 2160);
        let mut video_1080 = stream(1, "video", 1920, 1080);
        video_1080["default"] = json!(false);
        video_1080["disposition"] = json!({"forced": 1});
        let audio = stream(2, "audio", 0, 0);
        let mut subtitle = stream(3, "subtitle", 0, 0);
        subtitle["disposition"] = json!({"forced": 1});
        let attachment = stream(4, "attachment", 0, 0);
        let data = stream(5, "data", 0, 0);
        let hdr_profile = json!({
            "target_pix_fmt":"yuv420p10le","format_level":"L5.1",
            "color_primaries":"bt2020","transfer":"smpte2084",
            "color_matrix":"bt2020nc","color_range_x265":"limited",
            "color_range_ffmpeg":"tv","chroma_location":2,
            "master_display":"G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50)",
            "max_cll":1000,"max_fall":400,"hdr10":true,"hlg":false,
            "dolby_vision":false,"interlaced":false
        });
        let sdr_profile = json!({
            "target_pix_fmt":"yuv420p","format_level":"L4.1",
            "color_primaries":null,"transfer":null,"color_matrix":null,
            "color_range_x265":null,"color_range_ffmpeg":null,"chroma_location":null,
            "master_display":null,"max_cll":null,"max_fall":null,
            "hdr10":false,"hlg":false,"dolby_vision":false,"interlaced":false
        });
        let mut root_profile = json!({
            "path":"/fixture/source.mkv","primary_video":video_4k.clone(),
            "secondary_videos":[video_1080.clone()],"audio":[audio.clone()],"subtitles":[subtitle.clone()],
            "attachments":[attachment.clone()],"data":[data.clone()],
            "source_bit_depth":10,"source_chroma":"4:2:0"
        });
        for (name, value) in hdr_profile.as_object().unwrap() {
            root_profile[name] = value.clone();
        }
        let request = json!({
            "tools":{
                "ffmpeg":"ffmpeg-test",
                "x265_pixel_formats":["yuv420p","yuv420p10le"],
                "x265_dolbyvision":true
            },
            "profile":root_profile,
            "video_profiles":{"0":hdr_profile,"1":sdr_profile},
            "hdr10plus_jsons":{"0":"/tmp/hdr10plus.json"},
            "selected_streams":[video_1080,video_4k,audio,subtitle,attachment,data],
            "encode_video_indices":[0,1],
            "resource_plan":{"worker_threads":8,"frame_threads":2},
            "options":{
                "output":"/fixture/output.mkv","crf":20,"preset":"slow","tune":"animation",
                "preserve_dolbyvision":false,"strip_dolbyvision":false,"test_seconds":null,
                "preserve_source_level":true,"disable_hdr10_opt":false,
                "secondary_video_policy":"encode","data_policy":"preserve",
                "force_deinterlace":false,"disable_deinterlace":false
            }
        });

        let plan = build_plan(&request).expect("selected multi-track plan should be valid");
        let args = plan["args"].as_array().unwrap();
        assert_eq!(plan["output_video_map"], json!({"0":0,"1":1}));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "0:0"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-map" && pair[1] == "0:1"));
        for source_index in 2..=5 {
            assert!(args
                .windows(2)
                .any(|pair| pair[0] == "-map" && pair[1] == format!("0:{source_index}")));
        }
        assert!(args.windows(2).any(|pair| {
            pair[0] == "-x265-params:v:0"
                && pair[1].as_str().unwrap().contains(
                    "dhdr10-info=/tmp/hdr10plus.json:dhdr10-opt=1:pools=8:frame-threads=2",
                )
        }));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v:1" && pair[1] == "libx265"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "copy"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-disposition:s:0" && pair[1] == "forced"));
    }
}
