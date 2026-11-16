//! Planning helpers for unattended library batches: which files, how much disk,
//! what free-space floor, and whether a plan can still start. Pure logic lives in
//! `crate::batch_plan`; this file adapts it to settings and the filesystem.

use super::*;

/// Builds the plan for `plan_library_batch`: which files, how much space, what floor.
pub(super) fn build_batch_plan(
    root: &Path,
    params: &PlanBatchParams,
) -> Result<crate::batch_plan::BatchPlan, String> {
    use crate::batch_plan::{
        default_floor_bytes, forecast_volume, BatchPlan, PlannedFile, MAX_PLANNED_FILES,
    };
    const GIB: u64 = 1024 * 1024 * 1024;
    let include = params.filename_include_patterns.clone().unwrap_or_default();
    let mut exclude = params.filename_exclude_patterns.clone().unwrap_or_default();
    // An unattended batch must never treat its own backups as new work, so this
    // exclusion cannot be removed through the plan.
    if !exclude
        .iter()
        .any(|pattern| pattern.eq_ignore_ascii_case(crate::backup_folder::LEGACY_BACKUP_FOLDER))
    {
        exclude.push(crate::backup_folder::LEGACY_BACKUP_FOLDER.to_owned());
    }
    let discovered = crate::media_scan::discover_media_paths(
        root,
        params.recursive.unwrap_or(true),
        params.minimum_size_mib.unwrap_or(5),
        &include,
        &exclude,
        None,
    )?;
    let mut files = Vec::new();
    let mut already_processed = 0usize;
    for path in discovered {
        let backed_up = path
            .parent()
            .zip(path.file_name())
            .is_some_and(|(parent, name)| crate::backup_folder::has_backup(parent, name));
        if backed_up {
            already_processed += 1;
        } else {
            files.push(PlannedFile::read(&path)?);
        }
    }
    if files.is_empty() {
        return Err(if already_processed > 0 {
            format!("Every eligible file ({already_processed}) already has a backup, so there is nothing left to rebuild.")
        } else {
            "No eligible media files were found under that folder.".into()
        });
    }
    if files.len() > MAX_PLANNED_FILES {
        return Err(format!(
            "That folder has {} files; one batch is limited to {MAX_PLANNED_FILES}. Plan a subfolder or use filename filters.",
            files.len()
        ));
    }
    let mut settings = if let Some(serialized) = params.settings_json.as_deref() {
        serde_json::from_str::<Value>(serialized)
            .map_err(|error| format!("settings_json is invalid: {error}"))?
    } else {
        let effective = crate::settings::load_effective_settings(&root.to_string_lossy())?;
        serde_json::to_value(effective)
            .map_err(|error| error.to_string())?
            .get("settings")
            .cloned()
            .ok_or("Could not load effective settings.")?
    };
    if let Some(work_root) = params.work_root.as_deref() {
        if !settings.get("paths").is_some_and(Value::is_object) {
            settings["paths"] = json!({});
        }
        settings["paths"]["work_root"] = json!(work_root);
    }
    let ratio = settings
        .pointer("/resources/preflight_output_ratio")
        .and_then(Value::as_f64)
        .unwrap_or(1.25)
        .clamp(0.25, 4.0);
    let configured_floor = settings
        .pointer("/resources/min_free_disk_gib")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0);
    if let Some(requested) = params.reserve_gib {
        if !requested.is_finite() || !(0.0..=1_000_000.0).contains(&requested) {
            return Err("reserve_gib must be between 0 and 1,000,000.".into());
        }
    }

    // Judge space per filesystem, because a library can span several disks.
    let mut by_device: BTreeMap<u64, Vec<&PlannedFile>> = BTreeMap::new();
    for file in &files {
        by_device.entry(file.device).or_default().push(file);
    }
    let mut measured = Vec::new();
    let mut floor_bytes = 0u64;
    for group in by_device.values() {
        let sample = group[0]
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        let free = crate::media_scan::disk_free_bytes(&sample).ok_or_else(|| {
            format!(
                "Cannot read free space for {}, so an unattended batch cannot be protected. Refusing to plan.",
                sample.display()
            )
        })?;
        let total = crate::media_scan::disk_total_bytes(&sample);
        let volume_floor = match params.reserve_gib {
            Some(gib) => (gib * GIB as f64) as u64,
            None => default_floor_bytes(total),
        };
        floor_bytes = floor_bytes.max(volume_floor);
        measured.push((
            sample,
            group.len(),
            group.iter().map(|file| file.size).sum::<u64>(),
            free,
            total,
        ));
    }
    // The engine takes one floor, so it is the strictest any volume needs, and
    // never below what the user already configured.
    let floor_gib = ((floor_bytes as f64 / GIB as f64).max(configured_floor) * 10.0).ceil() / 10.0;
    let applied_floor_bytes = (floor_gib * GIB as f64) as u64;
    let volumes = measured
        .into_iter()
        .map(|(sample, count, source, free, total)| {
            forecast_volume(
                sample,
                count,
                source,
                free,
                total,
                applied_floor_bytes,
                ratio,
            )
        })
        .collect();
    if !settings.get("resources").is_some_and(Value::is_object) {
        settings["resources"] = json!({});
    }
    settings["resources"]["min_free_disk_gib"] = json!(floor_gib);
    let pick = |pointer: &str| settings.pointer(pointer).cloned().unwrap_or(Value::Null);
    let settings_summary = json!({
        "crf": pick("/video/crf"),
        "preset": pick("/video/preset"),
        "promote_8bit_to_10bit": pick("/video/promote_8bit_to_10bit"),
        "tune": pick("/content/tune"),
        "preserve_hdr10plus": pick("/hdr/preserve_hdr10plus"),
        "dolby_vision": pick("/hdr/dolby_vision"),
        "embed_artwork": pick("/metadata/embed_artwork"),
    });
    let work_root = settings
        .pointer("/paths/work_root")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_owned();
    Ok(BatchPlan {
        id: format!("plan-{}", job_id()),
        root: root.to_path_buf(),
        files,
        already_processed,
        volumes,
        floor_gib,
        max_consecutive_failures: 3,
        settings_summary,
        work_root,
        settings_json: serde_json::to_string(&settings).map_err(|error| error.to_string())?,
        created: std::time::Instant::now(),
    })
}

/// A full rebuild keeps every original in `old/`, so disk use only goes up. Over
/// MCP a zero reserve would let an unattended run fill the volume, so a missing
/// or zero floor becomes the default one. A larger configured floor is kept.
pub(super) fn ensure_disk_floor(settings: &mut Value, root: &Path) {
    let configured = settings
        .pointer("/resources/min_free_disk_gib")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if configured > 0.0 {
        return;
    }
    let probe = if root.is_dir() {
        root.to_path_buf()
    } else {
        root.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf())
    };
    let floor = crate::batch_plan::default_floor_bytes(crate::media_scan::disk_total_bytes(&probe));
    let floor_gib = ((floor as f64 / 1_073_741_824.0) * 10.0).ceil() / 10.0;
    if !settings.get("resources").is_some_and(Value::is_object) {
        settings["resources"] = json!({});
    }
    settings["resources"]["min_free_disk_gib"] = json!(floor_gib);
}

/// Free space can shrink between planning and starting; refuse to start below the floor.
pub(super) fn plan_space_problem(plan: &crate::batch_plan::BatchPlan) -> Option<String> {
    let floor = (plan.floor_gib * 1024.0 * 1024.0 * 1024.0) as u64;
    for volume in &plan.volumes {
        match crate::media_scan::disk_free_bytes(&volume.sample_path) {
            Some(free) if free >= floor => {}
            Some(free) => {
                return Some(format!(
                    "Free space on the volume holding {} is {:.1} GiB, below the protected floor of {:.1} GiB.",
                    volume.sample_path.display(),
                    free as f64 / 1_073_741_824.0,
                    plan.floor_gib
                ));
            }
            None => {
                return Some(format!(
                    "Cannot read free space for {}.",
                    volume.sample_path.display()
                ));
            }
        }
    }
    None
}
