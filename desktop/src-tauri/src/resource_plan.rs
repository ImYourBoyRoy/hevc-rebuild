//! Shared Rust profile-aware resource planning for preflight and engine runs.
//!
//! Rust-launched engine runs call the same planner used by native media scans.
//! The Python compatibility engine still samples host resources and applies the
//! resulting worker/thread values to FFmpeg. Keep direct-Python parity tests in sync.

use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
};

fn setting_number(settings: &Value, key: &str, fallback: f64) -> f64 {
    settings
        .pointer(key)
        .and_then(Value::as_f64)
        .unwrap_or(fallback)
}

fn value_i64_ref(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn setting_integer(settings: &Value, key: &str, fallback: i64) -> i64 {
    settings
        .pointer(key)
        .and_then(value_i64_ref)
        .unwrap_or(fallback)
}

fn setting_string<'a>(settings: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    settings
        .pointer(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
}

pub(crate) fn memory_forecast(
    profile: &Value,
    settings: &Value,
    host: &Value,
    full_memory_psi: Option<f64>,
    target_dimensions: Option<(i64, i64)>,
    preset_override: Option<&str>,
    frame_threads_override: Option<i64>,
    concurrent_video_streams: i64,
) -> Value {
    let concurrent = concurrent_video_streams.clamp(1, 16);
    let hardware = host.get("hardware").cloned().unwrap_or_else(|| json!({}));
    let cpu = hardware
        .get("logical_cpus")
        .and_then(value_i64_ref)
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let available = value_f64(hardware.get("memory_available_gib"));
    let total = value_f64(hardware.get("memory_total_gib"));
    let swap_free = value_f64(hardware.get("swap_free_gib"));
    let swap_used = value_f64(hardware.get("swap_used_gib"));
    let swap_percent = value_f64(hardware.get("swap_used_percent"));
    let floor = setting_number(settings, "/resources/min_free_memory_gib", 3.0).max(0.0);
    let reserve_percent =
        setting_number(settings, "/resources/memory_reserve_percent", 8.0).max(0.0);
    let reserve = total
        .map(|value| floor.max(value * reserve_percent / 100.0))
        .unwrap_or(floor);
    let (width, height) = target_dimensions.unwrap_or_else(|| {
        (
            profile
                .pointer("/video/width")
                .and_then(value_i64_ref)
                .unwrap_or(0),
            profile
                .pointer("/video/height")
                .and_then(value_i64_ref)
                .unwrap_or(0),
        )
    });
    let pixels = (width.max(1) * height.max(1)) as f64;
    let threading = setting_string(settings, "/resources/threading", "auto");
    let reserved = setting_integer(settings, "/resources/reserved_cpu_threads", 1).max(0);
    let worker_limit = setting_integer(settings, "/resources/max_worker_threads", 0).max(0);
    let configured_frames = frame_threads_override
        .unwrap_or_else(|| setting_integer(settings, "/resources/frame_threads", 0))
        .max(0);
    let preset =
        preset_override.unwrap_or_else(|| setting_string(settings, "/video/preset", "slow"));
    let (mut workers, mut frames, mut reason) = if threading == "unlimited" {
        (
            if worker_limit > 0 { worker_limit } else { cpu },
            configured_frames,
            "unlimited mode requested".to_string(),
        )
    } else {
        let mut workers = (cpu - reserved).max(1);
        if worker_limit > 0 {
            workers = workers.min(worker_limit);
        }
        let frames = if configured_frames > 0 {
            configured_frames
        } else if pixels >= 1280.0 * 720.0 {
            2
        } else {
            1
        };
        (
            workers,
            frames,
            format!("all logical CPU threads minus {reserved}; dynamic memory/PSI guard"),
        )
    };
    let source_depth = profile
        .pointer("/video/source_bit_depth")
        .and_then(value_i64_ref)
        .unwrap_or(8);
    let target_format = profile
        .pointer("/video/target_pixel_format")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let target_depth = if target_format.contains("12") {
        12
    } else if target_format.contains("10") {
        10
    } else {
        source_depth
    }
    .max(source_depth);
    let chroma = profile
        .pointer("/video/source_chroma")
        .and_then(Value::as_str)
        .unwrap_or("4:2:0");
    let chroma_factor = if chroma.contains("4:4:4") {
        1.5
    } else if chroma.contains("4:2:2") {
        1.25
    } else {
        1.0
    };
    let lookahead = match preset {
        "ultrafast" => 5.0,
        "superfast" => 10.0,
        "veryfast" | "faster" | "fast" => 15.0,
        "medium" => 20.0,
        "slower" | "veryslow" => 40.0,
        _ => 25.0,
    };
    let preset_factor = lookahead / 25.0;
    let profile_factor = (pixels / (1920.0 * 1080.0)).max(0.25)
        * (target_depth as f64 / 8.0).max(1.0)
        * chroma_factor
        * preset_factor;
    let requested_workers = workers;
    let requested_frames = if frames > 0 {
        frames
    } else if pixels >= 1280.0 * 720.0 {
        2
    } else {
        1
    };
    let per_frame = profile_factor * concurrent as f64;
    let per_worker = profile_factor * 0.25 * concurrent as f64;
    let mut planning_budget = None;
    let mut memory_worker_cap = None;
    let mut messages = vec![
        "Profile-scaled concurrency allowance is a conservative scheduling heuristic, not a prediction of exact peak encoder RAM.".to_string(),
        "Current swap is reported for context but is not counted as fast memory headroom.".to_string(),
    ];
    let original_workers = workers;
    workers = (workers / concurrent).max(1);
    if concurrent > 1 {
        reason.push_str(&format!(
            "; shared across {concurrent} selected video encoders"
        ));
        messages.push(format!("The estimate reserves one conservative stream allowance for each of {concurrent} video encoders."));
    }
    if let Some(available) = available {
        if available < reserve {
            return json!({
                "model":"profile-scaled-concurrency-v1", "level":"blocked",
                "available_gib":available, "total_gib":total, "swap_free_gib":swap_free,
                "swap_used_gib":swap_used, "swap_used_percent":swap_percent, "reserve_gib":round3(reserve),
                "planning_budget_gib":Value::Null, "profile_factor":round3(profile_factor),
                "preset_factor":round3(preset_factor),
                "per_frame_allowance_gib":round3(per_frame), "per_worker_allowance_gib":round3(per_worker),
                "modeled_plan_gib":Value::Null, "memory_worker_cap":Value::Null,
                "requested_workers":requested_workers, "requested_frame_threads":requested_frames,
                "planned_workers":Value::Null, "planned_frame_threads":Value::Null,
                "retry_plan":Value::Null,
                "messages":[format!("Only {available:.1} GiB RAM is available; the dynamic safety reserve is {reserve:.1} GiB.")],
                "plan_reason":reason
            });
        }
        let pressure_factor = if full_memory_psi.unwrap_or(0.0) >= 1.0 {
            0.5
        } else {
            0.75
        };
        let budget = ((available - reserve).max(0.0) * pressure_factor).max(0.0);
        planning_budget = Some(budget);
        let frame_cap = requested_frames
            .min((budget / per_frame.max(f64::EPSILON)).floor() as i64)
            .max(1);
        if configured_frames == 0 {
            frames = frame_cap;
        } else {
            frames = configured_frames;
        }
        let worker_budget = (budget - frames as f64 * per_frame).max(0.0);
        let worker_cap = (worker_budget / per_worker.max(f64::EPSILON)).floor() as i64;
        memory_worker_cap = Some(worker_cap.max(1));
        workers = workers.min(worker_cap.max(1));
        if frames != requested_frames || workers != original_workers {
            reason.push_str("; concurrency limited by profile-aware memory budget");
            messages.push(format!("The current RAM plan adjusts this profile to {workers} worker(s) and {frames} frame thread(s)."));
        }
        if pressure_factor < 0.75 {
            messages.push(
                "Recent memory PSI indicates sustained pressure; the starting budget was reduced."
                    .into(),
            );
        }
    } else {
        frames = requested_frames;
        messages.push(
            "Available RAM could not be read; the CPU plan relies on live pressure monitoring."
                .into(),
        );
    }
    let modeled_plan =
        planning_budget.map(|_| frames as f64 * per_frame + workers as f64 * per_worker);
    let level = if let Some(plan) = modeled_plan {
        let budget = planning_budget.unwrap_or(0.0);
        let minimum = per_frame + per_worker;
        if full_memory_psi.unwrap_or(0.0) >= 1.0 || plan > budget * 1.15 || budget < minimum {
            "high"
        } else if workers != original_workers || frames != requested_frames {
            "constrained"
        } else {
            "guarded"
        }
    } else {
        "unknown"
    };
    if level == "high" {
        messages.push(
            "Modeled memory headroom is low; live pressure checks remain the final safety guard."
                .into(),
        );
    }
    json!({
        "model":"profile-scaled-concurrency-v1", "level":level,
        "available_gib":available, "total_gib":total, "swap_free_gib":swap_free,
        "swap_used_gib":swap_used, "swap_used_percent":swap_percent, "reserve_gib":round3(reserve),
        "planning_budget_gib":planning_budget.map(round3), "profile_factor":round3(profile_factor),
        "preset_factor":round3(preset_factor),
        "per_frame_allowance_gib":round3(per_frame), "per_worker_allowance_gib":round3(per_worker),
        "modeled_plan_gib":modeled_plan.map(round3), "memory_worker_cap":memory_worker_cap,
        "requested_workers":requested_workers, "requested_frame_threads":requested_frames,
        "planned_workers":workers, "planned_frame_threads":frames,
        "retry_plan":retry_plan(workers, frames, per_frame, per_worker, planning_budget),
        "messages":messages,
        "plan_reason":reason
    })
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round_ties_even() / 1000.0
}

fn retry_plan(
    workers: i64,
    frames: i64,
    per_frame: f64,
    per_worker: f64,
    planning_budget: Option<f64>,
) -> Value {
    let (retry_workers, retry_frames) = if frames > 1 {
        (
            if workers > 2 {
                ((workers as f64 * 0.80) as i64).max(2)
            } else {
                workers
            },
            1,
        )
    } else {
        (
            if workers > 2 {
                ((workers as f64 * 0.65) as i64).max(2)
            } else {
                1
            },
            frames.max(1),
        )
    };
    let modeled = retry_frames as f64 * per_frame + retry_workers as f64 * per_worker;
    let level = if planning_budget.is_some_and(|budget| modeled > budget * 1.15) {
        "high"
    } else {
        "constrained"
    };
    let reduces_parallelism = retry_workers < workers || retry_frames < frames;
    json!({
        "level":level,
        "planned_workers":retry_workers,
        "planned_frame_threads":retry_frames,
        "modeled_plan_gib":round3(modeled),
        "reduces_parallelism":reduces_parallelism
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimePlanRequest {
    profile: Value,
    settings: Value,
    #[serde(default)]
    host: Option<Value>,
    full_memory_psi_avg10: Option<f64>,
    #[serde(default)]
    sample_live_memory_pressure: bool,
    target_dimensions: Option<[i64; 2]>,
    preset_override: Option<String>,
    frame_threads_override: Option<i64>,
    concurrent_video_streams: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetryPlanRequest {
    worker_threads: i64,
    frame_threads: i64,
    per_frame_allowance_gib: f64,
    per_worker_allowance_gib: f64,
    planning_budget_gib: Option<f64>,
}

pub(crate) fn runtime_cli_mode(args: Vec<OsString>) -> i32 {
    if !args.is_empty() {
        eprintln!("The internal resource-plan command does not accept options.");
        return 2;
    }
    let request: RuntimePlanRequest = match serde_json::from_reader(io::stdin().lock()) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid Rust resource-plan request: {error}");
            return 1;
        }
    };
    let dimensions = request
        .target_dimensions
        .map(|[width, height]| (width, height));
    let live_snapshot = crate::resource_monitor::sample_current_host();
    let live_host = json!({
        "hardware": {
            "logical_cpus": crate::environment::logical_cpus(),
            "memory_available_gib": live_snapshot.memory_available_gib,
            "memory_total_gib": live_snapshot.memory_total_gib,
            "swap_free_gib": live_snapshot.swap_free_gib,
            "swap_used_gib": live_snapshot.swap_used_gib,
            "swap_used_percent": live_snapshot.swap_used_percent
        }
    });
    let host = request.host.unwrap_or(live_host);
    let cpu_threads = host
        .pointer("/hardware/logical_cpus")
        .and_then(value_i64_ref)
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let full_memory_psi_avg10 = if request.sample_live_memory_pressure {
        live_snapshot.memory_full_avg10
    } else {
        request.full_memory_psi_avg10
    };
    let mut result = memory_forecast(
        &request.profile,
        &request.settings,
        &host,
        full_memory_psi_avg10,
        dimensions,
        request.preset_override.as_deref(),
        request.frame_threads_override,
        request.concurrent_video_streams,
    );
    if let Some(fields) = result.as_object_mut() {
        fields.insert("cpu_threads".into(), json!(cpu_threads));
    }
    write_json_line(&result, "resource-plan")
}

pub(crate) fn retry_cli_mode(args: Vec<OsString>) -> i32 {
    if !args.is_empty() {
        eprintln!("The internal resource-retry command does not accept options.");
        return 2;
    }
    let request: RetryPlanRequest = match serde_json::from_reader(io::stdin().lock()) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid Rust resource-retry request: {error}");
            return 1;
        }
    };
    if request.worker_threads < 1
        || request.frame_threads < 1
        || !request.per_frame_allowance_gib.is_finite()
        || request.per_frame_allowance_gib <= 0.0
        || !request.per_worker_allowance_gib.is_finite()
        || request.per_worker_allowance_gib <= 0.0
        || request
            .planning_budget_gib
            .is_some_and(|budget| !budget.is_finite() || budget < 0.0)
    {
        eprintln!("Rust resource-retry values must be finite and positive, except for an optional non-negative budget.");
        return 1;
    }
    let result = retry_plan(
        request.worker_threads,
        request.frame_threads,
        request.per_frame_allowance_gib,
        request.per_worker_allowance_gib,
        request.planning_budget_gib,
    );
    write_json_line(&result, "resource-retry")
}

fn write_json_line(value: &Value, label: &str) -> i32 {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if let Err(error) = serde_json::to_writer(&mut output, value)
        .and_then(|_| writeln!(output).map_err(serde_json::Error::io))
        .and_then(|_| output.flush().map_err(serde_json::Error::io))
    {
        eprintln!("Could not write Rust {label} result: {error}");
        return 1;
    }
    0
}

pub(crate) fn memory_full_avg10() -> Option<f64> {
    fs::read_to_string("/proc/pressure/memory")
        .ok()?
        .lines()
        .find(|line| line.starts_with("full "))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix("avg10="))
        })
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::memory_forecast;
    use serde_json::Value;

    fn assert_expected(actual: &Value, expected: &Value) {
        if expected.is_null() {
            assert!(actual.is_null(), "expected null, got {actual}");
            return;
        }
        let expected_fields = expected
            .as_object()
            .expect("fixture expected output must be an object");
        for (key, expected_value) in expected_fields {
            let actual_value = actual
                .get(key)
                .unwrap_or_else(|| panic!("Rust forecast is missing expected field {key:?}"));
            match expected_value {
                Value::Number(expected_number) => {
                    if let Some(expected_integer) = expected_number.as_i64() {
                        assert_eq!(
                            actual_value.as_i64(),
                            Some(expected_integer),
                            "field {key:?}"
                        );
                    } else {
                        let expected_float = expected_number
                            .as_f64()
                            .expect("fixture number must be representable as f64");
                        let actual_float = actual_value
                            .as_f64()
                            .unwrap_or_else(|| panic!("field {key:?} is not numeric"));
                        assert!(
                            (actual_float - expected_float).abs() < 0.000_001,
                            "field {key:?}: expected {expected_float}, got {actual_float}"
                        );
                    }
                }
                _ => assert_eq!(actual_value, expected_value, "field {key:?}"),
            }
        }
    }

    #[test]
    fn resource_forecasts_match_shared_python_parity_cases() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/resource_plan_cases.json"
        ))
        .expect("shared resource-plan fixture must be valid JSON");
        let cases = fixture["cases"]
            .as_array()
            .expect("shared fixture must contain cases");

        for case in cases {
            let name = case["name"].as_str().unwrap_or("unnamed case");
            let target_dimensions = case["target_dimensions"].as_array().map(|values| {
                (
                    values[0].as_i64().expect("target width must be an integer"),
                    values[1]
                        .as_i64()
                        .expect("target height must be an integer"),
                )
            });
            let forecast = memory_forecast(
                &case["profile"],
                &case["settings"],
                &case["host"],
                case["memory_psi_full_avg10"].as_f64(),
                target_dimensions,
                case["preset_override"].as_str(),
                case["frame_threads_override"].as_i64(),
                case["concurrent_video_streams"].as_i64().unwrap_or(1),
            );
            assert_expected(&forecast, &case["expected"]);
            assert_expected(&forecast["retry_plan"], &case["retry_expected"]);
            assert!(
                forecast["model"].as_str().is_some(),
                "forecast model must be present for case {name:?}"
            );
        }
    }
}
