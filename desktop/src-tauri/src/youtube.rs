//! Rust-owned YouTube output policy, FFmpeg arguments, and MP4 validation.
//!
//! The Python compatibility workflow still owns batch orchestration and report
//! presentation. Rust is the source of truth for sizing, bitrate, HDR routing,
//! encoder arguments, and output acceptance when launched by the desktop app.

use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const STEREO_AUDIO_KBPS: i64 = 384;
const MIN_SOURCE_MULTIPLIER: f64 = 0.5;
const MAX_SOURCE_MULTIPLIER: f64 = 4.0;

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(dispatch) {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize YouTube plan: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<Value, String> {
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|error| format!("Could not read YouTube request: {error}"))?;
    if input.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "YouTube request exceeds {MAX_REQUEST_BYTES} bytes."
        ));
    }
    serde_json::from_slice(&input).map_err(|error| format!("Invalid YouTube request JSON: {error}"))
}

fn dispatch(request: Value) -> Result<Value, String> {
    match request.get("operation").and_then(Value::as_str) {
        Some("policy") => policy_request(&request),
        Some("dimensions") => {
            let profile = required(&request, "profile")?;
            let resolution = required_string(&request, "resolution")?;
            let (width, height) = expected_dimensions(profile, resolution)?;
            Ok(json!({"output_width": width, "output_height": height}))
        }
        Some("color-mode") => {
            let profile = required(&request, "profile")?;
            Ok(json!({"color_mode": upload_color_mode(profile)}))
        }
        Some("args") => build_args(&request),
        Some("validate") => validate_request(&request),
        Some("install") => {
            let temporary = Path::new(required_string(&request, "temporary")?);
            let output = Path::new(required_string(&request, "output")?);
            crate::transactions::install_export_output(temporary, output)?;
            Ok(json!({"installed": true}))
        }
        _ => Err(
            "YouTube operation must be policy, dimensions, color-mode, args, or validate.".into(),
        ),
    }
}

pub(crate) fn expected_dimensions(profile: &Value, resolution: &str) -> Result<(i64, i64), String> {
    let (width, height) = display_dimensions(profile)?;
    let target = match resolution {
        "source" => return Ok((width, height)),
        "1440p" => 1440.0,
        "4k" => 2160.0,
        _ => return Err("Upload resolution must be source, 1440p, or 4k.".into()),
    };
    // YouTube tiers name the short side. Keep the source aspect ratio and never
    // downscale a source that already meets or exceeds the selected tier.
    let tier_side = width.min(height).max(1) as f64;
    let scale = (target / tier_side).max(1.0);
    Ok((even_scaled(width, scale), even_scaled(height, scale)))
}

fn even_scaled(value: i64, scale: f64) -> i64 {
    (((value as f64 * scale / 2.0).round_ties_even() as i64) * 2).max(2)
}

fn display_dimensions(profile: &Value) -> Result<(i64, i64), String> {
    let width = integer_at(profile, &["/video/width", "/primary_video/width"])
        .ok_or("YouTube source profile has no video width.")?;
    let height = integer_at(profile, &["/video/height", "/primary_video/height"])
        .ok_or("YouTube source profile has no video height.")?;
    if width <= 0 || height <= 0 {
        return Err("YouTube source video dimensions must be positive.".into());
    }
    let primary_index = integer_at(profile, &["/primary_stream_index", "/primary_video/index"]);
    let raw = primary_index
        .and_then(|index| {
            profile
                .get("tracks")
                .and_then(Value::as_array)
                .and_then(|tracks| {
                    tracks
                        .iter()
                        .find(|track| track.get("index").and_then(value_i64) == Some(index))
                })
                .and_then(|track| track.get("ffprobe"))
        })
        .or_else(|| profile.get("primary_probe"));
    let rotation = raw
        .and_then(|stream| stream.get("side_data_list"))
        .and_then(Value::as_array)
        .and_then(|side| {
            side.iter().find(|item| {
                item.get("side_data_type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.to_ascii_lowercase().contains("display matrix"))
            })
        })
        .and_then(|item| item.get("rotation"))
        .and_then(value_f64)
        .or_else(|| {
            raw.and_then(|stream| stream.pointer("/tags/rotate"))
                .and_then(value_f64)
        })
        .unwrap_or(0.0)
        .abs()
        % 360.0;
    if (rotation - 90.0).abs().min((rotation - 270.0).abs()) <= 1.0 {
        Ok((height, width))
    } else {
        Ok((width, height))
    }
}

fn primary_probe(profile: &Value) -> Option<&Value> {
    if let Some(probe) = profile.get("primary_probe") {
        return Some(probe);
    }
    let index = integer_at(profile, &["/primary_stream_index", "/primary_video/index"])?;
    profile
        .get("tracks")
        .and_then(Value::as_array)?
        .iter()
        .find(|track| track.get("index").and_then(value_i64) == Some(index))?
        .get("ffprobe")
}

fn frame_rate(profile: &Value) -> f64 {
    let Some(raw) = primary_probe(profile) else {
        return 30.0;
    };
    for key in ["avg_frame_rate", "r_frame_rate"] {
        let Some(value) = raw.get(key).and_then(Value::as_str) else {
            continue;
        };
        let parsed = if let Some((numerator, denominator)) = value.split_once('/') {
            match (numerator.parse::<f64>(), denominator.parse::<f64>()) {
                (Ok(numerator), Ok(denominator)) if denominator != 0.0 => numerator / denominator,
                _ => continue,
            }
        } else if let Ok(value) = value.parse::<f64>() {
            value
        } else {
            continue;
        };
        if parsed.is_finite() && parsed >= 0.0 {
            return parsed;
        }
    }
    30.0
}

pub(crate) fn upload_color_mode(profile: &Value) -> &'static str {
    let transfer = string_at(profile, &["/transfer", "/color_transfer"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hdr = profile.pointer("/hdr");
    let has_hdr_signal = bool_at(hdr, "hdr10")
        || bool_at(hdr, "hdr10plus_hint")
        || bool_at(hdr, "hlg")
        || bool_at(hdr, "dolby_vision")
        || matches!(
            transfer.as_str(),
            "smpte2084" | "arib-std-b67" | "smpte-st-2084" | "hlg"
        )
        || (source_bit_depth(profile) >= 10 && transfer.starts_with("bt2020"));
    if !has_hdr_signal {
        return "sdr";
    }
    if bool_at(hdr, "dolby_vision") {
        return "unsupported-dolby-vision";
    }
    if source_bit_depth(profile) < 10 {
        return "unsupported-bit-depth";
    }
    if string_at(profile, &["/color_primaries"]).is_none_or(|value| value != "bt2020")
        || string_at(profile, &["/color_matrix"]).is_none_or(|value| value != "bt2020nc")
    {
        return "unsupported-color-tags";
    }
    match transfer.as_str() {
        "smpte2084" => "hdr10",
        "arib-std-b67" => "hlg",
        _ => "unsupported-transfer",
    }
}

fn source_bit_depth(profile: &Value) -> i64 {
    integer_at(profile, &["/source_bit_depth", "/video/source_bit_depth"]).unwrap_or(8)
}

pub(crate) fn recommended_video_rate(profile: &Value, resolution: &str) -> Result<i64, String> {
    let (width, height) = expected_dimensions(profile, resolution)?;
    let side = width.min(height);
    let hdr = matches!(upload_color_mode(profile), "hdr10" | "hlg");
    let high_fps = frame_rate(profile) > 30.0;
    Ok(if side >= 4320 {
        // YouTube's 8K guidance is a range (SDR 80-160/120-240 Mbps, HDR
        // 100-200/150-300 Mbps); use the midpoint like the 4K rows do.
        match (hdr, high_fps) {
            (true, true) => 225_000,
            (true, false) => 150_000,
            (false, true) => 180_000,
            (false, false) => 120_000,
        }
    } else if side >= 2160 {
        match (hdr, high_fps) {
            (true, true) => 75_500,
            (true, false) => 50_000,
            (false, true) => 60_500,
            (false, false) => 45_000,
        }
    } else if side >= 1440 {
        match (hdr, high_fps) {
            (true, true) => 30_000,
            (true, false) => 20_000,
            (false, true) => 24_000,
            (false, false) => 16_000,
        }
    } else if side >= 1080 {
        match (hdr, high_fps) {
            (true, true) => 15_000,
            (true, false) => 10_000,
            (false, true) => 12_000,
            (false, false) => 8_000,
        }
    } else if side >= 720 {
        match (hdr, high_fps) {
            (true, true) => 9_500,
            (true, false) => 6_500,
            (false, true) => 7_500,
            (false, false) => 5_000,
        }
    } else if hdr {
        0
    } else if high_fps {
        3_750
    } else {
        2_500
    })
}

fn policy_request(request: &Value) -> Result<Value, String> {
    let profile = required(request, "profile")?;
    let source_bytes = required_u64(request, "source_bytes")?;
    let duration = required_f64(request, "duration_seconds")?;
    let resolution = required_string(request, "resolution")?;
    let quality_mode = required_string(request, "quality_mode")?;
    let multiplier = required_f64(request, "source_multiplier")?;
    estimate(
        profile,
        source_bytes,
        duration,
        resolution,
        quality_mode,
        multiplier,
    )
}

pub(crate) fn estimate(
    profile: &Value,
    source_bytes: u64,
    duration: f64,
    resolution: &str,
    quality_mode: &str,
    source_multiplier: f64,
) -> Result<Value, String> {
    if !matches!(resolution, "source" | "1440p" | "4k") {
        return Err("Upload resolution must be source, 1440p, or 4k.".into());
    }
    if !matches!(quality_mode, "balanced" | "recommended") {
        return Err("YouTube size profile must be balanced or recommended.".into());
    }
    if !source_multiplier.is_finite()
        || (quality_mode == "balanced"
            && !(MIN_SOURCE_MULTIPLIER..=MAX_SOURCE_MULTIPLIER).contains(&source_multiplier))
    {
        return Err("YouTube source-size multiplier must be between 0.5 and 4.0.".into());
    }
    if !duration.is_finite() || duration < 0.0 {
        return Err("YouTube source duration must be a finite non-negative value.".into());
    }
    let recommended = recommended_video_rate(profile, resolution)?;
    let audio_count = integer_at(profile, &["/audio_count"]).unwrap_or(0).max(0);
    let bitrate = if quality_mode == "recommended" || duration <= 0.0 || recommended <= 0 {
        recommended
    } else {
        let average = source_bytes as f64 * 8.0 / duration / 1000.0;
        let budget =
            round_i64(average * source_multiplier - audio_count as f64 * STEREO_AUDIO_KBPS as f64);
        recommended.min(budget.max(250))
    };
    let (width, height) = expected_dimensions(profile, resolution)?;
    let (source_width, source_height) = display_dimensions(profile)?;
    let audio_rate = audio_count.saturating_mul(STEREO_AUDIO_KBPS);
    let output_bytes = (duration * (bitrate.saturating_add(audio_rate)) as f64 * 1000.0 / 8.0
        * 1.12)
        .round_ties_even()
        .max(0.0);
    Ok(json!({
        "color_mode": upload_color_mode(profile),
        "estimated_output_bytes": output_bytes as u64,
        "video_bitrate_kbps": bitrate,
        "recommended_video_bitrate_kbps": recommended,
        "output_width": width,
        "output_height": height,
        "source_width": source_width,
        "source_height": source_height,
        "source_bytes": source_bytes
    }))
}

pub(crate) fn build_args(request: &Value) -> Result<Value, String> {
    let profile = required(request, "profile")?;
    let tools = required(request, "tools")?;
    let ffmpeg = required_string(tools, "ffmpeg")?;
    let source = required_string(request, "source_path")?;
    let output = required_string(request, "output")?;
    let source_path = Path::new(source);
    let output_path = Path::new(output);
    if output_path
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("mp4")
    {
        return Err("The YouTube temporary output path must end in .mp4.".into());
    }
    let source_target = normalized_target(source_path)?;
    let output_target = normalized_target(output_path)?;
    if source_target == output_target {
        return Err("YouTube temporary output must be separate from its source.".into());
    }
    if output_path.exists() {
        return Err(
            "The YouTube temporary output path already exists; it will not be overwritten.".into(),
        );
    }
    let resolution = required_string(request, "resolution")?;
    let bitrate = required_i64(request, "video_bitrate_kbps")?;
    if bitrate <= 0 {
        return Err("YouTube video bitrate must be positive.".into());
    }
    let color_mode = upload_color_mode(profile);
    if !matches!(color_mode, "sdr" | "hdr10" | "hlg") {
        return Err(format!(
            "The source has unsupported YouTube color mode {color_mode}."
        ));
    }
    let sdr_codec = sdr_codec_of(request)?;
    let preset = preset_of(request)?;
    let sdr_hevc = color_mode == "sdr" && sdr_codec == "hevc";
    if sdr_hevc {
        let formats = tools
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .ok_or("The Rust YouTube plan has no FFmpeg x265 capability list.")?;
        if !formats
            .iter()
            .any(|value| value.as_str() == Some("yuv420p"))
        {
            return Err("This FFmpeg does not expose 8-bit libx265 (yuv420p), which the HEVC SDR export requires.".into());
        }
    }
    if matches!(color_mode, "hdr10" | "hlg") {
        let formats = tools
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .ok_or("The Rust YouTube plan has no FFmpeg 10-bit x265 capability list.")?;
        if !formats
            .iter()
            .any(|value| value.as_str() == Some("yuv420p10le"))
        {
            return Err("This FFmpeg does not expose 10-bit libx265 (yuv420p10le), which the HDR export requires.".into());
        }
    }
    let (width, height) = expected_dimensions(profile, resolution)?;
    let (source_width, source_height) = display_dimensions(profile)?;
    let interlaced = profile.get("interlaced").and_then(Value::as_bool) == Some(true);
    let workers = optional_u32(request, "worker_threads", 256)?;
    let frame_threads = optional_u32(request, "frame_threads", 256)?;
    let primary_index = integer_at(profile, &["/primary_stream_index", "/primary_video/index"])
        .ok_or("YouTube source profile has no selected video stream index.")?;
    if primary_index < 0 {
        return Err("YouTube source profile has an invalid selected video stream index.".into());
    }
    let mut args = vec![
        ffmpeg.to_owned(),
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
        source.to_owned(),
        "-map".into(),
        format!("0:{primary_index}"),
        "-map".into(),
        "0:a?".into(),
        "-map_metadata".into(),
        "0".into(),
        "-map_chapters".into(),
        "0".into(),
        "-sn".into(),
        "-dn".into(),
    ];
    let mut filters = Vec::new();
    if interlaced {
        filters.push("bwdif=mode=send_frame:parity=auto:deint=interlaced".to_owned());
    }
    if matches!(resolution, "1440p" | "4k") && (width, height) != (source_width, source_height) {
        filters.push(format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos"
        ));
    }
    if !filters.is_empty() {
        args.extend(["-filter:v:0".into(), filters.join(",")]);
    }
    if matches!(color_mode, "hdr10" | "hlg") {
        args.extend([
            "-c:v:0".into(),
            "libx265".into(),
            "-profile:v:0".into(),
            "main10".into(),
            "-pix_fmt:v:0".into(),
            "yuv420p10le".into(),
            "-preset:v:0".into(),
            preset.clone(),
            "-tag:v:0".into(),
            "hvc1".into(),
        ]);
        let primaries = required_profile_string(profile, "color_primaries")?;
        let transfer = required_profile_string(profile, "transfer")?;
        let matrix = required_profile_string(profile, "color_matrix")?;
        let mut parameters = vec![
            format!("colorprim={primaries}"),
            format!("transfer={transfer}"),
            format!("colormatrix={matrix}"),
        ];
        if let Some(range) = string_at(profile, &["/color_range_x265"]) {
            parameters.push(format!("range={range}"));
        }
        if color_mode == "hdr10" {
            parameters.push("hdr10=1".into());
            if let Some(mastering) = string_at(profile, &["/master_display", "/mastering_display"])
            {
                parameters.push(format!("master-display={mastering}"));
            }
            if let Some(max_cll) = integer_at(profile, &["/max_cll"]) {
                let max_fall = integer_at(profile, &["/max_fall"]).unwrap_or(0);
                parameters.push(format!("max-cll={max_cll},{max_fall}"));
            }
        } else {
            parameters.extend(["atc-sei=18".into(), "pic-struct=0".into()]);
        }
        if let Some(workers) = workers.filter(|value| *value > 0) {
            parameters.push(format!("pools={workers}"));
        }
        if let Some(frame_threads) = frame_threads {
            parameters.push(format!("frame-threads={frame_threads}"));
        }
        args.extend(["-x265-params:v:0".into(), parameters.join(":")]);
    } else if sdr_hevc {
        args.extend([
            "-c:v:0".into(),
            "libx265".into(),
            "-profile:v:0".into(),
            "main".into(),
            "-pix_fmt:v:0".into(),
            "yuv420p".into(),
            "-preset:v:0".into(),
            preset.clone(),
            "-tag:v:0".into(),
            "hvc1".into(),
        ]);
        let mut parameters = vec![
            format!(
                "keyint={}",
                (frame_rate(profile) / 2.0).round_ties_even().max(1.0) as i64
            ),
            "open-gop=0".to_owned(),
        ];
        for (name, field) in [
            ("colorprim", "color_primaries"),
            ("transfer", "transfer"),
            ("colormatrix", "color_matrix"),
        ] {
            if let Some(value) = profile_string(profile, field) {
                parameters.push(format!("{name}={value}"));
            }
        }
        if let Some(range) = string_at(profile, &["/color_range_x265"]) {
            parameters.push(format!("range={range}"));
        }
        if let Some(workers) = workers.filter(|value| *value > 0) {
            parameters.push(format!("pools={workers}"));
        }
        if let Some(frame_threads) = frame_threads {
            parameters.push(format!("frame-threads={frame_threads}"));
        }
        args.extend(["-x265-params:v:0".into(), parameters.join(":")]);
    } else {
        args.extend([
            "-c:v:0".into(),
            "libx264".into(),
            "-profile:v:0".into(),
            "high".into(),
            "-pix_fmt:v:0".into(),
            "yuv420p".into(),
            "-preset:v:0".into(),
            preset.clone(),
            // YouTube's recommended H.264 structure: 2 consecutive B-frames and
            // a closed GOP of half the frame rate.
            "-bf:v:0".into(),
            "2".into(),
            "-g:v:0".into(),
            ((frame_rate(profile) / 2.0).round_ties_even().max(1.0) as i64).to_string(),
            "-flags:v:0".into(),
            "+cgop".into(),
        ]);
    }
    args.extend([
        "-b:v:0".into(),
        format!("{bitrate}k"),
        "-maxrate:v:0".into(),
        format!("{bitrate}k"),
        "-bufsize:v:0".into(),
        format!("{}k", bitrate.saturating_mul(2)),
        "-fps_mode:v:0".into(),
        "passthrough".into(),
        "-threads:v:0".into(),
        workers.unwrap_or(1).max(1).to_string(),
    ]);
    match request.get("audio_plan").and_then(Value::as_array) {
        Some(plan) => args.extend(crate::youtube_optimize::audio_args_from_json(plan)?),
        None => args.extend([
            "-c:a".into(),
            "aac".into(),
            "-profile:a".into(),
            "aac_low".into(),
            "-b:a".into(),
            "384k".into(),
            "-ar".into(),
            "48000".into(),
        ]),
    }
    for (option, field) in [
        ("-color_primaries:v:0", "color_primaries"),
        ("-color_trc:v:0", "transfer"),
        ("-colorspace:v:0", "color_matrix"),
        ("-color_range:v:0", "color_range_ffmpeg"),
    ] {
        if let Some(value) = profile_string(profile, field) {
            args.extend([option.into(), value]);
        }
    }
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        output.into(),
    ]);
    Ok(json!({"args": args, "color_mode": color_mode}))
}

fn sdr_codec_of(request: &Value) -> Result<String, String> {
    match request.get("sdr_codec").and_then(Value::as_str) {
        None | Some("h264") => Ok("h264".into()),
        Some("hevc") => Ok("hevc".into()),
        Some(other) => Err(format!("SDR codec must be h264 or hevc, not {other:?}.")),
    }
}

fn preset_of(request: &Value) -> Result<String, String> {
    match request.get("preset").and_then(Value::as_str) {
        None => Ok("slow".into()),
        Some(value @ ("medium" | "slow" | "slower")) => Ok(value.into()),
        Some(other) => Err(format!(
            "Encoder preset must be medium, slow, or slower, not {other:?}."
        )),
    }
}

pub(crate) fn normalized_target(path: &Path) -> Result<std::path::PathBuf, String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .map_err(|error| format!("Could not resolve YouTube output directory: {error}"))?;
    let name = path
        .file_name()
        .ok_or("YouTube output path has no filename.")?;
    Ok(parent.join(name))
}

pub(crate) fn validate_request(request: &Value) -> Result<Value, String> {
    let profile = required(request, "profile")?;
    let ffprobe = required_string(required(request, "tools")?, "ffprobe")?;
    let output = Path::new(required_string(request, "output")?);
    let metadata = fs::metadata(output)
        .map_err(|error| format!("The MP4 output is missing or cannot be read: {error}"))?;
    if !metadata.is_file() || metadata.len() < 1024 {
        return Err(format!(
            "The MP4 output is missing or too small: {}",
            output.display()
        ));
    }
    let resolution = required_string(request, "resolution")?;
    let probe = crate::media_scan::probe_output(ffprobe, output)?;
    let expect = Expect {
        sdr_codec: sdr_codec_of(request)?,
        video_codec: request
            .get("expected_codec")
            .and_then(Value::as_str)
            .map(str::to_owned),
        allow_copied_audio: request.get("allow_copied_audio").and_then(Value::as_bool)
            == Some(true),
    };
    let (width, height) = validate_output_with(profile, &probe, resolution, &expect)?;
    Ok(json!({"width": width, "height": height}))
}

/// What an output must look like beyond the source-derived color and size rules.
pub(crate) struct Expect {
    /// SDR encode target when the codec is not pinned: `h264` or `hevc`.
    pub(crate) sdr_codec: String,
    /// Pins the video codec (a stream copy keeps the source codec).
    pub(crate) video_codec: Option<String>,
    /// Audio streams copied from the source may keep a YouTube-supported codec.
    pub(crate) allow_copied_audio: bool,
}

impl Default for Expect {
    fn default() -> Self {
        Self {
            sdr_codec: "h264".into(),
            video_codec: None,
            allow_copied_audio: false,
        }
    }
}

#[cfg(test)]
pub(crate) fn validate_output(
    source: &Value,
    probe: &Value,
    resolution: &str,
) -> Result<(i64, i64), String> {
    validate_output_with(source, probe, resolution, &Expect::default())
}

pub(crate) fn validate_output_with(
    source: &Value,
    probe: &Value,
    resolution: &str,
    expect: &Expect,
) -> Result<(i64, i64), String> {
    let color_mode = upload_color_mode(source);
    if !matches!(color_mode, "sdr" | "hdr10" | "hlg") {
        return Err(format!(
            "Cannot validate an upload from unsupported source color mode {color_mode}."
        ));
    }
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or("The upload copy has no usable stream list.")?;
    let videos = streams.iter().filter(|stream| {
        stream.get("codec_type").and_then(Value::as_str) == Some("video")
            && stream
                .pointer("/disposition/attached_pic")
                .and_then(value_i64)
                != Some(1)
    });
    let video = videos
        .into_iter()
        .next()
        .ok_or("The upload copy contains no usable video stream.")?;
    let expected_codec = match (&expect.video_codec, color_mode) {
        (Some(pinned), _) => pinned.as_str(),
        (None, "hdr10" | "hlg") => "hevc",
        (None, _) => expect.sdr_codec.as_str(),
    };
    let codec = video
        .get("codec_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if codec != expected_codec {
        return Err(format!(
            "Upload copy uses {codec:?}; expected {}.",
            expected_codec.to_ascii_uppercase()
        ));
    }
    let width = integer_at(video, &["/width"]).unwrap_or(0);
    let height = integer_at(video, &["/height"]).unwrap_or(0);
    if width <= 0 || height <= 0 {
        return Err("Upload copy has invalid video dimensions.".into());
    }
    let (expected_width, expected_height) = expected_dimensions(source, resolution)?;
    if (width - expected_width).abs() > 2 || (height - expected_height).abs() > 2 {
        return Err(format!(
            "Upload copy dimensions {width}x{height} do not match the selected {resolution} profile; expected about {expected_width}x{expected_height} with source aspect ratio preserved."
        ));
    }
    if matches!(color_mode, "hdr10" | "hlg") {
        let pixel_format = video
            .get("pix_fmt")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !pixel_format.contains("10") && !pixel_format.contains("12") {
            return Err("HDR upload copy is not encoded at 10-bit or greater.".into());
        }
        for (field, expected) in [
            ("color_primaries", "bt2020"),
            (
                "color_transfer",
                if color_mode == "hdr10" {
                    "smpte2084"
                } else {
                    "arib-std-b67"
                },
            ),
            ("color_space", "bt2020nc"),
        ] {
            let actual = video
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "HDR upload copy has {field}={actual}; expected {expected}."
                ));
            }
        }
    }
    let source_audio = integer_at(source, &["/audio_count"]).unwrap_or(0);
    let audio = streams
        .iter()
        .filter(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .collect::<Vec<_>>();
    if source_audio > 0
        && (audio.is_empty()
            || audio.iter().any(|stream| {
                let codec = stream.get("codec_name").and_then(Value::as_str);
                if expect.allow_copied_audio {
                    !codec.is_some_and(crate::youtube_optimize::is_copyable_audio)
                } else {
                    codec != Some("aac")
                }
            }))
    {
        return Err(
            "The source has audio, but the upload copy did not validate as AAC audio.".into(),
        );
    }
    let source_duration = number_at(source, &["/duration_seconds"]).unwrap_or(0.0);
    let actual_duration = probe
        .pointer("/format/duration")
        .and_then(value_f64)
        .unwrap_or(0.0);
    if source_duration > 0.0
        && (actual_duration - source_duration).abs() > 2.0_f64.max(source_duration * 0.005)
    {
        return Err(
            "Upload copy duration differs from the source by more than the validation tolerance."
                .into(),
        );
    }
    let formats = probe
        .pointer("/format/format_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if !formats.split(',').any(|name| matches!(name, "mp4" | "mov")) {
        return Err(format!(
            "Upload copy is not identified as an MP4-family container ({}).",
            formats
        ));
    }
    Ok((width, height))
}

fn required<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
    value
        .get(name)
        .ok_or_else(|| format!("YouTube request is missing {name:?}."))
}

fn required_string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty() && text.len() <= 8192)
        .ok_or_else(|| format!("YouTube request has an invalid {name:?}."))
}

fn required_i64(value: &Value, name: &str) -> Result<i64, String> {
    value
        .get(name)
        .and_then(value_i64)
        .ok_or_else(|| format!("YouTube request has an invalid {name:?}."))
}

fn required_u64(value: &Value, name: &str) -> Result<u64, String> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("YouTube request has an invalid {name:?}."))
}

fn required_f64(value: &Value, name: &str) -> Result<f64, String> {
    value
        .get(name)
        .and_then(value_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("YouTube request has an invalid {name:?}."))
}

fn optional_u32(value: &Value, name: &str, max: u32) -> Result<Option<u32>, String> {
    match value.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number <= max)
            .map(Some)
            .ok_or_else(|| format!("YouTube request has an invalid {name:?}.")),
    }
}

fn integer_at(value: &Value, pointers: &[&str]) -> Option<i64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(value_i64))
}

fn string_at(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn number_at(value: &Value, pointers: &[&str]) -> Option<f64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(value_f64))
}

fn bool_at(value: Option<&Value>, field: &str) -> bool {
    value
        .and_then(|object| object.get(field))
        .and_then(Value::as_bool)
        == Some(true)
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn value_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .filter(|number| number.is_finite())
}

fn round_i64(value: f64) -> i64 {
    value
        .round_ties_even()
        .clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

/// Profile keys for one color field. Native profiles call the transfer
/// characteristic `color_transfer`; the compatibility bridge calls it `transfer`.
fn profile_pointers(field: &str) -> Vec<String> {
    let mut pointers = vec![format!("/{field}")];
    if field == "transfer" {
        pointers.push("/color_transfer".into());
    }
    pointers
}

fn profile_string(profile: &Value, field: &str) -> Option<String> {
    let pointers = profile_pointers(field);
    let refs: Vec<&str> = pointers.iter().map(String::as_str).collect();
    string_at(profile, &refs)
}

/// The skip message for an `unsupported-*` color mode, or `None` when supported.
pub(crate) fn unsupported_color_message(color_mode: &str) -> Option<&'static str> {
    match color_mode {
        "unsupported-dolby-vision" => Some("Dolby Vision requires a dedicated RPU-aware workflow; this export does not preserve Dolby Vision metadata."),
        "unsupported-bit-depth" => Some("HDR upload requires a 10-bit or 12-bit source video stream."),
        "unsupported-color-tags" => Some("HDR upload requires Rec. 2020 primaries and the Rec. 2020 non-constant-luminance matrix."),
        "unsupported-transfer" => Some("HDR upload requires PQ or HLG transfer metadata; this source's HDR transfer tag is missing or unsupported."),
        _ => None,
    }
}

fn required_profile_string(profile: &Value, field: &str) -> Result<String, String> {
    profile_string(profile, field)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("YouTube HDR profile is missing {field}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdr_profile(width: i64, height: i64, fps: &str) -> Value {
        json!({
            "video": {"width": width, "height": height, "source_bit_depth": 8},
            "hdr": {"hdr10": false, "hdr10plus_hint": false, "hlg": false, "dolby_vision": false},
            "audio_count": 1,
            "primary_stream_index": 0,
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": fps, "side_data_list": []}}],
            "duration_seconds": 3600.0,
            "source_bit_depth": 8,
            "interlaced": false
        })
    }

    fn hdr_profile() -> Value {
        json!({
            "video": {"width": 3840, "height": 2160, "source_bit_depth": 10},
            "hdr": {"hdr10": true, "hdr10plus_hint": true, "hlg": false, "dolby_vision": false},
            "audio_count": 1,
            "primary_stream_index": 0,
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "60/1", "side_data_list": []}}],
            "duration_seconds": 600.0,
            "source_bit_depth": 10,
            "color_primaries": "bt2020",
            "transfer": "smpte2084",
            "color_matrix": "bt2020nc",
            "color_range_x265": "limited",
            "color_range_ffmpeg": "tv",
            "master_display": "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,50)",
            "max_cll": 1000,
            "max_fall": 400,
            "interlaced": false
        })
    }

    #[test]
    fn policy_matches_source_aware_1440p_budget_without_downscaling() {
        let profile = sdr_profile(1280, 720, "24/1");
        assert_eq!(
            expected_dimensions(&profile, "1440p").unwrap(),
            (2560, 1440)
        );
        let estimate = estimate(&profile, 80_000_000, 3600.0, "1440p", "balanced", 2.0).unwrap();
        assert_eq!(estimate["video_bitrate_kbps"], 250);
        assert!(estimate["estimated_output_bytes"].as_u64().unwrap() < 1_000_000_000);
        let already_4k = sdr_profile(3840, 2160, "24/1");
        assert_eq!(
            expected_dimensions(&already_4k, "1440p").unwrap(),
            (3840, 2160)
        );
    }

    #[test]
    fn hdr_policy_uses_main10_and_keeps_source_color_mastering_data() {
        let profile = hdr_profile();
        assert_eq!(upload_color_mode(&profile), "hdr10");
        assert_eq!(recommended_video_rate(&profile, "4k").unwrap(), 75_500);
        let request = json!({
            "profile": profile, "tools": {"ffmpeg":"ffmpeg","x265_pixel_formats":["yuv420p10le"]},
            "source_path":"source.mkv", "output":"output.mp4", "resolution":"4k",
            "video_bitrate_kbps": 75_500, "worker_threads": 8, "frame_threads": 0
        });
        let plan = build_args(&request).unwrap();
        let args = plan["args"].as_array().unwrap();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-c:v:0" && pair[1] == "libx265"));
        assert!(args.windows(2).any(|pair| pair[0] == "-x265-params:v:0"
            && pair[1].as_str().unwrap().contains("master-display=")));
        assert!(args.windows(2).any(|pair| pair[0] == "-x265-params:v:0"
            && pair[1]
                .as_str()
                .unwrap()
                .contains("max-cll=1000,400:pools=8:frame-threads=0")));
    }

    #[test]
    fn eight_k_uses_the_eight_k_bitrate_tier_not_the_four_k_one() {
        let sdr = sdr_profile(7680, 4320, "30/1");
        assert_eq!(recommended_video_rate(&sdr, "source").unwrap(), 120_000);
        let fast = sdr_profile(7680, 4320, "60/1");
        assert_eq!(recommended_video_rate(&fast, "source").unwrap(), 180_000);
        let hdr = json!({
            "video": {"width": 7680, "height": 4320, "source_bit_depth": 10},
            "hdr": {"hdr10": true}, "color_primaries": "bt2020",
            "transfer": "smpte2084", "color_matrix": "bt2020nc",
            "primary_stream_index": 0,
            "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "24/1"}}]
        });
        assert_eq!(recommended_video_rate(&hdr, "source").unwrap(), 150_000);
    }

    #[test]
    fn sdr_h264_follows_youtube_gop_guidance() {
        let request = json!({
            "profile": sdr_profile(2560, 1440, "60/1"),
            "tools": {"ffmpeg": "ffmpeg"},
            "source_path": "source.mkv", "output": "output.mp4",
            "resolution": "source", "video_bitrate_kbps": 24_000
        });
        let plan = build_args(&request).unwrap();
        let args = plan["args"].as_array().unwrap();
        let has = |key: &str, value: &str| {
            args.windows(2)
                .any(|pair| pair[0] == key && pair[1] == value)
        };
        assert!(has("-c:v:0", "libx264"));
        assert!(has("-bf:v:0", "2"));
        assert!(has("-g:v:0", "30"));
        assert!(has("-flags:v:0", "+cgop"));
    }

    #[test]
    fn unsupported_hdr_routes_fail_closed_instead_of_becoming_sdr() {
        let mut profile = hdr_profile();
        profile["hdr"]["dolby_vision"] = json!(true);
        assert_eq!(upload_color_mode(&profile), "unsupported-dolby-vision");
        let mut profile = hdr_profile();
        profile["color_matrix"] = json!("bt709");
        assert_eq!(upload_color_mode(&profile), "unsupported-color-tags");
    }

    #[test]
    fn validates_hdr_mp4_streams_and_rejects_bad_color_tags() {
        let source = hdr_profile();
        let output = json!({
            "streams": [
                {"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,
                 "pix_fmt":"yuv420p10le","color_primaries":"bt2020","color_transfer":"smpte2084","color_space":"bt2020nc"},
                {"codec_type":"audio","codec_name":"aac"}
            ],
            "format":{"duration":"600.0","format_name":"mov,mp4,m4a,3gp,3g2,mj2"}
        });
        assert_eq!(
            validate_output(&source, &output, "4k").unwrap(),
            (3840, 2160)
        );
        let mut corrupt = output.clone();
        corrupt["streams"][0]["color_transfer"] = json!("bt709");
        assert!(validate_output(&source, &corrupt, "4k")
            .unwrap_err()
            .contains("color_transfer"));
    }
}
