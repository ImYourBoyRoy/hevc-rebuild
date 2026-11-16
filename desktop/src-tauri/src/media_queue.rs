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

mod model;
mod queue_validation;
mod recovery;
mod updates;

pub(crate) use model::{
    QueueArtworkCandidate, QueueDocument, QueueLoadResult, QueueMetadataSuggestion,
    QueueMovieReview, QueueRunState, QueueSeriesReview, QueueSource, QueueSourceStatus,
    QueueTvEpisode,
};
use queue_validation::*;
use recovery::*;
pub(crate) use recovery::{
    interrupted_recovery_skipped, reconcile_interrupted, InterruptedRecovery,
};
pub(crate) use updates::{
    save_metadata_suggestions, save_movie_reviews, save_run_states, save_selected_paths,
    save_selection, save_series_reviews,
};

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
                validate(&document).map_err(<serde_json::Error as serde::de::Error>::custom)?;
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
mod tests;
