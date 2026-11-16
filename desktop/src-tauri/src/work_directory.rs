//! Resolution and preview of the per-source work directory shared by the GUI, CLI and MCP.

use super::*;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WorkDirectoryPreview {
    pub source_path: String,
    pub configured_root: String,
    pub directory: String,
    pub resolution: String,
    pub fallback_directory: Option<String>,
}

fn work_root_selection(configured: &str) -> Result<(Option<PathBuf>, String, String), String> {
    if let Some(root) = std::env::var_os("HEVC_REBUILD_WORK_ROOT") {
        let path = PathBuf::from(root);
        if path.as_os_str().is_empty() {
            return Err("HEVC_REBUILD_WORK_ROOT cannot be empty.".into());
        }
        let text = path
            .to_str()
            .ok_or(
                "HEVC_REBUILD_WORK_ROOT must be an absolute UTF-8 folder path up to 8192 bytes.",
            )?
            .to_owned();
        if text.len() > 8192 || text.chars().any(char::is_control) || !path.is_absolute() {
            return Err(
                "HEVC_REBUILD_WORK_ROOT must be an absolute folder path up to 8192 bytes.".into(),
            );
        }
        return Ok((Some(path), "environment-override".into(), text));
    }
    let configured = configured.trim();
    if configured.is_empty() || configured.eq_ignore_ascii_case("auto") {
        return Ok((None, "source-adjacent".into(), "auto".into()));
    }
    let path = PathBuf::from(configured);
    if configured.len() > 8192 || configured.chars().any(char::is_control) || !path.is_absolute() {
        return Err(
            "The work root must be 'auto' or an absolute folder path up to 8192 bytes.".into(),
        );
    }
    Ok((Some(path), "configured".into(), configured.into()))
}

fn work_directory_anchor(source: &Path) -> &Path {
    if source.is_dir() {
        source
    } else {
        source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }
}

/// Resolve the work directory without creating it. This is used by settings,
/// scan, CLI, MCP, and the GUI so each surface can show the same dynamic path.
pub(crate) fn preview_work_directory(
    source: &Path,
    configured: &str,
) -> Result<WorkDirectoryPreview, String> {
    let (override_root, resolution, configured_root) = work_root_selection(configured)?;
    let directory =
        override_root.unwrap_or_else(|| work_directory_anchor(source).join(".hevc-rebuild-work"));
    let fallback_directory = if resolution == "source-adjacent" {
        app_state_directory()
            .ok()
            .map(|path| path.join("work").to_string_lossy().into_owned())
    } else {
        None
    };
    Ok(WorkDirectoryPreview {
        source_path: source.to_string_lossy().into_owned(),
        configured_root,
        directory: directory.to_string_lossy().into_owned(),
        resolution,
        fallback_directory,
    })
}

fn create_writable_work_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("Could not create work folder {}: {error}", path.display()))?;
    let probe = path.join(format!(
        ".hevc-rebuild-write-test-{}-{}",
        std::process::id(),
        new_job_id()
    ));
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map(|_| ())
        .map_err(|error| format!("Could not write work folder {}: {error}", path.display()));
    let _ = fs::remove_file(&probe);
    result
}

/// Resolve and create a work directory for one source file.
///
/// The default is dynamic: work is created beside each source in
/// `.hevc-rebuild-work`, keeping temporary outputs on the source volume. A
/// configured `paths.work_root` or the process-level environment override uses
/// the selected folder directly. If the automatic adjacent folder is
/// unwritable, the app-state work folder is used as a clearly reported fallback.
pub(crate) fn media_work_directory(source: &Path, configured: &str) -> Result<PathBuf, String> {
    let preview = preview_work_directory(source, configured)?;
    let directory = PathBuf::from(&preview.directory);
    if preview.resolution != "source-adjacent" {
        create_writable_work_directory(&directory)?;
        return Ok(directory);
    }
    match create_writable_work_directory(&directory) {
        Ok(()) => Ok(directory),
        Err(adjacent_error) => {
            let fallback = preview
                .fallback_directory
                .as_deref()
                .map(PathBuf::from)
                .ok_or_else(|| {
                    format!(
                        "Could not create source-adjacent work folder {} ({adjacent_error}), and no app-state fallback is available.",
                        directory.display()
                    )
                })?;
            create_writable_work_directory(&fallback).map_err(|fallback_error| {
                format!(
                    "Could not use source-adjacent work folder {} ({adjacent_error}); app-state fallback {} also failed ({fallback_error}).",
                    directory.display(),
                    fallback.display()
                )
            })?;
            Ok(fallback)
        }
    }
}

/// Resolve the effective work-directory policy for a file or folder without
/// creating any directory. This is the shared discovery API used by the GUI,
/// JSON CLI, and MCP.
pub(crate) fn resolve_work_directory(
    root: &str,
    configured: Option<&str>,
) -> Result<WorkDirectoryPreview, String> {
    let source = PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media path: {error}"))?;
    if !source.is_file() && !source.is_dir() {
        return Err("The selected media path is not a file or folder.".into());
    }
    let configured = match configured {
        Some(value) => value.to_string(),
        None => {
            let payload = serde_json::to_value(settings::load_effective_settings(
                &source.to_string_lossy(),
            )?)
            .map_err(|error| format!("Could not encode effective settings: {error}"))?;
            payload
                .pointer("/settings/paths/work_root")
                .and_then(Value::as_str)
                .unwrap_or("auto")
                .to_string()
        }
    };
    preview_work_directory(&source, &configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_work_directory_defaults_beside_source_and_honors_override() {
        let _guard = TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("HEVC_REBUILD_WORK_ROOT");
        std::env::remove_var("HEVC_REBUILD_WORK_ROOT");
        let root = test_dir("media-work");
        let source = root.join("Film.mkv");
        fs::write(&source, b"fixture").unwrap();
        let preview = preview_work_directory(&source, "auto").unwrap();
        assert_eq!(preview.source_path, source.to_string_lossy());
        assert_eq!(preview.configured_root, "auto");
        assert_eq!(
            preview.directory,
            root.join(".hevc-rebuild-work").to_string_lossy()
        );
        assert_eq!(preview.resolution, "source-adjacent");
        assert_eq!(
            media_work_directory(&source, "auto").unwrap(),
            root.join(".hevc-rebuild-work")
        );

        let blocked_root = root.join("blocked-source");
        fs::create_dir_all(&blocked_root).unwrap();
        let blocked_source = blocked_root.join("Film.mkv");
        fs::write(&blocked_source, b"fixture").unwrap();
        fs::write(blocked_root.join(".hevc-rebuild-work"), b"not a folder").unwrap();
        let blocked_preview = preview_work_directory(&blocked_source, "auto").unwrap();
        let fallback = blocked_preview
            .fallback_directory
            .clone()
            .expect("automatic work directories expose an app-state fallback");
        assert_eq!(
            media_work_directory(&blocked_source, "auto").unwrap(),
            PathBuf::from(&fallback)
        );

        let override_root = root.join("custom-work");
        let configured_preview =
            preview_work_directory(&source, &override_root.to_string_lossy()).unwrap();
        assert_eq!(
            configured_preview.configured_root,
            override_root.to_string_lossy()
        );
        assert_eq!(
            configured_preview.directory,
            override_root.to_string_lossy()
        );
        assert_eq!(configured_preview.resolution, "configured");
        std::env::set_var("HEVC_REBUILD_WORK_ROOT", &override_root);
        let environment_preview = preview_work_directory(&source, "auto").unwrap();
        assert_eq!(environment_preview.resolution, "environment-override");
        assert_eq!(
            environment_preview.configured_root,
            override_root.to_string_lossy()
        );
        assert_eq!(
            media_work_directory(&source, "auto").unwrap(),
            override_root
        );

        match previous {
            Some(value) => std::env::set_var("HEVC_REBUILD_WORK_ROOT", value),
            None => std::env::remove_var("HEVC_REBUILD_WORK_ROOT"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
