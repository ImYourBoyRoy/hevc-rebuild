//! Bounded FFprobe/MediaInfo process runs, packet counting and raw JSON capture.

use super::*;

pub(super) const MAX_TOOL_JSON_BYTES: usize = 32 * 1024 * 1024;

/// Packet counting demuxes the whole file, so allow for very large remuxes.
const PACKET_COUNT_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

pub(super) struct CapturedJson {
    pub(super) value: Value,
}

pub(super) fn probe_media(
    ffprobe: &str,
    mediainfo: Option<&str>,
    path: &Path,
    settings: &Value,
    timings: &mut ScanTimings,
) -> Result<(Profile, Option<String>), String> {
    let ffprobe_start = Instant::now();
    let probe = run_json_tool(
        ffprobe,
        &[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-show_data_hash",
            "sha256",
            "-of",
            "json",
        ],
        path,
        Duration::from_secs(120),
        MAX_TOOL_JSON_BYTES,
    )?
    .value;
    timings.ffprobe_ms += elapsed_ms(ffprobe_start);
    let mediainfo_start = Instant::now();
    let (media_info, media_info_error) = match mediainfo {
        Some(program) => match run_json_tool(
            program,
            &["--Output=JSON"],
            path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        ) {
            Ok(output) => (output.value, None),
            Err(error) => (Value::Null, Some(format!("MediaInfo enrichment failed: {error}"))),
        },
        None => (
            Value::Null,
            Some("MediaInfo is unavailable; this FFprobe profile is partial. MediaInfo-specific fields may be missing.".into()),
        ),
    };
    timings.mediainfo_ms += elapsed_ms(mediainfo_start);
    let profile_start = Instant::now();
    let profile = build_profile(&probe, &media_info, path, settings, ffprobe)?;
    timings.profile_ms += elapsed_ms(profile_start);
    Ok((profile, media_info_error))
}

pub(crate) fn probe_output(ffprobe: &str, path: &Path) -> Result<Value, String> {
    run_json_tool(
        ffprobe,
        &[
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-show_data_hash",
            "sha256",
            "-of",
            "json",
        ],
        path,
        Duration::from_secs(120),
        MAX_TOOL_JSON_BYTES,
    )
    .map(|captured| captured.value)
}

/// Count the packets of one stream that produce output frames. This demuxes the
/// container without decoding, so it is far cheaper than a frame count, yet it
/// still exposes a video stream that was cut short while the container's
/// overall duration (the longest stream) looks right.
///
/// Packets flagged `D` (discard) are excluded: they are decoder preroll that
/// the decoder drops after decoding, so they never become frames. Clips cut with
/// a stream copy, and files with leading edit-list trims, carry many of them; a
/// raw packet count would overstate the source and falsely fail a good encode.
/// The flags are counted line by line as they stream in, so memory stays flat
/// however long the title is.
pub(crate) fn count_stream_packets(
    ffprobe: &str,
    path: &Path,
    stream_index: i64,
) -> Result<u64, String> {
    let selector = stream_index.to_string();
    let mut child = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            &selector,
            "-show_entries",
            "packet=flags",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", Path::new(ffprobe).display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture the packet listing.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture media-tool diagnostics.")?;
    let counter = thread::spawn(move || count_decodable_packets(BufReader::new(stdout)));
    let stderr_reader = thread::spawn(move || read_capped(stderr, 1024 * 1024));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < PACKET_COUNT_TIMEOUT => {
                thread::sleep(Duration::from_millis(40));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not wait for {}: {error}",
                    Path::new(ffprobe).display()
                ));
            }
        }
    };
    let count = counter.join().unwrap_or(0);
    let (diagnostics, _) = stderr_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{} timed out while counting packets in {}.",
            Path::new(ffprobe).display(),
            path.display()
        ));
    };
    if !status.success() {
        let detail = String::from_utf8_lossy(&diagnostics)
            .trim()
            .chars()
            .take(2500)
            .collect::<String>();
        return Err(if detail.is_empty() {
            format!(
                "{} could not count packets in {} (exit {status}).",
                Path::new(ffprobe).display(),
                path.display()
            )
        } else {
            detail
        });
    }
    Ok(count)
}

/// Count packet-flag lines (for example `K__`, `_D_`) whose flags do not mark the
/// packet as discardable preroll.
pub(super) fn count_decodable_packets(reader: impl BufRead) -> u64 {
    reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| {
            let flags = line.trim();
            !flags.is_empty() && !flags.contains('D')
        })
        .count() as u64
}

/// Build the same HDR/color profile used during scan analysis for a specific
/// video stream in an already-probed output. The stream ordinal is FFprobe's
/// zero-based video ordinal, which is also the FFmpeg `v:N` selector.
pub(crate) fn output_video_profiles(
    ffprobe: &str,
    mediainfo: Option<&str>,
    path: &Path,
    probe: &Value,
    video_ordinals: &[usize],
) -> Result<BTreeMap<usize, Value>, String> {
    if video_ordinals.len() > MAX_VIDEO_PROFILES {
        return Err(format!(
            "Output HDR profiling is limited to {MAX_VIDEO_PROFILES} video tracks."
        ));
    }
    let unique = video_ordinals.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != video_ordinals.len() {
        return Err("Output HDR profile video ordinals must be unique.".into());
    }
    let media_info = match mediainfo {
        Some(program) => run_json_tool(
            program,
            &["--Output=JSON"],
            path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        )
        .map(|output| output.value)
        .unwrap_or(Value::Null),
        None => Value::Null,
    };
    video_ordinals
        .iter()
        .map(|ordinal| {
            build_profile_for_video(
                probe,
                &media_info,
                path,
                &json!({}),
                ffprobe,
                Some(*ordinal),
            )
            .map(|profile| (*ordinal, profile.payload))
        })
        .collect()
}

pub(super) fn run_json_tool(
    program: &str,
    args: &[&str],
    path: &Path,
    timeout: Duration,
    max_bytes: usize,
) -> Result<CapturedJson, String> {
    let mut child = Command::new(program)
        .args(args)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", Path::new(program).display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture media-tool output.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture media-tool diagnostics.")?;
    let stdout_reader = thread::spawn(move || read_capped(stdout, max_bytes));
    let stderr_reader = thread::spawn(move || read_capped(stderr, 1024 * 1024));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(40)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not wait for {}: {error}",
                    Path::new(program).display()
                ));
            }
        }
    };
    let (stdout, stdout_truncated) = stdout_reader.join().unwrap_or_default();
    let (stderr, _) = stderr_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{} timed out while inspecting {}.",
            Path::new(program).display(),
            path.display()
        ));
    };
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr)
            .trim()
            .chars()
            .take(2500)
            .collect::<String>();
        return Err(if detail.is_empty() {
            format!(
                "{} could not inspect {} (exit {}).",
                Path::new(program).display(),
                path.display(),
                status
            )
        } else {
            detail
        });
    }
    if stdout_truncated {
        return Err(format!(
            "{} returned more than {max_bytes} bytes for {}.",
            Path::new(program).display(),
            path.display()
        ));
    }
    let value = serde_json::from_slice(&stdout).map_err(|error| {
        format!(
            "{} returned invalid JSON for {}: {error}",
            Path::new(program).display(),
            path.display()
        )
    })?;
    Ok(CapturedJson { value })
}

pub(super) fn read_capped(mut reader: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let room = limit.saturating_sub(output.len());
                let retained = count.min(room);
                output.extend_from_slice(&buffer[..retained]);
                truncated |= retained != count;
            }
        }
    }
    (output, truncated)
}

pub(super) fn frame_probe(ffprobe: &str, path: &Path, ordinal: usize) -> Option<Value> {
    let selector = format!("v:{ordinal}");
    run_json_tool(
        ffprobe,
        &[
            "-v", "error", "-select_streams", &selector, "-read_intervals", "%+1",
            "-show_frames", "-show_entries",
            "frame=color_range,color_space,color_primaries,color_transfer,interlaced_frame:frame_side_data",
            "-of", "json",
        ],
        path,
        Duration::from_secs(30),
        8 * 1024 * 1024,
    )
    .ok()
    .map(|output| output.value)
}
