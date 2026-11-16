//! Bounded, read-only discovery of candidate media files and the filename include/exclude filters.

use super::*;

const MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm",
];

const EXCLUDED_DIRS: &[&str] = &[
    ".hevc-rebuild",
    ".hevc-rebuild-work",
    ".hevc-rebuild-originals",
    ".git",
    "node_modules",
    "youtube uploads",
];

pub(super) const MAX_DISCOVERED_FILES: usize = 10_000;

#[derive(Default)]
pub(super) struct Discovery {
    pub(super) files: Vec<PathBuf>,
    pub(super) warnings: Vec<String>,
    pub(super) limit_reached: bool,
}

pub(crate) fn validate_filename_patterns(patterns: &[String], label: &str) -> Result<(), String> {
    if patterns.len() > 100 {
        return Err(format!("Filename {label} list is limited to 100 patterns."));
    }
    let mut seen = BTreeSet::new();
    for (index, pattern) in patterns.iter().enumerate() {
        if pattern.is_empty() || pattern.trim() != pattern {
            return Err(format!(
                "Filename {label} pattern {} must not be empty or padded with spaces.",
                index + 1
            ));
        }
        if pattern.len() > 128 {
            return Err(format!(
                "Filename {label} pattern {} is limited to 128 UTF-8 bytes.",
                index + 1
            ));
        }
        if pattern
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        {
            return Err(format!(
                "Filename {label} patterns cannot contain control characters or path separators."
            ));
        }
        if pattern
            .chars()
            .all(|character| matches!(character, '*' | '?'))
        {
            return Err(format!(
                "Filename {label} pattern {} must include at least one filename character.",
                index + 1
            ));
        }
        if !seen.insert(pattern.to_lowercase()) {
            return Err(format!(
                "Filename {label} patterns cannot contain case-insensitive duplicates."
            ));
        }
    }
    Ok(())
}

pub(crate) fn filename_matches_filters(name: &str, include: &[String], exclude: &[String]) -> bool {
    let included = include.is_empty()
        || include
            .iter()
            .any(|pattern| filename_glob_matches(name, pattern));
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    included
        && !exclude.iter().any(|pattern| {
            filename_glob_matches(name, pattern) || filename_glob_matches(stem, pattern)
        })
}

fn filename_glob_matches(name: &str, pattern: &str) -> bool {
    let name = name
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let pattern = pattern
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<Vec<_>>();
    let (mut name_index, mut pattern_index) = (0, 0);
    let mut last_star = None;
    let mut retry_name_index = 0;
    while name_index < name.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == name[name_index])
        {
            name_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            last_star = Some(pattern_index);
            pattern_index += 1;
            retry_name_index = name_index;
        } else if let Some(star) = last_star {
            retry_name_index += 1;
            name_index = retry_name_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

/// Rust-owned discovery entry point. The same bounded walk powers GUI/MCP/JSON
/// analysis, so every interface discovers files identically.
pub(crate) fn discover_cli_mode() -> i32 {
    const MAX_REQUEST_BYTES: u64 = 64 * 1024;
    const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
    let mut input = Vec::new();
    if let Err(error) = std::io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input)
    {
        eprintln!("Could not read the native discovery request: {error}");
        return 2;
    }
    if input.len() as u64 > MAX_REQUEST_BYTES {
        eprintln!("Native discovery request exceeds 64 KiB.");
        return 2;
    }
    let request: Value = match serde_json::from_slice(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("Native discovery request is invalid JSON: {error}");
            return 2;
        }
    };
    let result = (|| {
        let raw_root = request
            .get("root")
            .and_then(Value::as_str)
            .filter(|root| !root.is_empty() && root.len() <= 8192)
            .ok_or("Discovery request must contain a path no longer than 8192 bytes.")?;
        let recursive = request
            .get("recursive")
            .and_then(Value::as_bool)
            .ok_or("Discovery request must specify whether to search recursively.")?;
        let minimum_bytes = request
            .get("minimum_size_bytes")
            .and_then(Value::as_u64)
            .ok_or("Discovery request must contain a nonnegative minimum file size.")?;
        let excluded_names = request
            .get("filename_exclude_patterns")
            .and_then(Value::as_array)
            .map(|patterns| {
                patterns
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        validate_filename_patterns(&excluded_names, "exclude")?;
        let root = expand_home(Path::new(raw_root))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !root.is_file() && !root.is_dir() {
            return Err("Media input must be a regular file or folder.".to_owned());
        }
        let discovery = if root.is_file() {
            discover_file(&root, minimum_bytes)
        } else {
            discover_folder(&root, recursive, minimum_bytes, &excluded_names, None)
        };
        let payload = json!({
            "files": discovery.files.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>(),
            "warnings": discovery.warnings,
            "limit_reached": discovery.limit_reached,
        });
        let output = serde_json::to_string(&payload)
            .map_err(|error| format!("Could not format the native discovery result: {error}"))?;
        if output.len() > MAX_RESPONSE_BYTES {
            return Err("Native discovery result exceeds the 64 MiB response limit; narrow the selected folder.".to_owned());
        }
        Ok(output)
    })();
    match result {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub(super) fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(text.strip_prefix("~/").unwrap_or(""));
        }
    }
    path.to_path_buf()
}

fn excluded_directory(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXCLUDED_DIRS.contains(&lower.as_str())
}

fn is_season_directory(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower
        .strip_prefix("season")
        .map(str::trim_start)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|character| character.is_ascii_digit())
}

fn is_candidate(path: &Path, minimum_bytes: u64) -> bool {
    if path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.starts_with("Test_") || name.contains(".hevc-rebuild.")
    }) {
        return false;
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    MEDIA_EXTENSIONS.contains(&extension.as_str())
        && fs::metadata(path)
            .map(|metadata| metadata.is_file() && metadata.len() >= minimum_bytes)
            .unwrap_or(false)
}

/// Probe-free discovery for native mutating runs: same candidate rules and
/// filename filters as `scan_media`, but returns canonical paths only.
pub(crate) fn discover_media_paths(
    root: &Path,
    recursive: bool,
    minimum_size_mib: u32,
    include: &[String],
    exclude: &[String],
    stop_file: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    if minimum_size_mib > 4096 {
        return Err("Minimum input size must be 4096 MiB or less.".into());
    }
    validate_filename_patterns(include, "include")?;
    validate_filename_patterns(exclude, "exclude")?;
    let root = expand_home(root)
        .canonicalize()
        .map_err(|error| format!("Media input is unavailable: {error}"))?;
    let minimum_bytes = u64::from(minimum_size_mib) * 1024 * 1024;
    let mut discovery = if root.is_file() {
        discover_file(&root, minimum_bytes)
    } else if root.is_dir() {
        discover_folder(&root, recursive, minimum_bytes, exclude, stop_file)
    } else {
        return Err("Media input must be a regular file or folder.".into());
    };
    discovery.files.retain(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        filename_matches_filters(&name, include, exclude)
    });
    Ok(discovery.files)
}

pub(super) fn discover_file(path: &Path, minimum_bytes: u64) -> Discovery {
    let mut result = Discovery::default();
    if is_candidate(path, minimum_bytes) {
        result.files.push(path.to_path_buf());
    }
    result
}

fn read_directory(path: &Path, result: &mut Discovery) -> Vec<PathBuf> {
    let mut entries = match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry.path()),
                Err(error) => {
                    result.warnings.push(format!(
                        "Could not read an item under {}: {error}",
                        path.display()
                    ));
                    None
                }
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            result
                .warnings
                .push(format!("Could not read folder {}: {error}", path.display()));
            Vec::new()
        }
    };
    entries.sort_by_key(|entry| {
        entry
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    entries
}

fn add_candidate(
    path: &Path,
    minimum_bytes: u64,
    seen: &mut BTreeSet<PathBuf>,
    result: &mut Discovery,
) {
    if !is_candidate(path, minimum_bytes) {
        return;
    }
    if result.files.len() >= MAX_DISCOVERED_FILES {
        result.limit_reached = true;
        return;
    }
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if seen.insert(key) {
        result.files.push(path.to_path_buf());
    }
}

pub(super) fn discover_folder(
    root: &Path,
    recursive: bool,
    minimum_bytes: u64,
    excluded_names: &[String],
    stop_file: Option<&Path>,
) -> Discovery {
    let mut result = Discovery::default();
    let mut seen = BTreeSet::new();
    if recursive {
        let mut stack = vec![root.to_path_buf()];
        let mut visited = BTreeSet::new();
        while let Some(directory) = stack.pop() {
            if scan_stop_requested(stop_file) {
                result
                    .warnings
                    .push("Folder listing stopped before every path was collected.".into());
                break;
            }
            let canonical = directory
                .canonicalize()
                .unwrap_or_else(|_| directory.clone());
            if !visited.insert(canonical) {
                continue;
            }
            let mut children = Vec::new();
            for path in read_directory(&directory, &mut result) {
                let link = fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false);
                if path.is_dir() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if !link
                        && !excluded_directory(&name)
                        && !excluded_names
                            .iter()
                            .any(|pattern| filename_glob_matches(&name, pattern))
                    {
                        children.push(path);
                    }
                } else {
                    add_candidate(&path, minimum_bytes, &mut seen, &mut result);
                }
                if result.limit_reached {
                    break;
                }
            }
            children.reverse();
            stack.extend(children);
            if result.limit_reached {
                break;
            }
        }
        result
            .files
            .sort_by_key(|path| path.to_string_lossy().to_lowercase());
    } else {
        let mut seasons = Vec::new();
        for path in read_directory(root, &mut result) {
            if scan_stop_requested(stop_file) {
                result
                    .warnings
                    .push("Folder listing stopped before every path was collected.".into());
                break;
            }
            if path.is_dir() {
                let link = fs::symlink_metadata(&path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false);
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if link
                    || excluded_directory(&name)
                    || excluded_names
                        .iter()
                        .any(|pattern| filename_glob_matches(&name, pattern))
                {
                    continue;
                }
                if is_season_directory(&name) {
                    seasons.push(path);
                } else {
                    for child in read_directory(&path, &mut result) {
                        let child_link = fs::symlink_metadata(&child)
                            .map(|metadata| metadata.file_type().is_symlink())
                            .unwrap_or(false);
                        if child.is_dir()
                            && !child_link
                            && !excluded_names.iter().any(|pattern| {
                                filename_glob_matches(
                                    &child.file_name().unwrap_or_default().to_string_lossy(),
                                    pattern,
                                )
                            })
                            && is_season_directory(
                                &child.file_name().unwrap_or_default().to_string_lossy(),
                            )
                        {
                            seasons.push(child);
                        }
                    }
                }
            } else {
                add_candidate(&path, minimum_bytes, &mut seen, &mut result);
            }
            if result.limit_reached {
                break;
            }
        }
        seasons.sort_by_key(|path| path.to_string_lossy().to_lowercase());
        seasons.dedup();
        for season in seasons {
            if scan_stop_requested(stop_file) {
                if !result
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("Folder listing stopped"))
                {
                    result
                        .warnings
                        .push("Folder listing stopped before every path was collected.".into());
                }
                break;
            }
            for path in read_directory(&season, &mut result) {
                if !path.is_dir() {
                    add_candidate(&path, minimum_bytes, &mut seen, &mut result);
                }
                if result.limit_reached {
                    break;
                }
            }
            if result.limit_reached {
                break;
            }
        }
    }
    result
}
