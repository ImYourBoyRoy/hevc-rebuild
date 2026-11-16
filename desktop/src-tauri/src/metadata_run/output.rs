//! Output paths, destination checks, work directories and the storage preflight for metadata-only runs.

use super::*;

pub(super) const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;

pub(super) static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// Pick the destination; mirrors `rebuild_run::resolved_output_path`.
pub(super) fn resolved_output_path(
    source: &Path,
    options: &MetadataRunOptions,
) -> Result<(PathBuf, bool), String> {
    let per_file_dir = options.file_output_dirs.get(source);
    let Some(base) = per_file_dir.or(options.output_dir.as_ref()) else {
        return Ok((source.with_extension("mkv"), false));
    };
    let file_name = source
        .file_name()
        .map(PathBuf::from)
        .ok_or("The source has no file name.")?
        .with_extension("mkv");
    let relative = if per_file_dir.is_some() || !options.root.is_dir() {
        file_name
    } else {
        source
            .strip_prefix(&options.root)
            .map_err(|_| {
                format!(
                    "Source file is outside the selected input folder: {}",
                    source.display()
                )
            })?
            .with_extension("mkv")
    };
    let mut target = base.join(relative);
    if canonicalize_allow_missing(&target)? == source {
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        target = target.with_file_name(format!("{stem}.hevc-rebuilt.mkv"));
    }
    let resolved = canonicalize_allow_missing(&target)?;
    let base = base
        .canonicalize()
        .map_err(|error| format!("The output folder is unavailable: {error}"))?;
    if !resolved
        .parent()
        .is_some_and(|parent| parent.starts_with(&base))
    {
        return Err(format!(
            "Output folder resolves outside the selected destination: {}",
            target.parent().unwrap_or(&target).display()
        ));
    }
    Ok((resolved, true))
}

/// A new MKV path for metadata-only work; never resolves to the source.
pub(super) fn metadata_output_path(
    source: &Path,
    options: &MetadataRunOptions,
) -> Result<PathBuf, String> {
    let (base, _) = resolved_output_path(source, options)?;
    if base == source {
        let stem = source.file_stem().unwrap_or_default().to_string_lossy();
        return Ok(source.with_file_name(format!("{stem}.metadata.mkv")));
    }
    Ok(base)
}

pub(super) fn validate_output_destinations(
    files: &[PathBuf],
    options: &MetadataRunOptions,
) -> Result<(), String> {
    let mut planned: BTreeMap<PathBuf, &Path> = BTreeMap::new();
    for source in files {
        let output = metadata_output_path(source, options)?;
        if output.exists() {
            return Err(format!(
                "Output already exists and will not be overwritten: {}",
                output.display()
            ));
        }
        if let Some(previous) = planned.insert(output.clone(), source) {
            if previous != source.as_path() {
                return Err(format!(
                    "Two selected media files would write to the same output: {} and {} -> {}",
                    previous.display(),
                    source.display(),
                    output.display()
                ));
            }
        }
    }
    Ok(())
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

/// Copy-only budget: enough room to start plus the configured reserve.
pub(super) fn storage_preflight(
    source_size: u64,
    free_bytes: u64,
    min_free_disk_gib: f64,
) -> Result<(), String> {
    let reserve = (min_free_disk_gib.max(0.0) * 1024.0 * 1024.0 * 1024.0) as u64;
    let minimum_temp = (16 * MIB).max(source_size / 20);
    let required = reserve.saturating_add(minimum_temp);
    if free_bytes < required {
        return Err(format!(
            "Insufficient free space even to start safely: {} free, need at least {} ({} reserve + {} working headroom).",
            human_bytes(free_bytes),
            human_bytes(required),
            human_bytes(reserve),
            human_bytes(minimum_temp)
        ));
    }
    Ok(())
}

pub(super) fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:x}{:x}{:x}",
        std::process::id(),
        nanos,
        UNIQUE.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn make_work_dir(source: &Path, configured: &str) -> Result<PathBuf, String> {
    let root = crate::media_work_directory(source, configured)?;
    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        let candidate = root.join(format!("hevc-metadata-only-{}", unique_token()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create a metadata work folder: {error}")),
        }
    }
    Err("Could not allocate a unique metadata work folder.".into())
}

// ----------------------------------------------------------------- profiling
