//! Builds and previews a rename plan: companions, collisions, directory moves and fingerprints.

use super::*;

fn normalize_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn is_inside(root: &Path, path: &Path) -> bool {
    path != root && path.starts_with(root)
}

pub(super) fn fingerprint(metadata: &fs::Metadata) -> (u64, u128, bool) {
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

// Compatibility delegate retained; canonical callers/tests use the pattern-aware path.
#[allow(dead_code)]
pub(super) fn build_plan(
    root: &str,
    inputs: Vec<RenameTemplateInput>,
) -> Result<InternalPlan, String> {
    build_plan_with_patterns(root, inputs, &[])
}

pub(super) fn build_plan_with_patterns(
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

// Compatibility delegate retained; canonical callers/tests use the pattern-aware path.
#[allow(dead_code)]
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
