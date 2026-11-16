//! The persisted queue document and the per-source, review and run-state records inside it.

use super::*;

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
    #[serde(default)]
    pub(crate) excluded_paths: Vec<String>,
    #[serde(default)]
    pub(crate) entry_order: Vec<String>,
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
