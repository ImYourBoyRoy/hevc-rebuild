//! Output naming, per-file HDR/Dolby Vision source policy, HDR10+ carry-over, resource planning and tool resolution.

use super::*;

pub(super) struct Tools {
    pub(super) ffmpeg: String,
    pub(super) ffprobe: String,
    pub(super) mediainfo: String,
    pub(super) x265_pixel_formats: Vec<String>,
    /// Quietvoid helper path when available; enables post-encode HDR10+ inject.
    pub(super) hdr10plus: Option<String>,
}

/// Names an output, disambiguating sources that share a stem but differ in
/// extension (`Movie.mkv` + `Movie.mp4` become `Movie [mkv].mp4` / `Movie [mp4].mp4`).
pub(super) fn stem_key(relative: &Path) -> String {
    relative
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

pub(super) fn output_relative_path(relative: &Path, shared_stem: bool) -> PathBuf {
    if shared_stem {
        let suffix = relative
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "media".into());
        let stem = relative
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();
        relative.with_file_name(format!("{stem} [{suffix}].mp4"))
    } else {
        relative.with_extension("mp4")
    }
}

pub(super) struct SourcePolicy {
    pub(super) color_mode: &'static str,
    pub(super) note: Option<String>,
}

/// Decide whether a source can be exported and how. `Err` carries the SKIPPED
/// message. When `hdr10plus_can_inject` is true, HDR10+ is preserved after
/// encode via `hdr10plus_tool`; otherwise per-file loss consent is required.
pub(super) fn source_policy(
    profile: &Value,
    resolution: &str,
    x265_pixel_formats: &[String],
    hdr10plus_loss_accepted: bool,
    hdr10plus_can_inject: bool,
) -> Result<SourcePolicy, String> {
    let color_mode = youtube::upload_color_mode(profile);
    let unsupported = youtube::unsupported_color_message(color_mode);
    if let Some(message) = unsupported {
        return Err(message.into());
    }
    if color_mode == "sdr" {
        return Ok(SourcePolicy {
            color_mode: "sdr",
            note: None,
        });
    }
    let (width, height) = youtube::expected_dimensions(profile, resolution)?;
    if width.min(height) < 720 {
        return Err("YouTube's HDR upload guidance lists 720p and higher; this selected output would be below 720p.".into());
    }
    if !x265_pixel_formats.iter().any(|name| name == "yuv420p10le") {
        return Err("This FFmpeg does not expose 10-bit libx265 (yuv420p10le), which the HDR export requires.".into());
    }
    let mut note = None;
    if profile
        .pointer("/hdr/hdr10plus_hint")
        .and_then(Value::as_bool)
        == Some(true)
    {
        if hdr10plus_can_inject {
            note = Some(
                "HDR10+ will be preserved after encode with hdr10plus_tool inject.".to_string(),
            );
        } else if !hdr10plus_loss_accepted {
            return Err("HDR10+ dynamic metadata cannot be retained without hdr10plus_tool. No copy was written; install the helper, or explicitly accept this loss for the individual file to export static HDR10/PQ only.".into());
        } else {
            note = Some("HDR10+ dynamic metadata was not retained after explicit per-file approval; static HDR10/PQ signaling is retained.".to_string());
        }
    }
    Ok(SourcePolicy { color_mode, note })
}

/// Extract source HDR10+ JSON into the app work tree for a later MP4 inject.
pub(super) fn extract_youtube_hdr10plus_json(
    tools: &Tools,
    source: &Path,
    profile: &Value,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    let helper = tools
        .hdr10plus
        .as_deref()
        .ok_or("HDR10+ inject requires hdr10plus_tool.")?;
    let stream_index = profile
        .get("primary_stream_index")
        .and_then(Value::as_i64)
        .or_else(|| profile.pointer("/video/index").and_then(Value::as_i64))
        .unwrap_or(0);
    let codec = profile
        .pointer("/video/codec")
        .and_then(Value::as_str)
        .unwrap_or("hevc");
    fs::create_dir_all(work_dir)
        .map_err(|error| format!("Could not create HDR10+ work folder: {error}"))?;
    let metadata_json = work_dir.join(format!("youtube-hdr10plus-{}.json", std::process::id()));
    if metadata_json.exists() {
        return Err(format!(
            "HDR10+ extract refused to overwrite {}",
            metadata_json.display()
        ));
    }
    crate::hdr_metadata::extract_hdr10plus(
        &tools.ffmpeg,
        helper,
        source,
        stream_index,
        codec,
        &metadata_json,
        None,
    )?;
    Ok(metadata_json)
}

pub(super) fn inject_youtube_hdr10plus(
    tools: &Tools,
    mp4: &Path,
    metadata_json: &Path,
    work_dir: &Path,
) -> Result<(), String> {
    let helper = tools
        .hdr10plus
        .as_deref()
        .ok_or("HDR10+ inject requires hdr10plus_tool.")?;
    crate::hdr_metadata::inject_hdr10plus_into_mp4(
        &tools.ffmpeg,
        helper,
        mp4,
        0,
        metadata_json,
        work_dir,
    )
}

pub(super) struct EncodePlan {
    pub(super) workers: i64,
    pub(super) frames: i64,
    pub(super) per_frame: f64,
    pub(super) per_worker: f64,
    pub(super) budget: Option<f64>,
    pub(super) memory_guard_gib: f64,
}

pub(super) fn plan_resources(
    settings: &Value,
    profile: &Value,
    dimensions: (i64, i64),
    preset: &str,
) -> Result<EncodePlan, String> {
    let host = resource_plan::live_host();
    let forecast = resource_plan::memory_forecast(MemoryForecastInput {
        profile,
        settings,
        host: &host,
        full_memory_psi: resource_plan::memory_full_avg10(),
        target_dimensions: Some(dimensions),
        preset_override: Some(preset),
        frame_threads_override: Some(0),
        concurrent_video_streams: 1,
    });
    if forecast.get("level").and_then(Value::as_str) == Some("blocked") {
        let reason = forecast
            .pointer("/messages/0")
            .and_then(Value::as_str)
            .unwrap_or("Available memory is below the safety reserve.");
        return Err(format!(
            "{reason} Free memory or lower the reserve in Settings, then retry; the source remains untouched."
        ));
    }
    let number = |key: &str| forecast.get(key).and_then(Value::as_f64);
    Ok(EncodePlan {
        workers: forecast
            .get("planned_workers")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        frames: forecast
            .get("planned_frame_threads")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1),
        per_frame: number("per_frame_allowance_gib").unwrap_or(1.0),
        per_worker: number("per_worker_allowance_gib").unwrap_or(0.25),
        budget: number("planning_budget_gib"),
        memory_guard_gib: number("reserve_gib").unwrap_or(0.0),
    })
}

impl EncodePlan {
    /// Lower-parallelism plan for a memory-pressure retry, or `None` when no
    /// further reduction is possible.
    pub(super) fn reduced(&self) -> Option<Self> {
        let next = resource_plan::retry_plan(
            self.workers,
            self.frames,
            self.per_frame,
            self.per_worker,
            self.budget,
        );
        if next.get("reduces_parallelism").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        Some(Self {
            workers: next.get("planned_workers").and_then(Value::as_i64)?.max(1),
            frames: next
                .get("planned_frame_threads")
                .and_then(Value::as_i64)?
                .max(1),
            per_frame: self.per_frame,
            per_worker: self.per_worker,
            budget: self.budget,
            memory_guard_gib: self.memory_guard_gib,
        })
    }
}

pub(super) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(super) fn temporary_name(destination: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = destination
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "export".into());
    destination.with_file_name(format!(
        ".{stem}.upload-{:x}-{:x}-{:x}.tmp.mp4",
        std::process::id(),
        nonce,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

pub(super) fn resolve_tools(options: &YoutubeRunOptions) -> Result<Tools, String> {
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
        x265_pixel_formats: host
            .get("x265_pixel_formats")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        hdr10plus: optional_tool(&host, "hdr10plus_tool"),
    };
    for (key, label) in [
        ("libx264", "the libx264 encoder"),
        ("aac", "the AAC encoder"),
    ] {
        let available = host
            .get("checks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|check| {
                check.get("key").and_then(Value::as_str) == Some(key)
                    && check.get("available").and_then(Value::as_bool) == Some(true)
            });
        if !available {
            missing.push(label.to_string());
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "YouTube export needs {}. Install it or choose its path in Settings, then retry.",
            missing.join(", ")
        ));
    }
    Ok(tools)
}

// ------------------------------------------------------------------- running
