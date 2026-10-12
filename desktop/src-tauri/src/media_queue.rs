//! Durable, local queue state for scanned media sources.
//!
//! Scan results are kept beside the app's portable settings profile, or in the
//! per-user configuration directory for installed builds. Writes use a
//! same-directory temporary file and rename so an interrupted write does not
//! replace the last complete queue document.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

const QUEUE_VERSION: u32 = 1;
const MAX_QUEUE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_QUEUE_SOURCES: usize = 500;
const MAX_QUEUE_ENTRIES: usize = 25_000;
const MAX_SELECTED_PATHS: usize = 10_000;
const MAX_RUN_STATES: usize = MAX_QUEUE_ENTRIES;
const MAX_MOVIE_REVIEWS: usize = 25_000;
const MAX_SERIES_REVIEWS: usize = 5_000;
const MAX_METADATA_SUGGESTIONS: usize = 25_000;
const MAX_TV_EPISODES: usize = 10_000;
const MAX_ARTWORK_CANDIDATES: usize = 64;
const MAX_QUEUE_PAGE_SIZE: usize = 200;

static QUEUE_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Keep the queue's assigned artwork stable while an artwork-cache purge
/// checks which cached files are still referenced.
pub(crate) fn lock_for_artwork_cache_purge() -> Result<MutexGuard<'static, ()>, String> {
    QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.".to_owned())
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueueSourceStatus {
    Scanning,
    Ready,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueSource {
    pub(crate) root: String,
    pub(crate) status: QueueSourceStatus,
    #[serde(default)]
    pub(crate) scan_result: Option<Value>,
    #[serde(default)]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) updated_at_ms: u64,
    #[serde(default, skip_deserializing)]
    pub(crate) changed_files: Vec<String>,
    #[serde(default, skip_deserializing)]
    pub(crate) missing_files: Vec<String>,
}

/// Last known per-file run state. An interrupted encode is not resumed; startup
/// recovery only marks a saved success complete, or clears an unfinished file
/// whose original is still in place.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueRunState {
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) output_path: Option<String>,
    #[serde(default)]
    pub(crate) report_path: Option<String>,
    #[serde(default)]
    pub(crate) report_status: Option<String>,
    #[serde(default)]
    pub(crate) updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueArtworkCandidate {
    pub(crate) id: String,
    pub(crate) provider: String,
    pub(crate) title: String,
    pub(crate) image_url: String,
    pub(crate) thumbnail_url: String,
    pub(crate) source_url: String,
    pub(crate) creator: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) license_url: Option<String>,
    pub(crate) attribution: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueMetadataSuggestion {
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) title: Option<String>,
    pub(crate) year: Option<String>,
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) record_id: Option<String>,
    #[serde(default)]
    pub(crate) source_url: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) auto_associated: bool,
    pub(crate) artwork: Option<QueueArtworkCandidate>,
    #[serde(default)]
    pub(crate) error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueMovieReview {
    pub(crate) title: String,
    pub(crate) year: String,
    pub(crate) certification: String,
    pub(crate) record_id: String,
    pub(crate) short_description: String,
    pub(crate) plot: String,
    pub(crate) genres_text: String,
    pub(crate) rating_text: String,
    pub(crate) source_url: String,
    #[serde(default)]
    pub(crate) comment: String,
    pub(crate) artwork_candidates: Vec<QueueArtworkCandidate>,
    pub(crate) poster_id: String,
    #[serde(default)]
    pub(crate) local_poster_path: String,
    pub(crate) nfo_paths: Vec<String>,
    pub(crate) nfo_error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueTvEpisode {
    pub(crate) season: u32,
    pub(crate) episode: u32,
    pub(crate) title: String,
    pub(crate) airdate: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) source_url: String,
    #[serde(default)]
    pub(crate) comment: String,
    #[serde(default)]
    pub(crate) certification: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueSeasonReview {
    pub(crate) title: String,
    pub(crate) year: String,
    pub(crate) certification: String,
    pub(crate) plot: String,
    pub(crate) genres_text: String,
    pub(crate) rating_text: String,
    #[serde(default)]
    pub(crate) comment: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueSeriesReview {
    pub(crate) title: String,
    pub(crate) year: String,
    pub(crate) certification: String,
    pub(crate) record_id: String,
    pub(crate) short_description: String,
    pub(crate) plot: String,
    pub(crate) genres_text: String,
    pub(crate) rating_text: String,
    pub(crate) source_url: String,
    #[serde(default)]
    pub(crate) comment: String,
    pub(crate) artwork_candidates: Vec<QueueArtworkCandidate>,
    pub(crate) poster_id: String,
    #[serde(default)]
    pub(crate) local_poster_path: String,
    #[serde(default)]
    pub(crate) local_season_poster_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) season_records: BTreeMap<String, QueueSeasonReview>,
    pub(crate) nfo_paths: Vec<String>,
    pub(crate) nfo_error: String,
    pub(crate) episodes: Vec<QueueTvEpisode>,
    pub(crate) episode_assignments: BTreeMap<String, QueueTvEpisode>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueDocument {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) sources: Vec<QueueSource>,
    #[serde(default)]
    pub(crate) selected_media_paths: Vec<String>,
    #[serde(default)]
    pub(crate) file_tune_overrides: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) file_stream_overrides: BTreeMap<String, Vec<u16>>,
    #[serde(default)]
    pub(crate) movie_reviews: BTreeMap<String, QueueMovieReview>,
    #[serde(default)]
    pub(crate) series_reviews: BTreeMap<String, QueueSeriesReview>,
    #[serde(default)]
    pub(crate) match_suggestions: BTreeMap<String, QueueMetadataSuggestion>,
    #[serde(default)]
    pub(crate) run_states: BTreeMap<String, QueueRunState>,
    #[serde(default)]
    pub(crate) active_root: Option<String>,
    #[serde(default)]
    pub(crate) updated_at_ms: u64,
}

impl Default for QueueDocument {
    fn default() -> Self {
        Self {
            version: QUEUE_VERSION,
            sources: Vec::new(),
            selected_media_paths: Vec::new(),
            file_tune_overrides: BTreeMap::new(),
            file_stream_overrides: BTreeMap::new(),
            movie_reviews: BTreeMap::new(),
            series_reviews: BTreeMap::new(),
            match_suggestions: BTreeMap::new(),
            run_states: BTreeMap::new(),
            active_root: None,
            updated_at_ms: now_ms(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueLoadResult {
    pub(crate) queue: QueueDocument,
    pub(crate) queue_file: String,
    pub(crate) warning: Option<String>,
}

pub(crate) fn load(path: &Path) -> Result<QueueLoadResult, String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut warning = None;
    let mut queue = match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_QUEUE_FILE_BYTES => {
            let backup = preserve_unreadable(path)?;
            warning = Some(format!(
                "The saved media queue exceeded the {} MiB safety limit. The previous file was preserved at {}.",
                MAX_QUEUE_FILE_BYTES / (1024 * 1024),
                backup.display()
            ));
            QueueDocument::default()
        }
        Ok(_) => {
            let bytes = fs::read(path).map_err(|error| {
                format!(
                    "Could not read the saved media queue at {}: {error}",
                    path.display()
                )
            })?;
            let parsed = serde_json::from_slice::<QueueDocument>(&bytes);
            let checked = parsed.and_then(|mut document| {
                validate(&document)
                    .map_err(|error| <serde_json::Error as serde::de::Error>::custom(error))?;
                document.sources.iter_mut().for_each(|source| {
                    if matches!(source.status, QueueSourceStatus::Scanning) {
                        source.status = QueueSourceStatus::Interrupted;
                        source.error = Some(
                            "The previous scan was interrupted. Rescan this source to continue."
                                .into(),
                        );
                    }
                    inspect_cached_files(source);
                });
                for state in document.run_states.values_mut() {
                    if is_active_run_status(&state.status) {
                        state.status = "interrupted".into();
                        state.message = Some(
                            "The application closed while this item was active. Review its last operation and file state before continuing."
                                .into(),
                        );
                        state.updated_at_ms = now_ms();
                    }
                }
                Ok(document)
            });
            match checked {
                Ok(document) => document,
                Err(error) => {
                    let backup = preserve_unreadable(path)?;
                    warning = Some(format!(
                        "The saved media queue could not be read ({error}). The previous file was preserved at {}.",
                        backup.display()
                    ));
                    QueueDocument::default()
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => QueueDocument::default(),
        Err(error) => {
            return Err(format!(
                "Could not inspect the saved media queue at {}: {error}",
                path.display()
            ));
        }
    };
    queue.updated_at_ms = queue.updated_at_ms.max(now_ms());
    Ok(QueueLoadResult {
        queue,
        queue_file: path.to_string_lossy().into_owned(),
        warning,
    })
}

/// Read the queue as it is currently persisted without treating active work as
/// abandoned. Read-only CLI/MCP inspection uses this path while the GUI or an
/// MCP job may legitimately be running.
pub(crate) fn read_snapshot(path: &Path) -> Result<QueueDocument, String> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_QUEUE_FILE_BYTES => {
            Err("The saved media queue exceeds its safety limit.".into())
        }
        Ok(_) => {
            let bytes = fs::read(path)
                .map_err(|error| format!("Could not read the saved media queue: {error}"))?;
            let queue: QueueDocument = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Could not read the saved media queue: {error}"))?;
            validate(&queue)?;
            Ok(queue)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(QueueDocument::default()),
        Err(error) => Err(format!("Could not read the saved media queue: {error}")),
    }
}

/// Return a compact, paged view of queue state for CLI and MCP clients.
/// Provider artwork bytes, raw scan payloads, and unrelated profile settings
/// are intentionally omitted.
pub(crate) fn summarize(path: &Path, offset: usize, limit: usize) -> Result<Value, String> {
    if limit == 0 || limit > MAX_QUEUE_PAGE_SIZE {
        return Err(format!(
            "Queue page size must be between 1 and {MAX_QUEUE_PAGE_SIZE}."
        ));
    }
    let queue = read_snapshot(path)?;
    let selected = queue
        .selected_media_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut run_counts = BTreeMap::<String, usize>::new();
    let mut entries = Vec::new();
    let mut sources = Vec::new();
    let mut file_count = 0usize;
    let mut selected_count = 0usize;

    for source in &queue.sources {
        let scan_entries = source
            .scan_result
            .as_ref()
            .and_then(|result| result.get("entries"))
            .and_then(Value::as_array);
        let mut source_selected = 0usize;
        let mut source_files = 0usize;
        if let Some(scan_entries) = scan_entries {
            for item in scan_entries {
                let Some(media_path) = item.get("path").and_then(Value::as_str) else {
                    continue;
                };
                file_count = file_count.saturating_add(1);
                source_files = source_files.saturating_add(1);
                let is_selected = selected.contains(media_path);
                if is_selected {
                    selected_count = selected_count.saturating_add(1);
                    source_selected = source_selected.saturating_add(1);
                }
                let run_state = queue.run_states.get(media_path);
                if let Some(state) = run_state {
                    *run_counts.entry(state.status.clone()).or_default() += 1;
                }
                if file_count > offset && entries.len() < limit {
                    let profile = item.get("profile");
                    let video = profile.and_then(|value| value.get("video"));
                    let hdr = profile.and_then(|value| value.get("hdr"));
                    entries.push(json!({
                        "sourceRoot": source.root,
                        "path": media_path,
                        "relativePath": item.get("relative_path"),
                        "name": item.get("name").and_then(Value::as_str).or_else(|| Path::new(media_path).file_name().and_then(|name| name.to_str())),
                        "sizeBytes": item.get("size_bytes").and_then(Value::as_u64),
                        "scanStatus": item.get("status").and_then(Value::as_str),
                        "error": item.get("error").and_then(Value::as_str),
                        "selected": is_selected,
                        "videoCodec": video.and_then(|value| value.get("codec")).and_then(Value::as_str),
                        "width": video.and_then(|value| value.get("width")).and_then(Value::as_u64),
                        "height": video.and_then(|value| value.get("height")).and_then(Value::as_u64),
                        "hdr10": hdr.and_then(|value| value.get("hdr10")).and_then(Value::as_bool),
                        "hdr10plus": hdr.and_then(|value| value.get("hdr10plus")).and_then(Value::as_bool),
                        "dolbyVision": hdr.and_then(|value| value.get("dolby_vision")).and_then(Value::as_bool),
                        "runStatus": run_state.map(|state| state.status.as_str()),
                        "runMode": run_state.and_then(|state| state.mode.as_deref()),
                        "runMessage": run_state.and_then(|state| state.message.as_deref()),
                        "outputPath": run_state.and_then(|state| state.output_path.as_deref()),
                        "reportStatus": run_state.and_then(|state| state.report_status.as_deref()),
                    }));
                }
            }
        }
        sources.push(json!({
            "root": source.root,
            "status": serde_json::to_value(source.status).unwrap_or(Value::Null),
            "fileCount": source_files,
            "selectedCount": source_selected,
            "error": source.error,
            "changedFileCount": source.changed_files.len(),
            "missingFileCount": source.missing_files.len(),
            "updatedAtMs": source.updated_at_ms,
        }));
    }

    Ok(json!({
        "queueFile": path.to_string_lossy(),
        "warning": Value::Null,
        "version": queue.version,
        "updatedAtMs": queue.updated_at_ms,
        "sourceCount": queue.sources.len(),
        "fileCount": file_count,
        "selectedCount": selected_count,
        "activeRoot": queue.active_root,
        "runCounts": run_counts,
        "sources": sources,
        "offset": offset.min(file_count),
        "limit": limit,
        "nextOffset": if offset.saturating_add(entries.len()) < file_count { Some(offset.saturating_add(entries.len())) } else { None },
        "entries": entries,
    }))
}

/// Change only the persisted global file selection while preserving scans,
/// tune/stream overrides, reviews, and run history.
pub(crate) fn save_selected_paths(
    path: &Path,
    selected_media_paths: Vec<String>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    queue.selected_media_paths = selected_media_paths;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue selection: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

/// Persist run-state changes without replacing the rest of the queue document.
/// This is used by non-GUI clients after a run starts or its report is read.
pub(crate) fn save_run_states(
    path: &Path,
    run_states: BTreeMap<String, QueueRunState>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    for (media_path, state) in run_states {
        queue.run_states.insert(media_path, state);
    }
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue run states: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

fn is_active_run_status(status: &str) -> bool {
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

enum RecoveryOutcome {
    Complete(String),
    Recorded { status: String, message: String },
    Clear,
    Hold(String),
}

struct RecoveryDecision {
    outcome: RecoveryOutcome,
    removed_temporary_files: u32,
}

fn recover_interrupted_file(media_path: &str, state: &QueueRunState) -> RecoveryDecision {
    let source = Path::new(media_path);
    let source_exists = fs::symlink_metadata(source)
        .is_ok_and(|metadata| metadata.file_type().is_file());
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
                || state.output_path.as_deref().is_some_and(nonempty_regular_file);
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
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() > 0)
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

fn recovery_summary(completed: u32, cleared: u32, held: u32, removed_temporary_files: u32) -> String {
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
                if removed_temporary_files == 1 { " was" } else { "s were" },
            ));
        }
    } else if removed_temporary_files > 0 {
        parts.push(format!(
            "{removed_temporary_files} unfinished temporary file{} removed",
            if removed_temporary_files == 1 { " was" } else { "s were" },
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

pub(crate) fn save(path: &Path, mut queue: QueueDocument) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    queue.version = QUEUE_VERSION;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err(format!(
            "The media queue is too large to save safely ({} MiB maximum). Remove old or duplicate sources first.",
            MAX_QUEUE_FILE_BYTES / (1024 * 1024)
        ));
    }
    write_atomically(path, &payload)
}

pub(crate) fn save_selection(
    path: &Path,
    selected_media_paths: Vec<String>,
    active_root: Option<String>,
    file_tune_overrides: BTreeMap<String, String>,
    file_stream_overrides: BTreeMap<String, Vec<u16>>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    queue.selected_media_paths = selected_media_paths;
    queue.active_root = active_root;
    queue.file_tune_overrides = file_tune_overrides;
    queue.file_stream_overrides = file_stream_overrides;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue selection: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

pub(crate) fn save_movie_reviews(
    path: &Path,
    movie_reviews: BTreeMap<String, QueueMovieReview>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    queue.movie_reviews = movie_reviews;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue reviews: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

pub(crate) fn save_series_reviews(
    path: &Path,
    series_reviews: BTreeMap<String, QueueSeriesReview>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    queue.series_reviews = series_reviews;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue series reviews: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

pub(crate) fn save_metadata_suggestions(
    path: &Path,
    match_suggestions: BTreeMap<String, QueueMetadataSuggestion>,
) -> Result<(), String> {
    let _guard = QUEUE_WRITE_LOCK
        .lock()
        .map_err(|_| "The media queue store is unavailable.")?;
    let mut queue = read_for_update(path)?;
    queue.match_suggestions = match_suggestions;
    queue.updated_at_ms = now_ms();
    validate(&queue)?;
    let payload = serde_json::to_vec(&queue)
        .map_err(|error| format!("Could not encode media queue suggestions: {error}"))?;
    if payload.len() as u64 > MAX_QUEUE_FILE_BYTES {
        return Err("The saved media queue exceeds its safety limit.".into());
    }
    write_atomically(path, &payload)
}

fn read_for_update(path: &Path) -> Result<QueueDocument, String> {
    match fs::read(path) {
        Ok(bytes) if bytes.len() as u64 <= MAX_QUEUE_FILE_BYTES => {
            let queue: QueueDocument = serde_json::from_slice(&bytes)
                .map_err(|error| format!("Could not update the saved media queue: {error}"))?;
            validate(&queue)?;
            Ok(queue)
        }
        Ok(_) => Err("The saved media queue exceeds its safety limit.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(QueueDocument::default()),
        Err(error) => Err(format!("Could not read the saved media queue: {error}")),
    }
}

fn saved_poster_reference(path: &str) -> bool {
    !path.trim().is_empty()
        && path.len() <= 8192
        && (path.starts_with("https://") || Path::new(path).is_absolute())
}

fn validate(queue: &QueueDocument) -> Result<(), String> {
    if queue.version != QUEUE_VERSION {
        return Err(format!(
            "Queue format version {} is not supported by this application.",
            queue.version
        ));
    }
    if queue.sources.len() > MAX_QUEUE_SOURCES {
        return Err(format!(
            "The media queue is limited to {MAX_QUEUE_SOURCES} source folders or files."
        ));
    }
    if queue.selected_media_paths.len() > MAX_SELECTED_PATHS {
        return Err(format!(
            "The media queue can select at most {MAX_SELECTED_PATHS} files at once."
        ));
    }
    if queue
        .active_root
        .as_ref()
        .is_some_and(|root| !queue.sources.iter().any(|source| &source.root == root))
    {
        return Err("The active queue source must exist in the source list.".into());
    }
    let mut roots = std::collections::BTreeSet::new();
    let mut entry_count = 0_usize;
    let mut known_paths = std::collections::BTreeSet::new();
    for source in &queue.sources {
        if source.root.trim().is_empty() || source.root.len() > 8192 {
            return Err("A queue source path is empty or too long.".into());
        }
        if !roots.insert(source.root.as_str()) {
            return Err("The media queue contains a duplicate source path.".into());
        }
        if let Some(result) = &source.scan_result {
            let entries = result
                .get("entries")
                .and_then(Value::as_array)
                .ok_or("A saved scan result has no valid media entry list.")?;
            entry_count = entry_count.saturating_add(entries.len());
            for entry in entries {
                let path = entry
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("A saved media entry has no valid path.")?;
                if path.is_empty() || path.len() > 8192 {
                    return Err("A saved media entry path is empty or too long.".into());
                }
                known_paths.insert(path.to_owned());
            }
        }
    }
    if entry_count > MAX_QUEUE_ENTRIES {
        return Err(format!(
            "The saved media queue is limited to {MAX_QUEUE_ENTRIES} analyzed files."
        ));
    }
    let mut selected = std::collections::BTreeSet::new();
    for path in &queue.selected_media_paths {
        if path.is_empty() || path.len() > 8192 {
            return Err("A selected media path is empty or too long.".into());
        }
        if !selected.insert(path) {
            return Err("The selected media list contains duplicate paths.".into());
        }
        if !known_paths.contains(path) {
            return Err("Every selected path must belong to a saved queue source.".into());
        }
    }
    for (path, tune) in &queue.file_tune_overrides {
        if !known_paths.contains(path) || !["animation", "none"].contains(&tune.as_str()) {
            return Err("A saved animation choice is not attached to a queued file.".into());
        }
    }
    for (path, streams) in &queue.file_stream_overrides {
        if !known_paths.contains(path) || streams.is_empty() || streams.len() > 1024 {
            return Err("A saved stream selection is not attached to a queued file.".into());
        }
        let unique: std::collections::BTreeSet<_> = streams.iter().collect();
        if unique.len() != streams.len() {
            return Err("A saved stream selection contains duplicate track indices.".into());
        }
    }
    if queue.run_states.len() > MAX_RUN_STATES {
        return Err(format!(
            "The media queue can retain at most {MAX_RUN_STATES} per-file run states."
        ));
    }
    for (path, state) in &queue.run_states {
        if !known_paths.contains(path) {
            return Err("A saved run state must belong to an analyzed queue file.".into());
        }
        if ![
            "queued",
            "running",
            "scanning",
            "renaming",
            "remuxing",
            "rebuilding",
            "validating",
            "renamed",
            "completed",
            "failed",
            "skipped",
            "interrupted",
            "needs_review",
        ]
        .contains(&state.status.as_str())
        {
            return Err("A saved per-file run state has an unsupported status.".into());
        }
        if state.mode.as_deref().is_some_and(|mode| {
            !["check", "metadata", "test", "full", "youtube", "organize"].contains(&mode)
        }) {
            return Err("A saved per-file run mode is unsupported.".into());
        }
        for (label, value, maximum, must_be_absolute) in [
            ("run message", state.message.as_deref(), 4096, false),
            ("run output path", state.output_path.as_deref(), 8192, true),
            ("run report path", state.report_path.as_deref(), 8192, true),
            (
                "run report status",
                state.report_status.as_deref(),
                128,
                false,
            ),
        ] {
            if let Some(value) = value {
                if value.len() > maximum {
                    return Err(format!("A saved {label} is too long."));
                }
                if must_be_absolute && !value.is_empty() && !Path::new(value).is_absolute() {
                    return Err(format!("A saved {label} must be an absolute path."));
                }
            }
        }
    }
    if queue.movie_reviews.len() > MAX_MOVIE_REVIEWS {
        return Err(format!(
            "The media queue can retain at most {MAX_MOVIE_REVIEWS} reviewed movie records."
        ));
    }
    for (path, review) in &queue.movie_reviews {
        if path.trim().is_empty() || path.len() > 8192 || !Path::new(path).is_absolute() {
            return Err("A saved movie review must use an absolute media path.".into());
        }
        for (label, value, maximum) in [
            ("title", review.title.as_str(), 1024),
            ("year", review.year.as_str(), 64),
            ("certification", review.certification.as_str(), 128),
            ("provider record", review.record_id.as_str(), 1024),
            (
                "short description",
                review.short_description.as_str(),
                16_384,
            ),
            ("plot", review.plot.as_str(), 65_536),
            ("genres", review.genres_text.as_str(), 4096),
            ("rating", review.rating_text.as_str(), 64),
            ("source URL", review.source_url.as_str(), 4096),
            ("comment", review.comment.as_str(), 500),
            ("poster selection", review.poster_id.as_str(), 1024),
            ("local poster path", review.local_poster_path.as_str(), 8192),
            ("NFO error", review.nfo_error.as_str(), 4096),
        ] {
            if value.len() > maximum {
                return Err(format!("A saved movie review {label} is too long."));
            }
        }
        if !review.source_url.is_empty() && !review.source_url.starts_with("https://") {
            return Err("A saved movie review source URL must use HTTPS.".into());
        }
        if !review.local_poster_path.is_empty() && !saved_poster_reference(&review.local_poster_path)
        {
            return Err("A saved local movie poster must use an absolute path.".into());
        }
        if review.nfo_paths.len() > 32
            || review
                .nfo_paths
                .iter()
                .any(|candidate| candidate.len() > 8192 || !Path::new(candidate).is_absolute())
        {
            return Err("Saved movie NFO paths must be absolute and limited to 32 files.".into());
        }
        if review.artwork_candidates.len() > MAX_ARTWORK_CANDIDATES {
            return Err(format!(
                "A movie review can retain at most {MAX_ARTWORK_CANDIDATES} poster choices."
            ));
        }
        let mut artwork_ids = std::collections::BTreeSet::new();
        for artwork in &review.artwork_candidates {
            if artwork.id.is_empty()
                || artwork.id.len() > 1024
                || !artwork_ids.insert(artwork.id.as_str())
            {
                return Err("Saved poster choices must have unique, bounded IDs.".into());
            }
            for (label, value) in [
                ("provider", artwork.provider.as_str()),
                ("title", artwork.title.as_str()),
                ("image URL", artwork.image_url.as_str()),
                ("thumbnail URL", artwork.thumbnail_url.as_str()),
                ("source URL", artwork.source_url.as_str()),
            ] {
                if value.len() > 4096 {
                    return Err(format!("A saved poster {label} is too long."));
                }
            }
            for (label, value) in [
                ("creator", artwork.creator.as_deref()),
                ("license", artwork.license.as_deref()),
                ("license URL", artwork.license_url.as_deref()),
                ("attribution", artwork.attribution.as_deref()),
            ] {
                if value.is_some_and(|text| text.len() > 4096) {
                    return Err(format!("A saved poster {label} is too long."));
                }
            }
            for url in [
                artwork.image_url.as_str(),
                artwork.thumbnail_url.as_str(),
                artwork.source_url.as_str(),
                artwork.license_url.as_deref().unwrap_or_default(),
            ] {
                if !url.is_empty() && !url.starts_with("https://") {
                    return Err("Saved poster links must use HTTPS.".into());
                }
            }
        }
        if !review.poster_id.is_empty() && !artwork_ids.contains(review.poster_id.as_str()) {
            return Err(
                "A saved poster selection must match one of its reviewed poster choices.".into(),
            );
        }
    }
    if queue.series_reviews.len() > MAX_SERIES_REVIEWS {
        return Err(format!(
            "The media queue can retain at most {MAX_SERIES_REVIEWS} reviewed TV series."
        ));
    }
    for (root, review) in &queue.series_reviews {
        if root.trim().is_empty() || root.len() > 8192 || !Path::new(root).is_absolute() {
            return Err("A saved series review must use an absolute series folder path.".into());
        }
        for (label, value, maximum) in [
            ("title", review.title.as_str(), 1024),
            ("year", review.year.as_str(), 64),
            ("certification", review.certification.as_str(), 128),
            ("provider record", review.record_id.as_str(), 1024),
            (
                "short description",
                review.short_description.as_str(),
                16_384,
            ),
            ("plot", review.plot.as_str(), 65_536),
            ("genres", review.genres_text.as_str(), 4096),
            ("rating", review.rating_text.as_str(), 64),
            ("source URL", review.source_url.as_str(), 4096),
            ("comment", review.comment.as_str(), 500),
            ("poster selection", review.poster_id.as_str(), 1024),
            ("local poster path", review.local_poster_path.as_str(), 8192),
            ("NFO error", review.nfo_error.as_str(), 4096),
        ] {
            if value.len() > maximum {
                return Err(format!("A saved series review {label} is too long."));
            }
        }
        if !review.source_url.is_empty() && !review.source_url.starts_with("https://") {
            return Err("A saved series review source URL must use HTTPS.".into());
        }
        if !review.local_poster_path.is_empty() && !saved_poster_reference(&review.local_poster_path)
        {
            return Err("A saved local series poster must use an absolute path.".into());
        }
        if review.local_season_poster_paths.len() > 200
            || review
                .local_season_poster_paths
                .iter()
                .any(|(season, path)| {
                    !season
                        .parse::<u32>()
                        .is_ok_and(|number| number.to_string() == *season)
                        || !saved_poster_reference(path)
                })
        {
            return Err(
                "Saved local season posters must use bounded season numbers and absolute paths."
                    .into(),
            );
        }
        if review.season_records.len() > 200 {
            return Err("A saved series review has too many season records.".into());
        }
        for (season, record) in &review.season_records {
            if !season
                .parse::<u32>()
                .is_ok_and(|number| number.to_string() == *season)
            {
                return Err("A saved season record must use its season number as the key.".into());
            }
            for (label, value, maximum) in [
                ("title", record.title.as_str(), 1024),
                ("year", record.year.as_str(), 64),
                ("certification", record.certification.as_str(), 128),
                ("plot", record.plot.as_str(), 65_536),
                ("genres", record.genres_text.as_str(), 4096),
                ("rating", record.rating_text.as_str(), 64),
                ("comment", record.comment.as_str(), 500),
            ] {
                if value.len() > maximum {
                    return Err(format!("A saved season {label} is too long."));
                }
            }
        }
        if review.nfo_paths.len() > 32
            || review
                .nfo_paths
                .iter()
                .any(|candidate| candidate.len() > 8192 || !Path::new(candidate).is_absolute())
        {
            return Err("Saved series NFO paths must be absolute and limited to 32 files.".into());
        }
        validate_artwork_candidates(&review.artwork_candidates, &review.poster_id, "series")?;
        if review.episodes.len() > MAX_TV_EPISODES
            || review.episode_assignments.len() > MAX_SELECTED_PATHS
        {
            return Err("Saved TV episode metadata exceeds its queue safety limit.".into());
        }
        let mut episode_keys = std::collections::BTreeSet::new();
        for episode in &review.episodes {
            validate_tv_episode(episode)?;
            if !episode_keys.insert((episode.season, episode.episode)) {
                return Err("A saved series review contains duplicate episode numbers.".into());
            }
        }
        for (path, episode) in &review.episode_assignments {
            if path.trim().is_empty() || path.len() > 8192 || !Path::new(path).is_absolute() {
                return Err("A saved episode assignment must use an absolute media path.".into());
            }
            validate_tv_episode(episode)?;
        }
    }
    if queue.match_suggestions.len() > MAX_METADATA_SUGGESTIONS {
        return Err(format!(
            "The media queue can retain at most {MAX_METADATA_SUGGESTIONS} title suggestions."
        ));
    }
    for (path, suggestion) in &queue.match_suggestions {
        if !known_paths.contains(path) {
            return Err("A title suggestion must belong to an analyzed queue file.".into());
        }
        if !["movie", "tv"].contains(&suggestion.kind.as_str())
            || !["searching", "ready", "error"].contains(&suggestion.status.as_str())
        {
            return Err("A saved title suggestion has an unsupported kind or status.".into());
        }
        for (label, value, maximum) in [
            ("suggested title", suggestion.title.as_deref(), 1024),
            ("suggested year", suggestion.year.as_deref(), 64),
            ("suggestion provider", suggestion.provider.as_deref(), 64),
            ("suggested record ID", suggestion.record_id.as_deref(), 1024),
            (
                "suggestion source URL",
                suggestion.source_url.as_deref(),
                4096,
            ),
            (
                "suggested description",
                suggestion.description.as_deref(),
                16_384,
            ),
            ("suggestion error", Some(suggestion.error.as_str()), 4096),
        ] {
            if value.is_some_and(|text| text.len() > maximum) {
                return Err(format!("A saved {label} is too long."));
            }
        }
        if suggestion
            .provider
            .as_deref()
            .is_some_and(|provider| !["wikidata", "tvmaze", "tvdb", "tmdb"].contains(&provider))
        {
            return Err("A title suggestion uses an unsupported metadata provider.".into());
        }
        if suggestion
            .source_url
            .as_deref()
            .is_some_and(|url| !url.is_empty() && !url.starts_with("https://"))
        {
            return Err("A title suggestion source URL must use HTTPS.".into());
        }
        if suggestion.auto_associated
            && (suggestion.status != "ready"
                || suggestion.title.as_deref().is_none_or(str::is_empty)
                || suggestion.year.as_deref().is_none_or(str::is_empty)
                || suggestion.record_id.as_deref().is_none_or(str::is_empty)
                || suggestion.provider.is_none())
        {
            return Err("An automatic title association requires a ready, exact-title/year provider record.".into());
        }
        if let Some(artwork) = &suggestion.artwork {
            validate_artwork_candidates(std::slice::from_ref(artwork), "", "metadata suggestion")?;
        }
    }
    Ok(())
}

fn validate_tv_episode(episode: &QueueTvEpisode) -> Result<(), String> {
    for (label, value, maximum) in [
        ("title", episode.title.as_str(), 1024),
        (
            "air date",
            episode.airdate.as_deref().unwrap_or_default(),
            64,
        ),
        (
            "summary",
            episode.summary.as_deref().unwrap_or_default(),
            16_384,
        ),
        ("source URL", episode.source_url.as_str(), 4096),
        ("comment", episode.comment.as_str(), 500),
        ("certification", episode.certification.as_str(), 64),
    ] {
        if value.len() > maximum {
            return Err(format!("A saved episode {label} is too long."));
        }
    }
    if episode.episode == 0 {
        return Err("A saved episode number must be greater than zero.".into());
    }
    if !episode.source_url.is_empty() && !episode.source_url.starts_with("https://") {
        return Err("A saved episode source URL must use HTTPS.".into());
    }
    Ok(())
}

fn validate_artwork_candidates(
    candidates: &[QueueArtworkCandidate],
    selected_id: &str,
    record_kind: &str,
) -> Result<(), String> {
    if candidates.len() > MAX_ARTWORK_CANDIDATES {
        return Err(format!(
            "A {record_kind} review can retain at most {MAX_ARTWORK_CANDIDATES} poster choices."
        ));
    }
    let mut artwork_ids = std::collections::BTreeSet::new();
    for artwork in candidates {
        if artwork.id.is_empty()
            || artwork.id.len() > 1024
            || !artwork_ids.insert(artwork.id.as_str())
        {
            return Err("Saved poster choices must have unique, bounded IDs.".into());
        }
        for (label, value) in [
            ("provider", artwork.provider.as_str()),
            ("title", artwork.title.as_str()),
            ("image URL", artwork.image_url.as_str()),
            ("thumbnail URL", artwork.thumbnail_url.as_str()),
            ("source URL", artwork.source_url.as_str()),
        ] {
            if value.len() > 4096 {
                return Err(format!("A saved poster {label} is too long."));
            }
        }
        for (label, value) in [
            ("creator", artwork.creator.as_deref()),
            ("license", artwork.license.as_deref()),
            ("license URL", artwork.license_url.as_deref()),
            ("attribution", artwork.attribution.as_deref()),
        ] {
            if value.is_some_and(|text| text.len() > 4096) {
                return Err(format!("A saved poster {label} is too long."));
            }
        }
        for url in [
            artwork.image_url.as_str(),
            artwork.thumbnail_url.as_str(),
            artwork.source_url.as_str(),
            artwork.license_url.as_deref().unwrap_or_default(),
        ] {
            if !url.is_empty() && !url.starts_with("https://") {
                return Err("Saved poster links must use HTTPS.".into());
            }
        }
    }
    if !selected_id.is_empty() && !artwork_ids.contains(selected_id) {
        return Err(
            "A saved poster selection must match one of its reviewed poster choices.".into(),
        );
    }
    Ok(())
}

fn inspect_cached_files(source: &mut QueueSource) {
    let Some(entries) = source
        .scan_result
        .as_ref()
        .and_then(|result| result.get("entries"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for entry in entries {
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        match fs::metadata(path) {
            Err(_) => source.missing_files.push(path.to_owned()),
            Ok(metadata) => {
                let stored_size = entry.get("size_bytes").and_then(Value::as_u64);
                let stored_mtime = entry.get("modified_unix_ms").and_then(Value::as_u64);
                let current_mtime = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64);
                if stored_size.is_some_and(|size| size != metadata.len())
                    || stored_mtime.is_some_and(|mtime| Some(mtime) != current_mtime)
                {
                    source.changed_files.push(path.to_owned());
                }
            }
        }
    }
}

fn preserve_unreadable(path: &Path) -> Result<PathBuf, String> {
    let suffix = now_ms();
    let mut backup = path.as_os_str().to_os_string();
    backup.push(format!(".corrupt-{suffix}"));
    let backup = PathBuf::from(backup);
    fs::rename(path, &backup).map_err(|error| {
        format!(
            "The saved media queue is unreadable and could not be preserved at {}: {error}",
            backup.display()
        )
    })?;
    Ok(backup)
}

fn write_atomically(path: &Path, payload: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("The media queue path has no parent folder.")?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create the media queue folder {}: {error}",
            parent.display()
        )
    })?;

    let mut temporary_name = path.as_os_str().to_os_string();
    temporary_name.push(format!(".tmp-{}-{}", std::process::id(), now_ms()));
    let temporary = PathBuf::from(temporary_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Could not create a temporary media queue file: {error}"))?;
    let write_result = (|| {
        file.write_all(payload)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not save the media queue: {error}"));
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_synthetic_scan(path: &Path, directory: &Path, media_paths: &[PathBuf]) {
        let entries = media_paths
            .iter()
            .map(|media_path| {
                json!({
                    "path": media_path,
                    "relative_path": media_path.file_name().and_then(|name| name.to_str()).unwrap_or("sample.mkv"),
                    "name": media_path.file_name().and_then(|name| name.to_str()).unwrap_or("sample.mkv"),
                    "size_bytes": 1234,
                    "status": "Ready",
                    "error": null,
                    "rawProbePayload": {"privateTestMarker": true},
                    "profile": {
                        "video": {"codec": "hevc", "width": 1920, "height": 1080},
                        "hdr": {"hdr10": true, "hdr10plus": false, "dolby_vision": false}
                    }
                })
            })
            .collect::<Vec<_>>();
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: directory.to_string_lossy().into_owned(),
            status: QueueSourceStatus::Ready,
            scan_result: Some(json!({"entries": entries})),
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        save(path, queue).expect("save synthetic queue fixture");
    }

    #[test]
    fn queue_summary_is_paged_and_omits_raw_scan_payloads() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-queue-summary-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let first = directory.join("one.mkv");
        let second = directory.join("two.mkv");
        fs::write(&first, []).expect("create synthetic path");
        fs::write(&second, []).expect("create synthetic path");
        let queue_path = directory.join("library-queue.json");
        save_synthetic_scan(&queue_path, &directory, &[first.clone(), second.clone()]);

        let page = summarize(&queue_path, 1, 1).expect("read bounded queue summary");
        assert_eq!(page["fileCount"], 2);
        assert_eq!(page["offset"], 1);
        assert_eq!(page["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            page["entries"][0]["path"],
            second.to_string_lossy().as_ref()
        );
        assert_eq!(page["entries"][0]["videoCodec"], "hevc");
        assert_eq!(page["entries"][0]["hdr10"], true);
        assert!(page["entries"][0].get("rawProbePayload").is_none());
        assert!(summarize(&queue_path, 0, MAX_QUEUE_PAGE_SIZE + 1).is_err());
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn queue_selection_update_preserves_other_queue_decisions_and_rejects_unknown_paths() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-queue-selection-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let media_path = directory.join("sample.mkv");
        fs::write(&media_path, []).expect("create synthetic path");
        let queue_path = directory.join("library-queue.json");
        save_synthetic_scan(&queue_path, &directory, &[media_path.clone()]);
        let mut queue = load(&queue_path).expect("load fixture").queue;
        queue.file_tune_overrides.insert(
            media_path.to_string_lossy().into_owned(),
            "animation".into(),
        );
        save(&queue_path, queue).expect("save existing tune decision");

        save_selected_paths(&queue_path, vec![media_path.to_string_lossy().into_owned()])
            .expect("select analyzed file");
        let selected = load(&queue_path).expect("reload selected queue").queue;
        assert_eq!(
            selected.selected_media_paths,
            vec![media_path.to_string_lossy()]
        );
        assert_eq!(
            selected.file_tune_overrides[media_path.to_string_lossy().as_ref()],
            "animation"
        );

        let unknown = directory.join("outside.mkv");
        let error = save_selected_paths(&queue_path, vec![unknown.to_string_lossy().into_owned()])
            .expect_err("reject a path not present in the queue");
        assert!(error.contains("belong to a saved queue source"));
        let unchanged = load(&queue_path)
            .expect("selection remains unchanged")
            .queue;
        assert_eq!(
            unchanged.selected_media_paths,
            selected.selected_media_paths
        );
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn metadata_suggestions_persist_for_analyzed_paths_without_replacing_queue_state() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-queue-suggestions-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let media_path = directory.join("Gnomeo & Juliet (2011).mkv");
        fs::write(&media_path, []).expect("create synthetic media path");
        let media_path_string = media_path.to_string_lossy().into_owned();
        let queue_path = directory.join("library-queue.json");
        save_synthetic_scan(&queue_path, &directory, &[media_path.clone()]);
        save_selected_paths(&queue_path, vec![media_path_string.clone()])
            .expect("save global queue selection");

        save_metadata_suggestions(
            &queue_path,
            BTreeMap::from([(
                media_path_string.clone(),
                QueueMetadataSuggestion {
                    kind: "movie".into(),
                    status: "ready".into(),
                    title: Some("Gnomeo & Juliet".into()),
                    year: Some("2011".into()),
                    provider: Some("tmdb".into()),
                    record_id: Some("43".into()),
                    source_url: Some("https://www.themoviedb.org/movie/43".into()),
                    description: None,
                    auto_associated: false,
                    artwork: None,
                    error: String::new(),
                },
            )]),
        )
        .expect("save queue title suggestion");

        let queue = load(&queue_path).expect("reload queue").queue;
        let suggestion = queue.match_suggestions.get(&media_path_string).unwrap();
        assert_eq!(suggestion.title.as_deref(), Some("Gnomeo & Juliet"));
        assert_eq!(suggestion.provider.as_deref(), Some("tmdb"));
        assert_eq!(queue.selected_media_paths, vec![media_path_string]);
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn metadata_suggestions_reject_unknown_paths_and_unapproved_providers() {
        let unknown = QueueMetadataSuggestion {
            kind: "movie".into(),
            status: "ready".into(),
            title: Some("Unknown Film".into()),
            year: None,
            provider: Some("tmdb".into()),
            record_id: None,
            source_url: None,
            description: None,
            auto_associated: false,
            artwork: None,
            error: String::new(),
        };
        let mut queue = QueueDocument::default();
        queue
            .match_suggestions
            .insert("/media/not-scanned.mkv".into(), unknown.clone());
        assert!(validate(&queue)
            .unwrap_err()
            .contains("analyzed queue file"));

        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-queue-suggestion-validation-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let media_path = directory.join("sample.mkv");
        fs::write(&media_path, []).expect("create synthetic media path");
        let mut scanned = QueueDocument::default();
        scanned.sources.push(QueueSource {
            root: directory.to_string_lossy().into_owned(),
            status: QueueSourceStatus::Ready,
            scan_result: Some(json!({"entries": [{"path": media_path}]})),
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        let mut unsupported = unknown;
        unsupported.provider = Some("untrusted-provider".into());
        scanned
            .match_suggestions
            .insert(media_path.to_string_lossy().into_owned(), unsupported);
        assert!(validate(&scanned)
            .unwrap_err()
            .contains("unsupported metadata provider"));
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    fn sample_review() -> QueueMovieReview {
        QueueMovieReview {
            title: "Gnomeo & Juliet".into(),
            year: "2011".into(),
            certification: "G".into(),
            record_id: "tmdb:41733".into(),
            short_description: String::new(),
            plot: String::new(),
            genres_text: "Animation, Family".into(),
            rating_text: String::new(),
            source_url: "https://www.themoviedb.org/movie/41733".into(),
            comment: String::new(),
            artwork_candidates: vec![QueueArtworkCandidate {
                id: "tmdb:poster-1".into(),
                provider: "TMDB".into(),
                title: "Gnomeo & Juliet poster".into(),
                image_url: "https://image.tmdb.org/t/p/original/poster.jpg".into(),
                thumbnail_url: "https://image.tmdb.org/t/p/w342/poster.jpg".into(),
                source_url: "https://www.themoviedb.org/movie/41733".into(),
                creator: None,
                license: Some("TMDB terms".into()),
                license_url: Some("https://www.themoviedb.org/api-terms-of-use".into()),
                attribution: Some("Metadata provided by TMDB.".into()),
            }],
            poster_id: "tmdb:poster-1".into(),
            local_poster_path: "/media/Gnomeo & Juliet-poster.jpg".into(),
            nfo_paths: vec!["/media/Gnomeo & Juliet.nfo".into()],
            nfo_error: String::new(),
        }
    }

    fn sample_series_review() -> QueueSeriesReview {
        QueueSeriesReview {
            title: "Example Series".into(),
            year: "2019–2021".into(),
            certification: "TV-14".into(),
            record_id: "tvmaze:42".into(),
            short_description: String::new(),
            plot: "A reviewed series summary.".into(),
            genres_text: "Drama, Mystery".into(),
            rating_text: "8.4".into(),
            source_url: "https://www.tvmaze.com/shows/42/example-series".into(),
            comment: String::new(),
            artwork_candidates: sample_review().artwork_candidates,
            poster_id: "tmdb:poster-1".into(),
            local_poster_path: "/media/Example Series/poster.jpg".into(),
            local_season_poster_paths: BTreeMap::from([(
                "1".into(),
                "/media/Example Series/Season 01/poster.jpg".into(),
            )]),
            season_records: BTreeMap::new(),
            nfo_paths: vec!["/media/Example Series/tvshow.nfo".into()],
            nfo_error: String::new(),
            episodes: vec![QueueTvEpisode {
                season: 1,
                episode: 2,
                title: "The Second Hour".into(),
                airdate: Some("2019-05-14".into()),
                summary: Some("A reviewed episode summary.".into()),
                source_url: "https://www.tvmaze.com/episodes/2002/example-series-1x02".into(),
                comment: String::new(),
                certification: String::new(),
            }],
            episode_assignments: BTreeMap::from([(
                "/media/Example Series/Season 01/episode.mkv".into(),
                QueueTvEpisode {
                    season: 1,
                    episode: 2,
                    title: "The Second Hour".into(),
                    airdate: Some("2019-05-14".into()),
                    summary: Some("Edited episode summary.".into()),
                    source_url: "https://www.tvmaze.com/episodes/2002/example-series-1x02".into(),
                    comment: String::new(),
                    certification: String::new(),
                },
            )]),
        }
    }

    #[test]
    fn older_queue_documents_default_to_no_movie_reviews() {
        let queue = serde_json::from_value::<QueueDocument>(serde_json::json!({
            "version": QUEUE_VERSION,
            "sources": [],
            "selectedMediaPaths": [],
            "fileTuneOverrides": {},
            "fileStreamOverrides": {},
            "activeRoot": null,
            "updatedAtMs": 0
        }))
        .expect("older queue schema remains readable");
        assert!(queue.movie_reviews.is_empty());
        assert!(queue.series_reviews.is_empty());
        assert!(queue.match_suggestions.is_empty());
        assert!(queue.run_states.is_empty());
        assert!(validate(&queue).is_ok());
    }

    #[test]
    fn queue_load_turns_only_active_run_states_into_interrupted() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-run-state-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let active_path = directory.join("active.mkv");
        let pending_path = directory.join("pending.mkv");
        fs::write(&active_path, b"synthetic active media").expect("create active fixture");
        fs::write(&pending_path, b"synthetic pending media").expect("create pending fixture");
        let active = active_path.to_string_lossy().into_owned();
        let pending = pending_path.to_string_lossy().into_owned();
        let queue_path = directory.join("library-queue.json");
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: directory.to_string_lossy().into_owned(),
            status: QueueSourceStatus::Ready,
            scan_result: Some(serde_json::json!({
                "entries": [
                    {"path": active, "size_bytes": fs::metadata(&active_path).unwrap().len()},
                    {"path": pending, "size_bytes": fs::metadata(&pending_path).unwrap().len()}
                ]
            })),
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        queue.run_states.insert(
            active.clone(),
            QueueRunState {
                status: "rebuilding".into(),
                mode: Some("full".into()),
                message: None,
                output_path: None,
                report_path: None,
                report_status: None,
                updated_at_ms: now_ms(),
            },
        );
        queue.run_states.insert(
            pending.clone(),
            QueueRunState {
                status: "queued".into(),
                mode: Some("full".into()),
                message: None,
                output_path: None,
                report_path: None,
                report_status: None,
                updated_at_ms: now_ms(),
            },
        );
        save(&queue_path, queue).expect("save interrupted-run fixture");

        let loaded = load(&queue_path).expect("load and recover queue");
        let states = loaded.queue.run_states;
        assert_eq!(states[&active].status, "interrupted");
        assert!(states[&active]
            .message
            .as_deref()
            .unwrap()
            .contains("while this item was active"));
        assert_eq!(states[&pending].status, "queued");
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn interrupted_session_keeps_the_original_and_removes_only_its_temporary_encode() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-recovery-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated recovery directory");
        let source = directory.join("Chaos Theory.mkv");
        let finished = directory.join("Finished.mkv");
        let output = directory.join("Finished.hevc.mkv");
        let missing = directory.join("Missing.mkv");
        fs::write(&source, b"original-video").expect("create original");
        fs::write(&finished, b"finished-source").expect("create finished source");
        fs::write(&output, b"validated-output").expect("create finished output");
        let temp = directory.join(
            ".Chaos Theory.hevc-rebuild.abcdef0123456789abcdef0123456789.tmp.mkv",
        );
        fs::write(&temp, b"partial-encode").expect("create temporary encode");
        fs::create_dir(directory.join("old")).expect("create backup folder");
        fs::write(directory.join("old").join("Chaos Theory.mkv"), b"older-backup")
            .expect("create backup");
        fs::write(directory.join("Chaos Theory.hevc-rebuild.nfo"), b"nfo").expect("create nfo");
        let queue_path = directory.join("library-queue.json");
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: directory.to_string_lossy().into_owned(),
            status: QueueSourceStatus::Ready,
            scan_result: Some(serde_json::json!({
                "entries": [
                    {"path": source.to_string_lossy(), "size_bytes": 15},
                    {"path": finished.to_string_lossy(), "size_bytes": 16},
                    {"path": missing.to_string_lossy(), "size_bytes": 1}
                ]
            })),
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        queue.run_states.insert(
            source.to_string_lossy().into_owned(),
            QueueRunState {
                status: "rebuilding".into(),
                mode: Some("full".into()),
                message: None,
                output_path: None,
                report_path: None,
                report_status: None,
                updated_at_ms: now_ms(),
            },
        );
        queue.run_states.insert(
            finished.to_string_lossy().into_owned(),
            QueueRunState {
                status: "validating".into(),
                mode: Some("full".into()),
                message: None,
                output_path: Some(output.to_string_lossy().into_owned()),
                report_path: None,
                report_status: Some("OK".into()),
                updated_at_ms: now_ms(),
            },
        );
        queue.run_states.insert(
            missing.to_string_lossy().into_owned(),
            QueueRunState {
                status: "running".into(),
                mode: Some("full".into()),
                message: None,
                output_path: None,
                report_path: None,
                report_status: None,
                updated_at_ms: now_ms(),
            },
        );
        save(&queue_path, queue).expect("save recovery fixture");

        let recovery = reconcile_interrupted(&queue_path).expect("recover interrupted files");
        assert!(!recovery.skipped);
        assert_eq!(recovery.cleared, 1);
        assert_eq!(recovery.completed, 1);
        assert_eq!(recovery.held, 1);
        assert_eq!(recovery.removed_temporary_files, 1);
        assert!(!temp.exists());
        assert_eq!(fs::read(&source).unwrap(), b"original-video");
        assert_eq!(fs::read(directory.join("old").join("Chaos Theory.mkv")).unwrap(), b"older-backup");
        assert_eq!(fs::read(directory.join("Chaos Theory.hevc-rebuild.nfo")).unwrap(), b"nfo");
        assert_eq!(fs::read(&output).unwrap(), b"validated-output");
        assert!(!recovery.run_states.contains_key(source.to_string_lossy().as_ref()));
        assert_eq!(
            recovery.run_states[&finished.to_string_lossy().into_owned()].status,
            "completed"
        );
        assert_eq!(
            recovery.run_states[&missing.to_string_lossy().into_owned()].status,
            "needs_review"
        );
        fs::remove_dir_all(directory).expect("remove isolated recovery directory");
    }

    #[test]
    fn operation_stage_states_are_active_until_they_reach_a_terminal_state() {
        for status in [
            "running",
            "scanning",
            "renaming",
            "remuxing",
            "rebuilding",
            "validating",
        ] {
            assert!(
                is_active_run_status(status),
                "{status} should be recovered as interrupted"
            );
        }
        for status in [
            "queued",
            "renamed",
            "completed",
            "failed",
            "skipped",
            "interrupted",
            "needs_review",
        ] {
            assert!(
                !is_active_run_status(status),
                "{status} should remain intact after reload"
            );
        }
    }

    #[test]
    fn run_states_require_known_paths_and_supported_values() {
        let mut queue = QueueDocument::default();
        queue.run_states.insert(
            "/media/missing.mkv".into(),
            QueueRunState {
                status: "completed".into(),
                mode: Some("full".into()),
                message: None,
                output_path: None,
                report_path: None,
                report_status: None,
                updated_at_ms: now_ms(),
            },
        );
        assert!(validate(&queue)
            .unwrap_err()
            .contains("analyzed queue file"));
    }

    #[test]
    fn movie_review_keeps_edited_fields_and_selected_attributed_poster() {
        let mut queue = QueueDocument::default();
        queue
            .movie_reviews
            .insert("/media/film.mkv".into(), sample_review());
        assert!(validate(&queue).is_ok());
        let encoded = serde_json::to_value(&queue).expect("queue serializes");
        let restored = serde_json::from_value::<QueueDocument>(encoded).expect("queue restores");
        let restored_review = restored.movie_reviews.get("/media/film.mkv").unwrap();
        assert_eq!(restored_review.title, "Gnomeo & Juliet");
        assert_eq!(restored_review.poster_id, "tmdb:poster-1");
        assert_eq!(
            restored_review.local_poster_path,
            "/media/Gnomeo & Juliet-poster.jpg"
        );
        assert_eq!(
            restored_review.artwork_candidates[0].license.as_deref(),
            Some("TMDB terms")
        );
    }

    #[test]
    fn older_movie_and_series_reviews_default_local_artwork_choices() {
        let mut movie = serde_json::to_value(sample_review()).expect("movie review serializes");
        movie.as_object_mut().unwrap().remove("localPosterPath");
        let restored_movie = serde_json::from_value::<QueueMovieReview>(movie)
            .expect("older movie review remains readable");
        assert!(restored_movie.local_poster_path.is_empty());

        let mut series =
            serde_json::to_value(sample_series_review()).expect("series review serializes");
        series.as_object_mut().unwrap().remove("localPosterPath");
        series
            .as_object_mut()
            .unwrap()
            .remove("localSeasonPosterPaths");
        let restored_series = serde_json::from_value::<QueueSeriesReview>(series)
            .expect("older series review remains readable");
        assert!(restored_series.local_poster_path.is_empty());
        assert!(restored_series.local_season_poster_paths.is_empty());
    }

    #[test]
    fn review_local_artwork_requires_absolute_paths_and_canonical_season_keys() {
        let mut queue = QueueDocument::default();
        let mut movie = sample_review();
        movie.local_poster_path = "posters/film.jpg".into();
        queue.movie_reviews.insert("/media/film.mkv".into(), movie);
        assert!(validate(&queue).unwrap_err().contains("absolute"));

        queue.movie_reviews.clear();
        let mut series = sample_series_review();
        series
            .local_season_poster_paths
            .insert("01".into(), "/media/show/Season 01/poster.jpg".into());
        queue.series_reviews.insert("/media/show".into(), series);
        assert!(validate(&queue)
            .unwrap_err()
            .contains("bounded season numbers"));

        let mut series = sample_series_review();
        series.local_poster_path = "https://image.tmdb.org/t/p/original/series.jpg".into();
        series
            .local_season_poster_paths
            .insert("1".into(), "https://artworks.thetvdb.com/banners/season.jpg".into());
        queue.series_reviews.insert("/media/show".into(), series);
        assert!(validate(&queue).is_ok());
    }

    #[test]
    fn saving_movie_reviews_preserves_existing_queue_sources() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-queue-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let queue_path = directory.join("library-queue.json");
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: "/media/library".into(),
            status: QueueSourceStatus::Ready,
            scan_result: None,
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        queue.active_root = Some("/media/library".into());
        save(&queue_path, queue).expect("save source queue");

        let mut reviews = BTreeMap::new();
        reviews.insert("/media/film.mkv".into(), sample_review());
        save_movie_reviews(&queue_path, reviews).expect("save movie review map");

        let bytes = fs::read(&queue_path).expect("read saved queue");
        let restored =
            serde_json::from_slice::<QueueDocument>(&bytes).expect("restore saved queue");
        assert_eq!(restored.sources.len(), 1);
        assert_eq!(restored.active_root.as_deref(), Some("/media/library"));
        assert_eq!(
            restored.movie_reviews["/media/film.mkv"].title,
            "Gnomeo & Juliet"
        );
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn movie_review_rejects_unsafe_poster_urls_and_unmatched_selection() {
        let mut queue = QueueDocument::default();
        let mut review = sample_review();
        review.artwork_candidates[0].image_url = "http://example.invalid/poster.jpg".into();
        queue.movie_reviews.insert("/media/film.mkv".into(), review);
        assert!(validate(&queue).is_err());

        let mut review = sample_review();
        review.poster_id = "tmdb:unknown-poster".into();
        queue.movie_reviews.insert("/media/film.mkv".into(), review);
        assert!(validate(&queue).is_err());
    }

    #[test]
    fn series_review_round_trips_episode_matches_and_edits() {
        let mut queue = QueueDocument::default();
        queue
            .series_reviews
            .insert("/media/Example Series".into(), sample_series_review());
        assert!(validate(&queue).is_ok());
        let encoded = serde_json::to_value(&queue).expect("queue serializes");
        let restored = serde_json::from_value::<QueueDocument>(encoded).expect("queue restores");
        let review = restored
            .series_reviews
            .get("/media/Example Series")
            .unwrap();
        assert_eq!(review.title, "Example Series");
        assert_eq!(review.poster_id, "tmdb:poster-1");
        assert_eq!(review.local_poster_path, "/media/Example Series/poster.jpg");
        assert_eq!(
            review.local_season_poster_paths["1"],
            "/media/Example Series/Season 01/poster.jpg"
        );
        assert_eq!(review.episodes[0].title, "The Second Hour");
        assert_eq!(
            review.episode_assignments["/media/Example Series/Season 01/episode.mkv"]
                .summary
                .as_deref(),
            Some("Edited episode summary.")
        );
    }

    #[test]
    fn saving_series_reviews_preserves_queue_sources_and_movie_reviews() {
        let directory = std::env::temp_dir().join(format!(
            "hevc-rebuild-series-queue-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir(&directory).expect("create isolated queue test directory");
        let queue_path = directory.join("library-queue.json");
        let mut queue = QueueDocument::default();
        queue.sources.push(QueueSource {
            root: "/media/TV".into(),
            status: QueueSourceStatus::Ready,
            scan_result: None,
            error: None,
            updated_at_ms: now_ms(),
            changed_files: Vec::new(),
            missing_files: Vec::new(),
        });
        queue
            .movie_reviews
            .insert("/media/film.mkv".into(), sample_review());
        save(&queue_path, queue).expect("save existing queue");

        let reviews = BTreeMap::from([("/media/TV/Example Series".into(), sample_series_review())]);
        save_series_reviews(&queue_path, reviews).expect("save series reviews");
        let restored = serde_json::from_slice::<QueueDocument>(
            &fs::read(&queue_path).expect("read saved queue"),
        )
        .expect("restore saved queue");
        assert_eq!(restored.sources.len(), 1);
        assert_eq!(
            restored.movie_reviews["/media/film.mkv"].title,
            "Gnomeo & Juliet"
        );
        assert_eq!(
            restored.series_reviews["/media/TV/Example Series"].title,
            "Example Series"
        );
        fs::remove_dir_all(directory).expect("remove isolated queue test directory");
    }

    #[test]
    fn series_review_rejects_duplicate_episodes_and_non_https_sources() {
        let mut queue = QueueDocument::default();
        let mut review = sample_series_review();
        review.episodes.push(review.episodes[0].clone());
        queue
            .series_reviews
            .insert("/media/Example Series".into(), review);
        assert!(validate(&queue).is_err());

        let mut review = sample_series_review();
        review.episodes[0].source_url = "http://example.invalid/episode".into();
        queue
            .series_reviews
            .insert("/media/Example Series".into(), review);
        assert!(validate(&queue).is_err());
    }
}
