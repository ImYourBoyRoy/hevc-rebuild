//! Releasing temporary outputs and retaining `.error.mkv`/`.error.log` review artifacts after a failure.

use super::*;

pub(super) fn release_temporary_output(
    state: &mut FileState,
    source: &Path,
    failure_detail: Option<&str>,
    options: &RebuildRunOptions,
) {
    let status = state.status();
    let late = state.phase.holds_finished_output() && !status.is_success();
    if !late {
        if let Some(temp) = state.temp_output.take() {
            if !options.keep_work && temp.exists() {
                let _ = fs::remove_file(temp);
            }
        }
        return;
    }
    let mut retained: Option<PathBuf> = None;
    if let Some(temp) = state.temp_output.clone().filter(|path| path.exists()) {
        retained = keep_finished_output(&temp, state.final_output.as_deref()).ok();
        if retained.is_none() {
            retained = Some(temp);
        }
    } else if let Some(final_output) = state.final_output.clone() {
        if fs::symlink_metadata(&final_output).is_ok() && final_output != source {
            retained = keep_finished_output(&final_output, Some(&final_output))
                .ok()
                .or(Some(final_output));
        }
    }
    if let Some(path) = &retained {
        state
            .row
            .insert("output".into(), json!(path.to_string_lossy()));
        if let Some(bytes) = file_size(path) {
            state.row.insert("output_bytes".into(), json!(bytes));
        }
    }
    let intended = state.final_output.clone().or_else(|| retained.clone());
    if let Some(intended) = intended {
        let log_target = retained.clone().unwrap_or_else(|| intended.clone());
        let log = write_retained_error_log(
            &log_target.with_extension("log"),
            source,
            &intended,
            retained.as_deref(),
            state.phase.label(),
            status.as_str(),
            state
                .row
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(""),
            failure_detail,
        );
        let message = state
            .row
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let note = match (&retained, &log) {
            (Some(path), Some(log)) => format!(
                " The completed output was retained at {}, but remains unvalidated; diagnostics: {}.",
                path.display(),
                log.display()
            ),
            (Some(path), None) => format!(
                " The completed output was retained at {}, but remains unvalidated.",
                path.display()
            ),
            _ => String::new(),
        };
        state
            .row
            .insert("message".into(), json!(format!("{message}{note}")));
    }
    state.temp_output = None;
}

pub(super) fn keep_finished_output(
    temp: &Path,
    final_output: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Ok(file) = fs::File::open(temp) {
        let _ = file.sync_all();
    }
    let parent = final_output
        .and_then(Path::parent)
        .or_else(|| temp.parent())
        .ok_or("The retained output has no parent folder.")?
        .to_path_buf();
    let stem = final_output
        .and_then(Path::file_stem)
        .or_else(|| temp.file_stem())
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    let suffix = final_output.and_then(Path::extension).map_or_else(
        || "mkv".to_string(),
        |value| value.to_string_lossy().into_owned(),
    );
    let mut candidates = vec![parent.join(format!("{stem}.error.{suffix}"))];
    candidates
        .extend((1..10_000).map(|number| parent.join(format!("{stem}.error.{number}.{suffix}"))));
    for candidate in candidates {
        if fs::symlink_metadata(&candidate).is_ok()
            || fs::symlink_metadata(candidate.with_extension("log")).is_ok()
        {
            continue;
        }
        match rename_no_replace(temp, &candidate) {
            Ok(()) => {
                let _ = sync_directory(&parent);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not retain the finished output: {error}")),
        }
    }
    Err(format!(
        "Unable to retain completed output without overwriting an existing file: {}",
        parent.display()
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_retained_error_log(
    path: &Path,
    source: &Path,
    intended: &Path,
    retained: Option<&Path>,
    phase: &str,
    status: &str,
    message: &str,
    failure_detail: Option<&str>,
) -> Option<PathBuf> {
    let contents = [
        "HEVC Rebuild retained-output diagnostic".to_string(),
        format!("Phase: {phase}"),
        format!("Status: {status}"),
        format!("Source: {}", source.display()),
        format!("Intended output: {}", intended.display()),
        format!(
            "Retained output: {}",
            retained.map_or_else(|| "not available".to_string(), |path| path.display().to_string())
        ),
        format!(
            "Error: {}",
            failure_detail
                .filter(|text| !text.is_empty())
                .unwrap_or(if message.is_empty() {
                    "unknown failure"
                } else {
                    message
                })
        ),
        format!("Message: {}", if message.is_empty() { "none" } else { message }),
        "Outcome: the completed output was retained for investigation but was not accepted as a successful rebuild.".into(),
        String::new(),
    ]
    .join("\n");
    for number in 0..10_000u32 {
        let candidate = if number == 0 {
            path.to_path_buf()
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let extension = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            path.with_file_name(format!("{stem}.{number}{extension}"))
        };
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        match open.open(&candidate) {
            Ok(mut file) => {
                return (file.write_all(contents.as_bytes()).is_ok() && file.sync_all().is_ok())
                    .then(|| {
                        if let Some(parent) = candidate.parent() {
                            let _ = sync_directory(parent);
                        }
                        candidate
                    });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}
