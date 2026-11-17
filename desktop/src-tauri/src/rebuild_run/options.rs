//! Parses the engine-style argument vector into `RebuildRunOptions` and resolves per-file effective settings.

use super::*;

pub(super) const REJECTED_FLAGS: [(&str, &str); 3] = [
    (
        "--youtube-upload",
        "YouTube export is a separate native workflow",
    ),
    (
        "--metadata-only",
        "metadata-only MKV is a separate native workflow",
    ),
    (
        "--upload-output-dir",
        "YouTube output folders do not apply to rebuild mode",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct RebuildRunOptions {
    pub root: PathBuf,
    pub recursive: bool,
    pub minimum_size_mib: u32,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub selected_files: Vec<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub file_output_dirs: BTreeMap<PathBuf, PathBuf>,
    pub poster_overrides: BTreeMap<PathBuf, PathBuf>,
    pub metadata_overrides: BTreeMap<PathBuf, Map<String, Value>>,
    pub file_tune_overrides: BTreeMap<PathBuf, String>,
    pub file_stream_overrides: BTreeMap<PathBuf, Vec<u16>>,
    pub file_settings_overrides: BTreeMap<PathBuf, Map<String, Value>>,
    pub comment: String,
    pub text_blacklist: Vec<String>,
    pub embed_artwork: bool,
    pub crf: i64,
    pub test_crf: i64,
    pub preset: String,
    pub tune: String,
    pub detect_animation: bool,
    pub promote_8bit_to_10bit: bool,
    pub preserve_hdr10plus: bool,
    pub dolby_vision_policy: String,
    pub secondary_video: String,
    pub data_streams: String,
    pub verify_copy_hashes: bool,
    /// Whether the finished picture is compared with the source before install, and what a low score does.
    pub quality_gate: crate::quality_gate::GateMode,
    /// Mean SSIM the output must reach when the gate enforces.
    pub quality_min_ssim: f64,
    pub resource_threading: String,
    pub reserved_cpu_threads: i64,
    pub max_worker_threads: i64,
    pub frame_threads: i64,
    pub min_free_memory_gib: f64,
    pub memory_reserve_percent: f64,
    pub min_free_disk_gib: f64,
    pub preflight_output_ratio: f64,
    pub disk_projection_margin_percent: f64,
    pub calibration_mode: String,
    pub calibration_cache_hours: f64,
    pub calibration_large_file_gib: f64,
    pub calibration_sample_seconds: f64,
    pub calibration_max_candidates: usize,
    pub calibration_tolerance_percent: f64,
    pub test_mode: bool,
    pub info_only: bool,
    pub test_seconds: i64,
    pub stop_on_error: bool,
    /// Halt a multi-file run after this many failures in a row (0 = never). Low disk space always halts.
    pub max_consecutive_failures: u32,
    pub keep_work: bool,
    pub reprocess: bool,
    pub force_deinterlace: bool,
    pub disable_deinterlace: bool,
    pub disable_hdr10_opt: bool,
    pub preserve_source_level: bool,
    pub work_root: String,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
}

impl RebuildRunOptions {
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            embed_artwork: true,
            crf: crate::settings::DEFAULT_CRF,
            test_crf: 30,
            preset: "slow".into(),
            tune: "auto".into(),
            detect_animation: true,
            promote_8bit_to_10bit: false,
            preserve_hdr10plus: true,
            dolby_vision_policy: "safe".into(),
            secondary_video: "encode".into(),
            data_streams: "preserve".into(),
            verify_copy_hashes: true,
            quality_gate: crate::quality_gate::GateMode::Enforce,
            quality_min_ssim: crate::quality_gate::DEFAULT_MIN_SSIM,
            resource_threading: "auto".into(),
            reserved_cpu_threads: 1,
            max_worker_threads: 0,
            frame_threads: 0,
            min_free_memory_gib: 3.0,
            memory_reserve_percent: 8.0,
            min_free_disk_gib: 0.0,
            preflight_output_ratio: 1.25,
            disk_projection_margin_percent: 15.0,
            calibration_mode: "auto".into(),
            calibration_cache_hours: 168.0,
            calibration_large_file_gib: 20.0,
            calibration_sample_seconds: 4.0,
            calibration_max_candidates: 3,
            calibration_tolerance_percent: 5.0,
            test_seconds: 5,
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut comment: Option<String> = None;
        let mut embed_artwork: Option<bool> = None;
        let mut blacklist: Vec<String> = Vec::new();
        let mut replace_blacklist = false;
        let mut output_dirs_json: Option<String> = None;
        let mut poster_json: Option<String> = None;
        let mut metadata_json: Option<String> = None;
        let mut tune_json: Option<String> = None;
        let mut stream_json: Option<String> = None;
        let mut settings_json: Option<String> = None;
        let mut index = 0;
        let mut explicit_exclusions = false;
        let mut quality_gate_flag: Option<crate::quality_gate::GateMode> = None;
        let mut quality_min_flag: Option<f64> = None;
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
                "--max-consecutive-failures" => {
                    options.max_consecutive_failures =
                        parse_i64(&value()?, 0, 100, "consecutive failure limit")? as u32;
                }
                "--keep-work" => {
                    options.keep_work = true;
                    consumed = 1;
                }
                "--reprocess" => {
                    options.reprocess = true;
                    consumed = 1;
                }
                "--test" => {
                    options.test_mode = true;
                    consumed = 1;
                }
                "--info-only" => {
                    options.info_only = true;
                    consumed = 1;
                }
                "--force-deinterlace" => {
                    options.force_deinterlace = true;
                    consumed = 1;
                }
                "--disable-deinterlace" => {
                    options.disable_deinterlace = true;
                    consumed = 1;
                }
                "--disable-hdr10-opt" => {
                    options.disable_hdr10_opt = true;
                    consumed = 1;
                }
                "--preserve-source-level" => {
                    options.preserve_source_level = true;
                    consumed = 1;
                }
                "--replace-metadata-text-blacklist" => {
                    replace_blacklist = true;
                    consumed = 1;
                }
                "--embed-artwork" => {
                    embed_artwork = Some(true);
                    consumed = 1;
                }
                "--no-embed-artwork" => {
                    embed_artwork = Some(false);
                    consumed = 1;
                }
                "--promote-8bit-to-10bit" => {
                    options.promote_8bit_to_10bit = true;
                    consumed = 1;
                }
                "--no-promote-8bit-to-10bit" => {
                    options.promote_8bit_to_10bit = false;
                    consumed = 1;
                }
                "--detect-animation" => {
                    options.detect_animation = true;
                    consumed = 1;
                }
                "--no-detect-animation" => {
                    options.detect_animation = false;
                    consumed = 1;
                }
                "--preserve-hdr10plus" => {
                    options.preserve_hdr10plus = true;
                    consumed = 1;
                }
                "--no-preserve-hdr10plus" => {
                    options.preserve_hdr10plus = false;
                    consumed = 1;
                }
                "--verify-copy-hashes" => {
                    options.verify_copy_hashes = true;
                    consumed = 1;
                }
                "--no-verify-copy-hashes" => {
                    options.verify_copy_hashes = false;
                    consumed = 1;
                }
                "--no-dependency-repair"
                | "--skip-update-check"
                | "--assume-yes"
                | "--allow-unsupported-os"
                | "--verbose" => consumed = 1,
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
                "--output-dir" => options.output_dir = Some(PathBuf::from(value()?)),
                "--file-output-dirs-json" => output_dirs_json = Some(value()?),
                "--file-poster-overrides-json" => poster_json = Some(value()?),
                "--file-metadata-overrides-json" => metadata_json = Some(value()?),
                "--file-tune-overrides-json" => tune_json = Some(value()?),
                "--file-stream-overrides-json" => stream_json = Some(value()?),
                "--file-settings-overrides-json" => settings_json = Some(value()?),
                "--allowed-poster-cache-root" => {
                    value()?;
                }
                "--comment" => comment = Some(value()?),
                "--metadata-text-blacklist" => blacklist.push(value()?),
                "--crf" => options.crf = parse_crf(&value()?)?,
                "--test-crf" => options.test_crf = parse_crf(&value()?)?,
                "--preset" => {
                    options.preset = parse_choice(
                        &value()?,
                        &[
                            "ultrafast",
                            "superfast",
                            "veryfast",
                            "faster",
                            "fast",
                            "medium",
                            "slow",
                            "slower",
                            "veryslow",
                        ],
                        "preset",
                    )?;
                }
                "--tune" => {
                    options.tune =
                        parse_choice(&value()?, &["auto", "animation", "grain", "none"], "tune")?;
                }
                "--dolby-vision-policy" => {
                    options.dolby_vision_policy = parse_choice(
                        &value()?,
                        &["safe", "skip", "hdr10-fallback"],
                        "Dolby Vision policy",
                    )?;
                }
                "--secondary-video" => {
                    options.secondary_video =
                        parse_choice(&value()?, &["encode", "copy", "drop"], "secondary video")?;
                }
                "--data-streams" => {
                    options.data_streams =
                        parse_choice(&value()?, &["preserve", "drop"], "data streams")?;
                }
                "--resource-threading" => {
                    options.resource_threading =
                        parse_choice(&value()?, &["auto", "unlimited"], "resource threading")?;
                }
                "--reserved-cpu-threads" => {
                    options.reserved_cpu_threads =
                        parse_i64(&value()?, 0, 64, "reserved CPU threads")?;
                }
                "--max-worker-threads" => {
                    options.max_worker_threads = parse_i64(&value()?, 0, 64, "max worker threads")?;
                }
                "--frame-threads" => {
                    options.frame_threads = parse_i64(&value()?, 0, 16, "frame threads")?;
                }
                "--min-free-memory-gib" => {
                    options.min_free_memory_gib =
                        parse_f64(&value()?, 0.0, 1_000_000.0, "memory floor")?;
                }
                "--memory-reserve-percent" => {
                    options.memory_reserve_percent =
                        parse_f64(&value()?, 0.0, 50.0, "memory reserve percent")?;
                }
                "--min-free-disk-gib" => {
                    options.min_free_disk_gib =
                        parse_f64(&value()?, 0.0, 1_000_000.0, "disk reserve")?;
                }
                "--quality-gate" => {
                    quality_gate_flag = Some(crate::quality_gate::GateMode::parse(&value()?)?)
                }
                "--quality-min-ssim" => {
                    let raw = value()?;
                    let parsed = raw.parse::<f64>().map_err(|_| {
                        "Minimum SSIM must be a number between 0.5 and 1.0.".to_string()
                    })?;
                    quality_min_flag = Some(crate::quality_gate::validate_min_ssim(parsed)?);
                }
                "--preflight-output-ratio" => {
                    options.preflight_output_ratio =
                        parse_f64(&value()?, 0.25, 4.0, "preflight output ratio")?;
                }
                "--disk-projection-margin-percent" => {
                    options.disk_projection_margin_percent =
                        parse_f64(&value()?, 0.0, 100.0, "disk projection margin")?;
                }
                "--calibration-mode" => {
                    options.calibration_mode = parse_choice(
                        &value()?,
                        &["auto", "ask", "always", "off"],
                        "calibration mode",
                    )?;
                }
                "--calibration-cache-hours" => {
                    options.calibration_cache_hours =
                        parse_f64(&value()?, 0.0, 720.0, "calibration cache hours")?;
                }
                "--calibration-large-file-gib" => {
                    options.calibration_large_file_gib =
                        parse_f64(&value()?, 0.0, 1000.0, "calibration large-file threshold")?;
                }
                "--calibration-sample-seconds" => {
                    options.calibration_sample_seconds =
                        parse_f64(&value()?, 1.0, 15.0, "calibration sample seconds")?;
                }
                "--calibration-max-candidates" => {
                    options.calibration_max_candidates =
                        parse_i64(&value()?, 1, 4, "calibration candidates")? as usize;
                }
                "--calibration-tolerance-percent" => {
                    options.calibration_tolerance_percent =
                        parse_f64(&value()?, 0.0, 20.0, "calibration tolerance")?;
                }
                "--test-seconds" => {
                    options.test_seconds = parse_i64(&value()?, 1, 600, "test seconds")?;
                }
                "--work-root" => {
                    options.work_root = value()?;
                    crate::preview_work_directory(Path::new("."), &options.work_root)?;
                }
                "--stop-request-file" => options.stop_file = Some(PathBuf::from(value()?)),
                "--ffmpeg-path" => options.tool_paths.ffmpeg = Some(value()?),
                "--ffprobe-path" => options.tool_paths.ffprobe = Some(value()?),
                "--mediainfo-path" => options.tool_paths.mediainfo = Some(value()?),
                "--mkvpropedit-path" => options.tool_paths.mkvpropedit = Some(value()?),
                "--mkvmerge-path" => options.tool_paths.mkvmerge = Some(value()?),
                "--mkvextract-path" => options.tool_paths.mkvextract = Some(value()?),
                flag if REJECTED_FLAGS.iter().any(|(rejected, _)| *rejected == flag) => {
                    let reason = REJECTED_FLAGS
                        .iter()
                        .find(|(rejected, _)| *rejected == flag)
                        .map(|(_, reason)| *reason)
                        .unwrap_or("this option is not supported");
                    return Err(format!("Rebuild runs reject this option: {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "Rebuild runs do not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("Rebuild runs accept exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        if options.info_only && options.test_mode {
            return Err("Check-only and short-test modes cannot be combined.".into());
        }
        if options.force_deinterlace && options.disable_deinterlace {
            return Err(
                "--force-deinterlace and --disable-deinterlace are mutually exclusive.".into(),
            );
        }
        let root = root.ok_or("Rebuild runs require a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;

        let settings = effective_settings_json(&options.root)?;
        options.comment = match comment {
            Some(value) => value,
            None => settings
                .pointer("/metadata/comment")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
        if options.comment.len() > 500 {
            return Err("Metadata comment is limited to 500 UTF-8 bytes.".into());
        }
        options.embed_artwork = embed_artwork.unwrap_or_else(|| {
            settings
                .pointer("/metadata/embed_artwork")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
        options.text_blacklist = merged_blacklist(&settings, blacklist, replace_blacklist);
        // An explicit flag wins; otherwise the saved settings; otherwise the safe default.
        options.quality_gate = match quality_gate_flag {
            Some(mode) => mode,
            None => match settings
                .pointer("/quality/ssim_gate")
                .and_then(Value::as_str)
            {
                Some(text) => crate::quality_gate::GateMode::parse(text)?,
                None => options.quality_gate,
            },
        };
        options.quality_min_ssim = match quality_min_flag {
            Some(value) => value,
            None => match settings
                .pointer("/quality/min_ssim")
                .and_then(Value::as_f64)
            {
                Some(value) => crate::quality_gate::validate_min_ssim(value)?,
                None => options.quality_min_ssim,
            },
        };
        options.file_output_dirs = parse_path_map(output_dirs_json, "per-file output folders")?;
        options.poster_overrides = parse_path_map(poster_json, "poster selections")?;
        options.metadata_overrides = parse_metadata_overrides(metadata_json)?;
        options.file_tune_overrides = parse_tune_overrides(tune_json)?;
        options.file_stream_overrides = parse_stream_overrides(stream_json)?;
        options.file_settings_overrides = parse_settings_overrides(settings_json)?;
        crate::backup_folder::apply_default_exclusions(&mut options.exclude, explicit_exclusions);
        Ok(options)
    }
}

fn parse_crf(raw: &str) -> Result<i64, String> {
    parse_i64(raw, 0, 51, "CRF")
}

fn parse_i64(raw: &str, min: i64, max: i64, label: &str) -> Result<i64, String> {
    let value = raw
        .parse::<i64>()
        .map_err(|_| format!("{label} must be a whole number from {min} to {max}."))?;
    if !(min..=max).contains(&value) {
        return Err(format!(
            "{label} must be a whole number from {min} to {max}."
        ));
    }
    Ok(value)
}

fn parse_f64(raw: &str, min: f64, max: f64, label: &str) -> Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("{label} must be a finite number."))?;
    if !(min..=max).contains(&value) {
        return Err(format!("{label} must be between {min} and {max}."));
    }
    Ok(value)
}

fn parse_choice(raw: &str, allowed: &[&str], label: &str) -> Result<String, String> {
    if allowed.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(format!("{label} must be one of: {}.", allowed.join(", ")))
    }
}

fn effective_settings_json(root: &Path) -> Result<Value, String> {
    let anchor = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    let payload = serde_json::to_value(crate::settings::load_effective_settings(
        &anchor.to_string_lossy(),
    )?)
    .map_err(|error| format!("Could not encode effective settings: {error}"))?;
    Ok(payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({})))
}

pub(super) fn merged_blacklist(settings: &Value, extra: Vec<String>, replace: bool) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    let configured = if replace {
        Vec::new()
    } else {
        settings
            .pointer("/metadata/text_blacklist")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    for pattern in configured.into_iter().chain(extra) {
        if seen.insert(pattern.to_lowercase()) {
            merged.push(pattern);
        }
    }
    merged
}

pub(super) fn parse_path_map(
    raw: Option<String>,
    label: &str,
) -> Result<BTreeMap<PathBuf, PathBuf>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("The {label} are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err(format!("The {label} are limited to 512 media files."));
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let value = value
            .as_str()
            .ok_or_else(|| format!("Each entry in the {label} must map a path to a path."))?;
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A media path in the {label} is unavailable: {error}"))?;
        result.insert(key, PathBuf::from(value));
    }
    Ok(result)
}

pub(super) fn parse_metadata_overrides(
    raw: Option<String>,
) -> Result<BTreeMap<PathBuf, Map<String, Value>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Reviewed metadata is not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Reviewed metadata is limited to 512 media files per run.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let Value::Object(fields) = value else {
            return Err("Each reviewed metadata value must be an object.".into());
        };
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A reviewed metadata source is unavailable: {error}"))?;
        result.insert(key, fields);
    }
    Ok(result)
}

fn parse_tune_overrides(raw: Option<String>) -> Result<BTreeMap<PathBuf, String>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file tune overrides are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file tune overrides are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let tune = value.as_str().ok_or("Each per-file tune must be text.")?;
        if !matches!(tune, "animation" | "none") {
            return Err("Per-file tune overrides must be 'animation' or 'none'.".into());
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A per-file tune source is unavailable: {error}"))?;
        result.insert(key, tune.to_string());
    }
    Ok(result)
}

fn parse_stream_overrides(raw: Option<String>) -> Result<BTreeMap<PathBuf, Vec<u16>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file stream selections are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file stream selections are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let indices = value
            .as_array()
            .ok_or("Each per-file stream selection must be an array.")?;
        if indices.is_empty() || indices.len() > 1024 {
            return Err(
                "Each media file must select at least one stream and no more than 1024 streams."
                    .into(),
            );
        }
        let mut unique = BTreeSet::new();
        let mut values = Vec::new();
        for item in indices {
            let index = item
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or("Stream indices must be integers from 0 to 65535.")?;
            if !unique.insert(index) {
                return Err("Per-file stream selections must contain unique indices.".into());
            }
            values.push(index);
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A stream-selection source is unavailable: {error}"))?;
        result.insert(key, values);
    }
    Ok(result)
}

fn parse_settings_overrides(
    raw: Option<String>,
) -> Result<BTreeMap<PathBuf, Map<String, Value>>, String> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: Map<String, Value> = serde_json::from_str(&raw)
        .map_err(|error| format!("Per-file run settings are not valid JSON: {error}"))?;
    if parsed.len() > 512 {
        return Err("Per-file run settings are limited to 512 media files.".into());
    }
    let mut result = BTreeMap::new();
    for (key, value) in parsed {
        let Value::Object(fields) = value else {
            return Err("Each per-file run setting must be an object.".into());
        };
        if fields.is_empty() {
            return Err("Each per-file run setting must be a non-empty object.".into());
        }
        let key = PathBuf::from(&key)
            .canonicalize()
            .map_err(|error| format!("A per-file settings source is unavailable: {error}"))?;
        result.insert(key, fields);
    }
    Ok(result)
}

pub(super) fn effective_for_file(options: &RebuildRunOptions, source: &Path) -> EffectiveSettings {
    let mut effective = EffectiveSettings {
        crf: options.crf,
        test_crf: options.test_crf,
        preset: options.preset.clone(),
        tune: options.tune.clone(),
        promote_8bit_to_10bit: options.promote_8bit_to_10bit,
        preserve_hdr10plus: options.preserve_hdr10plus,
        dolby_vision_policy: options.dolby_vision_policy.clone(),
        stream_indices: options.file_stream_overrides.get(source).cloned(),
        clear_file_tune: false,
    };
    let Some(override_map) = options.file_settings_overrides.get(source) else {
        return effective;
    };
    if let Some(value) = override_map.get("crf").and_then(Value::as_i64) {
        effective.crf = value;
        if options.test_mode {
            effective.test_crf = value;
        }
    }
    if let Some(value) = override_map.get("preset").and_then(Value::as_str) {
        effective.preset = value.to_string();
    }
    if let Some(value) = override_map.get("tune").and_then(Value::as_str) {
        effective.tune = value.to_string();
        effective.clear_file_tune = true;
    }
    if let Some(value) = override_map
        .get("promote_8bit_to_10bit")
        .and_then(Value::as_bool)
    {
        effective.promote_8bit_to_10bit = value;
    }
    if let Some(value) = override_map
        .get("preserve_hdr10plus")
        .and_then(Value::as_bool)
    {
        effective.preserve_hdr10plus = value;
    }
    if let Some(value) = override_map.get("dolby_vision").and_then(Value::as_str) {
        effective.dolby_vision_policy = value.to_string();
    }
    if let Some(indices) = override_map.get("stream_indices").and_then(Value::as_array) {
        effective.stream_indices = Some(
            indices
                .iter()
                .filter_map(Value::as_u64)
                .filter_map(|value| u16::try_from(value).ok())
                .collect(),
        );
    }
    effective
}

// ------------------------------------------------------------------ planning
