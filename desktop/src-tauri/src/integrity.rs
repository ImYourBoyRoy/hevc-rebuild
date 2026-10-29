//! ./desktop/src-tauri/src/integrity.rs
//! Native read-only media integrity checks. Quick inspects container/index
//! structure; Full decodes every stream to FFmpeg's null muxer. Neither path
//! writes media. GUI, JSON CLI, and MCP share this module so the app does not
//! need Python to decide whether a file is intact or corrupt.

use crate::stop_control;
use crate::ToolPaths;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_ERROR_CHARS: usize = 2500;
const MAX_STDERR_BYTES: usize = 1024 * 1024;
const TAIL_BYTES: u64 = 64 * 1024;
const QUICK_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const QUICK_DECODE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntegrityLevel {
    Quick,
    Full,
}

impl IntegrityLevel {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "quick" => Ok(Self::Quick),
            "full" => Ok(Self::Full),
            _ => Err("Integrity level must be 'quick' or 'full'.".into()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct IntegrityCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct IntegrityFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub level: String,
    pub checks: Vec<IntegrityCheck>,
    pub elapsed_seconds: f64,
}

pub(crate) struct IntegrityJobResult {
    pub files: Vec<IntegrityFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
}

#[derive(Clone)]
pub(crate) struct IntegrityProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: &'static str,
    pub file_percent: Option<f64>,
    pub queue_percent: Option<f64>,
}

pub(crate) fn check_paths(
    paths: &[PathBuf],
    level: IntegrityLevel,
    tool_paths: ToolPaths,
    stop_file: Option<&Path>,
    mut on_progress: impl FnMut(IntegrityProgress),
    mut on_line: impl FnMut(String),
) -> Result<IntegrityJobResult, String> {
    if paths.is_empty() {
        return Err("Select at least one media file to check.".into());
    }
    if paths.len() > 10_000 {
        return Err("Integrity checks accept at most 10,000 files.".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    let mut files = Vec::new();
    for path in paths {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
        if !canonical.is_file() {
            return Err(format!("{} is not a file.", canonical.display()));
        }
        if !unique.insert(canonical.clone()) {
            return Err("Selected media paths must be unique.".into());
        }
        files.push(canonical);
    }

    let host = crate::environment::inspect_environment(tool_paths, None)?;
    let ffprobe = required_tool(&host, "ffprobe")?;
    let ffmpeg = required_tool(&host, "ffmpeg")?;
    let mkvmerge = optional_tool(&host, "mkvmerge");

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_dir = crate::state_log_directory()?;
    fs::create_dir_all(&log_dir)
        .map_err(|error| format!("Could not create the run log folder: {error}"))?;
    let log_path = log_dir.join(format!("hevc-rebuild-{stamp}-integrity.log"));
    let report_path = log_dir.join(format!("hevc-rebuild-{stamp}-integrity.csv"));
    let mut log = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(|error| format!("Could not create the integrity log: {error}"))?;
    writeln!(
        log,
        "Integrity check · {} · {} file{}",
        level.as_str(),
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    )
    .ok();

    let total = files.len();
    let mut results = Vec::with_capacity(total);
    let mut failed = 0;
    let mut stop_requested = false;
    for (index, path) in files.iter().enumerate() {
        if stop_file.is_some_and(stop_control::requested) {
            stop_requested = true;
            break;
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        on_progress(IntegrityProgress {
            current: index + 1,
            total,
            file_name: name.clone(),
            file_path: path.to_string_lossy().into_owned(),
            phase: if level == IntegrityLevel::Full {
                "Decoding streams"
            } else {
                "Checking structure"
            },
            file_percent: Some(0.0),
            queue_percent: Some((index as f64 / total as f64) * 100.0),
        });
        let line = format!("Checking {} ({}/{})", name, index + 1, total);
        on_line(line.clone());
        let _ = writeln!(log, "{line}");

        let mut result = check_file(
            path,
            level,
            &ffprobe,
            &ffmpeg,
            mkvmerge.as_deref(),
            stop_file,
            |percent| {
                on_progress(IntegrityProgress {
                    current: index + 1,
                    total,
                    file_name: name.clone(),
                    file_path: path.to_string_lossy().into_owned(),
                    phase: if level == IntegrityLevel::Full {
                        "Decoding streams"
                    } else {
                        "Checking structure"
                    },
                    file_percent: Some(percent),
                    queue_percent: Some(((index as f64 + percent / 100.0) / total as f64) * 100.0),
                });
            },
        );
        result.level = level.as_str().into();
        if result.status == "INTERRUPTED" {
            stop_requested = true;
        }
        if matches!(result.status.as_str(), "CORRUPT" | "UNREADABLE") {
            failed += 1;
        }
        let summary = format!("{}: {} · {}", name, result.status, result.message);
        on_line(summary.clone());
        let _ = writeln!(log, "{summary}");
        append_integrity_row(&report_path, &result)?;
        results.push(result);
        if stop_requested {
            break;
        }
    }
    let _ = log.flush();
    Ok(IntegrityJobResult {
        files: results,
        log_path,
        report_path,
        stop_requested,
        failed,
    })
}

pub(crate) fn cli_mode(args: &[String]) -> i32 {
    let mut files = Vec::new();
    let mut level = IntegrityLevel::Quick;
    let mut tool_paths = ToolPaths::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--file" | "--path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Integrity option {} requires a path.", args[index]);
                    return 2;
                };
                files.push(PathBuf::from(value));
                index += 2;
            }
            "--level" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Integrity option --level requires 'quick' or 'full'.");
                    return 2;
                };
                match IntegrityLevel::parse(value) {
                    Ok(parsed) => level = parsed,
                    Err(error) => {
                        eprintln!("{error}");
                        return 2;
                    }
                }
                index += 2;
            }
            "--ffmpeg-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Integrity option --ffmpeg-path requires a path.");
                    return 2;
                };
                if tool_paths.ffmpeg.replace(value.clone()).is_some() {
                    eprintln!("Integrity option --ffmpeg-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--ffprobe-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Integrity option --ffprobe-path requires a path.");
                    return 2;
                };
                if tool_paths.ffprobe.replace(value.clone()).is_some() {
                    eprintln!("Integrity option --ffprobe-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            "--mkvmerge-path" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("Integrity option --mkvmerge-path requires a path.");
                    return 2;
                };
                if tool_paths.mkvmerge.replace(value.clone()).is_some() {
                    eprintln!("Integrity option --mkvmerge-path can be specified only once.");
                    return 2;
                }
                index += 2;
            }
            name => {
                eprintln!("Unknown integrity option {name}.");
                return 2;
            }
        }
    }
    if files.is_empty() {
        eprintln!("Integrity requires at least one --file PATH.");
        return 2;
    }
    match check_paths(
        &files,
        level,
        tool_paths,
        None,
        |progress| {
            eprintln!(
                "integrity\t{}\t{}\t{}\t{:.1}",
                progress.current,
                progress.total,
                progress.file_path,
                progress.file_percent.unwrap_or(0.0)
            );
        },
        |_| {},
    ) {
        Ok(job) => match serde_json::to_string(&json!({
            "level": level.as_str(),
            "log_path": job.log_path,
            "report_path": job.report_path,
            "failed": job.failed,
            "stop_requested": job.stop_requested,
            "files": job.files,
        })) {
            Ok(payload) => {
                println!("{payload}");
                if job.failed > 0 {
                    1
                } else {
                    0
                }
            }
            Err(error) => {
                eprintln!("Could not serialize the integrity result: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn check_file(
    path: &Path,
    level: IntegrityLevel,
    ffprobe: &str,
    ffmpeg: &str,
    mkvmerge: Option<&str>,
    stop_file: Option<&Path>,
    mut on_percent: impl FnMut(f64),
) -> IntegrityFileResult {
    let started = Instant::now();
    let mut checks = Vec::new();
    let display = path.to_string_lossy().into_owned();

    match inspect_container(path) {
        Ok(check) => {
            let passed = check.passed;
            checks.push(check);
            if !passed {
                return finish(
                    display,
                    "UNREADABLE",
                    "The file could not be read.",
                    checks,
                    started,
                );
            }
        }
        Err(detail) => {
            checks.push(IntegrityCheck {
                name: "file".into(),
                passed: false,
                detail: detail.clone(),
            });
            return finish(display, "UNREADABLE", &detail, checks, started);
        }
    }

    match probe_structure(ffprobe, path) {
        Ok((check, duration)) => {
            checks.push(check);
            if !checks.last().is_some_and(|check| check.passed) {
                return finish(
                    display,
                    "CORRUPT",
                    "FFprobe could not read the container or streams.",
                    checks,
                    started,
                );
            }
            if is_matroska(path) {
                if let Some(mkvmerge) = mkvmerge {
                    match identify_matroska(mkvmerge, path) {
                        Ok(check) => {
                            let passed = check.passed;
                            checks.push(check);
                            if !passed {
                                return finish(
                                    display,
                                    "CORRUPT",
                                    "MKVToolNix could not identify this Matroska file.",
                                    checks,
                                    started,
                                );
                            }
                        }
                        Err(detail) => checks.push(IntegrityCheck {
                            name: "mkvmerge".into(),
                            passed: false,
                            detail,
                        }),
                    }
                } else {
                    checks.push(IntegrityCheck {
                        name: "mkvmerge".into(),
                        passed: true,
                        detail: "MKVToolNix was not available; FFprobe structure was used.".into(),
                    });
                }
            }
            on_percent(if level == IntegrityLevel::Quick {
                40.0
            } else {
                8.0
            });
            if stop_file.is_some_and(stop_control::requested) {
                return finish(
                    display,
                    "INTERRUPTED",
                    "Stopped before decoding.",
                    checks,
                    started,
                );
            }
            match decode_streams(ffmpeg, path, level, duration, stop_file, &mut on_percent) {
                Ok(check) => {
                    let passed = check.passed;
                    let detail = check.detail.clone();
                    checks.push(check);
                    if stop_file.is_some_and(stop_control::requested) {
                        return finish(
                            display,
                            "INTERRUPTED",
                            "Stopped during the decode check.",
                            checks,
                            started,
                        );
                    }
                    if passed {
                        let message = if level == IntegrityLevel::Full {
                            "Every stream decoded without FFmpeg errors."
                        } else {
                            "Container, index, and a short decode look intact."
                        };
                        finish(display, "INTEGRITY_OK", message, checks, started)
                    } else {
                        finish(display, "CORRUPT", &detail, checks, started)
                    }
                }
                Err(detail) => {
                    checks.push(IntegrityCheck {
                        name: "decode".into(),
                        passed: false,
                        detail: detail.clone(),
                    });
                    if stop_file.is_some_and(stop_control::requested) {
                        finish(
                            display,
                            "INTERRUPTED",
                            "Stopped during the decode check.",
                            checks,
                            started,
                        )
                    } else {
                        finish(display, "CORRUPT", &detail, checks, started)
                    }
                }
            }
        }
        Err(detail) => {
            checks.push(IntegrityCheck {
                name: "ffprobe".into(),
                passed: false,
                detail: detail.clone(),
            });
            finish(display, "CORRUPT", &detail, checks, started)
        }
    }
}

fn inspect_container(path: &Path) -> Result<IntegrityCheck, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("Could not read the file: {error}"))?;
    if metadata.len() == 0 {
        return Ok(IntegrityCheck {
            name: "file".into(),
            passed: false,
            detail: "The file is empty.".into(),
        });
    }
    let mut file = File::open(path).map_err(|error| format!("Could not open the file: {error}"))?;
    let mut head = [0_u8; 16];
    let read = file
        .read(&mut head)
        .map_err(|error| format!("Could not read the start of the file: {error}"))?;
    if read == 0 {
        return Ok(IntegrityCheck {
            name: "file".into(),
            passed: false,
            detail: "The file has no readable bytes.".into(),
        });
    }
    let tail_len = TAIL_BYTES.min(metadata.len());
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|error| format!("Could not seek to the end of the file: {error}"))?;
    let mut tail = vec![0_u8; tail_len as usize];
    file.read_exact(&mut tail)
        .map_err(|error| format!("Could not read the end of the file: {error}"))?;
    Ok(IntegrityCheck {
        name: "file".into(),
        passed: true,
        detail: format!("{} bytes; start and end are readable.", metadata.len()),
    })
}

fn probe_structure(ffprobe: &str, path: &Path) -> Result<(IntegrityCheck, Option<f64>), String> {
    let output = run_tool(
        ffprobe,
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_error",
        ],
        path,
        QUICK_PROBE_TIMEOUT,
        true,
    )?;
    if !output.status {
        return Ok((
            IntegrityCheck {
                name: "ffprobe".into(),
                passed: false,
                detail: clip(&output.stderr),
            },
            None,
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(json!({}));
    if let Some(error) = value.get("error") {
        let detail = error
            .get("string")
            .and_then(Value::as_str)
            .unwrap_or("FFprobe reported a container error.");
        return Ok((
            IntegrityCheck {
                name: "ffprobe".into(),
                passed: false,
                detail: detail.into(),
            },
            None,
        ));
    }
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if streams == 0 {
        return Ok((
            IntegrityCheck {
                name: "ffprobe".into(),
                passed: false,
                detail: "FFprobe found no streams.".into(),
            },
            None,
        ));
    }
    let duration = value
        .get("format")
        .and_then(|format| format.get("duration"))
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    Ok((
        IntegrityCheck {
            name: "ffprobe".into(),
            passed: true,
            detail: format!(
                "{streams} stream{}; container index is readable.",
                if streams == 1 { "" } else { "s" }
            ),
        },
        duration,
    ))
}

fn identify_matroska(mkvmerge: &str, path: &Path) -> Result<IntegrityCheck, String> {
    let output = run_tool(mkvmerge, &["-J"], path, QUICK_PROBE_TIMEOUT, true)?;
    if output.status {
        Ok(IntegrityCheck {
            name: "mkvmerge".into(),
            passed: true,
            detail: "Matroska identification succeeded.".into(),
        })
    } else {
        Ok(IntegrityCheck {
            name: "mkvmerge".into(),
            passed: false,
            detail: clip(&output.stderr),
        })
    }
}

fn decode_streams(
    ffmpeg: &str,
    path: &Path,
    level: IntegrityLevel,
    duration: Option<f64>,
    stop_file: Option<&Path>,
    on_percent: &mut impl FnMut(f64),
) -> Result<IntegrityCheck, String> {
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-nostdin", "-v", "error"])
        .arg("-i")
        .arg(path);
    if level == IntegrityLevel::Quick {
        command.args(["-t", "2"]);
    }
    command.args([
        "-map",
        "0",
        "-f",
        "null",
        "-progress",
        "pipe:1",
        "-nostats",
        "-",
    ]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = crate::process_output::spawn_owned(&mut command)
        .map_err(|error| format!("Could not start FFmpeg: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stderr_thread = stderr.map(|stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut reader = BufReader::new(stream);
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if bytes.len() < MAX_STDERR_BYTES {
                            let room = MAX_STDERR_BYTES - bytes.len();
                            bytes.extend_from_slice(&buffer[..count.min(room)]);
                        }
                    }
                }
            }
            bytes
        })
    });
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines() {
            if stop_file.is_some_and(stop_control::requested) {
                terminate_child(&mut child);
                break;
            }
            let Ok(line) = line else { break };
            if let Some(percent) = progress_percent(&line, duration) {
                on_percent(percent);
            }
        }
    }
    if stop_file.is_some_and(stop_control::requested) {
        terminate_child(&mut child);
    }
    let timeout = if level == IntegrityLevel::Quick {
        Some(QUICK_DECODE_TIMEOUT)
    } else {
        None
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if timeout.is_some_and(|limit| started.elapsed() > limit) => {
                terminate_child(&mut child);
                let _ = child.wait();
                return Err("FFmpeg timed out while decoding a sample of this file.".into());
            }
            Ok(None) if stop_file.is_some_and(stop_control::requested) => {
                terminate_child(&mut child);
                let _ = child.wait();
                return Ok(IntegrityCheck {
                    name: "decode".into(),
                    passed: false,
                    detail: "Stopped during the decode check.".into(),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(error) => {
                terminate_child(&mut child);
                let _ = child.wait();
                return Err(format!("Could not wait for FFmpeg: {error}"));
            }
        }
    };
    let stderr = stderr_thread
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let diagnostic = clip(&stderr);
    if !status.success() || looks_corrupt(&diagnostic) {
        return Ok(IntegrityCheck {
            name: "decode".into(),
            passed: false,
            detail: if diagnostic.is_empty() {
                format!("FFmpeg exited with {status} while decoding.")
            } else {
                diagnostic
            },
        });
    }
    on_percent(100.0);
    Ok(IntegrityCheck {
        name: "decode".into(),
        passed: true,
        detail: if level == IntegrityLevel::Full {
            "Full decode to the null muxer reported no errors.".into()
        } else {
            "A short decode of the opening streams reported no errors.".into()
        },
    })
}

fn append_integrity_row(path: &Path, result: &IntegrityFileResult) -> Result<(), String> {
    let mut row = Map::new();
    row.insert("input".into(), json!(result.path));
    row.insert("output".into(), json!(result.path));
    row.insert("status".into(), json!(result.status));
    row.insert("message".into(), json!(result.message));
    row.insert(
        "elapsed_seconds".into(),
        json!((result.elapsed_seconds * 10.0).round() / 10.0),
    );
    crate::reports::append_native_row(path, row)
}

fn finish(
    path: String,
    status: &str,
    message: &str,
    checks: Vec<IntegrityCheck>,
    started: Instant,
) -> IntegrityFileResult {
    IntegrityFileResult {
        path,
        status: status.into(),
        message: message.into(),
        level: String::new(),
        checks,
        elapsed_seconds: started.elapsed().as_secs_f64(),
    }
}

fn required_tool(host: &Value, key: &str) -> Result<String, String> {
    optional_tool(host, key).ok_or_else(|| {
        format!("Install {key} or choose its executable path before checking integrity.")
    })
}

fn optional_tool(host: &Value, key: &str) -> Option<String> {
    host.get("checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("key").and_then(Value::as_str) == Some(key))
        .filter(|check| check.get("available").and_then(Value::as_bool) == Some(true))
        .and_then(|check| check.get("path")?.as_str().map(str::to_owned))
}

fn is_matroska(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv") || ext.eq_ignore_ascii_case("mka"))
}

struct ToolOutput {
    status: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_tool(
    program: &str,
    args: &[&str],
    path: &Path,
    timeout: Duration,
    json_stdout: bool,
) -> Result<ToolOutput, String> {
    let mut child = Command::new(program)
        .args(args)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", Path::new(program).display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = stdout.map(|stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream
                .take(if json_stdout {
                    8 * 1024 * 1024
                } else {
                    MAX_STDERR_BYTES as u64
                })
                .read_to_end(&mut bytes);
            bytes
        })
    });
    let stderr_reader = stderr.map(|stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.take(MAX_STDERR_BYTES as u64).read_to_end(&mut bytes);
            bytes
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(40))
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
                    Path::new(program).display()
                ));
            }
        }
    };
    let stdout = stdout_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{} timed out while inspecting {}.",
            Path::new(program).display(),
            path.display()
        ));
    };
    Ok(ToolOutput {
        status: status.success(),
        stdout,
        stderr,
    })
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        if let Ok(pid) = libc::pid_t::try_from(child.id()) {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

fn clip(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "The media tool reported an error without a diagnostic.".into()
    } else {
        trimmed.chars().take(MAX_ERROR_CHARS).collect()
    }
}

pub(crate) fn progress_percent(line: &str, duration: Option<f64>) -> Option<f64> {
    let (key, value) = line.split_once('=')?;
    match key.trim() {
        "out_time_us" => {
            let micros = value.trim().parse::<f64>().ok()?;
            let duration = duration.filter(|item| *item > 0.0)?;
            Some(((micros / 1_000_000.0) / duration * 100.0).clamp(0.0, 99.0))
        }
        "out_time_ms" => {
            let millis = value.trim().parse::<f64>().ok()?;
            let duration = duration.filter(|item| *item > 0.0)?;
            Some(((millis / 1_000.0) / duration * 100.0).clamp(0.0, 99.0))
        }
        "progress" if value.trim() == "end" => Some(100.0),
        _ => None,
    }
}

pub(crate) fn looks_corrupt(diagnostic: &str) -> bool {
    let lower = diagnostic.to_ascii_lowercase();
    [
        "corrupt",
        "invalid data",
        "error while decoding",
        "truncated",
        "missing picture",
        "concealing",
        "damage",
        "broken",
        "partial file",
        "end of file",
        "invalid nal",
        "error splitting the argument",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn progress_uses_duration_and_end_marker() {
        assert_eq!(
            progress_percent("out_time_us=5000000", Some(10.0)),
            Some(50.0)
        );
        assert_eq!(progress_percent("out_time_ms=2500", Some(10.0)), Some(25.0));
        assert_eq!(progress_percent("progress=end", Some(10.0)), Some(100.0));
        assert_eq!(progress_percent("speed=1.2", Some(10.0)), None);
        assert_eq!(progress_percent("out_time_us=5000000", None), None);
    }

    #[test]
    fn decode_diagnostics_are_classified() {
        assert!(looks_corrupt("error while decoding MB"));
        assert!(looks_corrupt("[h264] Invalid NAL unit"));
        assert!(!looks_corrupt("frame=  12 fps=0.0"));
    }

    #[test]
    fn empty_files_are_unreadable() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-integrity-empty-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("empty.mkv");
        File::create(&path).unwrap();
        let check = inspect_container(&path).unwrap();
        assert!(!check.passed);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn readable_files_pass_the_container_check() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-integrity-bytes-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sample.bin");
        let mut file = File::create(&path).unwrap();
        file.write_all(&[0x1a, 0x45, 0xdf, 0xa3]).unwrap();
        file.write_all(&vec![0_u8; 80_000]).unwrap();
        file.flush().unwrap();
        let check = inspect_container(&path).unwrap();
        assert!(check.passed);
        let _ = fs::remove_dir_all(directory);
    }
}
