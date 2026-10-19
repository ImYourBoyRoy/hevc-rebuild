//! Validated engine request arguments shared by GUI and MCP callers.
//! Filesystem checks remain explicit at the request boundary.

use crate::{library_assets, media_scan};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub(crate) struct ToolPaths {
    /// Encoder/muxer executable. A matching sibling FFprobe is detected when unset.
    pub(crate) ffmpeg: Option<String>,
    /// Optional explicit stream-analysis executable.
    pub(crate) ffprobe: Option<String>,
    /// Optional explicit MediaInfo executable.
    pub(crate) mediainfo: Option<String>,
    /// Optional MKVToolNix editor executable.
    pub(crate) mkvpropedit: Option<String>,
    /// Optional MKVToolNix merger executable.
    pub(crate) mkvmerge: Option<String>,
    /// Optional MKVToolNix extractor executable.
    pub(crate) mkvextract: Option<String>,
}

pub(crate) fn append_tool_path_arguments(
    args: &mut Vec<String>,
    mut paths: ToolPaths,
    legacy_ffmpeg_path: Option<String>,
) -> Result<(), String> {
    if paths.ffmpeg.is_none() {
        paths.ffmpeg = legacy_ffmpeg_path;
    }
    for (name, path) in [
        ("ffmpeg", paths.ffmpeg),
        ("ffprobe", paths.ffprobe),
        ("mediainfo", paths.mediainfo),
        ("mkvpropedit", paths.mkvpropedit),
        ("mkvmerge", paths.mkvmerge),
        ("mkvextract", paths.mkvextract),
    ] {
        let Some(path) = path.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        if path.len() > 8192 {
            return Err(format!("The selected {name} path is too long."));
        }
        let binary = PathBuf::from(&path)
            .canonicalize()
            .map_err(|error| format!("Selected {name} executable is unavailable: {error}"))?;
        if !binary.is_file() {
            return Err(format!("Selected {name} path is not a file."));
        }
        args.extend([
            format!("--{}-path", name.replace('_', "-")),
            binary.to_string_lossy().into_owned(),
        ]);
    }
    Ok(())
}

pub(crate) fn append_filename_filter_arguments(
    args: &mut Vec<String>,
    include: &[String],
    exclude: &[String],
) -> Result<(), String> {
    media_scan::validate_filename_patterns(include, "include")?;
    media_scan::validate_filename_patterns(exclude, "exclude")?;
    for pattern in include {
        args.extend(["--include-filename-pattern".into(), pattern.clone()]);
    }
    for pattern in exclude {
        args.extend(["--exclude-filename-pattern".into(), pattern.clone()]);
    }
    Ok(())
}

pub(crate) fn append_selected_file_arguments(
    args: &mut Vec<String>,
    root: &Path,
    selected_files: &[String],
) -> Result<(), String> {
    if selected_files.len() > 10_000 {
        return Err("A job can select at most 10,000 media files.".into());
    }
    if selected_files.is_empty() {
        return Ok(());
    }
    let mut unique = BTreeSet::new();
    for raw_path in selected_files {
        if raw_path.len() > 8192 || !Path::new(raw_path).is_absolute() {
            return Err("Selected media paths must be absolute paths.".into());
        }
        let path = PathBuf::from(raw_path)
            .canonicalize()
            .map_err(|error| format!("A selected media file is no longer available: {error}"))?;
        if !path.is_file()
            || (root.is_dir() && !path.starts_with(root))
            || (root.is_file() && path.as_path() != root)
        {
            return Err("Every selected media file must belong to the analyzed input.".into());
        }
        if !unique.insert(path.clone()) {
            return Err("Selected media paths must be unique.".into());
        }
        args.extend([
            "--selected-file".into(),
            path.to_string_lossy().into_owned(),
        ]);
    }
    Ok(())
}

pub(crate) fn rebuild_output_arguments(
    root: &Path,
    output_dir: Option<&str>,
    file_output_dirs: BTreeMap<String, String>,
) -> Result<(Vec<String>, Option<PathBuf>), String> {
    if file_output_dirs.len() > 512 {
        return Err("Per-file output folders are limited to 512 media files.".into());
    }
    let mut args = Vec::new();
    let mut resource_path = None;
    if let Some(raw) = output_dir.filter(|value| !value.trim().is_empty()) {
        if raw.len() > 8192 || !Path::new(raw).is_absolute() {
            return Err("The default rebuild output folder must be an absolute path of at most 8192 characters.".into());
        }
        let folder = PathBuf::from(raw)
            .canonicalize()
            .map_err(|error| format!("Default rebuild output folder is unavailable: {error}"))?;
        if !folder.is_dir() {
            return Err("Default rebuild output folder is not an existing folder.".into());
        }
        resource_path = Some(folder.clone());
        args.extend(["--output-dir".into(), folder.to_string_lossy().into_owned()]);
    }
    if file_output_dirs.is_empty() {
        return Ok((args, resource_path));
    }
    let input_root = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let mut canonical = BTreeMap::new();
    for (raw_source, raw_destination) in file_output_dirs {
        if raw_source.len() > 8192
            || raw_destination.len() > 8192
            || !Path::new(&raw_source).is_absolute()
            || !Path::new(&raw_destination).is_absolute()
        {
            return Err("Per-file output source and folder paths must be absolute and no longer than 8192 characters.".into());
        }
        let source = PathBuf::from(&raw_source)
            .canonicalize()
            .map_err(|error| format!("Per-file output source is unavailable: {error}"))?;
        if !source.is_file()
            || !source.starts_with(&input_root)
            || (root.is_file() && source != root)
        {
            return Err("Per-file output sources must belong to the analyzed input.".into());
        }
        let destination = PathBuf::from(raw_destination)
            .canonicalize()
            .map_err(|error| format!("Per-file output folder is unavailable: {error}"))?;
        if !destination.is_dir() {
            return Err("Each per-file output destination must be an existing folder.".into());
        }
        if canonical
            .insert(source.clone(), destination.clone())
            .is_some()
        {
            return Err(
                "Per-file output settings contain duplicate canonical source paths.".into(),
            );
        }
        if resource_path.is_none() {
            resource_path = Some(destination);
        }
    }
    let serialized = serde_json::to_string(
        &canonical
            .into_iter()
            .map(|(source, destination)| {
                (
                    source.to_string_lossy().into_owned(),
                    destination.to_string_lossy().into_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    )
    .map_err(|error| format!("Could not encode per-file output folders: {error}"))?;
    if serialized.len() > 65536 {
        return Err("Per-file output folders exceed the 64 KiB limit.".into());
    }
    args.extend(["--file-output-dirs-json".into(), serialized]);
    Ok((args, resource_path))
}

pub(crate) fn stream_override_arguments(
    root: &Path,
    overrides: BTreeMap<String, Vec<u16>>,
) -> Result<Vec<String>, String> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }
    if overrides.len() > 512 {
        return Err("Per-file stream selections are limited to 512 media files.".into());
    }
    let mut canonical_overrides = BTreeMap::new();
    for (raw_path, indices) in overrides {
        if !Path::new(&raw_path).is_absolute() {
            return Err("Per-file stream selection paths must be absolute.".into());
        }
        if indices.is_empty() || indices.len() > 1024 {
            return Err(
                "Each media file must select at least one stream and no more than 1024 streams."
                    .into(),
            );
        }
        let unique: BTreeSet<_> = indices.iter().copied().collect();
        if unique.len() != indices.len() {
            return Err(
                "Per-file stream selections must contain unique FFprobe stream indices.".into(),
            );
        }
        let selected = PathBuf::from(raw_path)
            .canonicalize()
            .map_err(|error| format!("Selected stream source is unavailable: {error}"))?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(root))
            || (root.is_file() && selected != root)
        {
            return Err(
                "Per-file stream selections must refer to media inside the selected input.".into(),
            );
        }
        if canonical_overrides
            .insert(selected.to_string_lossy().into_owned(), indices)
            .is_some()
        {
            return Err(
                "A media file was included more than once in the stream selections.".into(),
            );
        }
    }
    let serialized = serde_json::to_string(&canonical_overrides)
        .map_err(|error| format!("Could not encode per-file stream selections: {error}"))?;
    if serialized.len() > 65536 {
        return Err("Per-file stream selections exceed the 64 KiB limit.".into());
    }
    Ok(vec!["--file-stream-overrides-json".into(), serialized])
}

pub(crate) fn poster_override_arguments(
    root: &Path,
    overrides: BTreeMap<String, String>,
    artwork_cache_root: &Path,
) -> Result<Vec<String>, String> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }
    if overrides.len() > 512 {
        return Err("Per-file poster selections are limited to 512 media files.".into());
    }
    let input_root = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let canonical_cache_root = artwork_cache_root.canonicalize().ok();
    let mut uses_cached_artwork = false;
    let mut canonical_overrides = BTreeMap::new();
    for (raw_media, raw_poster) in overrides {
        if raw_media.len() > 8192
            || raw_poster.len() > 8192
            || !Path::new(&raw_media).is_absolute()
            || !Path::new(&raw_poster).is_absolute()
        {
            return Err("Per-file media and poster paths must be absolute and no longer than 8192 characters.".into());
        }
        let media = PathBuf::from(raw_media)
            .canonicalize()
            .map_err(|error| format!("Selected poster's media file is unavailable: {error}"))?;
        if !media.is_file() || !media.starts_with(&input_root) || (root.is_file() && media != root)
        {
            return Err("Poster selections must refer to media inside the analyzed input.".into());
        }
        let poster = PathBuf::from(raw_poster)
            .canonicalize()
            .map_err(|error| format!("Selected poster image is unavailable: {error}"))?;
        let inside_input = poster.starts_with(&input_root);
        let inside_app_cache = canonical_cache_root
            .as_ref()
            .is_some_and(|cache| poster.starts_with(cache));
        if (!inside_input && !inside_app_cache) || !library_assets::valid_image_file(&poster) {
            return Err("Selected poster images must be valid JPEG, PNG, or WebP files inside the analyzed input or the private app artwork cache.".into());
        }
        uses_cached_artwork |= inside_app_cache;
        if canonical_overrides
            .insert(
                media.to_string_lossy().into_owned(),
                poster.to_string_lossy().into_owned(),
            )
            .is_some()
        {
            return Err(
                "A media file was included more than once in the poster selections.".into(),
            );
        }
    }
    let serialized = serde_json::to_string(&canonical_overrides)
        .map_err(|error| format!("Could not encode per-file poster selections: {error}"))?;
    if serialized.len() > 65536 {
        return Err("Per-file poster selections exceed the 64 KiB limit.".into());
    }
    let mut args = Vec::with_capacity(4);
    if uses_cached_artwork {
        let cache = canonical_cache_root
            .ok_or("The private artwork cache is unavailable for this selected poster.")?;
        args.extend([
            "--allowed-poster-cache-root".into(),
            cache.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["--file-poster-overrides-json".into(), serialized]);
    Ok(args)
}

pub(crate) fn metadata_override_arguments(
    root: &Path,
    overrides: BTreeMap<String, Value>,
) -> Result<Vec<String>, String> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }
    if overrides.len() > 512 {
        return Err("Reviewed metadata is limited to 512 media files per run.".into());
    }
    let input_root = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    let mut canonical = BTreeMap::new();
    for (raw_path, metadata) in overrides {
        if raw_path.len() > 8192 || !Path::new(&raw_path).is_absolute() {
            return Err(
                "Reviewed metadata paths must be absolute and no longer than 8192 characters."
                    .into(),
            );
        }
        let path = PathBuf::from(raw_path)
            .canonicalize()
            .map_err(|error| format!("Reviewed metadata source is unavailable: {error}"))?;
        if !path.is_file() || !path.starts_with(&input_root) || (root.is_file() && path != root) {
            return Err("Reviewed metadata must refer to media inside the analyzed input.".into());
        }
        if !metadata.is_object() || metadata.as_object().is_some_and(|fields| fields.is_empty()) {
            return Err("Each reviewed metadata value must be a non-empty object.".into());
        }
        if canonical
            .insert(path.to_string_lossy().into_owned(), metadata)
            .is_some()
        {
            return Err("A media file was included more than once in reviewed metadata.".into());
        }
    }
    let serialized = serde_json::to_string(&canonical)
        .map_err(|error| format!("Could not encode reviewed metadata: {error}"))?;
    if serialized.len() > 65536 {
        return Err("Reviewed metadata overrides exceed the 64 KiB limit.".into());
    }
    Ok(vec!["--file-metadata-overrides-json".into(), serialized])
}

pub(crate) fn youtube_hdr10plus_loss_arguments(
    root: &Path,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    if paths.len() > 512 {
        return Err("YouTube HDR10+ loss consent is limited to 512 media files.".into());
    }
    let mut selected_paths = BTreeSet::new();
    let mut args = Vec::new();
    for raw_path in paths {
        if raw_path.len() > 8192 {
            return Err(
                "YouTube HDR10+ loss consent paths are limited to 8192 characters each.".into(),
            );
        }
        if !Path::new(&raw_path).is_absolute() {
            return Err("YouTube HDR10+ loss consent paths must be absolute.".into());
        }
        let selected = PathBuf::from(raw_path)
            .canonicalize()
            .map_err(|error| format!("Selected HDR10+ source is unavailable: {error}"))?;
        if !selected.is_file()
            || (root.is_dir() && !selected.starts_with(root))
            || (root.is_file() && selected != root)
        {
            return Err(
                "YouTube HDR10+ loss consent paths must refer to files inside the selected input."
                    .into(),
            );
        }
        let canonical = selected.to_string_lossy().into_owned();
        if !selected_paths.insert(canonical.clone()) {
            return Err("YouTube HDR10+ loss consent can be supplied only once per file.".into());
        }
        args.extend(["--allow-youtube-hdr10plus-loss".into(), canonical]);
    }
    if args.iter().map(String::len).sum::<usize>() > 65536 {
        return Err("YouTube HDR10+ loss consent exceeds the 64 KiB path limit.".into());
    }
    Ok(args)
}
