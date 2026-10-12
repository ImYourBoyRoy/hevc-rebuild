use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    #[serde(default)]
    tools: ToolPaths,
    #[serde(default)]
    input_path: Option<PathBuf>,
    #[serde(default)]
    stream_index: Option<u32>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    path_override: Option<PathBuf>,
    #[serde(default)]
    path_override_video_ordinal: Option<u32>,
    #[serde(default)]
    output_json: Option<PathBuf>,
    #[serde(default)]
    profile: Option<DolbyVisionFacts>,
    #[serde(default)]
    force_deinterlace: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolPaths {
    #[serde(default)]
    ffmpeg: Option<String>,
    #[serde(default)]
    hdr10plus_tool: Option<String>,
    #[serde(default)]
    x265_dolbyvision: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DolbyVisionFacts {
    #[serde(default)]
    dovi_profile: Option<u8>,
    #[serde(default)]
    dovi_rpu_present: bool,
    #[serde(default)]
    dovi_el_present: bool,
    #[serde(default)]
    dovi_bl_present: bool,
    #[serde(default)]
    hdr10: bool,
    #[serde(default)]
    interlaced: bool,
}

#[derive(Debug)]
struct PipelineOutput {
    producer_status: ExitStatus,
    parser_status: ExitStatus,
    producer_stderr: String,
    parser_stdout: String,
    parser_stderr: String,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(execute_request) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize HDR metadata response: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Verify HDR10+ on one encoded output video stream without returning to the
/// Python compatibility engine. `video_ordinal` is the output's FFprobe `v:N`.
pub(crate) fn verify_hdr10plus_output(
    ffmpeg: &str,
    hdr10plus_tool: &str,
    output: &Path,
    video_ordinal: usize,
) -> Result<bool, String> {
    let video_ordinal = u32::try_from(video_ordinal)
        .map_err(|_| "Output video stream ordinal exceeds the supported range.".to_string())?;
    let request = Request {
        operation: "verify_hdr10plus".into(),
        tools: ToolPaths {
            ffmpeg: Some(ffmpeg.to_string()),
            hdr10plus_tool: Some(hdr10plus_tool.to_string()),
            x265_dolbyvision: false,
        },
        input_path: None,
        stream_index: None,
        codec_name: None,
        path_override: Some(output.to_path_buf()),
        path_override_video_ordinal: Some(video_ordinal),
        output_json: None,
        profile: None,
        force_deinterlace: false,
    };
    verify_hdr10plus(&request)
}

fn read_request() -> Result<Request, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read HDR metadata request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("HDR metadata request exceeds the 64 KiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("HDR metadata request JSON is invalid: {error}"))
}

fn execute_request(request: Request) -> Result<Value, String> {
    match request.operation.as_str() {
        "verify_hdr10plus" => {
            verify_hdr10plus(&request).map(|verified| json!({"verified": verified}))
        }
        "extract_hdr10plus" => extract_hdr10plus(&request).map(|()| json!({"extracted": true})),
        "dolbyvision_safe_reasons" => {
            let profile = request
                .profile
                .as_ref()
                .ok_or_else(|| "Dolby Vision profile facts are required.".to_string())?;
            Ok(json!({
                "reasons": safe_preservation_reasons(
                    profile,
                    request.tools.x265_dolbyvision,
                    request.force_deinterlace,
                )
            }))
        }
        "dolbyvision_fallback_reasons" => {
            let profile = request
                .profile
                .as_ref()
                .ok_or_else(|| "Dolby Vision profile facts are required.".to_string())?;
            Ok(json!({"reasons": hdr10_fallback_reasons(profile)}))
        }
        _ => Err("Unknown HDR metadata operation.".into()),
    }
}

fn verify_hdr10plus(request: &Request) -> Result<bool, String> {
    if request.path_override.is_none() && !is_hevc(request.codec_name.as_deref()) {
        return Ok(false);
    }
    let output = run_hdr_pipeline(request, PipelineMode::Verify)?;
    let parser_text = format!("{}\n{}", output.parser_stdout, output.parser_stderr);
    let detected = reports_hdr10plus(&parser_text);
    let expected_pipe_close =
        !output.producer_status.success() && pipe_close_is_expected(&output.producer_stderr);

    if output.parser_status.success() && detected {
        if !output.producer_status.success() && !expected_pipe_close {
            eprintln!(
                "HDR10+ verifier detected metadata, but FFmpeg failed unexpectedly: {}",
                output.producer_stderr.trim()
            );
            return Ok(false);
        }
        return Ok(true);
    }
    if !output.producer_status.success() && !expected_pipe_close {
        eprintln!(
            "HDR10+ verification FFmpeg failed: {}",
            output.producer_stderr.trim()
        );
    }
    if !output.parser_status.success() {
        eprintln!(
            "HDR10+ verification reported no usable metadata: {}",
            parser_text.trim()
        );
    } else if !detected {
        eprintln!("HDR10+ verifier completed without an explicit metadata-detected result.");
    }
    Ok(false)
}

fn extract_hdr10plus(request: &Request) -> Result<(), String> {
    if !is_hevc(request.codec_name.as_deref()) {
        return Err("HDR10+ preservation currently requires an HEVC source video stream.".into());
    }
    let output_path = request
        .output_json
        .as_deref()
        .ok_or_else(|| "HDR10+ extraction requires an output JSON path.".to_string())?;
    if output_path.exists() {
        return Err("HDR10+ extraction output already exists; refusing to overwrite it.".into());
    }
    let output_parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !output_parent.is_dir() {
        return Err("HDR10+ extraction output directory does not exist.".into());
    }

    let output = run_hdr_pipeline(request, PipelineMode::Extract)?;
    let metadata_ok = output_path
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() >= 16);
    if !output.producer_status.success() || !output.parser_status.success() || !metadata_ok {
        let parser_detail = [output.parser_stderr.trim(), output.parser_stdout.trim()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let ffmpeg_detail = if !output.producer_status.success()
            && pipe_close_is_expected(&output.producer_stderr)
            && !output.parser_status.success()
        {
            "FFmpeg producer stopped after hdr10plus_tool closed the pipe.".to_string()
        } else {
            output.producer_stderr.trim().to_string()
        };
        let _ = std::fs::remove_file(output_path);
        return Err(format!(
            "HDR10+ extraction failed; refusing to continue because dynamic metadata would be lost.\nFFmpeg: {ffmpeg_detail}\nhdr10plus_tool: {parser_detail}"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum PipelineMode {
    Verify,
    Extract,
}

fn run_hdr_pipeline(request: &Request, mode: PipelineMode) -> Result<PipelineOutput, String> {
    let input = request
        .path_override
        .as_ref()
        .or(request.input_path.as_ref())
        .ok_or_else(|| "HDR metadata input path is required.".to_string())?;
    if !input.is_file() {
        return Err(format!(
            "HDR metadata input is not a readable file: {}",
            input.display()
        ));
    }
    let ffmpeg = request
        .tools
        .ffmpeg
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            "FFmpeg executable path is required for HDR metadata processing.".to_string()
        })?;
    let parser = request
        .tools
        .hdr10plus_tool
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            "hdr10plus_tool executable path is required for HDR metadata processing.".to_string()
        })?;
    if ffmpeg.len() > 8192 || parser.len() > 8192 {
        return Err("HDR metadata tool paths must be at most 8192 bytes.".into());
    }

    let mut producer = Command::new(ffmpeg);
    producer
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(input)
        .args(["-map"]);
    if request.path_override.is_some() {
        producer.arg(format!(
            "0:v:{}",
            request.path_override_video_ordinal.unwrap_or(0)
        ));
    } else {
        let stream_index = request
            .stream_index
            .ok_or_else(|| "The source FFprobe stream index is required.".to_string())?;
        producer.arg(format!("0:{stream_index}"));
    }
    producer
        .args([
            "-c",
            "copy",
            "-bsf:v",
            "hevc_mp4toannexb",
            "-f",
            "hevc",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut producer = producer
        .spawn()
        .map_err(|error| format!("Could not start FFmpeg HDR metadata stream: {error}"))?;
    let producer_stdout = producer
        .stdout
        .take()
        .ok_or_else(|| "FFmpeg did not provide its HEVC output stream.".to_string())?;
    let producer_stderr = producer
        .stderr
        .take()
        .ok_or_else(|| "FFmpeg did not provide its diagnostic stream.".to_string())?;
    let producer_stderr_reader = read_bounded_async(producer_stderr);

    let mut parser_command = Command::new(parser);
    match mode {
        PipelineMode::Verify => {
            parser_command.args(["--verify", "extract", "-"]);
        }
        PipelineMode::Extract => {
            let output = request
                .output_json
                .as_ref()
                .ok_or_else(|| "HDR10+ extraction requires an output JSON path.".to_string())?;
            parser_command.args(["extract", "-o"]).arg(output).arg("-");
        }
    }
    let parser_result = parser_command
        .stdin(Stdio::from(producer_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut parser = match parser_result {
        Ok(child) => child,
        Err(error) => {
            let _ = producer.kill();
            let _ = producer.wait();
            let _ = producer_stderr_reader.join();
            return Err(format!("Could not start hdr10plus_tool: {error}"));
        }
    };

    let parser_stdout = parser
        .stdout
        .take()
        .ok_or_else(|| "hdr10plus_tool did not provide a result stream.".to_string())?;
    let parser_stderr = parser
        .stderr
        .take()
        .ok_or_else(|| "hdr10plus_tool did not provide a diagnostic stream.".to_string())?;
    let parser_stdout_reader = read_bounded_async(parser_stdout);
    let parser_stderr_reader = read_bounded_async(parser_stderr);
    let parser_status = wait_child(&mut parser, "hdr10plus_tool")?;
    let producer_status = wait_child(&mut producer, "FFmpeg")?;
    Ok(PipelineOutput {
        producer_status,
        parser_status,
        producer_stderr: join_reader(producer_stderr_reader, "FFmpeg diagnostics")?,
        parser_stdout: join_reader(parser_stdout_reader, "hdr10plus_tool output")?,
        parser_stderr: join_reader(parser_stderr_reader, "hdr10plus_tool diagnostics")?,
    })
}

fn wait_child(child: &mut Child, label: &str) -> Result<ExitStatus, String> {
    child
        .wait()
        .map_err(|error| format!("Could not wait for {label}: {error}"))
}

fn read_bounded_async<R>(mut reader: R) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut tail = VecDeque::with_capacity(MAX_CAPTURE_BYTES);
        let mut chunk = [0_u8; 16 * 1024];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    tail.extend(&chunk[..count]);
                    while tail.len() > MAX_CAPTURE_BYTES {
                        let excess = tail.len() - MAX_CAPTURE_BYTES;
                        let discard = excess.min(tail.len());
                        tail.drain(..discard);
                    }
                }
            }
        }
        tail.into_iter().collect()
    })
}

fn join_reader(reader: thread::JoinHandle<Vec<u8>>, label: &str) -> Result<String, String> {
    reader
        .join()
        .map_err(|_| format!("Could not collect {label}."))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn is_hevc(codec_name: Option<&str>) -> bool {
    codec_name
        .is_some_and(|codec| matches!(codec.trim().to_ascii_lowercase().as_str(), "hevc" | "h265"))
}

fn pipe_close_is_expected(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    if !text.contains("broken pipe") {
        return false;
    }
    ![
        "invalid data found",
        "error while decoding",
        "corrupt",
        "input/output error",
        "no such file",
        "permission denied",
    ]
    .iter()
    .any(|token| text.contains(token))
}

fn reports_hdr10plus(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        let Some(hdr) = line.find("hdr10+") else {
            return false;
        };
        let rest = &line[hdr + "hdr10+".len()..];
        if let Some(metadata) = rest.find("metadata") {
            return rest[metadata + "metadata".len()..].contains("detected");
        }
        false
    }) || text.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        let Some(dynamic) = line.find("dynamic") else {
            return false;
        };
        let rest = &line[dynamic + "dynamic".len()..];
        let Some(hdr) = rest.find("hdr10+") else {
            return false;
        };
        let rest = &rest[hdr + "hdr10+".len()..];
        let Some(metadata) = rest.find("metadata") else {
            return false;
        };
        rest[metadata + "metadata".len()..].contains("detected")
    })
}

fn safe_preservation_reasons(
    profile: &DolbyVisionFacts,
    encoder_supports_dolbyvision: bool,
    force_deinterlace: bool,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if profile.dovi_profile != Some(8) {
        reasons.push(format!(
            "Profile {} is not the safe single-layer Profile 8 path",
            profile
                .dovi_profile
                .map_or_else(|| "unknown".to_string(), |value| value.to_string())
        ));
    }
    if !profile.dovi_rpu_present {
        reasons.push("RPU presence was not verified".into());
    }
    if profile.dovi_el_present {
        reasons.push("an enhancement layer is present".into());
    }
    if !profile.dovi_bl_present {
        reasons.push("base-layer presence was not verified".into());
    }
    if !encoder_supports_dolbyvision {
        reasons.push(
            "installed FFmpeg/libx265 does not expose native Dolby Vision RPU transcoding".into(),
        );
    }
    if profile.interlaced || force_deinterlace {
        reasons.push(
            "video filtering/deinterlacing would make per-frame RPU preservation unsafe".into(),
        );
    }
    reasons
}

fn hdr10_fallback_reasons(profile: &DolbyVisionFacts) -> Vec<String> {
    let mut reasons = Vec::new();
    if !matches!(profile.dovi_profile, Some(7 | 8)) {
        reasons.push(format!(
            "Dolby Vision Profile {} is not approved for this HDR10 fallback path",
            profile
                .dovi_profile
                .map_or_else(|| "unknown".to_string(), |value| value.to_string())
        ));
    }
    if !profile.dovi_bl_present {
        reasons.push("FFprobe did not verify a Dolby Vision base layer".into());
    }
    if !profile.hdr10 {
        reasons
            .push("the selected video was not independently identified as HDR10-compatible".into());
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr10plus_detection_matches_documented_parser_phrases() {
        assert!(reports_hdr10plus("Dynamic HDR10+ metadata detected"));
        assert!(reports_hdr10plus("HDR10+ dynamic metadata detected"));
        assert!(reports_hdr10plus("hdr10+ metadata was detected"));
        assert!(!reports_hdr10plus("HDR10+ metadata not found"));
        assert!(reports_hdr10plus(
            "Dynamic HDR10+ metadata detected\nNo metadata"
        ));
    }

    #[test]
    fn expected_pipe_close_never_hides_media_errors() {
        assert!(pipe_close_is_expected("Error writing trailer: Broken pipe"));
        assert!(!pipe_close_is_expected(
            "Broken pipe after invalid data found"
        ));
        assert!(!pipe_close_is_expected("Permission denied; Broken pipe"));
        assert!(!pipe_close_is_expected("Error decoding frame"));
    }

    #[test]
    fn dolby_vision_safe_and_fallback_reasons_match_python_policy() {
        let eligible = DolbyVisionFacts {
            dovi_profile: Some(8),
            dovi_rpu_present: true,
            dovi_el_present: false,
            dovi_bl_present: true,
            hdr10: true,
            interlaced: false,
        };
        assert!(safe_preservation_reasons(&eligible, true, false).is_empty());
        assert!(hdr10_fallback_reasons(&eligible).is_empty());

        let mut unsafe_profile = DolbyVisionFacts {
            dovi_profile: Some(7),
            ..eligible
        };
        unsafe_profile.interlaced = true;
        assert_eq!(
            safe_preservation_reasons(&unsafe_profile, false, false),
            [
                "Profile 7 is not the safe single-layer Profile 8 path",
                "installed FFmpeg/libx265 does not expose native Dolby Vision RPU transcoding",
                "video filtering/deinterlacing would make per-frame RPU preservation unsafe",
            ]
        );
        unsafe_profile.dovi_bl_present = false;
        assert_eq!(
            hdr10_fallback_reasons(&unsafe_profile),
            ["FFprobe did not verify a Dolby Vision base layer"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn hdr10plus_pipeline_verifies_and_extracts_with_fake_tools() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "hevc-rebuild-hdr-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let input = root.join("sample.mkv");
        std::fs::write(&input, b"source").unwrap();
        let ffmpeg = root.join("ffmpeg");
        let parser = root.join("hdr10plus_tool");
        std::fs::write(&ffmpeg, "#!/bin/sh\nprintf 'annex-b-hevc-data\\n'\n").unwrap();
        std::fs::write(
            &parser,
            "#!/bin/sh\nif [ \"$1\" = \"--verify\" ]; then cat >/dev/null; printf 'Dynamic HDR10+ metadata detected\\n'; exit 0; fi\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then shift; output=$1; fi; shift; done\ncat >/dev/null\nprintf '{\"metadata\":true,\"frames\":[]}' > \"$output\"\n",
        )
        .unwrap();
        for path in [&ffmpeg, &parser] {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        let request = Request {
            operation: "verify_hdr10plus".into(),
            tools: ToolPaths {
                ffmpeg: Some(ffmpeg.to_string_lossy().into_owned()),
                hdr10plus_tool: Some(parser.to_string_lossy().into_owned()),
                x265_dolbyvision: false,
            },
            input_path: Some(input.clone()),
            stream_index: Some(3),
            codec_name: Some("hevc".into()),
            path_override: None,
            path_override_video_ordinal: None,
            output_json: None,
            profile: None,
            force_deinterlace: false,
        };
        assert_eq!(execute_request(request).unwrap()["verified"], true);

        let output = root.join("metadata.json");
        let request = Request {
            operation: "extract_hdr10plus".into(),
            tools: ToolPaths {
                ffmpeg: Some(ffmpeg.to_string_lossy().into_owned()),
                hdr10plus_tool: Some(parser.to_string_lossy().into_owned()),
                x265_dolbyvision: false,
            },
            input_path: Some(input),
            stream_index: Some(3),
            codec_name: Some("h265".into()),
            path_override: None,
            path_override_video_ordinal: None,
            output_json: Some(output.clone()),
            profile: None,
            force_deinterlace: false,
        };
        assert_eq!(execute_request(request).unwrap()["extracted"], true);
        assert!(std::fs::metadata(output).unwrap().len() >= 16);
        std::fs::remove_dir_all(root).unwrap();
    }
}
