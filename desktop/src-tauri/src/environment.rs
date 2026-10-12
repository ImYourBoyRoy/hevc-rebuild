//! Native, read-only host and media-tool inspection.
//!
//! This intentionally mirrors `hevc_rebuild.environment.inspect_environment` while
//! the media engine is being migrated. Keep the serialized payload stable for the
//! Svelte UI, CLI, and MCP clients.

use crate::ToolPaths;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;
const TOOL_CHECKS: &[(&str, &str, &[&str], bool)] = &[
    ("python3", "Python 3 runtime", &["--version"], true),
    ("ffmpeg", "FFmpeg", &["-hide_banner", "-version"], true),
    ("ffprobe", "FFprobe", &["-version"], true),
    ("mediainfo", "MediaInfo", &["--Version"], true),
    ("mkvpropedit", "MKVToolNix editor", &["--version"], true),
    ("mkvmerge", "MKVToolNix merger", &["--version"], true),
    ("mkvextract", "MKVToolNix extractor", &["--version"], true),
    ("fc-list", "Fontconfig", &["--version"], true),
    ("hdr10plus_tool", "HDR10+ helper", &["--version"], true),
    ("apt-get", "APT package manager", &["--version"], true),
    ("apt-cache", "APT package catalog", &["--version"], true),
    ("dpkg", "Debian package database", &["--version"], true),
    ("dpkg-query", "Package query tool", &["--version"], true),
    (
        "pkexec",
        "Administrator authentication",
        &["--version"],
        false,
    ),
];

#[derive(Clone, Debug)]
struct Check {
    key: &'static str,
    label: &'static str,
    path: Option<String>,
    version: Option<String>,
    available: bool,
    required: bool,
    detail: String,
}

impl Check {
    fn payload(&self) -> Value {
        json!({
            "key": self.key,
            "label": self.label,
            "path": self.path,
            "version": self.version,
            "available": self.available,
            "required": self.required,
            "detail": self.detail,
        })
    }
}

struct CapturedCommand {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

/// Inspect system hardware and binaries without installing, downloading, or
/// changing anything. Explicit paths are reported as failed checks instead of
/// aborting the entire environment report when the selected file is invalid.
pub(crate) fn inspect_environment(
    mut paths: ToolPaths,
    legacy_ffmpeg_path: Option<String>,
) -> Result<Value, String> {
    if paths.ffmpeg.as_deref().is_none_or(str::is_empty) {
        paths.ffmpeg = legacy_ffmpeg_path.filter(|value| !value.trim().is_empty());
    }
    let custom_ffmpeg = paths
        .ffmpeg
        .as_deref()
        .is_some_and(|path| !path.trim().is_empty());
    validate_overrides(&paths)?;

    let os = os_details();
    let candidates = candidate_paths(&paths);
    let mut checks = Vec::with_capacity(17);
    for (key, label, args, required) in TOOL_CHECKS {
        let selected_ffmpeg_invalid = *key == "ffmpeg"
            && custom_ffmpeg
            && candidates
                .get(key)
                .and_then(Option::as_deref)
                .is_some_and(|path| !is_executable_file(Path::new(path)));
        checks.push(check_command(
            key,
            label,
            candidates.get(key).and_then(Option::as_deref),
            args,
            *required,
            selected_ffmpeg_invalid,
            Duration::from_secs(8),
        ));
    }

    let ffmpeg = checks.iter().find(|check| check.key == "ffmpeg");
    let mut pixel_formats = BTreeSet::new();
    let mut dolby_vision_x265 = false;
    let mut extra_checks = Vec::with_capacity(3);
    if let Some(ffmpeg) = ffmpeg.filter(|check| check.available) {
        if let Some(path) = ffmpeg.path.as_deref() {
            let encoder_help = run_capture(
                path,
                &["-hide_banner", "-h", "encoder=libx265"],
                Duration::from_secs(10),
            );
            let help = encoder_help
                .as_ref()
                .map(|output| command_text(&output.stdout, &output.stderr))
                .unwrap_or_default();
            if encoder_help.as_ref().is_ok_and(|output| output.success) && help.contains("libx265")
            {
                pixel_formats = parse_x265_pixel_formats(&help);
                dolby_vision_x265 = help.lines().any(|line| {
                    line.split_whitespace()
                        .collect::<Vec<_>>()
                        .windows(2)
                        .any(|pair| {
                            pair == ["-dolbyvision", "<bool>"]
                                || (pair[0] == "-dolbyvision" && pair[1].starts_with('<'))
                        })
                });
                let available = pixel_formats.contains("yuv420p");
                extra_checks.push(Check {
                    key: "libx265",
                    label: "libx265 encoder",
                    path: Some(path.to_string()),
                    version: None,
                    available,
                    required: true,
                    detail: if available {
                        "CPU libx265 ready".into()
                    } else {
                        "libx265 does not expose 8-bit 4:2:0".into()
                    },
                });
            } else {
                extra_checks.push(Check {
                    key: "libx265",
                    label: "libx265 encoder",
                    path: Some(path.to_string()),
                    version: None,
                    available: false,
                    required: true,
                    detail: "FFmpeg does not expose libx265".into(),
                });
            }
            for (key, label, encoder) in [
                ("libx264", "libx264 encoder", "libx264"),
                ("aac", "AAC encoder", "aac"),
            ] {
                let check = match run_capture(
                    path,
                    &["-hide_banner", "-h", &format!("encoder={encoder}")],
                    Duration::from_secs(10),
                ) {
                    Ok(output) => {
                        let text = command_text(&output.stdout, &output.stderr);
                        let available = output.success && text.contains(encoder);
                        Check {
                            key,
                            label,
                            path: Some(path.to_string()),
                            version: None,
                            available,
                            required: false,
                            detail: if available {
                                format!("{encoder} available")
                            } else {
                                format!("FFmpeg does not expose {encoder}")
                            },
                        }
                    }
                    Err(error) => Check {
                        key,
                        label,
                        path: Some(path.to_string()),
                        version: None,
                        available: false,
                        required: false,
                        detail: format!("Could not inspect {encoder}: {error}"),
                    },
                };
                extra_checks.push(check);
            }
        }
    } else {
        extra_checks.extend([
            Check {
                key: "libx265",
                label: "libx265 encoder",
                path: None,
                version: None,
                available: false,
                required: true,
                detail: "Requires a working FFmpeg binary".into(),
            },
            Check {
                key: "libx264",
                label: "libx264 encoder",
                path: None,
                version: None,
                available: false,
                required: false,
                detail: "Requires a working FFmpeg binary".into(),
            },
            Check {
                key: "aac",
                label: "AAC encoder",
                path: None,
                version: None,
                available: false,
                required: false,
                detail: "Requires a working FFmpeg binary".into(),
            },
        ]);
    }
    checks.extend(extra_checks);

    let available = |key: &str| {
        checks
            .iter()
            .any(|check| check.key == key && check.available)
    };
    let supported = os.get("supported").and_then(Value::as_bool) == Some(true);
    let analysis_ready = available("ffprobe") && available("mediainfo");
    let youtube_ready = supported
        && [
            "python3",
            "ffmpeg",
            "ffprobe",
            "mediainfo",
            "libx264",
            "aac",
        ]
        .iter()
        .all(|key| available(key));
    let metadata_ready = supported
        && [
            "python3",
            "ffmpeg",
            "ffprobe",
            "mediainfo",
            "mkvpropedit",
            "mkvmerge",
            "mkvextract",
        ]
        .iter()
        .all(|key| available(key));
    let ready = supported
        && checks
            .iter()
            .filter(|check| check.required)
            .all(|check| check.available);
    let missing_apt_packages = missing_apt_packages(&checks, custom_ffmpeg);
    let memory = memory_stats();

    Ok(json!({
        "os": os,
        "hardware": {
            "cpu_model": cpu_model(),
            "logical_cpus": logical_cpus(),
            "memory_total_gib": memory.0,
            "memory_available_gib": memory.1,
            "swap_total_gib": memory.2,
            "swap_free_gib": memory.3,
            "swap_used_gib": memory.4,
            "swap_used_percent": memory.5,
            "gpu_devices": gpu_devices(),
            "encoder": "CPU libx265",
        },
        "checks": checks.iter().map(Check::payload).collect::<Vec<_>>(),
        "x265_pixel_formats": pixel_formats,
        "dolby_vision_x265": dolby_vision_x265,
        "can_analyze": analysis_ready,
        "youtube_ready": youtube_ready,
        "metadata_ready": metadata_ready,
        "ready": ready,
        "missing_apt_packages": missing_apt_packages,
    }))
}

fn required_engine_tools(purpose: &str) -> Result<&'static [&'static str], String> {
    match purpose {
        "check" => Ok(&["ffmpeg", "ffprobe"]),
        "rebuild" => Ok(&[
            "ffmpeg",
            "ffprobe",
            "mediainfo",
            "mkvpropedit",
            "mkvmerge",
            "mkvextract",
            "hdr10plus_tool",
            "libx265",
        ]),
        "metadata" => Ok(&[
            "python3",
            "ffmpeg",
            "ffprobe",
            "mediainfo",
            "mkvpropedit",
            "mkvmerge",
            "mkvextract",
        ]),
        "youtube" => Ok(&["ffmpeg", "ffprobe", "mediainfo", "libx264", "aac"]),
        _ => Err("Toolchain purpose must be 'check', 'rebuild', 'metadata', or 'youtube'.".into()),
    }
}

/// Resolve and functionally check the toolchain required by an engine run.
/// Dependency installation remains an explicit Rust operation; this helper
/// only selects the configured/PATH binaries and checks the requested mode.
pub(crate) fn resolve_engine_toolchain(request: &Value) -> Result<Value, String> {
    let purpose = request
        .get("purpose")
        .and_then(Value::as_str)
        .ok_or("Toolchain request must specify a purpose.")?;
    let required = required_engine_tools(purpose)?;
    let allow_unsupported_os = request
        .get("allow_unsupported_os")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let paths: ToolPaths = serde_json::from_value(
        request
            .get("tool_paths")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| format!("Tool path settings are invalid: {error}"))?;
    let report = inspect_environment(paths, None)?;
    if !allow_unsupported_os && report["os"]["supported"].as_bool() != Some(true) {
        let name = report["os"]["name"]
            .as_str()
            .unwrap_or("this operating system");
        let version = report["os"]["version"]
            .as_str()
            .unwrap_or("unknown version");
        return Err(format!(
            "HEVC Rebuild supports Ubuntu 24.04+ and Linux Mint 22+; detected {name} {version}."
        ));
    }

    let checks = report["checks"]
        .as_array()
        .ok_or("Native environment inspection returned no tool checks.")?;
    let check = |key: &str| {
        checks
            .iter()
            .find(|item| item.get("key").and_then(Value::as_str) == Some(key))
    };
    let mut missing = Vec::new();
    for key in required {
        if check(key)
            .and_then(|item| item.get("available"))
            .and_then(Value::as_bool)
            != Some(true)
        {
            let detail = check(key)
                .and_then(|item| item.get("detail"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or("unavailable");
            missing.push(format!("{key} ({detail})"));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Required {purpose} tools are missing or not working: {}.",
            missing.join(", ")
        ));
    }

    let executable = |key: &str| -> Result<String, String> {
        check(key)
            .and_then(|item| item.get("path"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("Native environment inspection did not resolve {key}."))
    };
    let ffmpeg = executable("ffmpeg")?;
    let pixel_formats = report["x265_pixel_formats"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if purpose == "rebuild" {
        if !pixel_formats.contains("yuv420p") {
            return Err(
                "FFmpeg/libx265 does not advertise normal 8-bit 4:2:0 HEVC encoding support."
                    .into(),
            );
        }
        for pixel_format in ["yuv420p", "yuv420p10le", "yuv420p12le"] {
            if !pixel_formats.contains(pixel_format) {
                continue;
            }
            let args = [
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=size=64x64:rate=1:duration=0.1",
                "-c:v",
                "libx265",
                "-pix_fmt",
                pixel_format,
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ];
            verify_smoke_encode(&ffmpeg, &args, pixel_format)?;
        }
    } else if purpose == "youtube" {
        let args = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=1:duration=0.1",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            "0.1",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-f",
            "null",
            "-",
        ];
        verify_smoke_encode(&ffmpeg, &args, "H.264/AAC")?;
    }

    let mut resolved = serde_json::Map::new();
    for key in [
        "ffmpeg",
        "ffprobe",
        "mediainfo",
        "mkvpropedit",
        "mkvmerge",
        "mkvextract",
        "hdr10plus_tool",
    ] {
        if let Some(path) = check(key)
            .and_then(|item| item.get("path"))
            .filter(|value| !value.is_null())
        {
            resolved.insert(key.into(), path.clone());
        }
    }
    resolved.insert(
        "x265_pixel_formats".into(),
        serde_json::to_value(&pixel_formats).map_err(|error| error.to_string())?,
    );
    resolved.insert(
        "x265_dolbyvision".into(),
        report
            .get("dolby_vision_x265")
            .cloned()
            .unwrap_or(Value::Bool(false)),
    );
    Ok(Value::Object(resolved))
}

fn verify_smoke_encode(program: &str, args: &[&str], label: &str) -> Result<(), String> {
    let output = run_capture(program, args, Duration::from_secs(45))?;
    if output.success && !output.timed_out {
        return Ok(());
    }
    let detail = command_text(&output.stdout, &output.stderr);
    let detail = detail.trim();
    let detail = if detail.chars().count() > 1200 {
        detail
            .chars()
            .skip(detail.chars().count() - 1200)
            .collect::<String>()
    } else {
        detail.to_owned()
    };
    Err(format!(
        "FFmpeg capability smoke encode failed for {label}{}.",
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

pub(crate) fn resolve_toolchain_cli_mode() -> i32 {
    const MAX_REQUEST_BYTES: u64 = 64 * 1024;
    let mut input = Vec::new();
    let mut stdin = std::io::stdin().take(MAX_REQUEST_BYTES + 1);
    if let Err(error) = stdin.read_to_end(&mut input) {
        eprintln!("Could not read the native toolchain request: {error}");
        return 2;
    }
    if input.len() as u64 > MAX_REQUEST_BYTES {
        eprintln!("Native toolchain request exceeds 64 KiB.");
        return 2;
    }
    let request: Value = match serde_json::from_slice(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Native toolchain request is invalid JSON: {error}");
            return 2;
        }
    };
    match resolve_engine_toolchain(&request).and_then(|value| {
        serde_json::to_string(&value)
            .map_err(|error| format!("Could not format toolchain: {error}"))
    }) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn validate_overrides(paths: &ToolPaths) -> Result<(), String> {
    for (name, path) in [
        ("ffmpeg", &paths.ffmpeg),
        ("ffprobe", &paths.ffprobe),
        ("mediainfo", &paths.mediainfo),
        ("mkvpropedit", &paths.mkvpropedit),
        ("mkvmerge", &paths.mkvmerge),
        ("mkvextract", &paths.mkvextract),
    ] {
        if let Some(path) = path.as_deref().filter(|path| !path.trim().is_empty()) {
            if path.len() > 8192 || path.contains('\0') {
                return Err(format!("Selected {name} path is too long or invalid."));
            }
        }
    }
    Ok(())
}

fn candidate_paths(paths: &ToolPaths) -> BTreeMap<&'static str, Option<String>> {
    let keys = [
        "python3",
        "ffmpeg",
        "ffprobe",
        "mediainfo",
        "mkvpropedit",
        "mkvmerge",
        "mkvextract",
        "fc-list",
        "apt-get",
        "apt-cache",
        "dpkg",
        "dpkg-query",
        "pkexec",
    ];
    let mut found = keys
        .iter()
        .map(|key| (*key, find_in_path(key)))
        .collect::<BTreeMap<_, _>>();

    let overrides = [
        ("ffmpeg", paths.ffmpeg.as_deref()),
        ("ffprobe", paths.ffprobe.as_deref()),
        ("mediainfo", paths.mediainfo.as_deref()),
        ("mkvpropedit", paths.mkvpropedit.as_deref()),
        ("mkvmerge", paths.mkvmerge.as_deref()),
        ("mkvextract", paths.mkvextract.as_deref()),
    ];
    for (key, selected) in overrides {
        if let Some(selected) = selected.filter(|path| !path.trim().is_empty()) {
            found.insert(
                key,
                Some(
                    canonical_executable(Path::new(selected))
                        .unwrap_or_else(|| selected.to_string()),
                ),
            );
        }
    }
    if paths
        .ffmpeg
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && paths
            .ffprobe
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        if let Some(ffmpeg) = paths
            .ffmpeg
            .as_deref()
            .and_then(|path| canonical_executable(Path::new(path)))
        {
            let sibling = Path::new(&ffmpeg).with_file_name(executable_name("ffprobe"));
            if let Some(probe) = canonical_executable(&sibling) {
                found.insert("ffprobe", Some(probe));
            }
        }
    }

    found.insert(
        "hdr10plus_tool",
        find_managed_tool("hdr10plus_tool").or_else(|| find_in_path("hdr10plus_tool")),
    );
    found
}

fn executable_name(name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let extensions = std::env::var_os("PATHEXT")
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        let extension = extensions
            .split(';')
            .find(|part| !part.is_empty())
            .unwrap_or(".exe");
        format!("{name}{extension}")
    }
    #[cfg(not(target_os = "windows"))]
    {
        name.to_string()
    }
}

/// Directories that may hold the managed HDR10+ helper.
/// The per-user directory comes first so an AppImage still finds a helper
/// installed by a local build. The AppImage sidecar is included when that
/// profile is active, so a portable copy remains usable on its own.
pub(crate) fn managed_tool_directories() -> Vec<PathBuf> {
    managed_tool_directories_from(
        std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR").map(PathBuf::from),
        user_managed_bin_directory(),
    )
}

fn user_managed_bin_directory() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("hevc-rebuild/bin");
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/hevc-rebuild/bin");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/hevc-rebuild/bin");
    }
    PathBuf::from(".local/share/hevc-rebuild/bin")
}

fn managed_tool_directories_from(portable: Option<PathBuf>, user: PathBuf) -> Vec<PathBuf> {
    let mut directories = vec![user];
    if let Some(portable) = portable {
        let sidecar = portable.join("share/hevc-rebuild/bin");
        if directories.iter().all(|directory| directory != &sidecar) {
            directories.push(sidecar);
        }
    }
    directories
}

fn find_managed_tool(name: &str) -> Option<String> {
    let executable = executable_name(name);
    for directory in managed_tool_directories() {
        if let Some(found) = canonical_executable(&directory.join(&executable)) {
            return Some(found);
        }
    }
    None
}

fn preferred_version_line(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines
        .iter()
        .copied()
        .find(|line| {
            let bytes = line.as_bytes();
            bytes.windows(3).any(|window| {
                window[0].is_ascii_digit() && window[1] == b'.' && window[2].is_ascii_digit()
            })
        })
        .or_else(|| lines.first().copied())
        .map(str::to_string)
}

fn find_in_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(executable_name(name));
        if let Some(executable) = canonical_executable(&candidate) {
            return Some(executable);
        }
        #[cfg(target_os = "windows")]
        if Path::new(name).extension().is_none() {
            if let Some(extensions) = std::env::var_os("PATHEXT") {
                for extension in extensions.to_string_lossy().split(';') {
                    let candidate = directory.join(format!("{name}{extension}"));
                    if let Some(executable) = canonical_executable(&candidate) {
                        return Some(executable);
                    }
                }
            }
        }
    }
    None
}

fn canonical_executable(path: &Path) -> Option<String> {
    let resolved = path.canonicalize().ok()?;
    is_executable_file(&resolved).then(|| resolved.to_string_lossy().into_owned())
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn check_command(
    key: &'static str,
    label: &'static str,
    path: Option<&str>,
    args: &[&str],
    required: bool,
    invalid_selected_ffmpeg: bool,
    timeout: Duration,
) -> Check {
    let Some(path) = path else {
        return Check {
            key,
            label,
            path: None,
            version: None,
            available: false,
            required,
            detail: "Not found on PATH".into(),
        };
    };
    if invalid_selected_ffmpeg {
        return Check {
            key,
            label,
            path: Some(path.to_string()),
            version: None,
            available: false,
            required,
            detail: "Selected file is missing or not executable".into(),
        };
    }
    match run_capture(path, args, timeout) {
        Ok(output) => {
            let text = command_text(&output.stdout, &output.stderr);
            let version = preferred_version_line(&text);
            let detail = if output.timed_out {
                format!(
                    "Version check timed out after {} seconds",
                    timeout.as_secs()
                )
            } else if output.success {
                String::new()
            } else {
                format!(
                    "Version check exited {}",
                    output
                        .code
                        .map_or_else(|| "without a code".into(), |code| code.to_string())
                )
            };
            Check {
                key,
                label,
                path: Some(path.to_string()),
                version,
                available: output.success,
                required,
                detail,
            }
        }
        Err(error) => Check {
            key,
            label,
            path: Some(path.to_string()),
            version: None,
            available: false,
            required,
            detail: error,
        },
    }
}

fn run_capture(program: &str, args: &[&str], timeout: Duration) -> Result<CapturedCommand, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture command output")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture command errors")?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                timed_out = true;
                terminate_child_group(&mut child);
                break child.wait().ok();
            }
            Err(error) => {
                terminate_child_group(&mut child);
                let _ = child.wait();
                return Err(format!("Could not wait for command: {error}"));
            }
        }
    };
    // If the checked executable left background children with inherited pipes,
    // terminate its isolated process group so reader joins cannot hang.
    #[cfg(unix)]
    terminate_process_group(child.id());
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(CapturedCommand {
        success: !timed_out
            && status
                .as_ref()
                .is_some_and(std::process::ExitStatus::success),
        code: status.and_then(|value| value.code()),
        stdout,
        stderr,
        timed_out,
    })
}

fn read_bounded(mut reader: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let remaining = MAX_CAPTURE_BYTES.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..count.min(remaining)]);
            }
        }
    }
    output
}

fn terminate_child_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    terminate_process_group(child.id());
    let _ = child.kill();
}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        // The command runner places the checked executable in a fresh process group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_pid: u32) {}

fn command_text(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
}

fn parse_x265_pixel_formats(help: &str) -> BTreeSet<String> {
    help.lines()
        .find_map(|line| {
            line.split_once("Supported pixel formats:")
                .map(|(_, value)| value)
        })
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn os_details() -> Value {
    let read = fs::read_to_string("/etc/os-release");
    let info = match read {
        Ok(text) => parse_os_release(&text),
        Err(error) => {
            return json!({
                "name": std::env::consts::OS,
                "version": kernel_release(),
                "supported": false,
                "note": format!("Could not read distro information: {error}"),
            });
        }
    };
    let ident = info
        .get("ID")
        .map(String::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let version = info
        .get("VERSION_ID")
        .map(String::as_str)
        .unwrap_or("unknown");
    let id_like = info
        .get("ID_LIKE")
        .map(String::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>();
    if ident == "ubuntu" && version_at_least(version, &[24, 4]) {
        return json!({"name":"Ubuntu","version":version,"supported":true,"note":"Supported release"});
    }
    if ident == "linuxmint" && id_like.contains("ubuntu") && version_at_least(version, &[22]) {
        return json!({"name":"Linux Mint","version":version,"supported":true,"note":"Supported Ubuntu-based release"});
    }
    json!({
        "name": info.get("PRETTY_NAME").or_else(|| info.get("ID")).cloned().unwrap_or_else(|| "Linux".into()),
        "version": version,
        "supported": false,
        "note": "The current rebuild engine targets Ubuntu 24.04+ and Linux Mint 22+",
    })
}

fn parse_os_release(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.trim().trim_matches('"').to_string()))
        })
        .collect()
}

fn version_components(value: &str) -> Vec<u32> {
    let mut output = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_ascii_digit() {
            current.push(character);
        } else if !current.is_empty() {
            if let Ok(number) = current.parse() {
                output.push(number);
            }
            current.clear();
            if output.len() == 3 {
                break;
            }
        }
    }
    if !current.is_empty() && output.len() < 3 {
        if let Ok(number) = current.parse() {
            output.push(number);
        }
    }
    output
}

fn version_at_least(value: &str, minimum: &[u32]) -> bool {
    version_components(value).as_slice() >= minimum
}

fn kernel_release() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                matches!(key.trim(), "model name" | "Hardware").then(|| value.trim().to_string())
            })
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.into())
}

pub(crate) fn logical_cpus() -> usize {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut set) == 0 {
            let count = libc::CPU_COUNT(&set) as usize;
            if count > 0 {
                return count;
            }
        }
    }
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1)
}

fn memory_stats() -> (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
) {
    let values = fs::read_to_string("/proc/meminfo")
        .ok()
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (key, rest) = line.split_once(':')?;
                    let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
                    Some((key.to_string(), kib))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let gib = |key: &str| values.get(key).map(|kib| *kib as f64 / 1024.0 / 1024.0);
    let mut total = gib("MemTotal");
    let mut available = gib("MemAvailable");
    if let Some((limit, current)) = cgroup_memory_gib() {
        total = Some(total.map_or(limit, |value| value.min(limit)));
        let remaining = (limit - current).max(0.0);
        available = Some(available.map_or(remaining, |value| value.min(remaining)));
    }
    let swap_total = gib("SwapTotal");
    let swap_free = gib("SwapFree");
    let swap_used = swap_total
        .zip(swap_free)
        .map(|(total, free)| (total - free).max(0.0));
    let swap_percent = swap_used.zip(swap_total).map(|(used, total)| {
        if total > 0.0 {
            (used / total * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    });
    (
        total,
        available,
        swap_total,
        swap_free,
        swap_used,
        swap_percent,
    )
}

fn cgroup_memory_gib() -> Option<(f64, f64)> {
    let cgroup = fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    let base = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    let maximum = fs::read_to_string(base.join("memory.max")).ok()?;
    let limit_bytes = maximum.trim().parse::<u64>().ok()?;
    let current_bytes = fs::read_to_string(base.join("memory.current"))
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    let gib = 1024.0_f64.powi(3);
    Some((limit_bytes as f64 / gib, current_bytes as f64 / gib))
}

fn gpu_devices() -> Vec<String> {
    let mut devices = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return devices;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            name.strip_prefix("card").is_some_and(|tail| {
                !tail.is_empty() && tail.chars().all(|character| character.is_ascii_digit())
            })
        })
        .map(|path| path.join("device"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let vendor = fs::read_to_string(path.join("vendor"))
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let device_id = fs::read_to_string(path.join("device"))
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if vendor.is_empty() {
            continue;
        }
        let name = match vendor.as_str() {
            "0x10de" => "NVIDIA".to_string(),
            "0x1002" => "AMD".to_string(),
            "0x8086" => "Intel".to_string(),
            "0x106b" => "Apple".to_string(),
            "0x5143" => "Qualcomm".to_string(),
            _ => format!("GPU vendor {vendor}"),
        };
        let label = if device_id.is_empty() {
            name
        } else {
            format!("{name} ({device_id})")
        };
        if !devices.contains(&label) {
            devices.push(label);
        }
    }
    devices
}

fn missing_apt_packages(checks: &[Check], custom_ffmpeg: bool) -> Vec<String> {
    let missing = checks
        .iter()
        .filter(|check| check.required && !check.available)
        .map(|check| check.key)
        .collect::<BTreeSet<_>>();
    let packages: &[(&str, &[&str])] = &[
        ("python3", &["python3"]),
        ("ffmpeg", &["ffmpeg", "ffprobe"]),
        ("mediainfo", &["mediainfo"]),
        ("mkvtoolnix", &["mkvpropedit", "mkvmerge", "mkvextract"]),
        ("fontconfig", &["fc-list"]),
        ("ca-certificates", &[]),
    ];
    let mut output = Vec::new();
    for (package, commands) in packages {
        // A selected FFmpeg binary may be valid even when no FFprobe sibling
        // exists. Install the distro package in that case to provide FFprobe,
        // without changing the user's selected encoder path.
        if *package == "ffmpeg" && custom_ffmpeg && !missing.contains("ffprobe") {
            continue;
        }
        if *package == "ca-certificates" {
            if !package_installed(package) {
                output.push((*package).to_string());
            }
        } else if commands.iter().any(|command| missing.contains(command)) {
            output.push((*package).to_string());
        }
    }
    output.sort();
    output.dedup();
    output
}

fn package_installed(package: &str) -> bool {
    run_capture(
        "dpkg-query",
        &["-W", "-f=${Status}", package],
        Duration::from_secs(5),
    )
    .is_ok_and(|output| {
        output.success && String::from_utf8_lossy(&output.stdout).trim() == "install ok installed"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_check(key: &'static str, available: bool) -> Check {
        Check {
            key,
            label: key,
            path: None,
            version: None,
            available,
            required: true,
            detail: String::new(),
        }
    }

    #[test]
    fn missing_ffprobe_uses_distro_package_even_with_custom_ffmpeg() {
        let checks = [tool_check("ffmpeg", true), tool_check("ffprobe", false)];
        let packages = missing_apt_packages(&checks, true);
        assert!(packages.iter().any(|package| package == "ffmpeg"));
    }

    #[test]
    fn custom_ffmpeg_needs_no_distro_package_when_both_tools_are_ready() {
        let checks = [tool_check("ffmpeg", true), tool_check("ffprobe", true)];
        let packages = missing_apt_packages(&checks, true);
        assert!(!packages.iter().any(|package| package == "ffmpeg"));
    }

    #[test]
    fn version_line_prefers_the_line_that_contains_a_version() {
        let text = "MediaInfo Command line,\nMediaInfoLib - v25.09\n";
        assert_eq!(
            preferred_version_line(text).as_deref(),
            Some("MediaInfoLib - v25.09")
        );
        assert_eq!(
            preferred_version_line("Python 3.14.4\n").as_deref(),
            Some("Python 3.14.4")
        );
    }

    #[test]
    fn portable_helper_search_keeps_the_user_directory() {
        let user = PathBuf::from("/home/person/.local/share/hevc-rebuild/bin");
        let directories = managed_tool_directories_from(
            Some(PathBuf::from("/opt/app.AppImage.data")),
            user.clone(),
        );
        assert_eq!(directories[0], user);
        assert_eq!(
            directories[1],
            PathBuf::from("/opt/app.AppImage.data/share/hevc-rebuild/bin")
        );
    }

    #[test]
    fn os_release_reader_handles_comments_and_quoted_values() {
        let release = parse_os_release(
            "# comment\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\nPRETTY_NAME=\"Linux Mint 22.1\"\n",
        );
        assert_eq!(release.get("ID").map(String::as_str), Some("linuxmint"));
        assert_eq!(
            release.get("ID_LIKE").map(String::as_str),
            Some("ubuntu debian")
        );
        assert_eq!(
            release.get("PRETTY_NAME").map(String::as_str),
            Some("Linux Mint 22.1")
        );
    }

    #[test]
    fn version_components_match_supported_release_boundaries() {
        assert!(version_at_least("24.04.3 LTS (Noble)", &[24, 4]));
        assert!(version_at_least("22.1", &[22]));
        assert!(!version_at_least("20.04", &[24, 4]));
    }

    #[test]
    fn check_only_requires_analysis_tools_but_not_encode_or_remux_helpers() {
        assert_eq!(
            required_engine_tools("check").unwrap(),
            &["ffmpeg", "ffprobe"]
        );
        assert!(
            required_engine_tools("rebuild")
                .unwrap()
                .contains(&"hdr10plus_tool")
        );
        assert!(
            required_engine_tools("metadata")
                .unwrap()
                .contains(&"mkvmerge")
        );
        assert!(required_engine_tools("unknown").is_err());
    }

    #[test]
    fn x265_pixel_formats_are_parsed_from_the_documented_line() {
        let parsed = parse_x265_pixel_formats(
            "Encoder libx265\nSupported pixel formats: yuv420p yuv420p10le yuv422p10le\n",
        );
        assert!(parsed.contains("yuv420p"));
        assert!(parsed.contains("yuv420p10le"));
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn environment_payload_contains_stable_ui_contract_fields() {
        let payload = Check {
            key: "ffmpeg",
            label: "FFmpeg",
            path: Some("/usr/bin/ffmpeg".into()),
            version: Some("ffmpeg version fixture".into()),
            available: true,
            required: true,
            detail: String::new(),
        }
        .payload();
        assert_eq!(payload["key"], "ffmpeg");
        assert_eq!(payload["available"], true);
        assert_eq!(payload["path"], "/usr/bin/ffmpeg");
        assert_eq!(payload["version"], "ffmpeg version fixture");
    }
}
