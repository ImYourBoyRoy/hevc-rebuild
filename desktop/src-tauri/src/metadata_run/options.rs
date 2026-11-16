//! Parses the engine-style argument vector into `MetadataRunOptions`, falling back to the effective settings.

use super::*;

/// Engine flags the native metadata run accepts and deliberately ignores because
/// they only tune encoding, resources, or dependency repair.
pub(super) const IGNORED_VALUE_FLAGS: [&str; 22] = [
    "--crf",
    "--preset",
    "--tune",
    "--dolby-vision-policy",
    "--secondary-video",
    "--data-streams",
    "--resource-threading",
    "--reserved-cpu-threads",
    "--max-worker-threads",
    "--frame-threads",
    "--min-free-memory-gib",
    "--memory-reserve-percent",
    "--disk-projection-margin-percent",
    "--calibration-mode",
    "--calibration-cache-hours",
    "--calibration-large-file-gib",
    "--calibration-sample-seconds",
    "--calibration-max-candidates",
    "--calibration-tolerance-percent",
    "--preflight-output-ratio",
    "--quality-gate",
    "--quality-min-ssim",
];

pub(super) const IGNORED_BOOLEAN_FLAGS: [&str; 18] = [
    "--metadata-only",
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
];

pub(super) const REJECTED_FLAGS: [(&str, &str); 3] = [
    (
        "--file-stream-overrides-json",
        "metadata-only output always preserves every source stream; per-file stream selection is not allowed",
    ),
    (
        "--file-settings-overrides-json",
        "current-run encoder settings do not apply to metadata-only output",
    ),
    (
        "--file-tune-overrides-json",
        "per-file encoding tunes do not apply to metadata-only output",
    ),
];

#[derive(Clone, Default)]
pub(crate) struct MetadataRunOptions {
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
    pub comment: String,
    pub text_blacklist: Vec<String>,
    pub embed_artwork: bool,
    pub min_free_disk_gib: f64,
    pub stop_on_error: bool,
    pub keep_work: bool,
    pub reprocess: bool,
    pub work_root: String,
    pub stop_file: Option<PathBuf>,
    pub tool_paths: ToolPaths,
}

impl MetadataRunOptions {
    /// Parse the engine-style vector built by the GUI and MCP. Unknown flags are
    /// rejected so a future flag can never be silently dropped.
    pub(crate) fn from_engine_args(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            minimum_size_mib: 5,
            embed_artwork: true,
            work_root: "auto".into(),
            ..Self::default()
        };
        let mut root: Option<PathBuf> = None;
        let mut comment: Option<String> = None;
        let mut embed_artwork: Option<bool> = None;
        let mut blacklist: Vec<String> = Vec::new();
        let mut replace_blacklist = false;
        let mut min_free_disk_gib: Option<f64> = None;
        let mut output_dirs_json: Option<String> = None;
        let mut poster_json: Option<String> = None;
        let mut metadata_json: Option<String> = None;
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
                "--keep-work" => {
                    options.keep_work = true;
                    consumed = 1;
                }
                "--reprocess" => {
                    options.reprocess = true;
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
                "--allowed-poster-cache-root" => {
                    value()?;
                }
                "--comment" => comment = Some(value()?),
                "--metadata-text-blacklist" => blacklist.push(value()?),
                "--min-free-disk-gib" => {
                    min_free_disk_gib = Some(
                        value()?
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite() && *value >= 0.0)
                            .ok_or("The disk reserve must be a non-negative number of GiB.")?,
                    );
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
                    return Err(format!("Metadata-only {reason}."));
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "Metadata-only runs do not recognise the engine option {flag}."
                    ));
                }
                _ => {
                    if root.replace(PathBuf::from(name)).is_some() {
                        return Err("Metadata-only runs accept exactly one media path.".into());
                    }
                    consumed = 1;
                }
            }
            index += consumed;
        }
        let root = root.ok_or("Metadata-only runs require a media file or folder.")?;
        options.root = root
            .canonicalize()
            .map_err(|error| format!("Could not open the selected media path: {error}"))?;

        let settings = effective_settings(&options.root)?;
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
        options.min_free_disk_gib = min_free_disk_gib.unwrap_or_else(|| {
            settings
                .pointer("/resources/min_free_disk_gib")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        });
        options.text_blacklist = merged_blacklist(&settings, blacklist, replace_blacklist);
        options.file_output_dirs = parse_path_map(output_dirs_json, "per-file output folders")?
            .into_iter()
            .collect();
        options.poster_overrides = parse_path_map(poster_json, "poster selections")?
            .into_iter()
            .collect();
        options.metadata_overrides = parse_metadata_overrides(metadata_json)?;
        crate::backup_folder::apply_default_exclusions(&mut options.exclude, explicit_exclusions);
        Ok(options)
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
    Ok(payload
        .get("settings")
        .cloned()
        .unwrap_or_else(|| json!({})))
}

/// Configured patterns plus run patterns, or only run patterns when replaced.
pub(super) fn merged_blacklist(settings: &Value, extra: Vec<String>, replace: bool) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();
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

// ------------------------------------------------------------------ planning
