//! Read-only output planning shared by the desktop UI, JSON CLI, and MCP.
//!
//! A multi-source GUI queue is executed as sequential single-root engine runs.
//! This planner checks the complete selected set first so a collision in a
//! later source is visible before the first source starts. The engine still
//! performs its own per-run preflight immediately before writing.

use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SOURCES: usize = 500;
const MAX_FILES: usize = 25_000;
const MAX_FILE_OUTPUT_DIRS: usize = 512;
const MAX_PATH_BYTES: usize = 8192;
const MAX_PREVIEW_TARGETS: usize = 250;
const MAX_PREVIEW_COLLISIONS: usize = 1000;
const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OutputPreviewSource {
    /// One scanned queue source: either a folder or a single media file.
    pub(crate) root: String,
    /// Absolute media files selected from this source.
    pub(crate) files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OutputPreviewRequest {
    /// One of metadata, test, or full.
    pub(crate) mode: String,
    pub(crate) sources: Vec<OutputPreviewSource>,
    /// Existing default output folder, if configured.
    pub(crate) output_dir: Option<String>,
    /// Absolute source path to existing per-file output folder.
    #[serde(default)]
    pub(crate) file_output_dirs: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlannedOutputTarget {
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) destination: String,
    pub(crate) replaces_source: bool,
    pub(crate) note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputCollision {
    pub(crate) kind: String,
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) conflicts_with: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputPreview {
    pub(crate) mode: String,
    pub(crate) source_count: usize,
    pub(crate) file_count: usize,
    pub(crate) eligible: bool,
    pub(crate) targets: Vec<PlannedOutputTarget>,
    pub(crate) omitted_target_count: usize,
    pub(crate) collisions: Vec<OutputCollision>,
    pub(crate) collision_count: usize,
    pub(crate) omitted_collision_count: usize,
    pub(crate) test_outputs_may_receive_unique_suffixes: bool,
}

pub(crate) fn preview(request: OutputPreviewRequest) -> Result<OutputPreview, String> {
    if !matches!(request.mode.as_str(), "metadata" | "test" | "full") {
        return Err("Output preview mode must be metadata, test, or full.".into());
    }
    if request.sources.is_empty() || request.sources.len() > MAX_SOURCES {
        return Err(format!(
            "Output preview requires 1 to {MAX_SOURCES} queue sources."
        ));
    }
    if request.file_output_dirs.len() > MAX_FILE_OUTPUT_DIRS {
        return Err(format!(
            "Per-file output folders are limited to {MAX_FILE_OUTPUT_DIRS} media files."
        ));
    }

    let default_output_dir = request
        .output_dir
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| canonical_existing_dir(value, "Default output folder"))
        .transpose()?;
    let mut per_file_dirs = HashMap::new();
    for (raw_source, raw_dir) in request.file_output_dirs {
        let source = canonical_existing_path(&raw_source, "Per-file output source")?;
        let destination = canonical_existing_dir(&raw_dir, "Per-file output folder")?;
        if per_file_dirs.insert(source, destination).is_some() {
            return Err(
                "Per-file output settings contain duplicate canonical source paths.".into(),
            );
        }
    }

    let mut target_preview = Vec::new();
    let mut collision_preview = Vec::new();
    let mut collision_count = 0usize;
    let mut file_count = 0usize;
    let mut canonical_sources = HashMap::<PathBuf, String>::new();
    let mut canonical_targets = HashMap::<PathBuf, String>::new();

    for queue_source in &request.sources {
        if queue_source.files.is_empty() {
            return Err("Each queued source must contain at least one selected media file.".into());
        }
        let root = canonical_existing_path(&queue_source.root, "Queue source")?;
        let root_is_dir = root.is_dir();
        if !root_is_dir && !root.is_file() {
            return Err("A queue source must be an existing file or folder.".into());
        }

        for raw_source in &queue_source.files {
            file_count = file_count.saturating_add(1);
            if file_count > MAX_FILES {
                return Err(format!(
                    "Output preview is limited to {MAX_FILES} selected files."
                ));
            }
            let source = canonical_existing_path(raw_source, "Selected media file")?;
            if !source.is_file()
                || (root_is_dir && !source.starts_with(&root))
                || (!root_is_dir && source != root)
            {
                return Err("Every selected media file must belong to its queued source.".into());
            }
            let source_display = source.to_string_lossy().into_owned();
            if canonical_sources
                .insert(source.clone(), source_display.clone())
                .is_some()
            {
                return Err("Selected media paths must be unique across the queue.".into());
            }

            let per_file_dir = per_file_dirs.get(&source);
            let output_dir = per_file_dir.or(default_output_dir.as_ref());
            let (mut target, destination) = if let Some(base) = output_dir {
                let relative = if per_file_dir.is_some() || !root_is_dir {
                    mkv_path(
                        source
                            .file_name()
                            .ok_or("Selected media file has no filename.")?,
                    )
                } else {
                    let relative = source
                        .strip_prefix(&root)
                        .map_err(|_| "Selected media file is outside its queued source.")?;
                    mkv_path(relative)
                };
                let target = base.join(relative);
                (
                    target,
                    if per_file_dir.is_some() {
                        "per_file_folder"
                    } else {
                        "default_folder"
                    },
                )
            } else {
                (mkv_path(&source), "source_folder")
            };

            let is_separate_destination = output_dir.is_some();
            if is_separate_destination && canonicalize_allow_missing(&target)? == source {
                target = suffixed_target_path(&target, &source, ".hevc-rebuilt");
            }
            target = canonicalize_allow_missing(&target)?;
            if request.mode == "metadata" && target == source {
                target = suffixed_mkv_path(&source, ".metadata");
                target = canonicalize_allow_missing(&target)?;
            }
            if request.mode == "test" {
                target = test_path(&target);
                target = canonicalize_allow_missing(&target)?;
            }
            if let Some(base) = output_dir {
                if !target.starts_with(base) {
                    return Err(format!(
                        "Planned output resolves outside its destination folder: {}",
                        target.display()
                    ));
                }
            }

            if let Some(previous_source) =
                canonical_targets.insert(target.clone(), source_display.clone())
            {
                collision_count = collision_count.saturating_add(1);
                push_collision(
                    &mut collision_preview,
                    OutputCollision {
                        kind: "duplicate_target".into(),
                        source: source_display.clone(),
                        target: target.to_string_lossy().into_owned(),
                        conflicts_with: Some(previous_source),
                    },
                );
            }
            let replaces_source = request.mode == "full" && target == source;
            if target.exists() && !replaces_source && request.mode != "test" {
                collision_count = collision_count.saturating_add(1);
                push_collision(
                    &mut collision_preview,
                    OutputCollision {
                        kind: "existing_output".into(),
                        source: source_display.clone(),
                        target: target.to_string_lossy().into_owned(),
                        conflicts_with: None,
                    },
                );
            }

            if target_preview.len() < MAX_PREVIEW_TARGETS {
                let note = (request.mode == "test" && target.exists()).then(|| {
                    "This short-test name already exists; the engine will choose a unique timestamped name.".into()
                });
                target_preview.push(PlannedOutputTarget {
                    source: source_display,
                    target: target.to_string_lossy().into_owned(),
                    destination: destination.into(),
                    replaces_source,
                    note,
                });
            }
        }
    }

    if per_file_dirs
        .keys()
        .any(|source| !canonical_sources.contains_key(source))
    {
        return Err(
            "Every per-file output folder must refer to a selected media file in this queue."
                .into(),
        );
    }

    let omitted_collision_count = collision_count.saturating_sub(collision_preview.len());
    let test_outputs_may_receive_unique_suffixes = request.mode == "test";
    Ok(OutputPreview {
        mode: request.mode,
        source_count: request.sources.len(),
        file_count,
        eligible: collision_count == 0,
        targets: target_preview,
        omitted_target_count: file_count.saturating_sub(MAX_PREVIEW_TARGETS),
        collisions: collision_preview,
        collision_count,
        omitted_collision_count,
        test_outputs_may_receive_unique_suffixes,
    })
}

fn push_collision(collisions: &mut Vec<OutputCollision>, collision: OutputCollision) {
    if collisions.len() < MAX_PREVIEW_COLLISIONS {
        collisions.push(collision);
    }
}

fn canonical_existing_path(raw: &str, label: &str) -> Result<PathBuf, String> {
    if raw.len() > MAX_PATH_BYTES || !Path::new(raw).is_absolute() {
        return Err(format!(
            "{label} must be an absolute path of at most {MAX_PATH_BYTES} bytes."
        ));
    }
    Path::new(raw)
        .canonicalize()
        .map_err(|error| format!("{label} is unavailable: {error}"))
}

fn canonical_existing_dir(raw: &str, label: &str) -> Result<PathBuf, String> {
    let path = canonical_existing_path(raw, label)?;
    if !path.is_dir() {
        return Err(format!("{label} is not an existing folder."));
    }
    Ok(path)
}

fn mkv_path(path: impl AsRef<Path>) -> PathBuf {
    let mut target = path.as_ref().to_path_buf();
    target.set_extension("mkv");
    target
}

fn suffixed_mkv_path(source: &Path, suffix: &str) -> PathBuf {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    source.with_file_name(format!("{stem}{suffix}.mkv"))
}

fn suffixed_target_path(target: &Path, source: &Path, suffix: &str) -> PathBuf {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!("{stem}{suffix}.mkv"))
}

fn test_path(output: &Path) -> PathBuf {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    output.with_file_name(format!("Test_{stem}.mkv"))
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Planned output path is not absolute.".into());
    }
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(_) => {
                let name = ancestor
                    .file_name()
                    .ok_or("Could not resolve the planned output path.")?;
                missing.push(name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or("Could not resolve the planned output parent folder.")?;
            }
        }
    }
}

pub(crate) fn cli_mode(args: &[String]) -> i32 {
    if args.len() != 2 || args[0] != "--request" {
        eprintln!("Usage: hevc-rebuild-studio json output-plan --request REQUEST.json");
        return 2;
    }
    let path = PathBuf::from(&args[1]);
    let Ok(metadata) = path.metadata() else {
        eprintln!("Output-plan request file is unavailable.");
        return 2;
    };
    if !metadata.is_file() || metadata.len() > MAX_REQUEST_BYTES {
        eprintln!("Output-plan request must be a regular file no larger than 16 MiB.");
        return 2;
    }
    let request_bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Could not read output-plan request: {error}");
            return 2;
        }
    };
    let request: OutputPreviewRequest = match serde_json::from_slice(&request_bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Invalid output-plan JSON: {error}");
            return 2;
        }
    };
    match preview(request)
        .and_then(|result| serde_json::to_string(&result).map_err(|error| error.to_string()))
    {
        Ok(result) => {
            println!("{result}");
            0
        }
        Err(error) => {
            eprintln!("Output planning failed: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hevc-output-plan-{label}-{stamp}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn request(mode: &str, roots_and_files: &[(PathBuf, Vec<PathBuf>)]) -> OutputPreviewRequest {
        OutputPreviewRequest {
            mode: mode.into(),
            sources: roots_and_files
                .iter()
                .map(|(root, files)| OutputPreviewSource {
                    root: root.to_string_lossy().into_owned(),
                    files: files
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                })
                .collect(),
            output_dir: None,
            file_output_dirs: HashMap::new(),
        }
    }

    #[test]
    fn reports_collisions_across_separate_queue_sources() {
        let first = temp_root("first");
        let second = temp_root("second");
        let first_file = first.join("Movie.mp4");
        let second_file = second.join("Movie.avi");
        fs::write(&first_file, b"a").unwrap();
        fs::write(&second_file, b"b").unwrap();
        let shared = temp_root("shared-output");
        let mut request = request(
            "metadata",
            &[
                (first.clone(), vec![first_file]),
                (second.clone(), vec![second_file]),
            ],
        );
        request.output_dir = Some(shared.to_string_lossy().into_owned());

        let result = preview(request).unwrap();
        assert!(!result.eligible);
        assert_eq!(result.collision_count, 1);
        assert_eq!(result.collisions[0].kind, "duplicate_target");
        assert_eq!(
            result.collisions[0].target,
            shared.join("Movie.mkv").to_string_lossy()
        );

        fs::remove_dir_all(shared).unwrap();
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn metadata_mode_never_plans_over_the_mkv_source() {
        let root = temp_root("metadata-source");
        let source = root.join("Film.mkv");
        fs::write(&source, b"source").unwrap();

        let result = preview(request("metadata", &[(root.clone(), vec![source.clone()])])).unwrap();
        assert!(result.eligible);
        assert_eq!(
            result.targets[0].target,
            root.join("Film.metadata.mkv").to_string_lossy()
        );
        assert!(!result.targets[0].replaces_source);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_mode_allows_in_place_replacement_but_finds_sibling_conflicts() {
        let root = temp_root("in-place");
        let source = root.join("Film.mkv");
        fs::write(&source, b"source").unwrap();

        let result = preview(request("full", &[(root.clone(), vec![source.clone()])])).unwrap();
        assert!(result.eligible);
        assert!(result.targets[0].replaces_source);

        let mp4 = root.join("Film.mp4");
        fs::write(&mp4, b"another source").unwrap();
        let result = preview(request("full", &[(root.clone(), vec![source, mp4])])).unwrap();
        assert!(!result.eligible);
        assert!(result
            .collisions
            .iter()
            .any(|item| item.kind == "existing_output"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mirrors_relative_paths_and_applies_per_file_destinations() {
        let root = temp_root("mirror");
        let source = root.join("Season 01").join("Episode.mp4");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"source").unwrap();
        let output = temp_root("mirror-output");
        let one_file_output = temp_root("single-output");
        let mut per_file_request = request("full", &[(root.clone(), vec![source.clone()])]);
        per_file_request.output_dir = Some(output.to_string_lossy().into_owned());
        per_file_request.file_output_dirs.insert(
            source.to_string_lossy().into_owned(),
            one_file_output.to_string_lossy().into_owned(),
        );

        let result = preview(per_file_request).unwrap();
        assert!(result.eligible);
        assert_eq!(result.targets[0].destination, "per_file_folder");
        assert_eq!(
            result.targets[0].target,
            one_file_output.join("Episode.mkv").to_string_lossy()
        );

        let mut mirrored_request = request("full", &[(root.clone(), vec![source.clone()])]);
        mirrored_request.output_dir = Some(output.to_string_lossy().into_owned());
        let result = preview(mirrored_request).unwrap();
        assert_eq!(
            result.targets[0].target,
            output.join("Season 01/Episode.mkv").to_string_lossy()
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(one_file_output).unwrap();
    }

    #[test]
    fn short_test_preview_uses_test_prefix() {
        let root = temp_root("test");
        let source = root.join("Film.mp4");
        fs::write(&source, b"source").unwrap();
        let result = preview(request("test", &[(root.clone(), vec![source])])).unwrap();
        assert!(result.eligible);
        assert!(result.test_outputs_may_receive_unique_suffixes);
        assert_eq!(
            result.targets[0].target,
            root.join("Test_Film.mkv").to_string_lossy()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_output_subfolders_that_resolve_outside_the_selected_destination() {
        use std::os::unix::fs::symlink;

        let root = temp_root("symlink-source");
        let source = root.join("Season 01").join("Episode.mp4");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"source").unwrap();
        let output = temp_root("symlink-output");
        let outside = temp_root("symlink-outside");
        symlink(&outside, output.join("Season 01")).unwrap();
        let mut request = request("full", &[(root.clone(), vec![source])]);
        request.output_dir = Some(output.to_string_lossy().into_owned());

        let error = preview(request).unwrap_err();
        assert!(error.contains("resolves outside its destination folder"));

        fs::remove_file(output.join("Season 01")).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(output).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
