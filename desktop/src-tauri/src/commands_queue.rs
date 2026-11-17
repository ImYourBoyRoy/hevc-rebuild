//! Tauri commands that load and save the persisted library queue and preview output targets.

use super::*;

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn load_library_queue() -> Result<media_queue::QueueLoadResult, String> {
    crate::runtime::spawn_blocking(|| media_queue::load(&media_queue_file()?))
        .await
        .map_err(|error| format!("Queue load task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_queue(queue: media_queue::QueueDocument) -> Result<(), String> {
    crate::runtime::spawn_blocking(move || media_queue::save(&media_queue_file()?, queue))
        .await
        .map_err(|error| format!("Queue save task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_queue_selection(
    selected_media_paths: Vec<String>,
    active_root: Option<String>,
    file_tune_overrides: BTreeMap<String, String>,
    file_stream_overrides: BTreeMap<String, Vec<u16>>,
) -> Result<(), String> {
    crate::runtime::spawn_blocking(move || {
        media_queue::save_selection(
            &media_queue_file()?,
            selected_media_paths,
            active_root,
            file_tune_overrides,
            file_stream_overrides,
        )
    })
    .await
    .map_err(|error| format!("Queue selection save task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_queue_movie_reviews(
    movie_reviews: BTreeMap<String, media_queue::QueueMovieReview>,
) -> Result<(), String> {
    crate::runtime::spawn_blocking(move || {
        media_queue::save_movie_reviews(&media_queue_file()?, movie_reviews)
    })
    .await
    .map_err(|error| format!("Queue metadata save task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_queue_series_reviews(
    series_reviews: BTreeMap<String, media_queue::QueueSeriesReview>,
) -> Result<(), String> {
    crate::runtime::spawn_blocking(move || {
        media_queue::save_series_reviews(&media_queue_file()?, series_reviews)
    })
    .await
    .map_err(|error| format!("Queue series metadata save task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn save_library_queue_metadata_suggestions(
    match_suggestions: BTreeMap<String, media_queue::QueueMetadataSuggestion>,
) -> Result<(), String> {
    crate::runtime::spawn_blocking(move || {
        media_queue::save_metadata_suggestions(&media_queue_file()?, match_suggestions)
    })
    .await
    .map_err(|error| format!("Queue metadata suggestion save task failed: {error}"))?
}

#[cfg_attr(feature = "desktop", tauri::command)]
pub(crate) async fn preview_queue_output_targets(
    request: output_targets::OutputPreviewRequest,
) -> Result<output_targets::OutputPreview, String> {
    crate::runtime::spawn_blocking(move || output_targets::preview(request))
        .await
        .map_err(|error| format!("Output preview task failed: {error}"))?
}
