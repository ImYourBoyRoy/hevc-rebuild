//! Output and backup paths, destination checks, work directories and the storage preflight.

use super::*;

pub(super) const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;

pub(super) static UNIQUE: AtomicU64 = AtomicU64::new(0);

pub(super) fn resolved_output_path(
    source: &Path,
    options: &RebuildRunOptions,
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

fn test_output_path(output_base: &Path) -> Result<PathBuf, String> {
    let stem = output_base
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let parent = output_base.parent().unwrap_or_else(|| Path::new("."));
    let candidate = parent.join(format!("Test_{stem}.mkv"));
    if !candidate.exists() {
        return Ok(candidate);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("Test_{stem}_{timestamp}.mkv")
        } else {
            format!("Test_{stem}_{timestamp}_{suffix}.mkv")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not find a free short-test output name for {}.",
        output_base
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ))
}

pub(super) fn validate_output_destinations(
    files: &[PathBuf],
    options: &RebuildRunOptions,
) -> Result<(), String> {
    let mut planned: BTreeMap<PathBuf, &Path> = BTreeMap::new();
    for source in files {
        let (base, separate_output) = resolved_output_path(source, options)?;
        if !separate_output && !options.test_mode {
            preflight_backup_location(source, options.reprocess)?;
        }
        let output = if options.test_mode {
            test_output_path(&base)?
        } else {
            base
        };
        if output.exists() && canonicalize_allow_missing(&output)? != *source {
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

pub(super) fn backup_path_for(source: &Path, reprocess: bool) -> Result<PathBuf, String> {
    let parent = source.parent().ok_or("The source has no parent folder.")?;
    let name = source.file_name().ok_or("The source has no file name.")?;
    let old_dir = crate::backup_folder::backup_dir(parent);
    let candidate = old_dir.join(name);
    if !reprocess || !candidate.exists() {
        return Ok(candidate);
    }
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let extension = source
        .extension()
        .map(|value| format!(".{}", value.to_string_lossy()))
        .unwrap_or_default();
    for number in 1..10_000 {
        let candidate = old_dir.join(format!("{stem}.{number}{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Could not allocate a unique backup path in the originals folder.".into())
}

/// Reject an unusable backup location before any encoding starts.
/// `commit_output` enforces the same rule, but only after the encode and every
/// validation have finished, so a bad backup-folder entry would otherwise cost the whole
/// run's work and fail at the last step.
pub(super) fn preflight_backup_location(source: &Path, reprocess: bool) -> Result<(), String> {
    let parent = source.parent().ok_or("The source has no parent folder.")?;
    let old_dir = crate::backup_folder::backup_dir(parent);
    match fs::symlink_metadata(&old_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "The backup location {} must be a real folder, not a file or symlink; the original cannot be preserved there.",
                old_dir.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Could not inspect the backup location {}: {error}",
                old_dir.display()
            ));
        }
    }
    backup_path_for(source, reprocess).map(|_| ())
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

pub(super) fn storage_preflight(
    source_size: u64,
    free_bytes: u64,
    duration: f64,
    options: &RebuildRunOptions,
) -> Result<(u64, u64, bool), String> {
    let reserve = (options.min_free_disk_gib.max(0.0) * GIB as f64) as u64;
    let (estimated_temp, minimum_temp) = if options.test_mode {
        let fraction = if duration > 0.0 {
            (options.test_seconds as f64 / duration).clamp(0.0, 1.0)
        } else {
            0.05
        };
        (
            (512 * MIB).max((source_size as f64 * fraction.max(0.01) * 2.5) as u64),
            512 * MIB,
        )
    } else {
        (
            (2 * GIB).max((source_size as f64 * options.preflight_output_ratio) as u64),
            GIB.max((8 * GIB).min(source_size / 20)),
        )
    };
    let required = reserve.saturating_add(minimum_temp);
    let recommended = reserve.saturating_add(estimated_temp);
    if free_bytes < required {
        return Err(format!(
            "Insufficient free space even to start safely: {} free, need at least {} ({} reserve + {} working headroom).",
            human_bytes(free_bytes),
            human_bytes(required),
            human_bytes(reserve),
            human_bytes(minimum_temp)
        ));
    }
    Ok((estimated_temp, recommended, free_bytes >= recommended))
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

/// Drop the source-adjacent `.hevc-rebuild-work` folder once the last per-run
/// work folder inside it is gone, so a finished run leaves nothing hidden in the
/// user's library. Only that exact auto-created name is ever removed, and
/// `remove_dir` refuses a non-empty folder, so a shared absolute work root or a
/// concurrent run's folder is never touched.
pub(super) fn remove_empty_auto_work_root(work_dir: &Path) {
    if let Some(parent) = work_dir.parent() {
        if parent
            .file_name()
            .is_some_and(|name| name == ".hevc-rebuild-work")
        {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub(super) fn make_work_dir(source: &Path, configured: &str) -> Result<PathBuf, String> {
    let root = crate::media_work_directory(source, configured)?;
    for _ in 0..MAX_TEMP_NAME_ATTEMPTS {
        let candidate = root.join(format!("hevc-rebuild-{}", unique_token()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create a rebuild work folder: {error}")),
        }
    }
    Err("Could not allocate a unique rebuild work folder.".into())
}

pub(super) fn file_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.len())
}

/// Stage 6: choose the final and temporary output paths and check disk space.
pub(super) fn prepare_output(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    plan: &FilePlan,
) -> Result<OutputTargets, String> {
    let source = frame.source;
    let input_bytes = frame.input_bytes;
    let options = context.options;
    let profile = &plan.profile;
    let (output_base, separate_output) = resolved_output_path(source, options)?;
    let final_output = if options.test_mode {
        test_output_path(&output_base)?
    } else {
        output_base
    };
    if final_output != source && final_output.exists() {
        return Err(format!(
            "Output already exists and will not be overwritten: {}",
            final_output.display()
        ));
    }
    let output_parent = final_output
        .parent()
        .ok_or("The planned output has no parent folder.")?
        .to_path_buf();
    fs::create_dir_all(&output_parent)
        .map_err(|error| format!("Could not create the output folder: {error}"))?;
    let stem = final_output
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let temp_output =
        output_parent.join(format!(".{stem}.hevc-rebuild.{}.tmp.mkv", unique_token()));
    state.final_output = Some(final_output.clone());
    state.temp_output = Some(temp_output.clone());
    state
        .row
        .insert("output".into(), json!(final_output.to_string_lossy()));
    if separate_output {
        (context.line)(format!(
            "Validated output will be written to {}; source stays in place",
            final_output.parent().unwrap_or(&final_output).display()
        ));
    }

    state.phase = Phase::StoragePreflight;
    emit(
        context,
        frame,
        "Checking free disk space",
        0.05,
        None,
        None,
        None,
        None,
    );
    let free = disk_free_bytes(&output_parent).ok_or_else(|| {
        format!(
            "Cannot determine free space for output folder {}",
            output_parent.display()
        )
    })?;
    let duration = profile
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let (_estimated, recommended, conservative_ok) =
        storage_preflight(input_bytes.unwrap_or(0), free, duration, options)?;
    if !conservative_ok {
        (context.line)(format!(
            "Free space is below the conservative {} budget, but above the safe start floor. Live disk projection will stop before the reserve is consumed.",
            human_bytes(recommended)
        ));
    }
    Ok(OutputTargets {
        final_output,
        temp_output,
        separate_output,
        duration,
    })
}
