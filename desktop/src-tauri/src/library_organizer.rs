use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_normalization::UnicodeNormalization;

const MAX_PLAN_ITEMS: usize = 2_000;
const MAX_SCAN_FILES: usize = 10_000;
const MAX_TEMPLATE_BYTES: usize = 1_024;
const MAX_PLAN_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILENAME_BYTES: usize = 240;
const MAX_WALK_DEPTH: usize = 24;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RenameTemplateKind {
    Movie,
    Series,
    Season,
    Episode,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenameTemplateInput {
    pub(crate) source_path: String,
    pub(crate) kind: RenameTemplateKind,
    pub(crate) template: String,
    pub(crate) values: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RenamePlanDocument {
    pub(crate) root: String,
    pub(crate) items: Vec<RenameTemplateInput>,
    #[serde(default)]
    pub(crate) text_blacklist_patterns: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenamePreviewItem {
    pub(crate) source_path: String,
    pub(crate) target_path: String,
    pub(crate) target_name: String,
    pub(crate) is_directory: bool,
    pub(crate) unchanged: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameConflict {
    pub(crate) source_path: Option<String>,
    pub(crate) target_path: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenamePreview {
    pub(crate) preview_id: String,
    pub(crate) root: String,
    pub(crate) items: Vec<RenamePreviewItem>,
    pub(crate) conflicts: Vec<RenameConflict>,
    pub(crate) can_apply: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameApplyResult {
    pub(crate) renamed_count: usize,
    pub(crate) items: Vec<RenamePreviewItem>,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerMediaFile {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) parent_path: String,
    pub(crate) extension: String,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) multiple_episodes: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerSeasonDirectory {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) season: u32,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerEpisodeGap {
    pub(crate) season: u32,
    pub(crate) missing_count: u32,
    pub(crate) examples: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrganizerFolderScan {
    pub(crate) root: String,
    pub(crate) rename_root: String,
    pub(crate) files: Vec<OrganizerMediaFile>,
    pub(crate) season_directories: Vec<OrganizerSeasonDirectory>,
    pub(crate) episode_gaps: Vec<OrganizerEpisodeGap>,
    pub(crate) errors: Vec<String>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug)]
struct PlannedItem {
    source: PathBuf,
    target_name: String,
    target: PathBuf,
    is_directory: bool,
    unchanged: bool,
    fingerprint: (u64, u128, bool),
}

#[derive(Clone, Debug)]
struct InternalPlan {
    items: Vec<PlannedItem>,
    preview: RenamePreview,
}

fn allowed_shortcodes(kind: &RenameTemplateKind) -> &'static [&'static str] {
    match kind {
        RenameTemplateKind::Movie => &["title", "year", "certification", "ext"],
        RenameTemplateKind::Series => &["series_title", "first_year", "last_year", "year_range"],
        RenameTemplateKind::Season => &["season"],
        RenameTemplateKind::Episode => {
            &["series_title", "season", "episode", "episode_title", "ext"]
        }
    }
}

fn shortcode_value(
    token: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    let (name, width) = match token.split_once(':') {
        Some((name, "02")) => (name, Some(2)),
        Some((name, "03")) => (name, Some(3)),
        Some((name, "04")) => (name, Some(4)),
        Some(_) => {
            return Err(format!(
                "Unsupported format in naming shortcode {{{token}}}."
            ))
        }
        None => (token, None),
    };
    if !allowed_shortcodes(kind).contains(&name) {
        return Err(format!(
            "Unknown naming shortcode {{{token}}} for this item."
        ));
    }
    let value = values.get(name).cloned().unwrap_or_default();
    if let Some(width) = width {
        if !value.chars().all(|character| character.is_ascii_digit()) {
            return Err(format!("Shortcode {{{token}}} requires a numeric value."));
        }
        Ok(format!("{value:0>width$}"))
    } else {
        Ok(value)
    }
}

fn render_chunk(
    pattern: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<(String, bool), String> {
    let mut rendered = String::with_capacity(pattern.len());
    let mut found_nonempty = false;
    let mut rest = pattern;
    while !rest.is_empty() {
        let brace = rest.find('{');
        let close = rest.find('}');
        match (brace, close) {
            (None, None) => {
                rendered.push_str(rest);
                break;
            }
            (None, Some(_)) => {
                return Err("A naming shortcode has a closing brace but no opening brace.".into())
            }
            (Some(open), Some(close)) if close < open => {
                return Err(
                    "A naming shortcode has a closing brace before its opening brace.".into(),
                );
            }
            (Some(open), _) => {
                rendered.push_str(&rest[..open]);
                let after_open = &rest[open + 1..];
                let end = after_open
                    .find('}')
                    .ok_or("A naming shortcode is missing its closing brace.")?;
                if after_open[..end].contains('{') {
                    return Err("Naming shortcodes cannot be nested.".into());
                }
                let token = &after_open[..end];
                if token.is_empty() {
                    return Err("Empty naming shortcodes are not supported.".into());
                }
                let value = shortcode_value(token, kind, values)?;
                found_nonempty |= !value.is_empty();
                rendered.push_str(&value);
                rest = &after_open[end + 1..];
            }
        }
    }
    Ok((rendered, found_nonempty))
}

fn render_template(
    template: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    if template.trim().is_empty() || template.len() > MAX_TEMPLATE_BYTES {
        return Err("Naming templates must contain 1 to 1,024 UTF-8 bytes.".into());
    }
    let mut result = String::new();
    let mut rest = template;
    while !rest.is_empty() {
        let open = rest.find("[[");
        let close = rest.find("]]");
        match (open, close) {
            (None, None) => {
                let (text, _) = render_chunk(rest, kind, values)?;
                result.push_str(&text);
                break;
            }
            (None, Some(_)) => {
                return Err("An optional naming block is missing its opening [[ marker.".into())
            }
            (Some(_), Some(close)) if close < open.unwrap_or(usize::MAX) => {
                return Err("An optional naming block closes before it opens.".into());
            }
            (Some(open), _) => {
                let (prefix, _) = render_chunk(&rest[..open], kind, values)?;
                result.push_str(&prefix);
                let body = &rest[open + 2..];
                let end = body
                    .find("]]")
                    .ok_or("An optional naming block is missing its closing ]] marker.")?;
                if body[..end].contains("[[") {
                    return Err("Optional naming blocks cannot be nested.".into());
                }
                let (optional, has_value) = render_chunk(&body[..end], kind, values)?;
                if has_value {
                    result.push_str(&optional);
                }
                rest = &body[end + 2..];
            }
        }
    }

    let cleaned = result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace("[ ", "[")
        .replace(" ]", "]");
    let mut cleaned = cleaned.trim().to_string();
    for (left, right) in [('(', ')'), ('[', ']')] {
        let empty = format!("{left}{right}");
        cleaned = cleaned.replace(&empty, "");
    }
    cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return Err("The naming template produced an empty name.".into());
    }
    Ok(cleaned)
}

fn portable_component_error(name: &str, is_directory: bool) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." {
        return Some("The new name cannot be empty, '.' or '..'.".into());
    }
    if name.len() > MAX_FILENAME_BYTES {
        return Some(format!(
            "The new name exceeds the {MAX_FILENAME_BYTES}-byte portable component limit."
        ));
    }
    if matches!(name.chars().last(), Some(' ' | '.')) {
        return Some("Names cannot end with a space or period on supported filesystems.".into());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Some("The new name contains a path separator, control character, or character unsupported by Windows/macOS.".into());
    }
    let stem = if is_directory {
        name
    } else {
        name.rsplit_once('.').map_or(name, |(stem, _)| stem)
    };
    let device = stem
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or(stem)
        .to_ascii_uppercase();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
    {
        return Some("The new name is reserved by Windows.".into());
    }
    None
}

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_inside(root: &Path, path: &Path) -> bool {
    path != root && path.starts_with(root)
}

fn fingerprint(metadata: &fs::Metadata) -> (u64, u128, bool) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    (metadata.len(), modified, metadata.is_dir())
}

fn transform_with_directory_moves(path: &Path, items: &[PlannedItem]) -> PathBuf {
    let mut directories = items
        .iter()
        .filter(|item| item.is_directory)
        .collect::<Vec<_>>();
    directories.sort_by_key(|item| std::cmp::Reverse(item.source.components().count()));
    let mut result = path.to_path_buf();
    for directory in directories {
        if result != directory.source && result.starts_with(&directory.source) {
            if let Ok(suffix) = result.strip_prefix(&directory.source) {
                result = directory.target.join(suffix);
            }
        }
    }
    result
}

fn item_signature(item: &PlannedItem, state: &mut impl Hasher) {
    item.source.to_string_lossy().hash(state);
    item.target_name.hash(state);
    item.target.to_string_lossy().hash(state);
    item.is_directory.hash(state);
    item.fingerprint.hash(state);
}

fn media_companions(
    source_video: &Path,
    target_video_name: &str,
    directory_names: &mut HashMap<PathBuf, Vec<String>>,
) -> Result<Vec<(PathBuf, String)>, String> {
    const COMPANION_EXTENSIONS: &[&str] = &[
        "nfo", "srt", "ass", "ssa", "sub", "idx", "sup", "vtt", "webvtt", "mks", "smi", "ttml",
        "usf", "scc", "stl", "pgs", "ac3", "eac3", "ac4", "dts", "dtshd", "dtsma", "truehd", "thd",
        "flac", "alac", "aac", "m4a", "mp3", "wav", "opus", "ogg", "mka",
    ];
    let Some(source_stem) = source_video.file_stem().and_then(|value| value.to_str()) else {
        return Ok(Vec::new());
    };
    let Some(target_stem) = Path::new(target_video_name)
        .file_stem()
        .and_then(|value| value.to_str())
    else {
        return Ok(Vec::new());
    };
    let Some(parent) = source_video.parent() else {
        return Ok(Vec::new());
    };
    let names = if let Some(names) = directory_names.get(parent) {
        names.clone()
    } else {
        let entries = fs::read_dir(parent).map_err(|error| {
            format!(
                "Could not inspect companion files beside {}: {error}",
                source_video.display()
            )
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "Could not read companion-file list beside {}: {error}",
                    source_video.display()
                )
            })?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        directory_names.insert(parent.to_path_buf(), names.clone());
        names
    };
    let mut companions = Vec::new();
    let prefix = format!("{source_stem}.");
    for name in &names {
        let (Some(candidate_prefix), Some(remainder)) =
            (name.get(..prefix.len()), name.get(prefix.len()..))
        else {
            continue;
        };
        if remainder.is_empty() || !candidate_prefix.eq_ignore_ascii_case(&prefix) {
            continue;
        }
        let extension = Path::new(&name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if COMPANION_EXTENSIONS
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        {
            companions.push((parent.join(name), format!("{target_stem}.{remainder}")));
        }
    }
    const ARTWORK_SUFFIXES: &[&str] = &[
        "", "-poster", "_poster", ".poster", "-cover", "_cover", "-fanart", "_fanart", "-thumb",
    ];
    for name in &names {
        let Some(remainder) = name.get(source_stem.len()..) else {
            continue;
        };
        let Some(candidate_stem) = name.get(..source_stem.len()) else {
            continue;
        };
        if !candidate_stem.eq_ignore_ascii_case(source_stem) {
            continue;
        }
        let Some((suffix, extension)) = remainder.rsplit_once('.') else {
            continue;
        };
        if !["jpg", "jpeg", "png", "webp"]
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            || !ARTWORK_SUFFIXES
                .iter()
                .any(|allowed| suffix.eq_ignore_ascii_case(allowed))
        {
            continue;
        }
        let target_name = format!("{target_stem}{remainder}");
        let artwork = parent.join(name);
        let attribution = PathBuf::from(format!("{}.attribution.json", artwork.display()));
        let attribution_target = format!("{target_name}.attribution.json");
        companions.push((artwork, target_name));
        companions.push((attribution, attribution_target));
    }
    Ok(companions)
}

fn build_plan(root: &str, inputs: Vec<RenameTemplateInput>) -> Result<InternalPlan, String> {
    build_plan_with_patterns(root, inputs, &[])
}

fn build_plan_with_patterns(
    root: &str,
    inputs: Vec<RenameTemplateInput>,
    text_blacklist_patterns: &[String],
) -> Result<InternalPlan, String> {
    // Validate once even when a caller supplies no text shortcode values.
    crate::metadata_text::clean_value("", text_blacklist_patterns)?;
    if inputs.is_empty() || inputs.len() > MAX_PLAN_ITEMS {
        return Err(format!(
            "A rename plan must contain 1 to {MAX_PLAN_ITEMS} items."
        ));
    }
    let requested_root = Path::new(root);
    let root = requested_root
        .canonicalize()
        .map_err(|error| format!("Could not open the selected library folder: {error}"))?;
    if !root.is_dir() {
        return Err("The selected rename root must be a directory.".into());
    }

    let mut items = Vec::with_capacity(inputs.len());
    let mut source_keys = HashSet::new();
    for input in inputs {
        let requested_source = Path::new(&input.source_path);
        let raw_metadata = fs::symlink_metadata(requested_source)
            .map_err(|error| format!("Could not inspect {}: {error}", input.source_path))?;
        if raw_metadata.file_type().is_symlink() {
            return Err(format!(
                "Symbolic links cannot be renamed by a library plan: {}",
                input.source_path
            ));
        }
        if !raw_metadata.is_file() && !raw_metadata.is_dir() {
            return Err(format!(
                "Only regular files and directories can be renamed: {}",
                input.source_path
            ));
        }
        let source = requested_source
            .canonicalize()
            .map_err(|error| format!("Could not resolve {}: {error}", input.source_path))?;
        if !is_inside(&root, &source) {
            return Err(format!(
                "Every source must be inside the selected library folder: {}",
                input.source_path
            ));
        }
        let source_key = normalize_path_key(&source);
        if !source_keys.insert(source_key) {
            return Err(format!(
                "The same source path appears more than once: {}",
                input.source_path
            ));
        }
        let parent = source
            .parent()
            .ok_or("A rename source has no parent directory.")?;
        let current_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("A rename source name must be valid UTF-8.")?;
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        let mut values = input.values;
        for key in ["title", "series_title", "episode_title"] {
            if let Some(value) = values.get_mut(key) {
                *value = crate::metadata_text::clean_value(value, text_blacklist_patterns)?;
            }
        }
        values.insert(
            "ext".into(),
            if extension.is_empty() {
                String::new()
            } else {
                format!(".{extension}")
            },
        );
        let target_name = render_template(&input.template, &input.kind, &values)?;
        let is_directory = raw_metadata.is_dir();
        if let Some(reason) = portable_component_error(&target_name, is_directory) {
            return Err(format!(
                "Invalid target for {}: {reason}",
                input.source_path
            ));
        }
        if !is_directory {
            let new_extension = Path::new(&target_name)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !extension.eq_ignore_ascii_case(new_extension) {
                return Err(format!(
                    "The file extension must be preserved for {}.",
                    input.source_path
                ));
            }
        }
        let target = parent.join(&target_name);
        let source_meta = fs::metadata(&source)
            .map_err(|error| format!("Could not inspect {}: {error}", source.display()))?;
        let unchanged = current_name == target_name;
        items.push(PlannedItem {
            source,
            target_name,
            target,
            is_directory,
            unchanged,
            fingerprint: fingerprint(&source_meta),
        });
    }

    // Matching metadata, artwork, subtitle, and external-audio sidecars follow
    // the reviewed video rename so they remain associated with the media.
    let video_items = items
        .iter()
        .filter(|item| !item.is_directory && is_video_file(&item.source))
        .cloned()
        .collect::<Vec<_>>();
    let mut companion_directory_names = HashMap::new();
    for video in video_items {
        for (companion, target_name) in media_companions(
            &video.source,
            &video.target_name,
            &mut companion_directory_names,
        )? {
            let metadata = match fs::symlink_metadata(&companion) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata
                }
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(format!(
                        "Could not inspect companion sidecar {}: {error}",
                        companion.display()
                    ))
                }
            };
            let canonical = companion.canonicalize().map_err(|error| {
                format!(
                    "Could not resolve companion sidecar {}: {error}",
                    companion.display()
                )
            })?;
            if !is_inside(&root, &canonical) {
                return Err(format!(
                    "Companion sidecar is outside the selected library folder: {}",
                    companion.display()
                ));
            }
            let companion_key = normalize_path_key(&canonical);
            if !source_keys.insert(companion_key.clone()) {
                if let Some(previous) = items
                    .iter()
                    .find(|item| normalize_path_key(&item.source) == companion_key)
                {
                    if previous.target.file_name().and_then(|name| name.to_str())
                        != Some(target_name.as_str())
                    {
                        return Err(format!("Companion sidecar {} would belong to more than one renamed video. Review those assignments separately before renaming.", canonical.display()));
                    }
                }
                continue;
            }
            if items.len() >= MAX_PLAN_ITEMS {
                return Err(format!("The rename plan exceeds the {MAX_PLAN_ITEMS}-item limit after including matching NFO, artwork, subtitle, and audio sidecars."));
            }
            if let Some(reason) = portable_component_error(&target_name, false) {
                return Err(format!(
                    "Invalid companion sidecar target for {}: {reason}",
                    canonical.display()
                ));
            }
            let parent = canonical
                .parent()
                .ok_or("A companion sidecar has no parent directory.")?;
            let target = parent.join(&target_name);
            let current_name = canonical
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("A companion sidecar name must be valid UTF-8.")?;
            let unchanged = current_name == target_name;
            items.push(PlannedItem {
                source: canonical.clone(),
                target_name,
                target,
                is_directory: false,
                unchanged,
                fingerprint: fingerprint(&metadata),
            });
        }
    }

    let mut entries = Vec::with_capacity(items.len());
    let final_targets = items
        .iter()
        .map(|item| transform_with_directory_moves(&item.target, &items))
        .collect::<Vec<_>>();
    for (item, target) in items.iter_mut().zip(final_targets) {
        item.target = target;
        entries.push(RenamePreviewItem {
            source_path: item.source.to_string_lossy().into_owned(),
            target_path: item.target.to_string_lossy().into_owned(),
            target_name: item.target_name.clone(),
            is_directory: item.is_directory,
            // An unchanged basename can still move because a selected parent
            // directory is being renamed. Report the full path outcome.
            unchanged: normalize_path_key(&item.source) == normalize_path_key(&item.target),
        });
    }

    let mut conflicts = Vec::new();
    let mut destinations: HashMap<String, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let key = normalize_path_key(&item.target);
        if let Some(previous) = destinations.insert(key, index) {
            conflicts.push(RenameConflict {
                source_path: Some(item.source.to_string_lossy().into_owned()),
                target_path: item.target.to_string_lossy().into_owned(),
                reason: format!(
                    "Two selected items would use the same destination (also selected at {}).",
                    items[previous].source.display()
                ),
            });
        }
    }
    for (index, item) in items.iter().enumerate() {
        if item.unchanged {
            continue;
        }
        let target_key = normalize_path_key(&item.target);
        for (other_index, other) in items.iter().enumerate() {
            if index == other_index {
                continue;
            }
            let other_key = normalize_path_key(&other.target);
            if !item.is_directory
                && other_key.starts_with(&(target_key.clone() + std::path::MAIN_SEPARATOR_STR))
            {
                conflicts.push(RenameConflict {
                    source_path: Some(item.source.to_string_lossy().into_owned()),
                    target_path: item.target.to_string_lossy().into_owned(),
                    reason: "A file destination cannot contain another planned destination.".into(),
                });
                break;
            }
        }

        match fs::symlink_metadata(&item.target) {
            Ok(_) => {
                let target_source = items
                    .iter()
                    .find(|candidate| normalize_path_key(&candidate.source) == target_key);
                let vacated_by_source =
                    target_source.is_some_and(|source_item| !source_item.unchanged);
                let target_after_dir_moves = transform_with_directory_moves(&item.target, &items);
                let vacated_by_directory =
                    normalize_path_key(&target_after_dir_moves) != target_key;
                if !vacated_by_source && !vacated_by_directory {
                    conflicts.push(RenameConflict {
                        source_path: Some(item.source.to_string_lossy().into_owned()),
                        target_path: item.target.to_string_lossy().into_owned(),
                        reason:
                            "The destination already exists and is not being moved by this plan."
                                .into(),
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => conflicts.push(RenameConflict {
                source_path: Some(item.source.to_string_lossy().into_owned()),
                target_path: item.target.to_string_lossy().into_owned(),
                reason: format!("Could not inspect the destination: {error}"),
            }),
        }
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    for item in &items {
        item_signature(item, &mut hasher);
    }
    for conflict in &conflicts {
        conflict.source_path.hash(&mut hasher);
        conflict.target_path.hash(&mut hasher);
        conflict.reason.hash(&mut hasher);
    }
    let preview_id = format!("rename-{:016x}", hasher.finish());
    let can_apply = conflicts.is_empty() && items.iter().any(|item| !item.unchanged);
    let preview = RenamePreview {
        preview_id,
        root: root.to_string_lossy().into_owned(),
        items: entries,
        conflicts,
        can_apply,
    };
    Ok(InternalPlan { items, preview })
}

pub(crate) fn preview_rename_batch(
    root: String,
    inputs: Vec<RenameTemplateInput>,
) -> Result<RenamePreview, String> {
    preview_rename_batch_with_patterns(root, inputs, Vec::new())
}

pub(crate) fn preview_rename_batch_with_patterns(
    root: String,
    inputs: Vec<RenameTemplateInput>,
    text_blacklist_patterns: Vec<String>,
) -> Result<RenamePreview, String> {
    Ok(build_plan_with_patterns(&root, inputs, &text_blacklist_patterns)?.preview)
}

fn unique_temp_path(parent: &Path, transaction: u128, sequence: usize) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".hevc-rebuild-rename-{transaction:032x}-{sequence:04x}-{counter:016x}.tmp"
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Source path contains a NUL byte.",
        )
    })?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Destination path contains a NUL byte.",
        )
    })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Source path contains a NUL byte.",
        )
    })?;
    let target = CString::new(target.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Destination path contains a NUL byte.",
        )
    })?;
    let result = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    // Windows MoveFileEx semantics used by std::fs::rename fail when the
    // destination exists, so it does not silently replace a library item.
    fs::rename(source, target)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn rename_without_replacing(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Destination already exists.",
        ));
    }
    fs::rename(source, target)
}

#[derive(Clone)]
struct MoveGroup {
    parent: PathBuf,
    items: Vec<PlannedItem>,
}

fn group_by_parent(items: &[PlannedItem]) -> Vec<MoveGroup> {
    let mut grouped: HashMap<PathBuf, Vec<PlannedItem>> = HashMap::new();
    for item in items.iter().filter(|item| !item.unchanged) {
        if let Some(parent) = item.source.parent() {
            grouped
                .entry(parent.to_path_buf())
                .or_default()
                .push(item.clone());
        }
    }
    let mut groups = grouped
        .into_iter()
        .map(|(parent, mut items)| {
            items.sort_by(|left, right| left.source.cmp(&right.source));
            MoveGroup { parent, items }
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| std::cmp::Reverse(group.parent.components().count()));
    groups
}

fn rollback_group(
    group: &MoveGroup,
    current_names: &[PathBuf],
    transaction: u128,
) -> Result<(), String> {
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(group.items.len());
    for (index, current) in current_names.iter().enumerate() {
        let temp = unique_temp_path(&group.parent, transaction, index + MAX_PLAN_ITEMS);
        if let Err(error) = rename_without_replacing(current, &temp) {
            for (already_temp, original_current) in staged.iter().rev() {
                let _ = rename_without_replacing(already_temp, original_current);
            }
            return Err(format!(
                "Could not stage a rename rollback at {}: {error}",
                current.display()
            ));
        }
        staged.push((temp, current.clone()));
    }
    for (index, (temp, _)) in staged.iter().enumerate() {
        let original = &group.items[index].source;
        if let Err(error) = rename_without_replacing(temp, original) {
            return Err(format!(
                "Could not restore {} from {}: {error}",
                original.display(),
                temp.display()
            ));
        }
    }
    Ok(())
}

fn apply_plan(plan: InternalPlan) -> Result<RenameApplyResult, String> {
    if !plan.preview.can_apply {
        return Err(
            "This rename preview has conflicts or contains no changes; nothing was renamed.".into(),
        );
    }
    // Recheck source snapshots immediately before the first mutation. The
    // caller has already recomputed the plan and compared its preview id.
    for item in plan.items.iter().filter(|item| !item.unchanged) {
        let metadata = fs::symlink_metadata(&item.source).map_err(|error| {
            format!(
                "The source changed after preview: {} ({error})",
                item.source.display()
            )
        })?;
        if metadata.file_type().is_symlink() || fingerprint(&metadata) != item.fingerprint {
            return Err(format!(
                "The source changed after preview; create a new preview: {}",
                item.source.display()
            ));
        }
    }
    let groups = group_by_parent(&plan.items);
    let transaction = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        ^ (std::process::id() as u128);
    let mut completed: Vec<MoveGroup> = Vec::new();
    for group in groups {
        let mut current = group
            .items
            .iter()
            .map(|item| item.source.clone())
            .collect::<Vec<_>>();
        let mut stage_error = None;
        for (index, _item) in group.items.iter().enumerate() {
            let temp = unique_temp_path(&group.parent, transaction, index);
            if let Err(error) = rename_without_replacing(&current[index], &temp) {
                stage_error = Some(format!(
                    "Could not stage {}: {error}",
                    current[index].display()
                ));
                break;
            }
            current[index] = temp;
        }
        if let Some(error) = stage_error {
            let rollback = rollback_group(&group, &current, transaction);
            for finished in completed.iter().rev() {
                let current_targets = finished
                    .items
                    .iter()
                    .map(|item| finished.parent.join(&item.target_name))
                    .collect::<Vec<_>>();
                if let Err(rollback_error) = rollback_group(finished, &current_targets, transaction)
                {
                    return Err(format!("{error}; group rollback failed: {rollback_error}. Previous groups also need recovery."));
                }
            }
            return Err(match rollback {
                Ok(()) => format!("{error}; staged paths were restored and no rename was kept."),
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}. Check the displayed source and .tmp paths."),
            });
        }

        let mut commit_error = None;
        for (index, item) in group.items.iter().enumerate() {
            let target = group.parent.join(&item.target_name);
            if let Err(error) = rename_without_replacing(&current[index], &target) {
                commit_error = Some(format!(
                    "Could not place {} at {}: {error}",
                    item.source.display(),
                    target.display()
                ));
                break;
            }
            current[index] = target;
        }
        if let Some(error) = commit_error {
            let rollback = rollback_group(&group, &current, transaction);
            for finished in completed.iter().rev() {
                let current_targets = finished
                    .items
                    .iter()
                    .map(|item| finished.parent.join(&item.target_name))
                    .collect::<Vec<_>>();
                if let Err(rollback_error) = rollback_group(finished, &current_targets, transaction)
                {
                    return Err(format!("{error}; group rollback failed: {rollback_error}. Previous groups also need recovery."));
                }
            }
            return Err(match rollback {
                Ok(()) => format!("{error}; completed operations were rolled back."),
                Err(rollback_error) => format!("{error}; rollback failed: {rollback_error}. Check the displayed source and .tmp paths."),
            });
        }
        completed.push(group);
    }

    Ok(RenameApplyResult {
        renamed_count: plan
            .preview
            .items
            .iter()
            .filter(|item| !item.unchanged)
            .count(),
        items: plan.preview.items,
        message: "The reviewed rename plan completed. Media contents were not modified; matching NFO, artwork attribution, subtitle, and external-audio sidecars moved with their video files.".into(),
    })
}

pub(crate) fn apply_rename_batch(
    root: String,
    inputs: Vec<RenameTemplateInput>,
    preview_id: String,
    confirmed: bool,
) -> Result<RenameApplyResult, String> {
    apply_rename_batch_with_patterns(root, inputs, Vec::new(), preview_id, confirmed)
}

pub(crate) fn apply_rename_batch_with_patterns(
    root: String,
    inputs: Vec<RenameTemplateInput>,
    text_blacklist_patterns: Vec<String>,
    preview_id: String,
    confirmed: bool,
) -> Result<RenameApplyResult, String> {
    if !confirmed {
        return Err(
            "Explicit confirmation is required before renaming media-library items.".into(),
        );
    }
    let plan = build_plan_with_patterns(&root, inputs, &text_blacklist_patterns)?;
    if plan.preview.preview_id != preview_id {
        return Err("The rename plan or its sources changed after preview. Review a fresh before/after preview.".into());
    }
    if !plan.preview.can_apply {
        return Err("Rename conflicts block this batch. Resolve every conflict and preview again before applying.".into());
    }
    apply_plan(plan)
}

pub(crate) fn read_plan_document(path: &Path) -> Result<RenamePlanDocument, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect the rename plan file: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_PLAN_JSON_BYTES as u64 {
        return Err("Rename plan JSON must be a regular file no larger than 2 MiB.".into());
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read the rename plan file: {error}"))?;
    if bytes.len() > MAX_PLAN_JSON_BYTES {
        return Err("Rename plan JSON must be no larger than 2 MiB.".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Rename plan JSON is invalid: {error}"))
}

fn plan_text_blacklist(plan: &RenamePlanDocument) -> Result<Vec<String>, String> {
    match &plan.text_blacklist_patterns {
        Some(patterns) => Ok(patterns.clone()),
        None => crate::settings::metadata_text_blacklist_for_root(&plan.root),
    }
}

pub(crate) fn organizer_cli_mode(args: &[String]) -> i32 {
    let usage = "Usage:\n  hevc-rebuild-studio organize scan ROOT\n  hevc-rebuild-studio organize preview PLAN.json\n  hevc-rebuild-studio organize apply PLAN.json --preview-id ID --confirm";
    let result = match args {
        [mode, root] if mode == "scan" => scan_organizer_folder(root.clone())
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        [mode, plan_path] if mode == "preview" => read_plan_document(Path::new(plan_path))
            .and_then(|plan| {
                let patterns = plan_text_blacklist(&plan)?;
                preview_rename_batch_with_patterns(plan.root, plan.items, patterns)
            })
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        [mode, plan_path, preview_flag, preview_id, confirm]
            if mode == "apply" && preview_flag == "--preview-id" && confirm == "--confirm" =>
        {
            read_plan_document(Path::new(plan_path))
                .and_then(|plan| {
                    let patterns = plan_text_blacklist(&plan)?;
                    apply_rename_batch_with_patterns(
                        plan.root,
                        plan.items,
                        patterns,
                        preview_id.clone(),
                        true,
                    )
                })
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        _ => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not format organizer JSON: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("Organizer operation failed: {error}");
            1
        }
    }
}

fn parse_number(chars: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() && end - start < 4 {
        end += 1;
    }
    if end == start || (end < chars.len() && chars[end].is_ascii_digit()) {
        return None;
    }
    let number = std::str::from_utf8(&chars[start..end]).ok()?.parse().ok()?;
    Some((number, end))
}

fn parse_episode_id(stem: &str) -> (Option<u32>, Option<u32>, bool) {
    let upper = stem.to_ascii_uppercase();
    let upper = upper.as_bytes();
    for index in 0..upper.len() {
        if upper[index] != b'S' || (index > 0 && upper[index - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let Some((season, after_season)) = parse_number(upper, index + 1) else {
            continue;
        };
        let mut episode_marker = after_season;
        while episode_marker < upper.len()
            && matches!(upper[episode_marker], b' ' | b'.' | b'_' | b'-')
        {
            episode_marker += 1;
        }
        if episode_marker >= upper.len() || upper[episode_marker] != b'E' {
            continue;
        }
        let Some((episode, after_episode)) = parse_number(upper, episode_marker + 1) else {
            continue;
        };
        let mut next = after_episode;
        while next < upper.len() && matches!(upper[next], b' ' | b'.' | b'_' | b'-') {
            next += 1;
        }
        let multiple =
            next < upper.len() && upper[next] == b'E' && parse_number(upper, next + 1).is_some();
        return (Some(season), Some(episode), multiple);
    }
    (None, None, false)
}

fn episode_number_gaps(files: &[OrganizerMediaFile]) -> Vec<OrganizerEpisodeGap> {
    const MAX_EXAMPLES_PER_SEASON: usize = 8;
    let mut by_season: BTreeMap<u32, (BTreeSet<u32>, bool)> = BTreeMap::new();
    for file in files {
        let (Some(season), Some(episode)) = (file.season, file.episode) else {
            continue;
        };
        let (episodes, has_multi_episode_file) = by_season.entry(season).or_default();
        *has_multi_episode_file |= file.multiple_episodes;
        episodes.insert(episode);
    }

    by_season
        .into_iter()
        .filter_map(|(season, (episodes, has_multi_episode_file))| {
            // Multi-episode files encode more than one number in one filename;
            // avoid presenting an incomplete gap list as authoritative.
            if has_multi_episode_file || episodes.len() < 2 {
                return None;
            }
            let mut missing_count = 0u32;
            let mut examples = Vec::new();
            let first = *episodes.first()?;
            let last = *episodes.last()?;
            for episode in first.saturating_add(1)..last {
                if !episodes.contains(&episode) {
                    missing_count = missing_count.saturating_add(1);
                    if examples.len() < MAX_EXAMPLES_PER_SEASON {
                        examples.push(episode);
                    }
                }
            }
            (missing_count > 0).then_some(OrganizerEpisodeGap {
                season,
                missing_count,
                examples,
            })
        })
        .collect()
}

fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv"
                    | "mp4"
                    | "m4v"
                    | "mov"
                    | "avi"
                    | "webm"
                    | "wmv"
                    | "mpg"
                    | "mpeg"
                    | "m2ts"
                    | "mts"
                    | "ts"
                    | "vob"
                    | "flv"
                    | "ogv"
            )
        })
}

fn season_number(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if lower == "specials" {
        return Some(0);
    }
    let rest = lower
        .strip_prefix("season")?
        .trim_start_matches([' ', '_', '-']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub(crate) fn scan_organizer_folder(root: String) -> Result<OrganizerFolderScan, String> {
    let root = Path::new(&root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media folder: {error}"))?;
    if !root.is_dir() {
        return Err("Choose a directory to scan for movies or episodes.".into());
    }
    let mut files = Vec::new();
    let mut seasons = Vec::new();
    let mut errors = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    let mut truncated = false;
    while let Some((directory, depth)) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(format!("{}: {error}", directory.display()));
                if errors.len() >= 200 {
                    break;
                }
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("{}: {error}", directory.display()));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.')
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "old" | "output" | "converted" | "youtube uploads"
                    )
                {
                    continue;
                }
                if depth == 0 {
                    if let Some(number) = season_number(&name) {
                        seasons.push(OrganizerSeasonDirectory {
                            path: path.to_string_lossy().into_owned(),
                            name,
                            season: number,
                        });
                    }
                }
                if depth < MAX_WALK_DEPTH {
                    children.push((path, depth + 1));
                }
                continue;
            }
            if metadata.is_file() && is_video_file(&path) {
                if files.len() >= MAX_SCAN_FILES {
                    truncated = true;
                    break;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&name);
                let (season, episode, multiple_episodes) = parse_episode_id(stem);
                files.push(OrganizerMediaFile {
                    path: path.to_string_lossy().into_owned(),
                    name,
                    parent_path: directory.to_string_lossy().into_owned(),
                    extension: path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or("")
                        .to_string(),
                    season,
                    episode,
                    multiple_episodes,
                });
            }
        }
        children.sort_by(|left, right| left.0.cmp(&right.0));
        stack.extend(children.into_iter().rev());
        if truncated {
            break;
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    seasons.sort_by_key(|season| (season.season, season.path.clone()));
    let episode_gaps = episode_number_gaps(&files);
    Ok(OrganizerFolderScan {
        root: root.to_string_lossy().into_owned(),
        rename_root: root
            .parent()
            .unwrap_or(&root)
            .to_string_lossy()
            .into_owned(),
        files,
        season_directories: seasons,
        episode_gaps,
        errors,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

    fn scratch() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "hevc-rename-test-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn input(
        source: &Path,
        kind: RenameTemplateKind,
        template: &str,
        values: &[(&str, &str)],
    ) -> RenameTemplateInput {
        RenameTemplateInput {
            source_path: source.to_string_lossy().into_owned(),
            kind,
            template: template.into(),
            values: values
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        }
    }

    #[test]
    fn renders_film_and_episode_defaults_and_optional_parts() {
        let film = render_template(
            "{title} [[({year} {certification})]]{ext}",
            &RenameTemplateKind::Movie,
            &BTreeMap::from([
                ("title".into(), "Gnomeo & Juliet".into()),
                ("year".into(), "2011".into()),
                ("certification".into(), "G".into()),
                ("ext".into(), ".mkv".into()),
            ]),
        )
        .unwrap();
        assert_eq!(film, "Gnomeo & Juliet (2011 G).mkv");
        let no_rating = render_template(
            "{title} [[({year} {certification})]]{ext}",
            &RenameTemplateKind::Movie,
            &BTreeMap::from([
                ("title".into(), "Example".into()),
                ("year".into(), "2011".into()),
                ("ext".into(), ".mkv".into()),
            ]),
        )
        .unwrap();
        assert_eq!(no_rating, "Example (2011).mkv");
        let episode = render_template(
            "S{season:02}E{episode:02}.{episode_title}{ext}",
            &RenameTemplateKind::Episode,
            &BTreeMap::from([
                ("season".into(), "1".into()),
                ("episode".into(), "1".into()),
                ("episode_title".into(), "The Tale of the Frog Prince".into()),
                ("ext".into(), ".mkv".into()),
            ]),
        )
        .unwrap();
        assert_eq!(episode, "S01E01.The Tale of the Frog Prince.mkv");
    }

    #[test]
    fn invalid_shortcode_and_reserved_filename_are_rejected() {
        assert!(
            render_template("{unknown}", &RenameTemplateKind::Movie, &BTreeMap::new())
                .unwrap_err()
                .contains("Unknown")
        );
        assert!(portable_component_error("CON.mkv", false).is_some());
        assert!(portable_component_error("bad/name.mkv", false).is_some());
    }

    #[test]
    fn duplicate_normalized_targets_block_the_whole_plan() {
        let root = scratch();
        let first = root.join("first.mkv");
        let second = root.join("second.mkv");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let plan = build_plan(
            &root.to_string_lossy(),
            vec![
                input(
                    &first,
                    RenameTemplateKind::Movie,
                    "Caf\u{00e9}.mkv",
                    &[("title", "unused")],
                ),
                input(
                    &second,
                    RenameTemplateKind::Movie,
                    "Cafe\u{0301}.mkv",
                    &[("title", "unused")],
                ),
            ],
        )
        .unwrap();
        assert!(!plan.preview.can_apply);
        assert!(plan
            .preview
            .conflicts
            .iter()
            .any(|conflict| conflict.reason.contains("same destination")));
        assert!(first.exists() && second.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_destination_is_blocked_without_overwrite() {
        let root = scratch();
        let source = root.join("source.mkv");
        let destination = root.join("destination.mkv");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&destination, b"destination bytes").unwrap();
        let plan = build_plan(
            &root.to_string_lossy(),
            vec![input(
                &source,
                RenameTemplateKind::Movie,
                "destination{ext}",
                &[],
            )],
        )
        .unwrap();
        assert!(!plan.preview.can_apply);
        assert!(apply_rename_batch(
            root.to_string_lossy().into_owned(),
            vec![input(
                &source,
                RenameTemplateKind::Movie,
                "destination{ext}",
                &[]
            ),],
            plan.preview.preview_id,
            true
        )
        .is_err());
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert_eq!(fs::read(&destination).unwrap(), b"destination bytes");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cycles_and_nested_tv_folder_renames_keep_file_contents() {
        let root = scratch();
        let show = root.join("Faerie Tale Theatre");
        let season = show.join("Season 1");
        fs::create_dir_all(&season).unwrap();
        let episode = season.join("Faerie.Tale.Theatre.S01E01.avi");
        fs::write(&episode, b"video bytes").unwrap();
        let plan = build_plan(
            &root.to_string_lossy(),
            vec![
                input(
                    &show,
                    RenameTemplateKind::Series,
                    "{series_title} [[({year_range})]]",
                    &[
                        ("series_title", "Faerie Tale Theatre"),
                        ("year_range", "1982-1987"),
                    ],
                ),
                input(
                    &season,
                    RenameTemplateKind::Season,
                    "Season {season:02}",
                    &[("season", "1")],
                ),
                input(
                    &episode,
                    RenameTemplateKind::Episode,
                    "S{season:02}E{episode:02}.{episode_title}{ext}",
                    &[
                        ("season", "1"),
                        ("episode", "1"),
                        ("episode_title", "The Tale of the Frog Prince"),
                    ],
                ),
            ],
        )
        .unwrap();
        let expected = root.join(
            "Faerie Tale Theatre (1982-1987)/Season 01/S01E01.The Tale of the Frog Prince.avi",
        );
        assert!(plan.preview.can_apply);
        assert!(plan
            .preview
            .items
            .iter()
            .any(|item| item.target_path == expected.to_string_lossy()));
        let applied = apply_rename_batch(
            root.to_string_lossy().into_owned(),
            vec![
                input(
                    &show,
                    RenameTemplateKind::Series,
                    "{series_title} [[({year_range})]]",
                    &[
                        ("series_title", "Faerie Tale Theatre"),
                        ("year_range", "1982-1987"),
                    ],
                ),
                input(
                    &season,
                    RenameTemplateKind::Season,
                    "Season {season:02}",
                    &[("season", "1")],
                ),
                input(
                    &episode,
                    RenameTemplateKind::Episode,
                    "S{season:02}E{episode:02}.{episode_title}{ext}",
                    &[
                        ("season", "1"),
                        ("episode", "1"),
                        ("episode_title", "The Tale of the Frog Prince"),
                    ],
                ),
            ],
            plan.preview.preview_id,
            true,
        )
        .unwrap();
        assert_eq!(applied.renamed_count, 3);
        assert_eq!(fs::read(expected).unwrap(), b"video bytes");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_marks_unchanged_filenames_as_changed_when_parent_moves() {
        let root = scratch();
        let show = root.join("Old Show");
        let season = show.join("Season 1");
        fs::create_dir_all(&season).unwrap();
        let episode = season.join("Episode.mkv");
        fs::write(&episode, b"video bytes").unwrap();
        let inputs = || {
            vec![
                input(
                    &show,
                    RenameTemplateKind::Series,
                    "{series_title}",
                    &[("series_title", "New Show")],
                ),
                input(
                    &season,
                    RenameTemplateKind::Season,
                    "Season {season}",
                    &[("season", "1")],
                ),
                input(
                    &episode,
                    RenameTemplateKind::Episode,
                    "{episode_title}{ext}",
                    &[("episode_title", "Episode")],
                ),
            ]
        };
        let preview = preview_rename_batch(root.to_string_lossy().into_owned(), inputs()).unwrap();
        let episode_preview = preview
            .items
            .iter()
            .find(|item| item.source_path == episode.to_string_lossy())
            .unwrap();
        assert_eq!(
            Path::new(&episode_preview.target_path).file_name().unwrap(),
            "Episode.mkv"
        );
        assert!(!episode_preview.unchanged);

        let applied = apply_rename_batch(
            root.to_string_lossy().into_owned(),
            inputs(),
            preview.preview_id,
            true,
        )
        .unwrap();
        assert_eq!(applied.renamed_count, 3);
        assert_eq!(
            fs::read(root.join("New Show/Season 1/Episode.mkv")).unwrap(),
            b"video bytes"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_renames_include_matching_metadata_artwork_subtitle_and_audio_sidecars() {
        let root = scratch();
        let source = root.join("Movie.mkv");
        let nfo = root.join("Movie.nfo");
        let poster = root.join("Movie-poster.jpg");
        let credit = root.join("Movie-poster.jpg.attribution.json");
        let subtitle = root.join("movie.en.srt");
        let audio = root.join("Movie.commentary.ac3");
        let local_cover = root.join("Movie.jpg");
        let local_cover_credit = root.join("Movie.jpg.attribution.json");
        let local_fanart = root.join("Movie-cover.png");
        let local_fanart_credit = root.join("Movie-cover.png.attribution.json");
        let unrelated = root.join("MovieExtra.srt");
        fs::write(&source, b"video").unwrap();
        fs::write(&nfo, b"movie metadata").unwrap();
        fs::write(&poster, b"poster image").unwrap();
        fs::write(&credit, b"poster credit").unwrap();
        fs::write(&subtitle, b"subtitle").unwrap();
        fs::write(&audio, b"audio").unwrap();
        fs::write(&local_cover, b"local cover").unwrap();
        fs::write(&local_cover_credit, b"local cover credit").unwrap();
        fs::write(&local_fanart, b"local fanart").unwrap();
        fs::write(&local_fanart_credit, b"local fanart credit").unwrap();
        fs::write(&unrelated, b"unrelated subtitle").unwrap();
        let inputs = || {
            vec![input(
                &source,
                RenameTemplateKind::Movie,
                "Renamed Movie{ext}",
                &[],
            )]
        };

        let preview = preview_rename_batch(root.to_string_lossy().into_owned(), inputs()).unwrap();
        assert_eq!(preview.items.len(), 10);
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie.nfo")));
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie-poster.jpg")));
        assert!(preview.items.iter().any(|item| item
            .target_path
            .ends_with("Renamed Movie-poster.jpg.attribution.json")));
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie.en.srt")));
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie.commentary.ac3")));
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie.jpg")));
        assert!(preview.items.iter().any(|item| item
            .target_path
            .ends_with("Renamed Movie.jpg.attribution.json")));
        assert!(preview
            .items
            .iter()
            .any(|item| item.target_path.ends_with("Renamed Movie-cover.png")));
        assert!(preview.items.iter().any(|item| item
            .target_path
            .ends_with("Renamed Movie-cover.png.attribution.json")));
        assert!(preview.can_apply);

        let applied = apply_rename_batch(
            root.to_string_lossy().into_owned(),
            inputs(),
            preview.preview_id,
            true,
        )
        .unwrap();
        assert_eq!(applied.renamed_count, 10);
        assert_eq!(fs::read(root.join("Renamed Movie.mkv")).unwrap(), b"video");
        assert_eq!(
            fs::read(root.join("Renamed Movie.nfo")).unwrap(),
            b"movie metadata"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie-poster.jpg")).unwrap(),
            b"poster image"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie-poster.jpg.attribution.json")).unwrap(),
            b"poster credit"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie.en.srt")).unwrap(),
            b"subtitle"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie.commentary.ac3")).unwrap(),
            b"audio"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie.jpg")).unwrap(),
            b"local cover"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie.jpg.attribution.json")).unwrap(),
            b"local cover credit"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie-cover.png")).unwrap(),
            b"local fanart"
        );
        assert_eq!(
            fs::read(root.join("Renamed Movie-cover.png.attribution.json")).unwrap(),
            b"local fanart credit"
        );
        assert_eq!(fs::read(unrelated).unwrap(), b"unrelated subtitle");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_standard_and_multi_episode_ids() {
        assert_eq!(
            parse_episode_id("Faerie.Tale.S01E02.The.Show"),
            (Some(1), Some(2), false)
        );
        assert_eq!(
            parse_episode_id("Show.S1E2E3.Title"),
            (Some(1), Some(2), true)
        );
        assert_eq!(parse_episode_id("Movie.2020.1080p"), (None, None, false));
    }

    #[test]
    fn episode_gap_summary_is_bounded_and_ignores_multi_episode_seasons() {
        let file = |name: &str, season, episode, multiple_episodes| OrganizerMediaFile {
            path: name.into(),
            name: name.into(),
            parent_path: String::new(),
            extension: "mkv".into(),
            season,
            episode,
            multiple_episodes,
        };
        let files = vec![
            file("Show.S01E01.mkv", Some(1), Some(1), false),
            file("Show.S01E03.mkv", Some(1), Some(3), false),
            file("Show.S01E10.mkv", Some(1), Some(10), false),
            file("Show.S02E01.mkv", Some(2), Some(1), false),
            file("Show.S02E03E04.mkv", Some(2), Some(3), true),
        ];
        let gaps = episode_number_gaps(&files);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].season, 1);
        assert_eq!(gaps[0].missing_count, 7);
        assert_eq!(gaps[0].examples, vec![2, 4, 5, 6, 7, 8, 9]);

        let long_gap = vec![
            file("Show.S03E01.mkv", Some(3), Some(1), false),
            file("Show.S03E1000.mkv", Some(3), Some(1000), false),
        ];
        let long_gaps = episode_number_gaps(&long_gap);
        assert_eq!(long_gaps[0].missing_count, 998);
        assert_eq!(long_gaps[0].examples.len(), 8);
    }

    #[test]
    fn folder_scan_includes_filename_gap_advisories() {
        let root = scratch();
        fs::write(root.join("Show.S01E01.mkv"), b"fixture").unwrap();
        fs::write(root.join("Show.S01E03.mkv"), b"fixture").unwrap();

        let scan = scan_organizer_folder(root.to_string_lossy().into_owned()).unwrap();
        assert_eq!(scan.files.len(), 2);
        assert_eq!(scan.episode_gaps.len(), 1);
        assert_eq!(scan.episode_gaps[0].season, 1);
        assert_eq!(scan.episode_gaps[0].missing_count, 1);
        assert_eq!(scan.episode_gaps[0].examples, vec![2]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_preview_id_and_missing_confirmation_never_rename() {
        let root = scratch();
        let source = root.join("source.mkv");
        fs::write(&source, b"source bytes").unwrap();
        let make_plan = || {
            vec![input(
                &source,
                RenameTemplateKind::Movie,
                "renamed{ext}",
                &[("title", "renamed")],
            )]
        };
        let preview =
            preview_rename_batch(root.to_string_lossy().into_owned(), make_plan()).unwrap();
        assert!(apply_rename_batch(
            root.to_string_lossy().into_owned(),
            make_plan(),
            preview.preview_id.clone(),
            false
        )
        .is_err());
        assert!(apply_rename_batch(
            root.to_string_lossy().into_owned(),
            make_plan(),
            "stale-preview".into(),
            true
        )
        .is_err());
        assert_eq!(fs::read(&source).unwrap(), b"source bytes");
        assert!(!root.join("renamed.mkv").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
