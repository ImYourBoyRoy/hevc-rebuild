//! Memory and thread planning for an encode, and tool resolution.

use super::*;

pub(super) fn choose_resource_plan(
    options: &RebuildRunOptions,
    profile: &Value,
    preset: &str,
    concurrent: i64,
) -> Result<ResourcePlan, String> {
    let host = crate::resource_plan::live_host();
    let settings = json!({
        "video": { "preset": preset },
        "resources": {
            "threading": options.resource_threading,
            "reserved_cpu_threads": options.reserved_cpu_threads,
            "max_worker_threads": options.max_worker_threads,
            "frame_threads": options.frame_threads,
            "min_free_memory_gib": options.min_free_memory_gib,
            "memory_reserve_percent": options.memory_reserve_percent,
        }
    });
    let forecast = memory_forecast(MemoryForecastInput {
        profile,
        settings: &settings,
        host: &host,
        full_memory_psi: crate::resource_plan::memory_full_avg10(),
        target_dimensions: None,
        preset_override: Some(preset),
        frame_threads_override: Some(options.frame_threads),
        concurrent_video_streams: concurrent.max(1),
    });
    if forecast.get("level").and_then(Value::as_str) == Some("blocked") {
        let reason = forecast
            .pointer("/messages/0")
            .and_then(Value::as_str)
            .unwrap_or("Available memory is below the safety reserve.");
        return Err(reason.to_string());
    }
    let cpu = host
        .pointer("/hardware/logical_cpus")
        .and_then(Value::as_i64)
        .unwrap_or(1)
        .max(1);
    Ok(ResourcePlan {
        worker_threads: forecast
            .get("planned_workers")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        frame_threads: forecast
            .get("planned_frame_threads")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        per_frame: forecast
            .get("per_frame_allowance_gib")
            .and_then(Value::as_f64)
            .unwrap_or(1.0),
        per_worker: forecast
            .get("per_worker_allowance_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.25),
        budget: forecast.get("planning_budget_gib").and_then(Value::as_f64),
        memory_guard_gib: forecast
            .get("reserve_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        forecast,
        cpu_threads: cpu,
    })
}

pub(super) fn record_memory_forecast(row: &mut Map<String, Value>, plan: &ResourcePlan) {
    let forecast = &plan.forecast;
    for (key, field) in [
        ("memory_forecast", "level"),
        ("memory_forecast_message", "messages"),
    ] {
        if let Some(value) = forecast.get(field) {
            if field == "messages" {
                let text = value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                row.insert(key.into(), json!(text));
            } else {
                row.insert(key.into(), value.clone());
            }
        }
    }
    for (key, field) in [
        ("memory_available_gib", "available_gib"),
        ("memory_total_gib", "total_gib"),
        ("memory_swap_used_percent", "swap_used_percent"),
        ("memory_budget_gib", "planning_budget_gib"),
        ("memory_worker_cap", "memory_worker_cap"),
        ("planned_workers", "planned_workers"),
        ("planned_frame_threads", "planned_frame_threads"),
    ] {
        if let Some(value) = forecast.get(field) {
            row.insert(key.into(), value.clone());
        }
    }
}

pub(super) fn resolve_tools(options: &RebuildRunOptions) -> Result<Tools, String> {
    let host = crate::environment::inspect_environment(options.tool_paths.clone(), None)?;
    let mut missing = Vec::new();
    let mut need = |key: &str, label: &str| -> String {
        optional_tool(&host, key).unwrap_or_else(|| {
            missing.push(label.to_string());
            String::new()
        })
    };
    let tools = Tools {
        ffmpeg: need("ffmpeg", "FFmpeg"),
        ffprobe: need("ffprobe", "FFprobe"),
        mediainfo: need("mediainfo", "MediaInfo"),
        mkvmerge: need("mkvmerge", "MKVToolNix mkvmerge"),
        mkvpropedit: need("mkvpropedit", "MKVToolNix mkvpropedit"),
        mkvextract: need("mkvextract", "MKVToolNix mkvextract"),
        hdr10plus: optional_tool(&host, "hdr10plus_tool"),
        x265_pixel_formats: host
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        x265_dolbyvision: host
            .get("x265_dolbyvision")
            .or_else(|| host.get("dolby_vision_x265"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        x265_hdr10plus: host
            .get("x265_hdr10plus")
            .or_else(|| host.get("hdr10plus_x265"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    if !missing.is_empty() {
        return Err(format!(
            "Rebuild needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    if tools.x265_pixel_formats.is_empty() {
        return Err("Installed FFmpeg does not advertise libx265 pixel formats.".into());
    }
    Ok(tools)
}
