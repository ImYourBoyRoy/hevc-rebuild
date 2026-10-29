//! ./desktop/src-tauri/src/encode_run.rs
//! Native FFmpeg encode supervisor (T4.5). Replaces the Python `run_encode`
//! loop for coordinators that run entirely in Rust (YouTube export first).
//!
//! Run: called in-process by `youtube_run`; there is no CLI entry point.
//! Inputs: a full FFmpeg argument vector that includes `-progress pipe:1`, the
//! expected duration, the growing output path, and the memory/disk guard
//! limits. Outputs: live `EncodeProgress` callbacks and a map of peak resource
//! metrics that coordinators merge into the CSV report row.
//! Side effects: spawns FFmpeg in its own process group so a safe-stop abort,
//! guard stop, or pipe failure kills the encoder and every descendant. The
//! output file is left for the caller to validate, install, or delete.
//! Notes: the guard policy is the same `runtime_guard` policy the Python engine
//! used (memory reserve, swap pressure, disk reserve, projected output size).

use crate::process_output::{kill_owned_group, spawn_owned};
use crate::resource_monitor::RuntimeResourceSnapshot;
use crate::runtime_guard::{GuardObservation, LiveGuard};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const GUARD_INTERVAL: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const STDERR_TAIL_BYTES: usize = 16 * 1024;
const STDERR_REPORT_CHARS: usize = 4000;
const MAX_PROGRESS_LINE_BYTES: usize = 64 * 1024;
const MIB: f64 = 1024.0 * 1024.0;

/// One FFmpeg `-progress` block, already converted to display units.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EncodeProgress {
    pub out_seconds: f64,
    pub speed: Option<f64>,
    pub output_bytes: u64,
    pub file_percent: Option<f64>,
    pub eta_seconds: Option<f64>,
    pub finished: bool,
}

pub(crate) struct EncodeSpec<'a> {
    /// `[ffmpeg, args...]`; the first element is the executable.
    pub args: &'a [String],
    pub expected_duration: f64,
    pub output_path: &'a Path,
    pub memory_guard_gib: f64,
    pub disk_guard_bytes: u64,
    pub projection_margin_percent: f64,
    pub stop_file: Option<&'a Path>,
}

pub(crate) enum EncodeFailure {
    /// The guard asked for a lower-parallelism retry (reserve or swap pressure).
    MemoryPressure {
        reason: &'static str,
        resources: Map<String, Value>,
    },
    Failed {
        message: String,
        resources: Map<String, Value>,
    },
    /// The encoder was stopped from outside (abort); nothing is wrong with the file.
    Interrupted { resources: Map<String, Value> },
}

impl EncodeFailure {
    fn failed(message: impl Into<String>, resources: Map<String, Value>) -> Self {
        Self::Failed {
            message: message.into(),
            resources,
        }
    }
}

/// Incremental parser for FFmpeg's `key=value` progress stream.
pub(crate) struct ProgressParser {
    duration: f64,
    out_seconds: f64,
    speed: Option<f64>,
    output_bytes: u64,
}

impl ProgressParser {
    pub(crate) fn new(duration: f64) -> Self {
        Self {
            duration: if duration.is_finite() {
                duration.max(0.0)
            } else {
                0.0
            },
            out_seconds: 0.0,
            speed: None,
            output_bytes: 0,
        }
    }

    /// Returns a completed block when a `progress=` line closes it.
    pub(crate) fn feed(&mut self, line: &str) -> Option<EncodeProgress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        match key {
            // FFmpeg reports out_time_ms in microseconds as well.
            "out_time_us" | "out_time_ms" => {
                if let Ok(micros) = value.parse::<i64>() {
                    if micros >= 0 {
                        self.out_seconds = micros as f64 / 1_000_000.0;
                    }
                }
            }
            "speed" => {
                self.speed = value
                    .trim_end_matches('x')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|speed| speed.is_finite() && *speed > 0.0);
            }
            "total_size" => {
                if let Ok(bytes) = value.parse::<u64>() {
                    self.output_bytes = bytes;
                }
            }
            "progress" => {
                let finished = value == "end";
                let percent = (self.duration > 0.0).then(|| {
                    if finished {
                        100.0
                    } else {
                        (self.out_seconds / self.duration * 100.0).clamp(0.0, 100.0)
                    }
                });
                let eta = match (self.speed, self.duration > 0.0) {
                    (Some(speed), true) if !finished => {
                        Some(((self.duration - self.out_seconds).max(0.0) / speed).round())
                    }
                    _ => None,
                };
                return Some(EncodeProgress {
                    out_seconds: self.out_seconds,
                    speed: self.speed,
                    output_bytes: self.output_bytes,
                    file_percent: percent,
                    eta_seconds: eta,
                    finished,
                });
            }
            _ => {}
        }
        None
    }
}

#[derive(Default)]
struct PeakMetrics {
    min_available_gib: Option<f64>,
    max_swap_percent: Option<f64>,
    max_rss_gib: Option<f64>,
    max_encoder_swap_gib: Option<f64>,
    max_swap_in: Option<f64>,
    max_swap_out: Option<f64>,
    max_memory_psi: Option<f64>,
}

fn keep_min(slot: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value.filter(|value| value.is_finite()) {
        *slot = Some(slot.map_or(value, |current| current.min(value)));
    }
}

fn keep_max(slot: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value.filter(|value| value.is_finite()) {
        *slot = Some(slot.map_or(value, |current| current.max(value)));
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

impl PeakMetrics {
    fn record(&mut self, snapshot: &RuntimeResourceSnapshot) {
        keep_min(&mut self.min_available_gib, snapshot.memory_available_gib);
        keep_max(&mut self.max_swap_percent, snapshot.swap_used_percent);
        keep_max(
            &mut self.max_rss_gib,
            snapshot
                .ffmpeg_process_rss_gib
                .or(snapshot.engine_process_rss_gib),
        );
        keep_max(
            &mut self.max_encoder_swap_gib,
            snapshot
                .ffmpeg_process_swap_gib
                .or(snapshot.engine_process_swap_gib),
        );
        keep_max(
            &mut self.max_swap_in,
            snapshot.swap_in_bytes_per_second.map(|rate| rate / MIB),
        );
        keep_max(
            &mut self.max_swap_out,
            snapshot.swap_out_bytes_per_second.map(|rate| rate / MIB),
        );
        keep_max(&mut self.max_memory_psi, snapshot.memory_full_avg10);
    }

    fn into_report_fields(self) -> Map<String, Value> {
        let mut fields = Map::new();
        for (key, value) in [
            ("min_memory_available_gib", self.min_available_gib),
            ("max_swap_used_percent", self.max_swap_percent),
            ("max_encoder_rss_gib", self.max_rss_gib),
            ("max_encoder_swap_gib", self.max_encoder_swap_gib),
            ("max_swap_in_mib_s", self.max_swap_in),
            ("max_swap_out_mib_s", self.max_swap_out),
            ("max_memory_full_psi_percent", self.max_memory_psi),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), json!(round3(value)));
            }
        }
        fields
    }
}

fn read_progress(
    stream: impl Read,
    duration: f64,
    sender: mpsc::Sender<EncodeProgress>,
) -> Result<(), String> {
    let mut parser = ProgressParser::new(duration);
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::with_capacity(256);
    loop {
        buffer.clear();
        let read = reader
            .by_ref()
            .take(MAX_PROGRESS_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut buffer)
            .map_err(|error| format!("Could not read FFmpeg progress output: {error}"))?;
        if read == 0 {
            return Ok(());
        }
        if buffer.len() > MAX_PROGRESS_LINE_BYTES {
            return Err(format!(
                "FFmpeg progress line exceeded {MAX_PROGRESS_LINE_BYTES} bytes."
            ));
        }
        if let Some(block) = parser.feed(&String::from_utf8_lossy(&buffer)) {
            if sender.send(block).is_err() {
                return Ok(());
            }
        }
    }
}

fn drain_stderr_tail(mut stream: impl Read) -> String {
    let mut tail = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                tail.extend(&buffer[..read]);
                while tail.len() > STDERR_TAIL_BYTES {
                    tail.pop_front();
                }
            }
        }
    }
    let bytes: Vec<u8> = tail.into_iter().collect();
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    let characters = text.chars().count();
    if characters > STDERR_REPORT_CHARS {
        text.chars()
            .skip(characters - STDERR_REPORT_CHARS)
            .collect()
    } else {
        text
    }
}

fn killed_by_signal(status: &ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().is_some()
    }
    #[cfg(not(unix))]
    {
        status.code().is_none()
    }
}

enum Halt {
    Memory(&'static str),
    Failure(String),
}

fn describe_disk_halt(reason: Option<&str>, free: Option<u64>, projected: Option<u64>) -> String {
    let gib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    match (reason, free, projected) {
        (Some("projected_output"), Some(free), Some(projected)) => format!(
            "Stopped to protect disk space: the remaining output is projected at {:.1} GiB but only {:.1} GiB is free above the reserve.",
            gib(projected),
            gib(free)
        ),
        (_, Some(free), _) => format!(
            "Stopped to protect disk space: only {:.1} GiB is free, below the configured reserve.",
            gib(free)
        ),
        _ => "Stopped to protect disk space.".into(),
    }
}

/// Run one FFmpeg encode to completion under the live resource guard.
pub(crate) fn run_encode(
    spec: &EncodeSpec<'_>,
    mut on_progress: impl FnMut(&EncodeProgress),
    mut on_child: impl FnMut(Option<u32>),
) -> Result<Map<String, Value>, EncodeFailure> {
    let Some((program, rest)) = spec.args.split_first() else {
        return Err(EncodeFailure::failed(
            "The FFmpeg command was empty.",
            Map::new(),
        ));
    };
    let mut command = Command::new(program);
    command
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_owned(&mut command).map_err(|error| {
        EncodeFailure::failed(format!("Could not start FFmpeg: {error}"), Map::new())
    })?;
    let pid = child.id();
    on_child(Some(pid));

    let (sender, receiver) = mpsc::channel();
    let duration = spec.expected_duration;
    let progress_thread = child
        .stdout
        .take()
        .map(|stream| thread::spawn(move || read_progress(stream, duration, sender)));
    let stderr_thread = child
        .stderr
        .take()
        .map(|stream| thread::spawn(move || drain_stderr_tail(stream)));

    let mut guard = LiveGuard::new();
    let mut peaks = PeakMetrics::default();
    let mut last_guard = Instant::now()
        .checked_sub(GUARD_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut latest = EncodeProgress::default();
    let mut halt: Option<Halt> = None;
    loop {
        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(block) => {
                latest = block;
                on_progress(&latest);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_guard.elapsed() < GUARD_INTERVAL {
            continue;
        }
        last_guard = Instant::now();
        let output_bytes = fs::metadata(spec.output_path)
            .map(|meta| meta.len())
            .unwrap_or(latest.output_bytes);
        match guard.observe(GuardObservation {
            pid,
            disk_path: spec.output_path.parent().unwrap_or_else(|| Path::new(".")),
            memory_guard_gib: spec.memory_guard_gib,
            disk_guard_bytes: spec.disk_guard_bytes,
            projection_margin_percent: spec.projection_margin_percent,
            progress_seconds: latest.out_seconds,
            expected_duration: spec.expected_duration,
            output_bytes,
        }) {
            Ok((decision, snapshot)) => {
                peaks.record(&snapshot);
                match decision.action {
                    "retry_memory" => {
                        halt = Some(Halt::Memory(decision.reason.unwrap_or("memory_reserve")));
                    }
                    "stop_disk" => {
                        halt = Some(Halt::Failure(describe_disk_halt(
                            decision.reason,
                            decision.free_bytes,
                            decision.projected_remaining_bytes,
                        )));
                    }
                    _ => {}
                }
            }
            Err(error) => halt = Some(Halt::Failure(error)),
        }
        if halt.is_some() {
            let _ = kill_owned_group(pid);
            break;
        }
    }

    let waited = child.wait();
    on_child(None);
    let progress_error = progress_thread
        .and_then(|handle| handle.join().ok())
        .and_then(Result::err);
    let stderr_tail = stderr_thread
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let resources = peaks.into_report_fields();

    if let Some(halt) = halt {
        return Err(match halt {
            Halt::Memory(reason) => EncodeFailure::MemoryPressure { reason, resources },
            Halt::Failure(message) => EncodeFailure::failed(message, resources),
        });
    }
    let status = waited.map_err(|error| {
        EncodeFailure::failed(
            format!("Could not wait for FFmpeg: {error}"),
            resources.clone(),
        )
    })?;
    if status.success() {
        if let Some(error) = progress_error {
            return Err(EncodeFailure::failed(error, resources));
        }
        return Ok(resources);
    }
    let stop_requested = spec.stop_file.is_some_and(crate::stop_control::requested);
    // An abort requests a safe stop before killing the group, so a signal death
    // with a stop marker is an operator stop; a bare signal (for example the
    // kernel OOM killer) is a real failure the report must say so about.
    if killed_by_signal(&status) && stop_requested {
        return Err(EncodeFailure::Interrupted { resources });
    }
    if killed_by_signal(&status) {
        return Err(EncodeFailure::failed(
            "FFmpeg was terminated by the system (possibly out of memory) before it finished.",
            resources,
        ));
    }
    let code = status
        .code()
        .map_or_else(|| "terminated".to_string(), |code| code.to_string());
    let detail = progress_error.unwrap_or(stderr_tail);
    Err(EncodeFailure::failed(
        if detail.is_empty() {
            format!("FFmpeg failed (exit {code}).")
        } else {
            format!("FFmpeg failed (exit {code}): {detail}")
        },
        resources,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_blocks_convert_to_percent_speed_and_eta() {
        let mut parser = ProgressParser::new(100.0);
        for line in [
            "frame=120",
            "total_size=2048",
            "out_time_us=25000000",
            "out_time_ms=25000000",
            "speed=2.5x",
        ] {
            assert!(parser.feed(line).is_none());
        }
        let block = parser.feed("progress=continue").unwrap();
        assert_eq!(block.file_percent, Some(25.0));
        assert_eq!(block.speed, Some(2.5));
        assert_eq!(block.output_bytes, 2048);
        assert_eq!(block.eta_seconds, Some(30.0));
        assert!(!block.finished);
        let end = parser.feed("progress=end").unwrap();
        assert!(end.finished);
        assert_eq!(end.file_percent, Some(100.0));
        assert_eq!(end.eta_seconds, None);
    }

    #[test]
    fn unknown_values_never_break_the_parser() {
        let mut parser = ProgressParser::new(0.0);
        parser.feed("out_time_us=N/A");
        parser.feed("speed=N/A");
        parser.feed("total_size=N/A");
        let block = parser.feed("progress=continue").unwrap();
        assert_eq!(block.file_percent, None);
        assert_eq!(block.speed, None);
        assert_eq!(block.output_bytes, 0);
        assert!(parser.feed("not a key value line").is_none());
    }

    #[test]
    fn peak_metrics_keep_extremes_and_use_report_keys() {
        let mut peaks = PeakMetrics::default();
        for (available, swap_in) in [(12.0, 1.0 * MIB), (4.5, 6.0 * MIB), (8.0, 2.0 * MIB)] {
            peaks.record(&RuntimeResourceSnapshot {
                memory_available_gib: Some(available),
                swap_in_bytes_per_second: Some(swap_in),
                ..RuntimeResourceSnapshot::default()
            });
        }
        let fields = peaks.into_report_fields();
        assert_eq!(fields["min_memory_available_gib"], json!(4.5));
        assert_eq!(fields["max_swap_in_mib_s"], json!(6.0));
        assert!(!fields.contains_key("max_encoder_rss_gib"));
    }

    #[cfg(unix)]
    #[test]
    fn run_encode_reports_progress_and_success() {
        let script = "printf 'out_time_us=1000000\\nspeed=1.0x\\ntotal_size=10\\nprogress=continue\\nout_time_us=2000000\\nprogress=end\\n'";
        let args = vec!["sh".to_string(), "-c".into(), script.into()];
        let output = std::env::temp_dir().join("hevc-encode-run-success.mp4");
        let mut blocks = Vec::new();
        let mut children = Vec::new();
        let result = run_encode(
            &EncodeSpec {
                args: &args,
                expected_duration: 2.0,
                output_path: &output,
                memory_guard_gib: 0.0,
                disk_guard_bytes: 0,
                projection_margin_percent: 15.0,
                stop_file: None,
            },
            |block| blocks.push(block.clone()),
            |pid| children.push(pid),
        );
        assert!(result.is_ok());
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].file_percent, Some(50.0));
        assert!(blocks[1].finished);
        assert!(matches!(children.as_slice(), [Some(_), None]));
    }

    #[cfg(unix)]
    #[test]
    fn run_encode_surfaces_ffmpeg_errors_and_external_kills() {
        let output = std::env::temp_dir().join("hevc-encode-run-failure.mp4");
        let args = vec![
            "sh".to_string(),
            "-c".into(),
            "echo 'Unknown encoder' >&2; exit 3".into(),
        ];
        let result = run_encode(
            &EncodeSpec {
                args: &args,
                expected_duration: 1.0,
                output_path: &output,
                memory_guard_gib: 0.0,
                disk_guard_bytes: 0,
                projection_margin_percent: 15.0,
                stop_file: None,
            },
            |_| {},
            |_| {},
        );
        match result {
            Err(EncodeFailure::Failed { message, .. }) => {
                assert!(message.contains("exit 3") && message.contains("Unknown encoder"));
            }
            _ => panic!("a non-zero FFmpeg exit must be a failure"),
        }
        let killed = vec!["sh".to_string(), "-c".into(), "kill -9 $$".into()];
        let spec = |stop_file| EncodeSpec {
            args: &killed,
            expected_duration: 1.0,
            output_path: &output,
            memory_guard_gib: 0.0,
            disk_guard_bytes: 0,
            projection_margin_percent: 15.0,
            stop_file,
        };
        // A bare signal death (OOM killer) is a failure; with a stop marker it is an abort.
        assert!(matches!(
            run_encode(&spec(None), |_| {}, |_| {}),
            Err(EncodeFailure::Failed { .. })
        ));
        let token = crate::stop_control::StopToken::new().unwrap();
        crate::stop_control::request(token.path()).unwrap();
        assert!(matches!(
            run_encode(&spec(Some(token.path())), |_| {}, |_| {}),
            Err(EncodeFailure::Interrupted { .. })
        ));
    }
}
