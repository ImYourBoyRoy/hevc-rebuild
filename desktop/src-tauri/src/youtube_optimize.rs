//! Lossless-first YouTube "Optimize" planning for sources already at 1440p+.
//!
//! Upscale mode raises small sources into a higher YouTube tier. Optimize mode
//! never resizes: a 1440p/4K/8K source is stream-copied into a faststart MP4
//! with YouTube-friendly tags whenever possible (video packets untouched), and
//! re-encoded only when the file is far above YouTube's recommended bitrate or
//! cannot be copied cleanly. A re-encode is accepted only after a sampled SSIM
//! quality check against the source; otherwise the copy is used instead.
//!
//! Pure policy lives here (`plan`, `copy_args`, `sample_windows`,
//! `parse_ssim_all`); `ssim_gate` is the only function that runs FFmpeg. The
//! per-file orchestration is in `youtube_run.rs`.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::youtube;

/// YouTube's short-side tier floor for this mode (2K / QHD).
pub(crate) const MIN_SHORT_SIDE: i64 = 1440;
/// `auto` re-encodes only when the source runs this far above the guidance.
pub(crate) const AUTO_OVERSIZE_RATIO: f64 = 1.5;
/// `aggressive` re-encodes anything above the guidance.
pub(crate) const AGGRESSIVE_OVERSIZE_RATIO: f64 = 1.0;
/// A re-encode must save at least this fraction of the input size to be kept.
pub(crate) const MIN_SAVING_FRACTION: f64 = 0.10;
const SAMPLE_SECONDS: f64 = 6.0;
const SHORT_CLIP_SECONDS: f64 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shrink {
    Off,
    Auto,
    Aggressive,
}

impl Shrink {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "aggressive" => Ok(Self::Aggressive),
            other => Err(format!(
                "YouTube shrink mode must be off, auto, or aggressive, not {other:?}."
            )),
        }
    }

    fn ratio(self) -> Option<f64> {
        match self {
            Self::Off => None,
            Self::Auto => Some(AUTO_OVERSIZE_RATIO),
            Self::Aggressive => Some(AGGRESSIVE_OVERSIZE_RATIO),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QualityCheck {
    Off,
    Relaxed,
    Standard,
    Strict,
}

impl QualityCheck {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(Self::Off),
            "relaxed" => Ok(Self::Relaxed),
            "standard" => Ok(Self::Standard),
            "strict" => Ok(Self::Strict),
            other => Err(format!(
                "YouTube quality check must be off, relaxed, standard, or strict, not {other:?}."
            )),
        }
    }

    /// Minimum acceptable SSIM (All planes) across the sampled windows.
    pub(crate) fn floor(self) -> Option<f64> {
        match self {
            Self::Off => None,
            Self::Relaxed => Some(0.975),
            Self::Standard => Some(0.985),
            Self::Strict => Some(0.99),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AudioAction {
    Copy,
    Transcode { kbps: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Copy,
    Encode,
}

pub(crate) struct PlanInput<'a> {
    pub shrink: Shrink,
    pub sdr_codec: &'a str,
    pub hdr10plus_loss_accepted: bool,
    /// When true, a re-encode can keep HDR10+ via post-encode hdr10plus_tool inject.
    pub hdr10plus_can_inject: bool,
    pub x265_pixel_formats: &'a [String],
    pub file_bytes: u64,
    pub duration: f64,
}

#[derive(Debug)]
pub(crate) struct OptimizePlan {
    pub color_mode: &'static str,
    pub action: Action,
    /// A stream copy is possible, so a rejected re-encode can fall back to it.
    pub copy_fallback: bool,
    pub reason: String,
    pub encode_bitrate_kbps: i64,
    pub recommended_kbps: i64,
    pub source_video_kbps: Option<f64>,
    pub source_codec: String,
    pub audio: Vec<AudioAction>,
    pub notes: Vec<String>,
}

pub(crate) fn is_copyable_audio(codec: &str) -> bool {
    matches!(codec, "aac" | "opus" | "mp3" | "ac3" | "eac3")
}

/// Relative compression efficiency against H.264 at the same visual quality.
fn codec_efficiency(codec: &str) -> f64 {
    match codec {
        "hevc" => 0.7,
        "vp9" => 0.75,
        "av1" => 0.55,
        _ => 1.0,
    }
}

fn missing_tag(value: Option<&str>) -> bool {
    value.is_none_or(|text| {
        let text = text.trim().to_ascii_lowercase();
        text.is_empty() || text == "unknown" || text == "unspecified"
    })
}

fn number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn text<'a>(profile: &'a Value, pointer: &str) -> Option<&'a str> {
    profile.pointer(pointer).and_then(Value::as_str)
}

fn tracks(profile: &Value) -> &[Value] {
    profile
        .get("tracks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn primary_track(profile: &Value) -> Option<&Value> {
    let index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)?;
    tracks(profile)
        .iter()
        .find(|track| track.get("index").and_then(Value::as_i64) == Some(index))
}

fn audio_tracks(profile: &Value) -> Vec<&Value> {
    tracks(profile)
        .iter()
        .filter(|track| track.get("type").and_then(Value::as_str) == Some("audio"))
        .collect()
}

fn audio_kbps_for_channels(channels: Option<i64>) -> i64 {
    match channels {
        Some(1) => 128,
        Some(count) if count >= 6 => 512,
        _ => 384,
    }
}

/// Average video bitrate: the stream's own rate when present, otherwise the file
/// average minus the audio streams' estimated share.
fn source_video_kbps(profile: &Value, file_bytes: u64, duration: f64) -> Option<f64> {
    if let Some(rate) = primary_track(profile)
        .and_then(|track| number(track.get("bit_rate")))
        .filter(|rate| *rate > 0.0)
    {
        return Some(rate / 1000.0);
    }
    if duration <= 0.0 || file_bytes == 0 {
        return None;
    }
    let audio: f64 = audio_tracks(profile)
        .iter()
        .map(|track| {
            number(track.get("bit_rate"))
                .map(|rate| rate / 1000.0)
                .unwrap_or_else(|| {
                    audio_kbps_for_channels(track.get("channels").and_then(Value::as_i64)) as f64
                })
        })
        .sum();
    let average = file_bytes as f64 * 8.0 / duration / 1000.0;
    Some((average - audio).max(1.0))
}

fn copy_blockers(profile: &Value, codec: &str, color_mode: &str) -> Vec<String> {
    let mut blockers = Vec::new();
    if !matches!(codec, "h264" | "hevc" | "av1" | "vp9") {
        blockers.push(format!(
            "{} video cannot be stream-copied into an MP4 that YouTube reads reliably",
            if codec.is_empty() { "unknown" } else { codec }
        ));
    }
    if profile.get("interlaced").and_then(Value::as_bool) == Some(true) {
        blockers.push("interlaced video must be deinterlaced before upload".into());
    }
    let pixel_format = primary_track(profile)
        .and_then(|track| track.get("pixel_format"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !pixel_format.is_empty()
        && !matches!(
            pixel_format,
            "yuv420p" | "yuvj420p" | "yuv420p10le" | "yuv420p12le"
        )
    {
        blockers.push(format!(
            "{pixel_format} is not the 4:2:0 chroma YouTube recommends"
        ));
    }
    if color_mode != "sdr" && codec == "h264" {
        blockers.push("HDR uploads need HEVC, VP9, or AV1 video".into());
    }
    blockers
}

fn encoder_ready(hdr: bool, sdr_codec: &str, formats: &[String]) -> Result<(), String> {
    let (needed, label) = if hdr {
        (
            "yuv420p10le",
            "10-bit libx265 (yuv420p10le), which the HDR re-encode requires",
        )
    } else if sdr_codec == "hevc" {
        (
            "yuv420p",
            "8-bit libx265 (yuv420p), which the HEVC SDR re-encode requires",
        )
    } else {
        return Ok(());
    };
    if formats.iter().any(|name| name == needed) {
        Ok(())
    } else {
        Err(format!("This FFmpeg does not expose {label}."))
    }
}

/// Decide copy versus re-encode for one source. `Err` is a SKIPPED message.
pub(crate) fn plan(profile: &Value, input: &PlanInput<'_>) -> Result<OptimizePlan, String> {
    let (width, height) = youtube::expected_dimensions(profile, "source")?;
    if width.min(height) < MIN_SHORT_SIDE {
        return Err(format!(
            "{width}x{height} is below 1440p; use Upscale mode to raise its resolution for YouTube."
        ));
    }
    let color_mode = youtube::upload_color_mode(profile);
    if let Some(message) = youtube::unsupported_color_message(color_mode) {
        return Err(message.into());
    }
    let hdr = color_mode != "sdr";
    let codec = text(profile, "/video/codec")
        .unwrap_or("")
        .to_ascii_lowercase();
    let recommended = youtube::recommended_video_rate(profile, "source")?;
    let source_kbps = source_video_kbps(profile, input.file_bytes, input.duration);
    let blockers = copy_blockers(profile, &codec, color_mode);
    let copy_ok = blockers.is_empty();
    if !copy_ok && input.shrink == Shrink::Off {
        return Err(format!(
            "This source cannot be copied as-is ({}) and shrinking is off; enable shrinking to re-encode it.",
            blockers.join("; ")
        ));
    }

    // The YouTube table is H.264 guidance for SDR and HEVC-class for HDR.
    let target_efficiency = if hdr {
        1.0
    } else {
        codec_efficiency(input.sdr_codec)
    };
    let source_equivalent = source_kbps.map(|rate| {
        if hdr {
            rate
        } else {
            rate / codec_efficiency(&codec)
        }
    });
    let oversize = recommended > 0
        && input.shrink.ratio().is_some_and(|ratio| {
            source_equivalent.is_some_and(|rate| rate >= ratio * recommended as f64)
        });
    let mut encode_kbps = ((recommended as f64) * target_efficiency).round() as i64;
    if let Some(rate) = source_kbps {
        if rate < encode_kbps as f64 {
            encode_kbps = rate.round() as i64;
        }
    }
    let encode_kbps = encode_kbps.max(250);

    let mut notes = Vec::new();
    let mut action = if !copy_ok || oversize {
        Action::Encode
    } else {
        Action::Copy
    };
    let mut reason = if copy_ok {
        match source_kbps {
            Some(rate) => format!(
                "stream copy: {} at {:.1} Mb/s is within the range YouTube re-encodes well (guidance {:.1} Mb/s)",
                codec.to_uppercase(),
                rate / 1000.0,
                recommended as f64 / 1000.0
            ),
            None => format!("stream copy: {} video kept untouched", codec.to_uppercase()),
        }
    } else {
        format!("re-encode needed: {}", blockers.join("; "))
    };
    if action == Action::Encode {
        if let Err(message) = encoder_ready(hdr, input.sdr_codec, input.x265_pixel_formats) {
            if copy_ok {
                notes.push(format!("Shrinking was skipped: {message}"));
                action = Action::Copy;
            } else {
                return Err(message);
            }
        }
    }
    if action == Action::Encode
        && hdr
        && profile
            .pointer("/hdr/hdr10plus_hint")
            .and_then(Value::as_bool)
            == Some(true)
    {
        if input.hdr10plus_can_inject {
            notes.push(
                "HDR10+ will be preserved after re-encode with hdr10plus_tool inject.".into(),
            );
        } else if input.hdr10plus_loss_accepted {
            notes.push("HDR10+ dynamic metadata was not retained after explicit per-file approval; static HDR10/PQ signaling is retained.".into());
        } else if copy_ok {
            notes.push("HDR10+ dynamic metadata is kept by the stream copy; shrinking was skipped because a re-encode would drop it.".into());
            action = Action::Copy;
        } else {
            return Err("HDR10+ dynamic metadata cannot be retained by a re-encode without hdr10plus_tool, and this source cannot be stream-copied. No copy was written; install the helper, or explicitly accept this loss for the individual file.".into());
        }
    }
    if action == Action::Encode && copy_ok {
        reason = format!(
            "re-encode: {} at {:.1} Mb/s is {:.1}x YouTube's {:.1} Mb/s guidance",
            codec.to_uppercase(),
            source_kbps.unwrap_or(0.0) / 1000.0,
            source_equivalent.unwrap_or(0.0) / recommended.max(1) as f64,
            recommended as f64 / 1000.0
        );
    }

    let audio = audio_tracks(profile)
        .iter()
        .map(|track| {
            let codec = track
                .get("codec")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if is_copyable_audio(&codec) {
                AudioAction::Copy
            } else {
                AudioAction::Transcode {
                    kbps: audio_kbps_for_channels(track.get("channels").and_then(Value::as_i64)),
                }
            }
        })
        .collect::<Vec<_>>();
    if audio
        .iter()
        .any(|action| matches!(action, AudioAction::Transcode { .. }))
    {
        notes.push("Audio tracks YouTube does not read reliably in MP4 were converted to AAC; the rest were copied.".into());
    }

    Ok(OptimizePlan {
        color_mode,
        action,
        copy_fallback: copy_ok,
        reason,
        encode_bitrate_kbps: encode_kbps,
        recommended_kbps: recommended,
        source_video_kbps: source_kbps,
        source_codec: codec,
        audio,
        notes,
    })
}

/// FFmpeg per-stream audio arguments for the output audio streams, in source order.
pub(crate) fn audio_args(plan: &[AudioAction]) -> Vec<String> {
    let mut args = Vec::new();
    for (index, action) in plan.iter().enumerate() {
        match action {
            AudioAction::Copy => args.extend([format!("-c:a:{index}"), "copy".into()]),
            AudioAction::Transcode { kbps } => args.extend([
                format!("-c:a:{index}"),
                "aac".into(),
                format!("-profile:a:{index}"),
                "aac_low".into(),
                format!("-b:a:{index}"),
                format!("{kbps}k"),
                format!("-ar:a:{index}"),
                "48000".into(),
            ]),
        }
    }
    args
}

pub(crate) fn audio_plan_json(plan: &[AudioAction]) -> Value {
    Value::Array(
        plan.iter()
            .map(|action| match action {
                AudioAction::Copy => serde_json::json!({"copy": true}),
                AudioAction::Transcode { kbps } => serde_json::json!({"kbps": kbps}),
            })
            .collect(),
    )
}

pub(crate) fn audio_args_from_json(plan: &[Value]) -> Result<Vec<String>, String> {
    let mut actions = Vec::with_capacity(plan.len());
    for entry in plan {
        if entry.get("copy").and_then(Value::as_bool) == Some(true) {
            actions.push(AudioAction::Copy);
        } else if let Some(kbps) = entry
            .get("kbps")
            .and_then(Value::as_i64)
            .filter(|kbps| (32..=1024).contains(kbps))
        {
            actions.push(AudioAction::Transcode { kbps });
        } else {
            return Err(
                "YouTube audio plan entries must be {copy: true} or {kbps: 32-1024}.".into(),
            );
        }
    }
    Ok(audio_args(&actions))
}

/// Container-level color tags for a stream copy. SDR tags that are unspecified
/// are filled with BT.709 (what YouTube assumes) only when nothing contradicts it.
fn copy_color_tags(profile: &Value, color_mode: &str) -> Vec<String> {
    let field = |name: &str| -> Option<String> {
        let pointers: &[&str] = if name == "transfer" {
            &["/transfer", "/color_transfer"]
        } else {
            &[]
        };
        let direct = format!("/{name}");
        pointers
            .iter()
            .copied()
            .chain(std::iter::once(direct.as_str()))
            .find_map(|pointer| text(profile, pointer))
            .map(str::to_owned)
    };
    let mut primaries = field("color_primaries");
    let mut transfer = field("transfer");
    let mut matrix = field("color_matrix");
    if color_mode == "sdr" {
        let specified: Vec<&str> = [&primaries, &transfer, &matrix]
            .into_iter()
            .filter(|value| !missing_tag(value.as_deref()))
            .filter_map(|value| value.as_deref())
            .collect();
        if specified
            .iter()
            .all(|value| value.eq_ignore_ascii_case("bt709"))
        {
            for slot in [&mut primaries, &mut transfer, &mut matrix] {
                if missing_tag(slot.as_deref()) {
                    *slot = Some("bt709".into());
                }
            }
        }
    }
    let mut args = Vec::new();
    for (option, value) in [
        ("-color_primaries:v:0", primaries),
        ("-color_trc:v:0", transfer),
        ("-colorspace:v:0", matrix),
        ("-color_range:v:0", field("color_range_ffmpeg")),
    ] {
        if let Some(value) = value.filter(|value| !missing_tag(Some(value))) {
            args.extend([option.into(), value]);
        }
    }
    args
}

/// FFmpeg arguments for a lossless-first stream copy into a faststart MP4.
pub(crate) fn copy_args(
    ffmpeg: &str,
    source: &str,
    output: &str,
    profile: &Value,
    plan: &OptimizePlan,
) -> Result<Vec<String>, String> {
    let source_path = Path::new(source);
    let output_path = Path::new(output);
    if output_path.extension().and_then(|value| value.to_str()) != Some("mp4") {
        return Err("The YouTube temporary output path must end in .mp4.".into());
    }
    if youtube::normalized_target(source_path)? == youtube::normalized_target(output_path)? {
        return Err("YouTube temporary output must be separate from its source.".into());
    }
    if output_path.exists() {
        return Err(
            "The YouTube temporary output path already exists; it will not be overwritten.".into(),
        );
    }
    let primary_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .filter(|index| *index >= 0)
        .ok_or("YouTube source profile has no selected video stream index.")?;
    let mut args: Vec<String> = [
        ffmpeg,
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-nostats",
        "-stats_period",
        "0.5",
        "-progress",
        "pipe:1",
        "-i",
        source,
        "-map",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(format!("0:{primary_index}"));
    args.extend(
        [
            "-map",
            "0:a?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
            "-sn",
            "-dn",
            "-c:v:0",
            "copy",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    if plan.source_codec == "hevc" {
        args.extend(["-tag:v:0".into(), "hvc1".into()]);
    }
    args.extend(copy_color_tags(profile, plan.color_mode));
    args.extend(audio_args(&plan.audio));
    args.extend(
        ["-movflags", "+faststart", "-f", "mp4", output]
            .into_iter()
            .map(str::to_owned),
    );
    Ok(args)
}

// ------------------------------------------------------------ quality check

/// Start offsets (seconds) of the windows compared by the SSIM check: the whole
/// clip when short, otherwise three windows near 15%, 50%, and 85%.
pub(crate) fn sample_windows(duration: f64) -> Vec<(f64, f64)> {
    if !duration.is_finite() || duration <= 0.0 {
        return vec![(0.0, SAMPLE_SECONDS)];
    }
    if duration <= SHORT_CLIP_SECONDS {
        return vec![(0.0, duration)];
    }
    [0.15, 0.5, 0.85]
        .into_iter()
        .map(|fraction| {
            let start =
                (duration * fraction - SAMPLE_SECONDS / 2.0).clamp(0.0, duration - SAMPLE_SECONDS);
            (start, SAMPLE_SECONDS)
        })
        .collect()
}

/// Pull the `All:` value from an FFmpeg SSIM summary line.
pub(crate) fn parse_ssim_all(line: &str) -> Option<f64> {
    let rest = line.split("SSIM ").nth(1)?;
    let value = rest.split("All:").nth(1)?;
    value
        .split(|character: char| character.is_whitespace() || character == '(')
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

pub(crate) enum GateError {
    Interrupted,
    Failed(String),
}

/// Lowest SSIM across the sample windows of `source` versus `output`. Both are
/// decoded at the same size and converted to the same 10-bit pixel format.
pub(crate) fn ssim_gate(
    ffmpeg: &str,
    source: &Path,
    output: &Path,
    source_stream_index: i64,
    duration: f64,
    mut on_child: impl FnMut(Option<u32>),
) -> Result<f64, GateError> {
    let mut lowest: Option<f64> = None;
    for (start, length) in sample_windows(duration) {
        let filter = format!(
            "[0:{source_stream_index}]format=yuv420p10le,setpts=PTS-STARTPTS[a];\
             [1:v:0]format=yuv420p10le,setpts=PTS-STARTPTS[b];[a][b]ssim"
        );
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"])
            .args([
                "-ss",
                &format!("{start:.3}"),
                "-t",
                &format!("{length:.3}"),
                "-i",
            ])
            .arg(source)
            .args([
                "-ss",
                &format!("{start:.3}"),
                "-t",
                &format!("{length:.3}"),
                "-i",
            ])
            .arg(output)
            .args([
                "-filter_complex",
                &filter,
                "-an",
                "-sn",
                "-dn",
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = crate::process_output::spawn_owned(&mut command).map_err(|error| {
            GateError::Failed(format!("Could not start the SSIM check: {error}"))
        })?;
        on_child(Some(child.id()));
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| GateError::Failed("Could not read the SSIM check output.".into()))?;
        let reader = std::thread::spawn(move || {
            let mut ssim = None;
            let mut tail: Vec<String> = Vec::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(value) = parse_ssim_all(&line) {
                    ssim = Some(value);
                }
                tail.push(line);
                if tail.len() > 12 {
                    tail.remove(0);
                }
            }
            (ssim, tail)
        });
        let status = child.wait();
        on_child(None);
        let (ssim, tail) = reader.join().map_err(|_| {
            GateError::Failed("The SSIM output reader stopped unexpectedly.".into())
        })?;
        let status = status.map_err(|error| {
            GateError::Failed(format!("The SSIM check did not finish: {error}"))
        })?;
        if status.code().is_none() {
            return Err(GateError::Interrupted);
        }
        if !status.success() {
            return Err(GateError::Failed(format!(
                "The SSIM check failed: {}",
                tail.join(" | ")
            )));
        }
        let value = ssim.ok_or_else(|| {
            GateError::Failed("The SSIM check produced no similarity score.".into())
        })?;
        lowest = Some(lowest.map_or(value, |current| current.min(value)));
    }
    lowest.ok_or_else(|| GateError::Failed("The SSIM check had nothing to compare.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(codec: &str, width: i64, height: i64, kbps: Option<i64>, audio: &str) -> Value {
        json!({
            "video": {"codec": codec, "width": width, "height": height, "source_bit_depth": 8},
            "hdr": {"hdr10": false, "hdr10plus_hint": false, "hlg": false, "dolby_vision": false},
            "audio_count": 1, "subtitle_count": 0, "duration_seconds": 100.0,
            "interlaced": false, "primary_stream_index": 0,
            "color_primaries": "bt709", "color_transfer": "bt709", "color_matrix": "bt709",
            "tracks": [
                {"index": 0, "type": "video", "codec": codec, "pixel_format": "yuv420p",
                 "bit_rate": kbps.map(|value| json!((value * 1000).to_string())).unwrap_or(Value::Null),
                 "frame_rate": "30/1"},
                {"index": 1, "type": "audio", "codec": audio, "channels": 2, "bit_rate": "192000"}
            ]
        })
    }

    fn input(shrink: Shrink) -> PlanInput<'static> {
        PlanInput {
            shrink,
            sdr_codec: "h264",
            hdr10plus_loss_accepted: false,
            hdr10plus_can_inject: false,
            x265_pixel_formats: &[],
            file_bytes: 0,
            duration: 100.0,
        }
    }

    #[test]
    fn healthy_4k_h264_is_copied_untouched() {
        let source = profile("h264", 3840, 2160, Some(40_000), "aac");
        let plan = plan(&source, &input(Shrink::Auto)).unwrap();
        assert_eq!(plan.action, Action::Copy);
        assert_eq!(plan.audio, vec![AudioAction::Copy]);
        assert!(plan.reason.starts_with("stream copy"));
    }

    #[test]
    fn oversized_source_is_reencoded_only_when_shrinking_is_enabled() {
        let source = profile("h264", 3840, 2160, Some(150_000), "aac");
        assert_eq!(
            plan(&source, &input(Shrink::Off)).unwrap().action,
            Action::Copy
        );
        let auto = plan(&source, &input(Shrink::Auto)).unwrap();
        assert_eq!(auto.action, Action::Encode);
        assert_eq!(auto.encode_bitrate_kbps, 45_000);
        assert!(auto.copy_fallback);
        let near = profile("h264", 3840, 2160, Some(60_000), "aac");
        assert_eq!(
            plan(&near, &input(Shrink::Auto)).unwrap().action,
            Action::Copy
        );
        assert_eq!(
            plan(&near, &input(Shrink::Aggressive)).unwrap().action,
            Action::Encode
        );
    }

    #[test]
    fn efficient_codecs_are_compared_in_h264_equivalent_bitrate() {
        // 60 Mb/s HEVC is roughly 86 Mb/s of H.264; auto (1.5x of 45) = 67.5.
        let hevc = profile("hevc", 3840, 2160, Some(60_000), "aac");
        assert_eq!(
            plan(&hevc, &input(Shrink::Auto)).unwrap().action,
            Action::Encode
        );
        let modest = profile("hevc", 3840, 2160, Some(40_000), "aac");
        assert_eq!(
            plan(&modest, &input(Shrink::Auto)).unwrap().action,
            Action::Copy
        );
        let mut hevc_target = input(Shrink::Auto);
        hevc_target.sdr_codec = "hevc";
        hevc_target.x265_pixel_formats = &[];
        let formats = vec!["yuv420p".to_string()];
        hevc_target.x265_pixel_formats = &formats;
        let planned = plan(&hevc, &hevc_target).unwrap();
        assert_eq!(planned.encode_bitrate_kbps, 31_500);
    }

    #[test]
    fn uncopyable_sources_need_shrinking_and_unknown_audio_is_transcoded() {
        let mut prores = profile("prores", 3840, 2160, Some(700_000), "pcm_s24le");
        prores["tracks"][1]["channels"] = json!(6);
        assert!(plan(&prores, &input(Shrink::Off)).is_err());
        let planned = plan(&prores, &input(Shrink::Auto)).unwrap();
        assert_eq!(planned.action, Action::Encode);
        assert!(!planned.copy_fallback);
        assert_eq!(planned.audio, vec![AudioAction::Transcode { kbps: 512 }]);
        assert!(planned.reason.starts_with("re-encode needed"));
        let mut interlaced = profile("h264", 3840, 2160, Some(30_000), "aac");
        interlaced["interlaced"] = json!(true);
        assert!(plan(&interlaced, &input(Shrink::Off)).is_err());
    }

    #[test]
    fn below_1440p_and_dolby_vision_are_skipped() {
        let small = profile("h264", 1920, 1080, Some(8_000), "aac");
        assert!(plan(&small, &input(Shrink::Auto))
            .unwrap_err()
            .contains("Upscale"));
        let mut dolby = profile("hevc", 3840, 2160, Some(40_000), "aac");
        dolby["hdr"]["dolby_vision"] = json!(true);
        dolby["color_transfer"] = json!("smpte2084");
        assert!(plan(&dolby, &input(Shrink::Auto))
            .unwrap_err()
            .contains("Dolby Vision"));
    }

    fn hdr_source(kbps: i64, hdr10plus: bool) -> Value {
        let mut source = profile("hevc", 3840, 2160, Some(kbps), "eac3");
        source["video"]["source_bit_depth"] = json!(10);
        source["hdr"]["hdr10"] = json!(true);
        source["hdr"]["hdr10plus_hint"] = json!(hdr10plus);
        source["color_primaries"] = json!("bt2020");
        source["color_transfer"] = json!("smpte2084");
        source["color_matrix"] = json!("bt2020nc");
        source["tracks"][0]["pixel_format"] = json!("yuv420p10le");
        source
    }

    #[test]
    fn hdr10plus_is_kept_by_copy_and_never_dropped_silently() {
        let formats = vec!["yuv420p10le".to_string()];
        let mut options = input(Shrink::Auto);
        options.x265_pixel_formats = &formats;
        let huge = hdr_source(200_000, true);
        let planned = plan(&huge, &options).unwrap();
        assert_eq!(planned.action, Action::Copy);
        assert!(planned.notes.iter().any(|note| note.contains("HDR10+")));
        options.hdr10plus_loss_accepted = true;
        assert_eq!(plan(&huge, &options).unwrap().action, Action::Encode);
        options.hdr10plus_loss_accepted = false;
        options.hdr10plus_can_inject = true;
        let injected = plan(&huge, &options).unwrap();
        assert_eq!(injected.action, Action::Encode);
        assert!(injected
            .notes
            .iter()
            .any(|note| note.contains("hdr10plus_tool inject")));
        options.hdr10plus_can_inject = false;
        let plain = hdr_source(200_000, false);
        let planned = plan(&plain, &options).unwrap();
        assert_eq!(planned.action, Action::Encode);
        assert_eq!(planned.encode_bitrate_kbps, 50_000);
        assert_eq!(planned.audio, vec![AudioAction::Copy]);
        let missing = input(Shrink::Auto);
        assert_eq!(plan(&plain, &missing).unwrap().action, Action::Copy);
    }

    #[test]
    fn copy_arguments_tag_hevc_fill_bt709_and_plan_audio_per_stream() {
        let directory = std::env::temp_dir().join(format!("yt-optimize-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("in.mkv");
        std::fs::write(&source, b"x").unwrap();
        let mut hevc = profile("hevc", 3840, 2160, Some(40_000), "dts");
        hevc["color_primaries"] = Value::Null;
        hevc["color_transfer"] = json!("unknown");
        let planned = plan(&hevc, &input(Shrink::Auto)).unwrap();
        let output = directory.join("out.mp4");
        let args = copy_args(
            "ffmpeg",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
            &hevc,
            &planned,
        )
        .unwrap();
        let has = |key: &str, value: &str| {
            args.windows(2)
                .any(|pair| pair[0] == key && pair[1] == value)
        };
        assert!(has("-c:v:0", "copy"));
        assert!(has("-tag:v:0", "hvc1"));
        assert!(has("-color_primaries:v:0", "bt709"));
        assert!(has("-color_trc:v:0", "bt709"));
        assert!(has("-colorspace:v:0", "bt709"));
        assert!(has("-c:a:0", "aac"));
        assert!(has("-b:a:0", "384k"));
        assert!(has("-movflags", "+faststart"));
        assert!(copy_args(
            "ffmpeg",
            source.to_str().unwrap(),
            source.to_str().unwrap(),
            &hevc,
            &planned
        )
        .is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ssim_windows_and_summary_parse() {
        assert_eq!(sample_windows(20.0), vec![(0.0, 20.0)]);
        let windows = sample_windows(1000.0);
        assert_eq!(windows.len(), 3);
        assert!((windows[1].0 - 497.0).abs() < 1e-9);
        assert!(windows
            .iter()
            .all(|(start, length)| start + length <= 1000.0));
        assert_eq!(
            parse_ssim_all("[Parsed_ssim_5 @ 0x1] SSIM Y:0.99 (20.0) U:0.98 (17.0) V:0.98 (17.0) All:0.9876 (19.1)"),
            Some(0.9876)
        );
        assert_eq!(parse_ssim_all("frame=1 fps=0"), None);
        assert_eq!(
            QualityCheck::parse("standard").unwrap().floor(),
            Some(0.985)
        );
        assert!(QualityCheck::parse("loose").is_err());
        assert!(Shrink::parse("sometimes").is_err());
    }

    #[test]
    fn audio_plan_round_trips_through_json() {
        let plan = vec![AudioAction::Copy, AudioAction::Transcode { kbps: 128 }];
        let json = audio_plan_json(&plan);
        let args = audio_args_from_json(json.as_array().unwrap()).unwrap();
        assert_eq!(args, audio_args(&plan));
        assert!(audio_args_from_json(&[json!({"kbps": 5})]).is_err());
    }
}
