//! Live progress events and the closing summary lines.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn emit(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    phase: &str,
    file_fraction: f64,
    file_percent: Option<f64>,
    output_bytes: Option<u64>,
    speed: Option<f64>,
    eta_seconds: Option<f64>,
) {
    let fraction = file_fraction.clamp(0.0, 1.0);
    (context.progress)(RebuildProgress {
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
        queue_percent: ((frame.number.saturating_sub(1)) as f64 + fraction)
            / frame.total.max(1) as f64
            * 100.0,
        elapsed_seconds: frame.started.elapsed().as_secs_f64(),
        speed,
        eta_seconds,
        input_bytes: frame.input_bytes,
        output_bytes,
        estimated_output_bytes: None,
    });
}

pub(crate) fn closing_lines(job: &RebuildJobResult) -> Vec<String> {
    let total = job.files.len();
    let mode = if job
        .files
        .iter()
        .any(|f| f.status == RunStatus::TestOk.as_str())
    {
        "Short test"
    } else {
        "Full rebuild"
    };
    let summary = if let Some(reason) = job.halted {
        format!(
            "{mode} {}: {} succeeded, {} failed safely.",
            reason.code().replace('_', " "),
            job.succeeded,
            job.failed
        )
    } else if job.stop_requested {
        format!(
            "{mode} stopped after {total} file{}; remaining files were not changed.",
            if total == 1 { "" } else { "s" }
        )
    } else if job.failed > 0 {
        format!(
            "{mode} finished: {} succeeded, {} failed safely.",
            job.succeeded, job.failed
        )
    } else if total == 0 {
        format!("{mode} had no files to process.")
    } else {
        format!(
            "{mode} finished: {} file{} succeeded.",
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
