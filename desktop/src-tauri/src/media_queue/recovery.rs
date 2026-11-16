//! Reconciles queue entries left running by an interrupted process: finish, clean up owned partial outputs, or requeue.

use super::*;

pub(super) fn is_active_run_status(status: &str) -> bool {
    matches!(
        status,
        "running" | "scanning" | "renaming" | "remuxing" | "rebuilding" | "validating"
    )
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterruptedRecovery {
    pub(crate) skipped: bool,
    pub(crate) completed: u32,
    pub(crate) cleared: u32,
    pub(crate) held: u32,
    pub(crate) removed_temporary_files: u32,
    pub(crate) run_states: BTreeMap<String, QueueRunState>,
    pub(crate) summary: String,
}

impl InterruptedRecovery {
    fn skipped(run_states: BTreeMap<String, QueueRunState>) -> Self {
        Self {
            skipped: true,
            completed: 0,
            cleared: 0,
            held: 0,
            removed_temporary_files: 0,
            run_states,
            summary: String::new(),
        }
    }
}

/// Settle files that were active when the app last closed.
///
/// A saved successful report is marked complete. An unfinished file whose
/// original is still present is returned to the normal queue, and only an
/// unopened temporary encode created by this app is removed. Existing outputs,
/// originals, and `old/` backups are not replaced or deleted. A partial encode
/// is not resumed.
pub(crate) fn reconcile_interrupted(path: &Path) -> Result<InterruptedRecovery, String> {
    let loaded = load(path)?;
    let mut queue = loaded.queue;
    let interrupted = queue
        .run_states
        .iter()
        .filter(|(_, state)| state.status == "interrupted")
        .map(|(media_path, _)| media_path.clone())
        .collect::<Vec<_>>();
    if interrupted.is_empty() {
        return Ok(InterruptedRecovery {
            skipped: false,
            completed: 0,
            cleared: 0,
            held: 0,
            removed_temporary_files: 0,
            run_states: queue.run_states,
            summary: String::new(),
        });
    }

    let mut completed = 0;
    let mut cleared = 0;
    let mut held = 0;
    let mut removed_temporary_files = 0;
    for media_path in interrupted {
        let Some(mut state) = queue.run_states.get(&media_path).cloned() else {
            continue;
        };
        let decision = recover_interrupted_file(&media_path, &state);
        removed_temporary_files += decision.removed_temporary_files;
        match decision.outcome {
            RecoveryOutcome::Complete(message) => {
                state.status = "completed".into();
                state.message = Some(message);
                state.updated_at_ms = now_ms();
                queue.run_states.insert(media_path, state);
                completed += 1;
            }
            RecoveryOutcome::Recorded { status, message } => {
                state.status = status;
                state.message = Some(message);
                state.updated_at_ms = now_ms();
                queue.run_states.insert(media_path, state);
            }
            RecoveryOutcome::Clear => {
                queue.run_states.remove(&media_path);
                cleared += 1;
            }
            RecoveryOutcome::Hold(message) => {
                state.status = "needs_review".into();
                state.message = Some(message);
                state.updated_at_ms = now_ms();
                queue.run_states.insert(media_path, state);
                held += 1;
            }
        }
    }
    queue.updated_at_ms = now_ms();
    save(path, queue.clone())?;
    let summary = recovery_summary(completed, cleared, held, removed_temporary_files);
    Ok(InterruptedRecovery {
        skipped: false,
        completed,
        cleared,
        held,
        removed_temporary_files,
        run_states: queue.run_states,
        summary,
    })
}

pub(crate) fn interrupted_recovery_skipped(path: &Path) -> Result<InterruptedRecovery, String> {
    let queue = read_snapshot(path)?;
    Ok(InterruptedRecovery::skipped(queue.run_states))
}

pub(super) enum RecoveryOutcome {
    Complete(String),
    Recorded { status: String, message: String },
    Clear,
    Hold(String),
}

pub(super) struct RecoveryDecision {
    outcome: RecoveryOutcome,
    removed_temporary_files: u32,
}

fn recover_interrupted_file(media_path: &str, state: &QueueRunState) -> RecoveryDecision {
    let source = Path::new(media_path);
    let source_exists =
        fs::symlink_metadata(source).is_ok_and(|metadata| metadata.file_type().is_file());
    let removed_temporary_files = remove_incomplete_outputs(source, state.output_path.as_deref());
    let report_status = state
        .report_status
        .as_deref()
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .map(|status| status.to_ascii_uppercase());

    if state.mode.as_deref() == Some("organize") {
        return RecoveryDecision {
            removed_temporary_files,
            outcome: if source_exists {
                RecoveryOutcome::Clear
            } else {
                RecoveryOutcome::Hold(
                    "This file is no longer at its saved path, so the interrupted rename was left unchanged."
                        .into(),
                )
            },
        };
    }

    if let Some(status) = report_status.as_deref() {
        if matches!(status, "OK" | "METADATA_OK" | "TEST_OK" | "INFO") {
            let output_ok = state.output_path.is_none() && (status == "INFO" || source_exists)
                || state
                    .output_path
                    .as_deref()
                    .is_some_and(nonempty_regular_file);
            if output_ok {
                return RecoveryDecision {
                    removed_temporary_files,
                    outcome: RecoveryOutcome::Complete(
                        "Marked complete from the saved successful result. The existing file was left in place."
                            .into(),
                    ),
                };
            }
            return RecoveryDecision {
                removed_temporary_files,
                outcome: RecoveryOutcome::Hold(
                    "The saved result says this file finished, but the output is missing. Nothing else was changed."
                        .into(),
                ),
            };
        }
        if status == "FAILED" || status == "SKIPPED" {
            let skipped = status == "SKIPPED";
            return RecoveryDecision {
                removed_temporary_files,
                outcome: RecoveryOutcome::Recorded {
                    status: if skipped { "skipped" } else { "failed" }.into(),
                    message: if skipped {
                        "The last run skipped this file. The original was left in place.".into()
                    } else {
                        "The last run failed. The original file was left in place.".into()
                    },
                },
            };
        }
    }

    if source_exists {
        return RecoveryDecision {
            removed_temporary_files,
            outcome: RecoveryOutcome::Clear,
        };
    }
    RecoveryDecision {
        removed_temporary_files,
        outcome: RecoveryOutcome::Hold(
            "This file is no longer at its saved path, so it was left unchanged.".into(),
        ),
    }
}

fn nonempty_regular_file(path: &str) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
}

fn remove_incomplete_outputs(source: &Path, output_path: Option<&str>) -> u32 {
    let mut removed = 0;
    if let Some(parent) = source.parent() {
        if let Some(stem) = source.file_stem().and_then(|stem| stem.to_str()) {
            removed += remove_owned_temps(parent, stem);
        }
    }
    if let Some(output) = output_path {
        let output = Path::new(output);
        if output.parent() != source.parent() {
            if let (Some(parent), Some(stem)) = (
                output.parent(),
                output.file_stem().and_then(|stem| stem.to_str()),
            ) {
                removed += remove_owned_temps(parent, stem);
            }
        }
    }
    removed
}

fn remove_owned_temps(directory: &Path, stem: &str) -> u32 {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_owned_incomplete_temp(&name, stem) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        if regular_file_is_open(&path) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn is_owned_incomplete_temp(file_name: &str, stem: &str) -> bool {
    let Some(rest) = file_name.strip_prefix('.') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(stem) else {
        return false;
    };
    for (prefix, suffix) in [
        (".hevc-rebuild.", ".tmp.mkv"),
        (".hevc-metadata.", ".tmp.mkv"),
        (".upload-", ".tmp.mp4"),
    ] {
        let Some(token) = rest.strip_prefix(prefix) else {
            continue;
        };
        let Some(token) = token.strip_suffix(suffix) else {
            continue;
        };
        return token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    false
}

#[cfg(target_os = "linux")]
fn regular_file_is_open(path: &Path) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return true;
    };
    let Ok(processes) = fs::read_dir("/proc") else {
        return true;
    };
    for process in processes.flatten() {
        let Some(pid) = process.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(fds) = fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if fs::read_link(fd.path()).is_ok_and(|target| target == canonical) {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn regular_file_is_open(path: &Path) -> bool {
    // Linux can inspect every process' descriptors through /proc. Other hosts
    // do not expose an equivalent portable API, so preserve a file whenever
    // an exclusive read/write reopen is not possible; the OS then remains the
    // final authority on whether a stale temporary can be removed.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_err()
}

fn recovery_summary(
    completed: u32,
    cleared: u32,
    held: u32,
    removed_temporary_files: u32,
) -> String {
    let mut parts = Vec::new();
    if cleared > 0 {
        parts.push(format!(
            "{cleared} unfinished file{} can be run again. The original{} left in place",
            if cleared == 1 { "" } else { "s" },
            if cleared == 1 { " was" } else { "s were" },
        ));
        if removed_temporary_files > 0 {
            parts.push(format!(
                "{removed_temporary_files} unfinished temporary file{} removed",
                if removed_temporary_files == 1 {
                    " was"
                } else {
                    "s were"
                },
            ));
        }
    } else if removed_temporary_files > 0 {
        parts.push(format!(
            "{removed_temporary_files} unfinished temporary file{} removed",
            if removed_temporary_files == 1 {
                " was"
            } else {
                "s were"
            },
        ));
    }
    if completed > 0 {
        parts.push(format!(
            "{completed} file{} already finished and {} marked complete",
            if completed == 1 { " had" } else { "s had" },
            if completed == 1 { "was" } else { "were" },
        ));
    }
    if held > 0 {
        parts.push(format!(
            "{held} file{} left unchanged because the saved path or output could not be confirmed",
            if held == 1 { " was" } else { "s were" },
        ));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("Recovered the interrupted session. {}.", parts.join(". "))
}
