//! Parses the engine-style argument vector into `YoutubeRunOptions`, falling back to the effective settings.

use super::*;

const RESOURCE_FLAGS: [(&str, &str); 6] = [
    ("--resource-threading", "threading"),
    ("--reserved-cpu-threads", "reserved_cpu_threads"),
    ("--max-worker-threads", "max_worker_threads"),
    ("--min-free-memory-gib", "min_free_memory_gib"),
    ("--memory-reserve-percent", "memory_reserve_percent"),
    (
        "--disk-projection-margin-percent",
        "disk_projection_margin_percent",
    ),
];

pub(super) const IGNORED_VALUE_FLAGS: [&str; 22] = [
    "--crf",
    "--preset",
    "--tune",
    "--dolby-vision-policy",
    "--secondary-video",
    "--data-streams",
    "--frame-threads",
    "--calibration-mode",
    "--calibration-cache-hours",
    "--calibration-large-file-gib",
    "--calibration-sample-seconds",
    "--calibration-max-candidates",
    "--calibration-tolerance-percent",
    "--preflight-output-ratio",
    "--quality-gate",
    "--quality-min-ssim",
    "--comment",
    "--metadata-text-blacklist",
    "--allowed-poster-cache-root",
    "--file-output-dirs-json",
    "--file-poster-overrides-json",
    "--file-metadata-overrides-json",
];

pub(super) const IGNORED_BOOLEAN_FLAGS: [&str; 21] = [
    "--youtube-upload",
    "--no-dependency-repair",
    "--skip-update-check",
    "--assume-yes",
    "--allow-unsupported-os",
    "--verbose",
    "--promote-8bit-to-10bit",
    "--no-promote-8bit-to-10bit",
    "--detect-animation",
    "--no-detect-animation",
    "--preserve-hdr10plus",
    "--no-preserve-hdr10plus",
    "--verify-copy-hashes",
    "--no-verify-copy-hashes",
    "--force-deinterlace",
    "--disable-deinterlace",
    "--disable-hdr10-opt",
    "--preserve-source-level",
    "--embed-artwork",
    "--no-embed-artwork",
    "--replace-metadata-text-blacklist",
];

pub(super) const REJECTED_FLAGS: [(&str, &str); 4] = [
    (
        "--file-stream-overrides-json",
        "per-file stream selections do not apply to YouTube export mode",
    ),
    (
        "--file-settings-overrides-json",
        "current-run encoder settings do not apply to YouTube export mode",
    ),
    (
        "--file-tune-overrides-json",
        "per-file x265 content tunes do not apply to YouTube export mode",
    ),
    (
        "--output-dir",
        "rebuild output folders do not apply to YouTube export mode; use the YouTube output folder",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct YoutubeRunOptions {
    pub root: PathBuf,
    pub recursive: bool,
    pub minimum_size_mib: u32,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub selected_files: Vec<PathBuf>,
    pub resolution: String,
    pub quality_mode: String,
    pub source_multiplier: f64,
    pub output_dir: Option<PathBuf>,
    pub hdr10plus_loss: BTreeSet<PathBuf>,
    pub stop_on_error: bool,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
    pub settings: Value,
    /// `true` for Optimize mode (lossless-first, never resizes).
    pub optimize: bool,
    pub sdr_codec: String,
    pub preset: String,
    pub shrink: String,
    pub quality_check: String,
    pub work_root: String,
}

impl YoutubeRunOptions {
    /// Parse the engine-style vector built by the GUI and MCP. Unknown flags are
    /// rejected so a future flag can never be silently dropped.
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            resolution: "1440p".into(),
            quality_mode: "balanced".into(),
            source_multiplier: 2.0,
            sdr_codec: "h264".into(),
            preset: "slow".into(),
            shrink: "auto".into(),
            quality_check: "standard".into(),
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut resource_overrides: Vec<(&'static str, String)> = Vec::new();
        let mut min_free_disk_gib: Option<String> = None;
        let mut loss_paths: Vec<PathBuf> = Vec::new();
        let mut index = 0;
        let mut explicit_exclusions = false;
        while index < args.len() {
            let name = args[index].as_str();
            let value = || -> Result<String, String> {
                args.get(index + 1)
                    .cloned()
                    .ok_or_else(|| format!("Option {name} requires a value."))
            };
            let mut consumed = 2;
            match name {
                "--recursive" => {
                    options.recursive = true;
                    consumed = 1;
                }
                "--stop-on-error" => {
                    options.stop_on_error = true;
                    consumed = 1;
                }
                "--min-size-mib" => {
                    options.minimum_size_mib = value()?
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value <= 4096)
                        .ok_or("Minimum input size must be between 0 and 4096 MiB.")?;
                }
                "--include-filename-pattern" => options.include.push(value()?),
                "--exclude-filename-pattern" => options.exclude.push(value()?),
                "--no-default-exclusions" => {
                    explicit_exclusions = true;
                    consumed = 1;
                }
                "--selected-file" => options.selected_files.push(PathBuf::from(value()?)),
                "--upload-resolution" => options.resolution = value()?,
                "--upload-quality-mode" => options.quality_mode = value()?,
                "--upload-source-multiplier" => {
                    options.source_multiplier = value()?
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .ok_or("The YouTube source-size multiplier must be a number.")?;
                }
                "--youtube-mode" => {
                    options.optimize = match value()?.as_str() {
                        "upscale" => false,
                        "optimize" => true,
                        other => {
                            return Err(format!(
                                "YouTube mode must be upscale or optimize, not {other:?}."
                            ))
                        }
                    };
                }
                "--youtube-sdr-codec" => options.sdr_codec = value()?,
                "--youtube-preset" => options.preset = value()?,
                "--youtube-shrink" => options.shrink = value()?,
                "--youtube-quality-check" => options.quality_check = value()?,
                "--upload-output-dir" => options.output_dir = Some(PathBuf::from(value()?)),
                "--allow-youtube-hdr10plus-loss" => loss_paths.push(PathBuf::from(value()?)),
                "--work-root" => {
                    options.work_root = value()?;
                    crate::preview_work_directory(Path::new("."), &options.work_root)?;
                }
                "--min-free-disk-gib" => min_free_disk_gib = Some(value()?),
                "--stop-request-file" => options.stop_file = Some(PathBuf::from(value()?)),
                "--ffmpeg-path" => options.tool_paths.ffmpeg = Some(value()?),
                "--ffprobe-path" => options.tool_paths.ffprobe = Some(value()?),
                "--mediainfo-path" => options.tool_paths.mediainfo = Some(value()?),
                "--mkvpropedit-path" => options.tool_paths.mkvpropedit = Some(value()?),
                "--mkvmerge-path" => options.tool_paths.mkvmerge = Some(value()?),
                "--mkvextract-path" => options.tool_paths.mkvextract = Some(value()?),
                flag if RESOURCE_FLAGS.iter().any(|(known, _)| *known == flag) => {
                    let key = RESOURCE_FLAGS
                        .iter()
                        .find(|(known, _)| *known == flag)
                        .map(|(_, key)| *key)
                        .unwrap_or_default();
                    resource_overrides.push((key, value()?));
                }
                flag if IGNORED_VALUE_FLAGS.contains(&flag) => {
                    value()?;
                }
                flag if IGNORED_BOOLEAN_FLAGS.contains(&flag) => consumed = 1,
                flag if REJECTED_FLAGS.iter().any(|(rejected, _)| *rejected == flag) => {
                    let reason = REJECTED_FLAGS
                        .iter()
                        .find(|(rejected, _)| *rejected == flag)
                        .map(|(_, reason)| *reason)
                        .unwrap_or("this option is not supported");
                    return Err(format!("YouTube export: {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "YouTube export does not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("YouTube export accepts exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        if !matches!(options.sdr_codec.as_str(), "h264" | "hevc") {
            return Err("The YouTube SDR codec must be h264 or hevc.".into());
        }
        if !matches!(options.preset.as_str(), "medium" | "slow" | "slower") {
            return Err("The YouTube encoder preset must be medium, slow, or slower.".into());
        }
        youtube_optimize::Shrink::parse(&options.shrink)?;
        youtube_optimize::QualityCheck::parse(&options.quality_check)?;
        if options.optimize {
            // Optimize never resizes; any selected upscale tier is ignored.
            options.resolution = "source".into();
        }
        if !matches!(options.resolution.as_str(), "source" | "1440p" | "4k") {
            return Err("Upload resolution must be source, 1440p, or 4k.".into());
        }
        if !matches!(options.quality_mode.as_str(), "balanced" | "recommended") {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        if !(MIN_SOURCE_MULTIPLIER..=MAX_SOURCE_MULTIPLIER).contains(&options.source_multiplier) {
            return Err(format!(
                "YouTube source-size multiplier must be between {MIN_SOURCE_MULTIPLIER} and {MAX_SOURCE_MULTIPLIER}."
            ));
        }
        let root = root.ok_or("YouTube export requires a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;
        for path in loss_paths {
            let canonical = path.canonicalize().map_err(|error| {
                format!("An HDR10+ consent path is no longer available: {error}")
            })?;
            options.hdr10plus_loss.insert(canonical);
        }
        let mut settings = effective_settings(&options.root)?;
        for (key, raw) in resource_overrides {
            let parsed = if key == "threading" {
                json!(raw)
            } else if let Ok(number) = raw.parse::<i64>() {
                json!(number)
            } else {
                json!(raw
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("The resource setting {key} must be a number."))?)
            };
            settings["resources"][key] = parsed;
        }
        if let Some(raw) = min_free_disk_gib {
            settings["resources"]["min_free_disk_gib"] = json!(raw
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or("The disk reserve must be a non-negative number of GiB.")?);
        }
        options.settings = settings;
        crate::backup_folder::apply_default_exclusions(&mut options.exclude, explicit_exclusions);
        Ok(options)
    }

    pub(super) fn disk_reserve_bytes(&self) -> u64 {
        let gib = self
            .settings
            .pointer("/resources/min_free_disk_gib")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
        (gib * GIB) as u64
    }

    pub(super) fn projection_margin_percent(&self) -> f64 {
        self.settings
            .pointer("/resources/disk_projection_margin_percent")
            .and_then(Value::as_f64)
            .unwrap_or(15.0)
            .clamp(0.0, 100.0)
    }
}

pub(super) fn effective_settings(root: &Path) -> Result<Value, String> {
    let anchor = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    let payload = serde_json::to_value(crate::settings::load_effective_settings(
        &anchor.to_string_lossy(),
    )?)
    .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    let mut settings = payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !settings.get("resources").is_some_and(Value::is_object) {
        settings["resources"] = json!({});
    }
    Ok(settings)
}

// -------------------------------------------------------------- pure policy
