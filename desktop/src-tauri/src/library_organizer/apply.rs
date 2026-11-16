//! Collision-safe rename application with per-parent grouping and rollback.

use super::*;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

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
pub(super) struct MoveGroup {
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
    crate::transactions::sync_directory(&group.parent).map_err(|error| {
        format!("Rename rollback restored paths but directory durability is unconfirmed: {error}")
    })
}

fn apply_plan(plan: InternalPlan) -> Result<RenameApplyResult, String> {
    apply_plan_with_sync(plan, crate::transactions::sync_directory)
}

pub(super) fn apply_plan_with_sync(
    plan: InternalPlan,
    sync: impl Fn(&Path) -> io::Result<()>,
) -> Result<RenameApplyResult, String> {
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
        sync(&group.parent).map_err(|error| format!(
            "Reviewed renames were applied in {}, but directory durability could not be confirmed: {error}. Applied paths were retained; review the plan before retrying. No crash-recovery journal exists.",
            group.parent.display()
        ))?;
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

// Compatibility delegate retained; canonical callers/tests use the pattern-aware path.
#[allow(dead_code)]
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
