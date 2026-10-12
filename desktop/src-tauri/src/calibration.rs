//! Native benchmark execution and cache policy for x265 thread calibration.
//!
//! Rust builds a bounded null-output command from the analyzed source and
//! current encode options, then owns cache validation, candidate execution,
//! live pressure rejection, and result choice for app-launched GUI/CLI/MCP jobs.
//! The Python adapter supplies a typed profile/options payload and retains the
//! direct-run reference implementation until parity is accepted.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::{
    fs::{OpenOptionsExt, PermissionsExt},
    process::CommandExt,
};

const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 64;
const MAX_CACHE_AGE_HOURS: f64 = 720.0;
const MIB: f64 = 1024.0 * 1024.0;
const SWAP_IN_PRESSURE_BPS: f64 = 4.0 * MIB;
const SWAP_PRESSURE_MIN_SAMPLES: u32 = 5;
const MEMORY_FULL_PRESSURE_PERCENT: f64 = 1.0;
const MAX_DIAGNOSTIC_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize)]
struct CacheQuery {
    key: String,
    cache_hours: f64,
    base_worker_threads: Option<i64>,
    base_frame_threads: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
struct EligibilityRequest {
    width: u32,
    height: u32,
    source_size_bytes: u64,
    large_file_gib: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct CandidateInput {
    worker_threads: i64,
    frame_threads: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct CalibrationPlanKey {
    worker_threads: Option<i64>,
    frame_threads: Option<i64>,
    cpu_threads: i64,
    memory_guard_gib: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct CalibrationProfile {
    source_path: String,
    duration: Option<f64>,
    primary_video_index: u32,
    primary_video_width: u32,
    primary_video_height: u32,
    source_bit_depth: i64,
    source_chroma: String,
    target_pix_fmt: String,
    format_level: Option<String>,
    color_primaries: Option<String>,
    transfer: Option<String>,
    color_matrix: Option<String>,
    color_range_x265: Option<String>,
    color_range_ffmpeg: Option<String>,
    chroma_location: Option<i64>,
    master_display: Option<String>,
    max_cll: Option<i64>,
    max_fall: Option<i64>,
    hdr10: bool,
    hdr10plus: bool,
    hlg: bool,
    dolby_vision: bool,
    dovi_profile: Option<i64>,
    dovi_rpu_present: bool,
    interlaced: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct CalibrationToolchain {
    ffmpeg: String,
    x265_pixel_formats: Vec<String>,
    x265_dolbyvision: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct CalibrationOptions {
    preset: String,
    tune: Option<String>,
    crf: i64,
    sample_seconds: f64,
    hdr10plus_json: Option<String>,
    preserve_dolbyvision: bool,
    strip_dolbyvision: bool,
    preserve_source_level: bool,
    disable_hdr10_opt: bool,
    force_deinterlace: bool,
    disable_deinterlace: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct KeyRequest {
    profile: CalibrationProfile,
    toolchain: CalibrationToolchain,
    options: CalibrationOptions,
    base_plan: CalibrationPlanKey,
    total_memory_gib: Option<f64>,
    max_candidates: usize,
    throughput_tolerance_percent: f64,
    hdr10plus_enabled: bool,
    encoder_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RunRequest {
    key: String,
    max_candidates: usize,
    throughput_tolerance_percent: f64,
    memory_guard_gib: f64,
    profile: CalibrationProfile,
    toolchain: CalibrationToolchain,
    options: CalibrationOptions,
    base_plan: CalibrationPlanKey,
}

#[derive(Clone, Debug, Deserialize)]
struct StoreRequest {
    key: String,
    worker_threads: i64,
    frame_threads: i64,
    results: Vec<CalibrationResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CalibrationResult {
    worker_threads: i64,
    frame_threads: i64,
    elapsed_seconds: f64,
    media_seconds: f64,
    speed_x: f64,
    peak_rss_gib: Option<f64>,
    min_available_gib: Option<f64>,
    max_memory_psi_full: Option<f64>,
    max_swap_in_mib_s: Option<f64>,
    max_swap_out_mib_s: Option<f64>,
    healthy: bool,
    #[serde(default)]
    reason: String,
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    let Some(mode) = args
        .first()
        .map(|value| value.to_string_lossy().to_string())
    else {
        eprintln!("Missing calibration helper mode.");
        return 2;
    };
    if args.len() != 1 {
        eprintln!("The internal calibration helper accepts one mode and no options.");
        return 2;
    }
    let result = match mode.as_str() {
        "key" => read_request::<KeyRequest>().and_then(calibration_key),
        "eligible" => read_request::<EligibilityRequest>().and_then(calibration_eligible),
        "cache" => read_request::<CacheQuery>().and_then(cache_query),
        "run" => read_request::<RunRequest>().and_then(run_calibration),
        "store" => read_request::<StoreRequest>().and_then(store_selection),
        _ => Err("Unknown calibration helper mode.".into()),
    };
    match result {
        Ok(value) => match serde_json::to_writer(io::stdout().lock(), &value) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Could not write calibration helper response: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request<T: for<'de> Deserialize<'de>>() -> Result<T, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .lock()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read calibration request: {error}"))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err("Calibration request exceeds the 16 MiB limit.".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Invalid calibration request: {error}"))
}

fn calibration_eligible(request: EligibilityRequest) -> Result<Value, String> {
    if !request.large_file_gib.is_finite() {
        return Err("Calibration file-size threshold must be finite.".into());
    }
    let pixels = u64::from(request.width) * u64::from(request.height);
    let threshold = (request.large_file_gib.max(0.0) * 1024.0 * 1024.0 * 1024.0)
        .floor()
        .min(u64::MAX as f64) as u64;
    Ok(json!({
        "demanding":pixels >= 3840 * 2160 || request.source_size_bytes >= threshold
    }))
}

fn cache_path() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable).join("cache/calibration.json"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("Could not resolve the user home directory for calibration cache.")?;
    let cache_home = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"));
    Ok(cache_home.join("hevc-rebuild/calibration.json"))
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn calibration_key(request: KeyRequest) -> Result<Value, String> {
    if request.max_candidates == 0
        || request.max_candidates > 4
        || !request.throughput_tolerance_percent.is_finite()
        || !(0.0..=20.0).contains(&request.throughput_tolerance_percent)
        || request
            .total_memory_gib
            .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err("Calibration workload fingerprint inputs are invalid.".into());
    }
    let cpu_threads = request.base_plan.cpu_threads.max(1);
    let base_workers = request
        .base_plan
        .worker_threads
        .unwrap_or_else(|| (cpu_threads - 1).max(1));
    let base_frames = request.base_plan.frame_threads.unwrap_or(1).max(1);
    let pixels = u64::from(request.profile.primary_video_width)
        * u64::from(request.profile.primary_video_height);
    let resolution = if pixels >= 3840 * 2160 {
        "2160p+"
    } else if pixels >= 2560 * 1440 {
        "1440p+"
    } else if pixels >= 1920 * 1080 {
        "1080p"
    } else {
        "sub1080p"
    };
    let encoder = request
        .encoder_fingerprint
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| encoder_fingerprint(&request.toolchain.ffmpeg));
    let payload = json!({
        "encoder":encoder,
        "cpu":cpu_identity(),
        "cpu_threads":cpu_threads,
        "memory_gib":round_digits(request.total_memory_gib.unwrap_or(0.0), 1),
        "base_workers":base_workers,
        "base_frames":base_frames,
        "memory_guard_gib":round_digits(request.base_plan.memory_guard_gib, 2),
        "resolution":resolution,
        "bit_depth":request.profile.source_bit_depth,
        "pix_fmt":request.profile.target_pix_fmt,
        "chroma":request.profile.source_chroma,
        "preset":request.options.preset,
        "tune":request.options.tune.as_deref().unwrap_or("none"),
        "crf":request.options.crf,
        "sample_seconds":round_digits(request.options.sample_seconds, 2),
        "max_candidates":request.max_candidates,
        "throughput_tolerance_percent":round_digits(request.throughput_tolerance_percent, 2),
        "hdr10plus_enabled":request.hdr10plus_enabled,
        "hdr10":request.profile.hdr10,
        "hdr10plus":request.profile.hdr10plus,
        "hlg":request.profile.hlg,
        "dolby_vision":[request.profile.dolby_vision, request.profile.dovi_profile, request.profile.dovi_rpu_present],
        "interlaced":request.profile.interlaced,
        "preserve_source_level":request.options.preserve_source_level,
        "disable_hdr10_opt":request.options.disable_hdr10_opt,
        "force_deinterlace":request.options.force_deinterlace,
        "disable_deinterlace":request.options.disable_deinterlace,
        "preserve_dolbyvision":request.options.preserve_dolbyvision,
        "strip_dolbyvision":request.options.strip_dolbyvision,
    });
    let bytes = serde_json::to_vec(&payload).map_err(|error| {
        format!("Could not serialize calibration workload fingerprint: {error}")
    })?;
    let digest = Sha256::digest(bytes);
    let key = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(json!({"key":key, "encoder_fingerprint":encoder}))
}

fn round_digits(value: f64, digits: u32) -> f64 {
    let factor = 10_f64.powi(digits as i32);
    (value * factor).round_ties_even() / factor
}

static FINGERPRINTS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
static CPU_IDENTITY: OnceLock<String> = OnceLock::new();

fn encoder_fingerprint(ffmpeg: &str) -> String {
    let fingerprints = FINGERPRINTS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(cache) = fingerprints.lock() {
        if let Some(value) = cache.get(ffmpeg) {
            return value.clone();
        }
    }
    let executable = resolve_executable(ffmpeg);
    let mut parts = vec![executable.to_string_lossy().into_owned()];
    if let Ok(metadata) = fs::metadata(&executable) {
        parts.push(metadata.len().to_string());
        if let Ok(modified) = metadata.modified() {
            parts.push(
                modified
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string(),
            );
        }
    }
    for args in [
        vec!["-hide_banner", "-version"],
        vec!["-hide_banner", "-h", "encoder=libx265"],
    ] {
        parts.push(run_fingerprint_command(&executable, &args));
    }
    let digest = Sha256::digest(parts.join("\n").as_bytes());
    let fingerprint: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if let Ok(mut cache) = fingerprints.lock() {
        cache.insert(ffmpeg.to_owned(), fingerprint.clone());
    }
    fingerprint
}

fn resolve_executable(value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.components().count() > 1 || path.is_absolute() {
        return path;
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(&path))
        .find(|candidate| candidate.is_file())
        .unwrap_or(path)
}

fn read_bounded<R: Read>(mut reader: R) -> String {
    const CAP: usize = 1024 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) if output.len() < CAP => {
                let available = CAP - output.len();
                output.extend_from_slice(&buffer[..count.min(available)]);
            }
            Ok(_) => {}
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn run_fingerprint_command(executable: &PathBuf, args: &[&str]) -> String {
    let mut child = match Command::new(executable)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return format!("spawn-error:{:?}", error.kind()),
    };
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(|| read_bounded(pipe)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(|| read_bounded(pipe)));
    let deadline = Instant::now() + Duration::from_secs(5);
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                break (child.wait().ok(), true);
            }
            Err(_) => {
                let _ = child.kill();
                break (child.wait().ok(), false);
            }
        }
    };
    let stdout = stdout
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    format!(
        "{stdout}{stderr}{}{}",
        status
            .and_then(|status| status.code())
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".into()),
        if timed_out { ":timeout" } else { "" }
    )
}

fn cpu_identity() -> &'static str {
    CPU_IDENTITY.get_or_init(|| {
        let mut identity = vec![std::env::consts::ARCH.to_string()];
        #[cfg(target_os = "linux")]
        if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
            if let Some(value) = cpuinfo.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                matches!(
                    key.trim().to_ascii_lowercase().as_str(),
                    "model name" | "hardware"
                )
                .then(|| value.trim().to_owned())
            }) {
                identity.push(value);
            }
        }
        #[cfg(target_os = "windows")]
        if let Some(value) = std::env::var_os("PROCESSOR_IDENTIFIER") {
            identity.push(value.to_string_lossy().into_owned());
        }
        identity.join("|")
    })
}

fn load_cache(path: &PathBuf) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({"schema": 1, "entries": {}}))
}

fn cache_query(request: CacheQuery) -> Result<Value, String> {
    if request.key.is_empty() || request.key.len() > 256 || !request.cache_hours.is_finite() {
        return Err("Calibration cache query contains an invalid key or age.".into());
    }
    let path = cache_path()?;
    let data = load_cache(&path);
    let entry = data
        .pointer(&format!("/entries/{}", escape_pointer_token(&request.key)))
        .and_then(Value::as_object);
    let Some(entry) = entry else {
        return Ok(json!({"found": false}));
    };
    let timestamp = entry.get("timestamp").and_then(Value::as_f64);
    let workers = entry.get("worker_threads").and_then(Value::as_i64);
    let frames = entry.get("frame_threads").and_then(Value::as_i64);
    let (Some(timestamp), Some(workers), Some(frames)) = (timestamp, workers, frames) else {
        return Ok(json!({"found": false}));
    };
    let age_hours = (now_seconds() - timestamp) / 3600.0;
    let worker_cap = request.base_worker_threads.unwrap_or(i64::MAX);
    let frame_cap = request.base_frame_threads.unwrap_or(1);
    if !age_hours.is_finite()
        || age_hours < 0.0
        || age_hours > request.cache_hours.clamp(0.0, MAX_CACHE_AGE_HOURS)
        || workers < 1
        || frames < 1
        || workers > worker_cap
        || frames > frame_cap
    {
        return Ok(json!({"found": false}));
    }
    Ok(json!({
        "found": true,
        "worker_threads": workers,
        "frame_threads": frames,
        "age_hours": age_hours
    }))
}

fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn media_seconds(profile: &CalibrationProfile, options: &CalibrationOptions) -> f64 {
    let duration = profile
        .duration
        .filter(|duration| *duration > 0.0)
        .unwrap_or(options.sample_seconds)
        .max(1.0);
    options.sample_seconds.clamp(1.0, 15.0).min(duration)
}

fn benchmark_window(profile: &CalibrationProfile, options: &CalibrationOptions) -> (f64, f64) {
    let duration = profile.duration.unwrap_or_default().max(0.0);
    let sample = if duration > 0.0 {
        options
            .sample_seconds
            .clamp(1.0, 15.0)
            .min((duration - 0.5).max(1.0))
    } else {
        options.sample_seconds.clamp(1.0, 15.0)
    };
    let seek = if duration > sample + 5.0 {
        (duration * 0.25).min((duration - sample - 2.0).max(0.0))
    } else {
        0.0
    };
    (sample, seek)
}

fn safe_x265_token(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || value.contains(':')
        || value.chars().any(|character| character.is_control())
    {
        return Err("A source color/level field is unsafe for x265 calibration parameters.".into());
    }
    Ok(())
}

fn build_candidate_args(
    profile: &CalibrationProfile,
    tools: &CalibrationToolchain,
    options: &CalibrationOptions,
    candidate: &CandidateInput,
) -> Result<(Vec<String>, f64), String> {
    if !options.sample_seconds.is_finite()
        || !(1.0..=15.0).contains(&options.sample_seconds)
        || !(0..=51).contains(&options.crf)
        || candidate.worker_threads < 1
        || candidate.frame_threads < 1
    {
        return Err("Calibration candidate has invalid duration, CRF, or thread limits.".into());
    }
    if ![
        "ultrafast",
        "superfast",
        "veryfast",
        "faster",
        "fast",
        "medium",
        "slow",
        "slower",
        "veryslow",
        "placebo",
    ]
    .contains(&options.preset.as_str())
    {
        return Err("Calibration preset is not a supported x265 preset.".into());
    }
    if options.tune.as_deref().is_some_and(|tune| {
        ![
            "none",
            "psnr",
            "ssim",
            "grain",
            "zerolatency",
            "fastdecode",
            "animation",
            "stillimage",
            "film",
        ]
        .contains(&tune)
    }) {
        return Err("Calibration tune is not a supported x265 tune.".into());
    }
    if !tools
        .x265_pixel_formats
        .iter()
        .any(|format| format == &profile.target_pix_fmt)
    {
        return Err(format!(
            "Installed FFmpeg/libx265 does not advertise required pixel format {:?}.",
            profile.target_pix_fmt
        ));
    }
    safe_x265_token(&profile.target_pix_fmt)?;
    let input = PathBuf::from(&profile.source_path)
        .canonicalize()
        .map_err(|error| format!("Calibration source is unavailable: {error}"))?;
    if !input.is_file() {
        return Err("Calibration source must be a regular file.".into());
    }
    if tools.ffmpeg.trim().is_empty() || tools.ffmpeg.len() > 8192 {
        return Err("Configured FFmpeg path is empty or too long.".into());
    }
    let duration = profile.duration.unwrap_or_default();
    if !duration.is_finite() || duration < 0.0 {
        return Err("Calibration source duration is invalid.".into());
    }
    if profile.hlg
        && profile.interlaced
        && options.disable_deinterlace
        && !options.force_deinterlace
    {
        return Err("Interlaced HLG with deinterlacing disabled cannot be signaled safely.".into());
    }

    let mut args = vec![
        tools.ffmpeg.clone(),
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
    ];
    let (sample, seek) = benchmark_window(profile, options);
    if seek > 0.0 {
        args.extend(["-ss".into(), format!("{seek:.3}")]);
    }
    args.extend([
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-t".into(),
        format!("{sample:.3}"),
        "-map".into(),
        format!("0:{}", profile.primary_video_index),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-c:v:0".into(),
        "libx265".into(),
        "-pix_fmt:v:0".into(),
        profile.target_pix_fmt.clone(),
        "-preset:v:0".into(),
        options.preset.clone(),
        "-crf:v:0".into(),
        options.crf.to_string(),
        "-fps_mode:v:0".into(),
        "passthrough".into(),
    ]);
    if let Some(tune) = options.tune.as_deref().filter(|tune| *tune != "none") {
        args.extend(["-tune:v:0".into(), tune.into()]);
    }
    if options.preserve_dolbyvision {
        if !profile.dolby_vision {
            return Err(
                "Dolby Vision preservation was requested for a source without Dolby Vision.".into(),
            );
        }
        args.extend(["-dolbyvision:v:0".into(), "1".into()]);
    } else if options.strip_dolbyvision && profile.dolby_vision && tools.x265_dolbyvision {
        args.extend(["-dolbyvision:v:0".into(), "0".into()]);
    }
    if options.force_deinterlace || (profile.interlaced && !options.disable_deinterlace) {
        args.extend([
            "-filter:v:0".into(),
            "bwdif=mode=send_frame:parity=auto:deint=interlaced".into(),
        ]);
    }
    for (option, value) in [
        ("-color_primaries:v:0", profile.color_primaries.as_deref()),
        ("-color_trc:v:0", profile.transfer.as_deref()),
        ("-colorspace:v:0", profile.color_matrix.as_deref()),
        ("-color_range:v:0", profile.color_range_ffmpeg.as_deref()),
    ] {
        if let Some(value) = value {
            safe_x265_token(value)?;
            args.extend([option.into(), value.into()]);
        }
    }

    let mut params = Vec::new();
    if options.preserve_source_level {
        if let Some(level) = profile
            .format_level
            .as_deref()
            .and_then(first_numeric_token)
        {
            if level.parse::<f64>().is_ok_and(|value| value >= 5.0) {
                params.push(format!("level-idc={level}"));
            }
        }
    }
    for (name, value) in [
        ("colorprim", profile.color_primaries.as_deref()),
        ("transfer", profile.transfer.as_deref()),
        ("colormatrix", profile.color_matrix.as_deref()),
        ("range", profile.color_range_x265.as_deref()),
    ] {
        if let Some(value) = value {
            safe_x265_token(value)?;
            params.push(format!("{name}={value}"));
        }
    }
    if let Some(chroma) = profile
        .chroma_location
        .filter(|_| profile.target_pix_fmt.contains("420"))
    {
        if !(0..=6).contains(&chroma) {
            return Err("Calibration chroma location is outside the supported range.".into());
        }
        params.push(format!("chromaloc={chroma}"));
    }
    if let Some(master_display) = profile.master_display.as_deref() {
        safe_x265_token(master_display)?;
        params.push(format!("master-display={master_display}"));
    }
    if let Some(max_cll) = profile.max_cll {
        if max_cll < 0 || profile.max_fall.is_some_and(|value| value < 0) {
            return Err("Calibration HDR light-level metadata is invalid.".into());
        }
        params.push(format!(
            "max-cll={max_cll},{}",
            profile.max_fall.unwrap_or(0)
        ));
    }
    if profile.hdr10 {
        params.push("hdr10=1".into());
        if !options.disable_hdr10_opt
            && profile.target_pix_fmt == "yuv420p10le"
            && profile.color_primaries.as_deref() == Some("bt2020")
            && profile.transfer.as_deref() == Some("smpte2084")
        {
            params.push("hdr10-opt=1".into());
        }
    }
    if profile.hlg {
        params.extend(["atc-sei=18".into(), "pic-struct=0".into()]);
    }
    if let Some(path) = options.hdr10plus_json.as_deref() {
        if path.contains(':') || path.chars().any(|character| character.is_control()) {
            return Err(format!(
                "HDR10+ JSON path cannot be embedded safely in x265-params: {path}"
            ));
        }
        if !PathBuf::from(path).is_file() {
            return Err("Calibration HDR10+ metadata file is unavailable.".into());
        }
        params.extend([format!("dhdr10-info={path}"), "dhdr10-opt=1".into()]);
    }
    params.push(format!("pools={}", candidate.worker_threads));
    params.push(format!("frame-threads={}", candidate.frame_threads));
    if !params.is_empty() {
        args.extend(["-x265-params:v:0".into(), params.join(":")]);
    }
    args.extend(["-f".into(), "null".into(), "-".into()]);
    Ok((args, media_seconds(profile, options)))
}

fn first_numeric_token(value: &str) -> Option<&str> {
    let start = value.find(|character: char| character.is_ascii_digit())?;
    let tail = &value[start..];
    let end = tail
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(tail.len());
    let token = &tail[..end];
    (!token.is_empty() && token.parse::<f64>().is_ok()).then_some(token)
}

fn run_calibration(request: RunRequest) -> Result<Value, String> {
    if request.key.is_empty()
        || request.key.len() > 256
        || !request.throughput_tolerance_percent.is_finite()
        || !(0.0..=20.0).contains(&request.throughput_tolerance_percent)
        || !request.memory_guard_gib.is_finite()
        || request.memory_guard_gib < 0.0
    {
        return Err("Calibration request contains invalid limits.".into());
    }
    let host = crate::resource_monitor::sample_current_host();
    let Some(available) = host.memory_available_gib else {
        return Ok(json!({
            "source":"base", "reason":"Memory telemetry is unavailable; calibration was skipped.",
            "results":[]
        }));
    };
    if available < request.memory_guard_gib {
        return Ok(json!({
            "source":"base",
            "reason":format!("Available RAM is below the configured {:.1} GiB reserve; calibration was skipped.", request.memory_guard_gib),
            "results":[]
        }));
    }

    let candidate_limit = request.max_candidates.clamp(1, 4);
    let candidates = candidate_layouts(&request.base_plan, candidate_limit);
    let mut results = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let result = match build_candidate_args(
            &request.profile,
            &request.toolchain,
            &request.options,
            &candidate,
        ) {
            Ok((args, media_seconds)) => {
                benchmark_candidate(&candidate, &args, media_seconds, request.memory_guard_gib)
            }
            Err(error) => failed_result(
                &candidate,
                media_seconds(&request.profile, &request.options),
                error,
            ),
        };
        results.push(result);
    }
    let best = choose_best_result(&results, request.throughput_tolerance_percent);
    let (source, workers, frames, reason) = if let Some(best) = best {
        (
            "benchmark",
            Some(best.worker_threads),
            Some(best.frame_threads),
            "Selected by quick hardware calibration; live resource guards remain active.",
        )
    } else {
        (
            "base",
            None,
            None,
            "No calibration candidate completed with acceptable resource pressure; using the normal plan.",
        )
    };
    Ok(json!({
        "source":source,
        "worker_threads":workers,
        "frame_threads":frames,
        "reason":reason,
        "results":results
    }))
}

fn candidate_layouts(plan: &CalibrationPlanKey, limit: usize) -> Vec<CandidateInput> {
    let workers = plan
        .worker_threads
        .unwrap_or_else(|| (plan.cpu_threads.max(1) - 1).max(1));
    let frames = plan.frame_threads.unwrap_or(1).max(1);
    let mut candidates = vec![(workers, frames)];
    if frames > 1 {
        candidates.push((workers, 1));
    }
    if workers > 2 {
        candidates.push((((workers as f64 * 0.80).round_ties_even() as i64).max(2), 1));
        candidates.push((((workers as f64 * 0.60).round_ties_even() as i64).max(2), 1));
    }
    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|pair| seen.insert(*pair))
        .take(limit.max(1))
        .map(|(worker_threads, frame_threads)| CandidateInput {
            worker_threads,
            frame_threads,
        })
        .collect()
}

fn failed_result(
    candidate: &CandidateInput,
    media_seconds: f64,
    reason: String,
) -> CalibrationResult {
    CalibrationResult {
        worker_threads: candidate.worker_threads,
        frame_threads: candidate.frame_threads,
        elapsed_seconds: 0.0,
        media_seconds: media_seconds.max(1.0),
        speed_x: 0.0,
        peak_rss_gib: None,
        min_available_gib: None,
        max_memory_psi_full: None,
        max_swap_in_mib_s: None,
        max_swap_out_mib_s: None,
        healthy: false,
        reason,
    }
}

fn benchmark_candidate(
    candidate: &CandidateInput,
    args: &[String],
    media_seconds: f64,
    memory_guard_gib: f64,
) -> CalibrationResult {
    let Some(executable) = args.first() else {
        return failed_result(
            candidate,
            media_seconds,
            "Calibration command is empty.".into(),
        );
    };
    let mut command = Command::new(executable);
    command
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(target_os = "linux")]
    {
        let parent_pid = std::process::id() as libc::pid_t;
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent_pid {
                    let _ = libc::kill(libc::getpid(), libc::SIGTERM);
                }
                Ok(())
            });
        }
    }
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return failed_result(
                candidate,
                media_seconds,
                format!("Could not start calibration: {error}"),
            );
        }
    };
    let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
    let drain_thread = child.stderr.take().map(|mut stderr| {
        let tail = Arc::clone(&stderr_tail);
        thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if let Ok(mut stored) = tail.lock() {
                            stored.extend(&buffer[..count]);
                            while stored.len() > MAX_DIAGNOSTIC_BYTES {
                                stored.pop_front();
                            }
                        }
                    }
                }
            }
        })
    });

    let mut sampler = crate::resource_monitor::Sampler::new();
    let mut peak_rss: Option<f64> = None;
    let mut min_available: Option<f64> = None;
    let mut max_psi: Option<f64> = None;
    let mut max_swap_in: Option<f64> = None;
    let mut max_swap_out: Option<f64> = None;
    let mut low_memory_samples = 0_u32;
    let mut swap_pressure_samples = 0_u32;
    let mut severe_psi_samples = 0_u32;
    let mut healthy = true;
    let mut reason = String::new();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => {
                healthy = false;
                reason = format!("Could not inspect calibration process: {error}");
                terminate_candidate(&mut child);
                break child.try_wait().ok().flatten();
            }
        }
        let sample = sampler.sample(child.id(), None);
        let rss = sample
            .ffmpeg_process_rss_gib
            .or(sample.engine_process_rss_gib);
        if let Some(value) = rss {
            peak_rss = Some(peak_rss.map_or(value, |previous| previous.max(value)));
        }
        if let Some(available) = sample.memory_available_gib {
            min_available =
                Some(min_available.map_or(available, |previous| previous.min(available)));
            low_memory_samples = if memory_guard_gib > 0.0 && available < memory_guard_gib {
                low_memory_samples.saturating_add(1)
            } else {
                0
            };
        }
        if let Some(psi) = sample.memory_full_avg10 {
            max_psi = Some(max_psi.map_or(psi, |previous| previous.max(psi)));
            severe_psi_samples = if psi >= 20.0 {
                severe_psi_samples.saturating_add(1)
            } else {
                0
            };
        }
        if let Some(rate) = sample.swap_in_bytes_per_second {
            max_swap_in = Some(max_swap_in.map_or(rate, |previous| previous.max(rate)));
        }
        if let Some(rate) = sample.swap_out_bytes_per_second {
            max_swap_out = Some(max_swap_out.map_or(rate, |previous| previous.max(rate)));
        }
        if sample
            .swap_in_bytes_per_second
            .is_some_and(|rate| rate >= SWAP_IN_PRESSURE_BPS)
            && sample
                .memory_full_avg10
                .is_some_and(|psi| psi >= MEMORY_FULL_PRESSURE_PERCENT)
        {
            swap_pressure_samples = swap_pressure_samples.saturating_add(1);
        } else {
            swap_pressure_samples = 0;
        }
        if low_memory_samples >= 3 {
            healthy = false;
            reason = format!("available RAM stayed below the {memory_guard_gib:.1} GiB reserve");
        } else if swap_pressure_samples >= SWAP_PRESSURE_MIN_SAMPLES {
            healthy = false;
            reason = "sustained swap-in and memory PSI pressure".into();
        } else if severe_psi_samples >= 3 {
            healthy = false;
            reason = "severe full-memory PSI pressure".into();
        }
        if !healthy {
            terminate_candidate(&mut child);
            break child.try_wait().ok().flatten();
        }
        thread::sleep(Duration::from_millis(500));
    };
    if child.try_wait().ok().flatten().is_none() {
        terminate_candidate(&mut child);
    }
    if let Some(drain_thread) = drain_thread {
        let _ = drain_thread.join();
    }
    if status.as_ref().is_none_or(|value| !value.success()) {
        healthy = false;
        if reason.is_empty() {
            let diagnostic = stderr_tail
                .lock()
                .ok()
                .map(|bytes| {
                    String::from_utf8_lossy(&bytes.iter().copied().collect::<Vec<_>>())
                        .trim()
                        .to_string()
                })
                .unwrap_or_default();
            let code = status
                .and_then(|value| value.code().map(|code| code.to_string()))
                .unwrap_or_else(|| "unknown".into());
            reason = format!(
                "FFmpeg calibration exited {code}{}",
                if diagnostic.is_empty() {
                    String::new()
                } else {
                    format!(": {diagnostic}")
                }
            );
        }
    }
    if max_psi.is_some_and(|value| value >= 20.0) {
        healthy = false;
        if reason.is_empty() {
            reason = format!(
                "full-memory PSI peaked at {:.1}%",
                max_psi.unwrap_or_default()
            );
        }
    }
    if max_swap_out.is_some_and(|value| value >= 64.0 * MIB) {
        healthy = false;
        if reason.is_empty() {
            reason = format!(
                "heavy swap-out peaked at {:.0} MiB/s",
                max_swap_out.unwrap_or_default() / MIB
            );
        }
    }
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    CalibrationResult {
        worker_threads: candidate.worker_threads,
        frame_threads: candidate.frame_threads,
        elapsed_seconds: elapsed,
        media_seconds,
        speed_x: if healthy {
            media_seconds / elapsed
        } else {
            0.0
        },
        peak_rss_gib: peak_rss,
        min_available_gib: min_available,
        max_memory_psi_full: max_psi,
        max_swap_in_mib_s: max_swap_in.map(|value| value / MIB),
        max_swap_out_mib_s: max_swap_out.map(|value| value / MIB),
        healthy,
        reason,
    }
}

fn terminate_candidate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    #[cfg(unix)]
    unsafe {
        let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn choose_best_result(results: &[CalibrationResult], tolerance: f64) -> Option<&CalibrationResult> {
    let healthy = results
        .iter()
        .filter(|result| result.healthy && result.speed_x > 0.0 && result.speed_x.is_finite())
        .collect::<Vec<_>>();
    let fastest = healthy
        .iter()
        .map(|result| result.speed_x)
        .max_by(f64::total_cmp)?;
    let threshold = fastest * (1.0 - tolerance.max(0.0) / 100.0);
    healthy
        .into_iter()
        .filter(|result| result.speed_x >= threshold)
        .min_by(|left, right| {
            let left_key = (
                left.peak_rss_gib.unwrap_or(f64::INFINITY),
                -left.min_available_gib.unwrap_or(f64::NEG_INFINITY),
                left.frame_threads,
                left.worker_threads,
                -left.speed_x,
            );
            let right_key = (
                right.peak_rss_gib.unwrap_or(f64::INFINITY),
                -right.min_available_gib.unwrap_or(f64::NEG_INFINITY),
                right.frame_threads,
                right.worker_threads,
                -right.speed_x,
            );
            left_key
                .0
                .total_cmp(&right_key.0)
                .then_with(|| left_key.1.total_cmp(&right_key.1))
                .then_with(|| left_key.2.cmp(&right_key.2))
                .then_with(|| left_key.3.cmp(&right_key.3))
                .then_with(|| left_key.4.total_cmp(&right_key.4))
        })
}

fn store_selection(request: StoreRequest) -> Result<Value, String> {
    if request.key.is_empty()
        || request.key.len() > 256
        || request.worker_threads < 1
        || request.frame_threads < 1
    {
        return Err("Calibration cache selection contains invalid values.".into());
    }
    let path = cache_path()?;
    let mut data = load_cache(&path);
    if !data.get("entries").is_some_and(Value::is_object) {
        data["entries"] = Value::Object(Map::new());
    }
    let now = now_seconds();
    let Some(entries) = data.get_mut("entries").and_then(Value::as_object_mut) else {
        return Err("Could not prepare the calibration cache entry map.".into());
    };
    entries.retain(|_, entry| {
        entry
            .get("timestamp")
            .and_then(Value::as_f64)
            .is_some_and(|timestamp| {
                let age = (now - timestamp) / 3600.0;
                age.is_finite() && (0.0..=MAX_CACHE_AGE_HOURS).contains(&age)
            })
    });
    entries.insert(
        request.key,
        json!({
            "timestamp":now,
            "worker_threads":request.worker_threads,
            "frame_threads":request.frame_threads,
            "results":request.results
        }),
    );
    let mut sorted = entries
        .iter()
        .filter_map(|(key, entry)| {
            entry
                .get("timestamp")
                .and_then(Value::as_f64)
                .map(|timestamp| (timestamp, key.clone(), entry.clone()))
        })
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| right.0.total_cmp(&left.0));
    let limited = sorted
        .into_iter()
        .take(MAX_CACHE_ENTRIES)
        .map(|(_, key, value)| (key, value))
        .collect::<Map<_, _>>();
    data["schema"] = json!(1);
    data["entries"] = Value::Object(limited);
    save_cache(&path, &data)?;
    Ok(json!({"stored":true}))
}

fn save_cache(path: &PathBuf, data: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Calibration cache path has no parent directory.")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create calibration cache directory: {error}"))?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("Could not secure calibration cache directory: {error}"))?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!("calibration-{}-{suffix}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Could not create calibration cache temporary file: {error}"))?;
    let result: Result<(), String> = (|| {
        serde_json::to_writer_pretty(&mut file, data)
            .map_err(|error| format!("Could not serialize calibration cache: {error}"))?;
        file.write_all(b"\n")
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("Could not sync calibration cache: {error}"))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|error| format!("Could not atomically replace calibration cache: {error}"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Could not sync calibration cache directory: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(())
}
