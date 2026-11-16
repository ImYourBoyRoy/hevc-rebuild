//! Progress events, per-file and per-job results, and the closing summary lines.

use super::*;

#[derive(Clone, Debug)]
pub(crate) struct MetadataProgress {
    pub current: usize,
    pub total: usize,
    pub file_name: String,
    pub file_path: String,
    pub phase: String,
    pub file_percent: Option<f64>,
    pub queue_percent: f64,
    pub elapsed_seconds: f64,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetadataFileResult {
    pub path: String,
    pub status: String,
    pub message: String,
    pub output: Option<String>,
    pub elapsed_seconds: f64,
}

pub(crate) struct MetadataJobResult {
    pub files: Vec<MetadataFileResult>,
    pub log_path: PathBuf,
    pub report_path: PathBuf,
    pub stop_requested: bool,
    pub failed: usize,
    pub succeeded: usize,
    pub skipped: usize,
}

/// Per-file facts shared by every progress event.
pub(super) struct FileFrame<'a> {
    pub(super) source: &'a Path,
    pub(super) number: usize,
    pub(super) total: usize,
    pub(super) started: Instant,
    pub(super) input_bytes: Option<u64>,
}

pub(super) fn emit(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    output_bytes: Option<u64>,
) {
    let FileFrame {
        source,
        number,
        total,
        started,
        input_bytes,
    } = *frame;
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(MetadataProgress {
        current: number,
        total,
        file_name: source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        file_path: source.to_string_lossy().into_owned(),
        phase: phase.into(),
        file_percent,
        queue_percent: ((number.saturating_sub(1)) as f64 + fraction) / total.max(1) as f64 * 100.0,
        elapsed_seconds: started.elapsed().as_secs_f64(),
        input_bytes,
        output_bytes,
    });
}

pub(super) fn failure_fraction(phase: &str) -> f64 {
    match phase {
        "metadata embedding" => 0.90,
        "lossless output validation" | "output validation" => 0.97,
        "validated output install" | "transaction commit" => 0.99,
        _ => 0.0,
    }
}

/// Lines the desktop and MCP consumers read from engine output after a run.
pub(crate) fn closing_lines(job: &MetadataJobResult) -> Vec<String> {
    let total = job.files.len();
    let summary = if job.stop_requested {
        format!(
            "Metadata-only MKV stopped after {total} file{}; remaining files were not changed.",
            if total == 1 { "" } else { "s" }
        )
    } else if job.failed > 0 {
        format!(
            "Metadata-only MKV finished: {} validated, {} failed safely.",
            job.succeeded, job.failed
        )
    } else if total == 0 {
        "Metadata-only MKV had no files to process.".to_string()
    } else {
        format!(
            "Metadata-only MKV finished: {} validated output{}. Originals stayed in place.",
            job.succeeded,
            if job.succeeded == 1 { "" } else { "s" }
        )
    };
    let skipped = if job.skipped > 0 {
        format!(
            " {} already-processed file{} skipped.",
            job.skipped,
            if job.skipped == 1 { " was" } else { "s were" }
        )
    } else {
        String::new()
    };
    vec![
        format!("Log: {}", job.log_path.display()),
        format!("Run report: {}", job.report_path.display()),
        format!("{summary}{skipped}"),
    ]
}
