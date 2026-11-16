//! Supervised FFmpeg encode with adaptive memory-aware retries, shared by the upscale and optimize paths.

use super::*;

pub(super) struct EncodeJob<'a> {
    pub(super) duration: f64,
    pub(super) estimated_bytes: u64,
    pub(super) reserve: u64,
    pub(super) phase: &'a str,
    pub(super) temp_path: &'a Path,
}

pub(super) fn string_args(built: &Value) -> Result<Vec<String>, String> {
    Ok(built
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("The YouTube planner returned no FFmpeg arguments."))?
        .iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect())
}

/// Run one supervised FFmpeg job, retrying with lower parallelism on memory
/// pressure. `make_args` rebuilds the argument vector for each attempt's plan.
pub(super) fn encode_with_retries(
    context: &mut Context<'_>,
    frame: &Frame<'_>,
    live: &mut Live,
    row: &mut Map<String, Value>,
    job: &EncodeJob<'_>,
    mut plan: EncodePlan,
    make_args: &dyn Fn(&EncodePlan) -> Result<Vec<String>, String>,
) -> Result<(), Halt> {
    let (source, number, total) = (frame.source, frame.number, frame.total);
    let options = context.options;
    let name = source
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut merged: Map<String, Value> = Map::new();
    let mut attempt = 1;
    loop {
        let args = make_args(&plan)?;
        emit(context, frame, job.phase, 0.05, Some(0.0), live);
        let progress = &mut *context.progress;
        let child = &mut *context.child;
        let spec = EncodeSpec {
            args: &args,
            expected_duration: job.duration,
            output_path: job.temp_path,
            memory_guard_gib: plan.memory_guard_gib,
            disk_guard_bytes: job.reserve,
            projection_margin_percent: options.projection_margin_percent(),
            stop_file: options.stop_file.as_deref(),
        };
        let result = run_encode(
            &spec,
            |block: &EncodeProgress| {
                live.speed = block.speed;
                live.eta_seconds = block.eta_seconds;
                live.output_bytes = Some(block.output_bytes);
                let percent = block.file_percent;
                progress(YoutubeProgress {
                    current: number,
                    total,
                    file_name: name.clone(),
                    file_path: source.to_string_lossy().into_owned(),
                    phase: job.phase.into(),
                    file_percent: percent,
                    queue_percent: (number.saturating_sub(1) as f64
                        + 0.05
                        + 0.90 * percent.unwrap_or(0.0) / 100.0)
                        / total.max(1) as f64
                        * 100.0,
                    elapsed_seconds: frame.started.elapsed().as_secs_f64(),
                    speed: block.speed,
                    eta_seconds: block.eta_seconds,
                    input_bytes: Some(frame.input_bytes),
                    output_bytes: Some(block.output_bytes),
                    estimated_output_bytes: Some(job.estimated_bytes),
                });
            },
            child,
        );
        match result {
            Ok(metrics) => {
                merged.extend(metrics);
                break;
            }
            Err(EncodeFailure::MemoryPressure { reason, resources }) => {
                merged.extend(resources);
                let next = if attempt >= MAX_MEMORY_ATTEMPTS {
                    None
                } else {
                    plan.reduced()
                };
                let Some(next) = next else {
                    row.extend(merged);
                    return Err(Halt::Fail(
                        "Memory pressure persisted after four safe attempts; the source remains untouched."
                            .into(),
                    ));
                };
                let _ = fs::remove_file(job.temp_path);
                (context.line)(format!(
                    "Memory pressure detected ({reason}); retrying {name} with {} encoder thread(s)",
                    next.workers
                ));
                plan = next;
                attempt += 1;
            }
            Err(EncodeFailure::Interrupted { resources }) => {
                row.extend(resources);
                return Err(Halt::Interrupted);
            }
            Err(EncodeFailure::Failed { message, resources }) => {
                row.extend(resources);
                return Err(Halt::Fail(message));
            }
        }
    }
    row.extend(merged);
    Ok(())
}
