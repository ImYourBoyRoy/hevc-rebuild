//! Stateful live memory and disk guard policy used by the compatibility engine.
//!
//! The Python engine remains the FFmpeg process/UI supervisor during migration.
//! It sends process/output/progress context over this private JSON-lines
//! interface; Rust samples the host and owns live retry/stop decisions.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use std::{ffi::OsString, path::PathBuf};

use crate::resource_monitor::{RuntimeResourceSnapshot, Sampler};

const SWAP_IN_PRESSURE_BPS: f64 = 4.0 * 1024.0 * 1024.0;
const SWAP_PRESSURE_MIN_SAMPLES: u32 = 5;
const MEMORY_FULL_PRESSURE_PERCENT: f64 = 1.0;
const MEMORY_RESERVE_MIN_SAMPLES: u32 = 3;
const DISK_RESERVE_MIN_SAMPLES: u32 = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardSample {
    available_gib: Option<f64>,
    memory_guard_gib: f64,
    swap_in_bytes_per_second: Option<f64>,
    memory_full_avg10: Option<f64>,
    disk_free_bytes: Option<u64>,
    disk_guard_bytes: u64,
    projection_margin_percent: f64,
    progress_seconds: f64,
    expected_duration: f64,
    output_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuardRequest {
    pid: u32,
    disk_path: Option<PathBuf>,
    memory_guard_gib: f64,
    disk_guard_bytes: u64,
    projection_margin_percent: f64,
    progress_seconds: f64,
    expected_duration: f64,
    output_bytes: u64,
}

#[derive(Debug, Serialize)]
struct GuardResponse {
    decision: GuardDecision,
    snapshot: RuntimeResourceSnapshot,
}

#[derive(Debug, Serialize)]
struct GuardDecision {
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    available_gib: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    projected_remaining_bytes: Option<u64>,
}

impl GuardDecision {
    fn continue_run() -> Self {
        Self {
            action: "continue",
            reason: None,
            available_gib: None,
            free_bytes: None,
            projected_remaining_bytes: None,
        }
    }

    fn memory_retry(reason: &'static str, available_gib: Option<f64>) -> Self {
        Self {
            action: "retry_memory",
            reason: Some(reason),
            available_gib,
            free_bytes: None,
            projected_remaining_bytes: None,
        }
    }

    fn stop_disk(
        reason: &'static str,
        free_bytes: u64,
        projected_remaining_bytes: Option<u64>,
    ) -> Self {
        Self {
            action: "stop_disk",
            reason: Some(reason),
            available_gib: None,
            free_bytes: Some(free_bytes),
            projected_remaining_bytes,
        }
    }
}

#[derive(Default)]
struct GuardPolicy {
    low_memory_samples: u32,
    swap_pressure_samples: u32,
    low_disk_samples: u32,
}

impl GuardPolicy {
    fn evaluate(&mut self, sample: GuardSample) -> Result<GuardDecision, String> {
        validate_sample(&sample)?;

        if sample.memory_guard_gib > 0.0
            && sample
                .available_gib
                .is_some_and(|available| available < sample.memory_guard_gib)
        {
            self.low_memory_samples = self.low_memory_samples.saturating_add(1);
        } else {
            self.low_memory_samples = 0;
        }
        if self.low_memory_samples >= MEMORY_RESERVE_MIN_SAMPLES {
            return Ok(GuardDecision::memory_retry(
                "memory_reserve",
                sample.available_gib,
            ));
        }

        if sample
            .swap_in_bytes_per_second
            .is_some_and(|rate| rate >= SWAP_IN_PRESSURE_BPS)
            && sample
                .memory_full_avg10
                .is_some_and(|pressure| pressure >= MEMORY_FULL_PRESSURE_PERCENT)
        {
            self.swap_pressure_samples = self.swap_pressure_samples.saturating_add(1);
        } else {
            self.swap_pressure_samples = 0;
        }
        if self.swap_pressure_samples >= SWAP_PRESSURE_MIN_SAMPLES {
            return Ok(GuardDecision::memory_retry(
                "swap_pressure",
                sample.available_gib,
            ));
        }

        if let Some(free_bytes) = sample.disk_free_bytes {
            if sample.disk_guard_bytes > 0 {
                if free_bytes < sample.disk_guard_bytes {
                    self.low_disk_samples = self.low_disk_samples.saturating_add(1);
                } else {
                    self.low_disk_samples = 0;
                }
                if self.low_disk_samples >= DISK_RESERVE_MIN_SAMPLES {
                    return Ok(GuardDecision::stop_disk("disk_reserve", free_bytes, None));
                }
            }

            let fraction = if sample.expected_duration > 0.0 {
                sample.progress_seconds / sample.expected_duration
            } else {
                0.0
            };
            if fraction >= 0.10 && sample.progress_seconds >= 10.0 && sample.output_bytes > 0 {
                let projected_total = (sample.output_bytes as f64 / fraction.max(0.001)).trunc();
                let projected_remaining = (projected_total - sample.output_bytes as f64).max(0.0);
                let safe_remaining = free_bytes.saturating_sub(sample.disk_guard_bytes);
                let margin = 1.0 + sample.projection_margin_percent / 100.0;
                if projected_remaining * margin > safe_remaining as f64 {
                    return Ok(GuardDecision::stop_disk(
                        "projected_output",
                        free_bytes,
                        Some(projected_remaining as u64),
                    ));
                }
            }
        }

        Ok(GuardDecision::continue_run())
    }
}

fn validate_sample(sample: &GuardSample) -> Result<(), String> {
    for (name, value) in [
        ("available_gib", sample.available_gib),
        ("swap_in_bytes_per_second", sample.swap_in_bytes_per_second),
        ("memory_full_avg10", sample.memory_full_avg10),
    ] {
        if value.is_some_and(|number| !number.is_finite() || number < 0.0) {
            return Err(format!(
                "Runtime guard field {name} must be finite and non-negative."
            ));
        }
    }
    for (name, value) in [
        ("memory_guard_gib", sample.memory_guard_gib),
        (
            "projection_margin_percent",
            sample.projection_margin_percent,
        ),
        ("progress_seconds", sample.progress_seconds),
        ("expected_duration", sample.expected_duration),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!(
                "Runtime guard field {name} must be finite and non-negative."
            ));
        }
    }
    Ok(())
}

fn handle_line(
    policy: &mut GuardPolicy,
    sampler: &mut Sampler,
    line: &str,
) -> Result<Value, String> {
    let request: GuardRequest = serde_json::from_str(line)
        .map_err(|error| format!("Invalid Rust runtime-guard request: {error}"))?;
    if request.pid == 0
        || request
            .disk_path
            .as_ref()
            .is_some_and(|path| path.as_os_str().len() > 4096)
    {
        return Err("Rust runtime-guard process ID or disk path is invalid.".into());
    }
    let snapshot = sampler.sample(request.pid, request.disk_path.as_deref());
    let sample = GuardSample {
        available_gib: snapshot.memory_available_gib,
        memory_guard_gib: request.memory_guard_gib,
        swap_in_bytes_per_second: snapshot.swap_in_bytes_per_second,
        memory_full_avg10: snapshot.memory_full_avg10,
        disk_free_bytes: snapshot.disk_free_bytes,
        disk_guard_bytes: request.disk_guard_bytes,
        projection_margin_percent: request.projection_margin_percent,
        progress_seconds: request.progress_seconds,
        expected_duration: request.expected_duration,
        output_bytes: request.output_bytes,
    };
    let decision = policy.evaluate(sample)?;
    serde_json::to_value(GuardResponse { decision, snapshot })
        .map_err(|error| format!("Could not serialize Rust runtime-guard decision: {error}"))
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    if !args.is_empty() {
        eprintln!("The internal runtime-guard command does not accept options.");
        return 2;
    }
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut policy = GuardPolicy::default();
    let mut sampler = Sampler::new();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("Could not read runtime-guard input: {error}");
                return 1;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match handle_line(&mut policy, &mut sampler, &line) {
            Ok(decision) => decision,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };
        if let Err(error) = serde_json::to_writer(&mut output, &response)
            .and_then(|_| writeln!(output).map_err(serde_json::Error::io))
            .and_then(|_| output.flush().map_err(serde_json::Error::io))
        {
            eprintln!("Could not write runtime-guard decision: {error}");
            return 1;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{handle_line, GuardPolicy, GuardSample};
    use crate::resource_monitor::Sampler;

    fn sample() -> GuardSample {
        GuardSample {
            available_gib: Some(8.0),
            memory_guard_gib: 3.0,
            swap_in_bytes_per_second: None,
            memory_full_avg10: None,
            disk_free_bytes: Some(1000),
            disk_guard_bytes: 0,
            projection_margin_percent: 15.0,
            progress_seconds: 1.0,
            expected_duration: 100.0,
            output_bytes: 0,
        }
    }

    #[test]
    fn memory_reserve_requires_three_consecutive_samples() {
        let mut policy = GuardPolicy::default();
        let mut input = sample();
        input.available_gib = Some(2.9);
        assert_eq!(policy.evaluate(input).unwrap().action, "continue");
        let mut input = sample();
        input.available_gib = Some(2.9);
        assert_eq!(policy.evaluate(input).unwrap().action, "continue");
        let mut input = sample();
        input.available_gib = Some(2.9);
        let decision = policy.evaluate(input).unwrap();
        assert_eq!(decision.action, "retry_memory");
        assert_eq!(decision.reason, Some("memory_reserve"));
        assert_eq!(decision.available_gib, Some(2.9));

        let mut recovered = sample();
        recovered.available_gib = Some(6.0);
        assert_eq!(policy.evaluate(recovered).unwrap().action, "continue");
    }

    #[test]
    fn swap_retry_requires_five_pressure_samples_and_full_memory_psi() {
        let mut policy = GuardPolicy::default();
        for _ in 0..4 {
            let mut input = sample();
            input.swap_in_bytes_per_second = Some(4.0 * 1024.0 * 1024.0);
            input.memory_full_avg10 = Some(1.0);
            assert_eq!(policy.evaluate(input).unwrap().action, "continue");
        }
        let mut input = sample();
        input.swap_in_bytes_per_second = Some(4.0 * 1024.0 * 1024.0);
        input.memory_full_avg10 = Some(1.0);
        let decision = policy.evaluate(input).unwrap();
        assert_eq!(decision.action, "retry_memory");
        assert_eq!(decision.reason, Some("swap_pressure"));
    }

    #[test]
    fn configured_disk_reserve_requires_two_samples() {
        let mut policy = GuardPolicy::default();
        let mut input = sample();
        input.disk_guard_bytes = 2000;
        input.disk_free_bytes = Some(1999);
        assert_eq!(policy.evaluate(input).unwrap().action, "continue");
        let mut input = sample();
        input.disk_guard_bytes = 2000;
        input.disk_free_bytes = Some(1000);
        let decision = policy.evaluate(input).unwrap();
        assert_eq!(decision.action, "stop_disk");
        assert_eq!(decision.reason, Some("disk_reserve"));
    }

    #[test]
    fn projected_output_guard_stays_active_when_optional_reserve_is_zero() {
        let mut policy = GuardPolicy::default();
        let mut input = sample();
        input.disk_free_bytes = Some(450);
        input.progress_seconds = 20.0;
        input.expected_duration = 100.0;
        input.output_bytes = 100;
        let decision = policy.evaluate(input).unwrap();
        assert_eq!(decision.action, "stop_disk");
        assert_eq!(decision.reason, Some("projected_output"));
        assert_eq!(decision.projected_remaining_bytes, Some(400));

        let mut enough_space = sample();
        enough_space.disk_free_bytes = Some(460);
        enough_space.progress_seconds = 20.0;
        enough_space.expected_duration = 100.0;
        enough_space.output_bytes = 100;
        assert_eq!(policy.evaluate(enough_space).unwrap().action, "continue");
    }

    #[test]
    fn invalid_sample_is_rejected_without_a_continue_decision() {
        let mut policy = GuardPolicy::default();
        let mut input = sample();
        input.progress_seconds = f64::NAN;
        assert!(policy.evaluate(input).is_err());
    }

    #[test]
    fn json_lines_protocol_returns_native_sample_and_guard_decision() {
        let mut policy = GuardPolicy::default();
        let mut sampler = Sampler::new();
        let request = serde_json::json!({
            "pid": std::process::id(),
            "disk_path": null,
            "memory_guard_gib": 0.0,
            "disk_guard_bytes": 0,
            "projection_margin_percent": 15.0,
            "progress_seconds": 0.0,
            "expected_duration": 1.0,
            "output_bytes": 0
        });
        let response = handle_line(&mut policy, &mut sampler, &request.to_string()).unwrap();
        assert_eq!(response["decision"]["action"], "continue");
        assert!(response["snapshot"]["memory_available_gib"].is_number());
        assert!(
            response["snapshot"]["memory_full_avg10"].is_number()
                || response["snapshot"]["memory_full_avg10"].is_null()
        );
    }
}
