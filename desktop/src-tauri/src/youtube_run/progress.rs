//! Progress events, per-file and per-job results, and the closing summary lines.

use super::*;

#[derive(Clone, Debug)]
pub(crate) struct YoutubeProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: String,
    pub file_percent: Option<f64>,
    pub queue_percent: f64,
    pub elapsed_seconds: f64,
    pub speed: Option<f64>,
    pub eta_seconds: Option<f64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct YoutubeFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
}

pub(crate) struct YoutubeJobResult {
    pub files: Vec<YoutubeFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub destination_root: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
    pub succeeded: usize,
    pub skipped: usize,
}

pub(super) struct Frame<'a> {
    pub(super) source: &'a Path,
    pub(super) number: usize,
    pub(super) total: usize,
    pub(super) started: Instant,
    pub(super) input_bytes: u64,
}

#[derive(Default)]
pub(super) struct Live {
    pub(super) estimated_output_bytes: Option<u64>,
    pub(super) output_bytes: Option<u64>,
    pub(super) speed: Option<f64>,
    pub(super) eta_seconds: Option<f64>,
}

pub(super) fn emit(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    live: &Live,
) {
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(YoutubeProgress {
        current: frame.number,
        total: frame.total,
        file_name: frame
            .source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_path: frame.source.to_string_lossy().into_owned(),
        phase: phase.into(),
        file_percent,
        queue_percent: (frame.number.saturating_sub(1) as f64 + fraction)
            / frame.total.max(1) as f64
            * 100.0,
        elapsed_seconds: frame.started.elapsed().as_secs_f64(),
        speed: live.speed,
        eta_seconds: live.eta_seconds,
        input_bytes: Some(frame.input_bytes),
        output_bytes: live.output_bytes,
        estimated_output_bytes: live.estimated_output_bytes,
    });
}

pub(super) fn note(log: &mut fs::File, line: &mut dyn FnMut(String), text: String) {
    let _ = writeln!(log, "{text}");
    line(text);
}

/// Lines the desktop and MCP consumers read from engine output after a run.
pub(crate) fn closing_lines(job: &YoutubeJobResult) -> Vec<String> {
    let total = job.files.len();
    let summary = if job.stop_requested {
        format!(
            "YouTube export stopped after {total} file{}; remaining files were not started.",
            if total == 1 { "" } else { "s" }
        )
    } else if total == 0 {
        "YouTube export had no files to process.".to_string()
    } else {
        format!(
            "YouTube export finished: {} validated cop{}, {} skipped, {} failed safely. Originals stayed in place.",
            job.succeeded,
            if job.succeeded == 1 { "y" } else { "ies" },
            job.skipped,
            job.failed
        )
    };
    vec![
        format!("Log: {}", job.log_path.display()),
        format!("Run report: {}", job.report_path.display()),
        summary,
        format!("Output folder: {}", job.destination_root.display()),
    ]
}
