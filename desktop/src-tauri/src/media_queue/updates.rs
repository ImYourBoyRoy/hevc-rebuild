//! Partial queue saves (selection, run states, reviews, suggestions) that re-read the stored document under the write lock.

use super::*;

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
