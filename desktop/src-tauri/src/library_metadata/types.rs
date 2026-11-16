//! Provider-neutral metadata records returned to the GUI, CLI and MCP.

use super::*;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilenameMetadataClue {
    pub(crate) title: String,
    pub(crate) year: Option<String>,
    pub(crate) year_end: Option<String>,
    pub(crate) certification: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetadataProvider {
    Wikidata,
    Wikipedia,
    WikimediaCommons,
    TvMaze,
    Tmdb,
    Tvdb,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkCandidate {
    pub(crate) id: String,
    pub(crate) provider: MetadataProvider,
    pub(crate) title: String,
    pub(crate) image_url: String,
    pub(crate) thumbnail_url: String,
    pub(crate) source_url: String,
    pub(crate) creator: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) license_url: Option<String>,
    pub(crate) attribution: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilmSearchResult {
    pub(crate) entity_id: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) year: Option<String>,
    /// Ranking hint only; the user must select and review the match.
    pub(crate) confidence: f32,
    pub(crate) source_url: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FilmMetadataDetails {
    pub(crate) entity_id: String,
    pub(crate) title: String,
    pub(crate) year: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) short_description: Option<String>,
    pub(crate) long_description: Option<String>,
    pub(crate) description_source_url: Option<String>,
    pub(crate) source_url: String,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvShowSearchResult {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) year_start: Option<String>,
    pub(crate) year_end: Option<String>,
    pub(crate) summary: Option<String>,
    /// TVmaze's audience rating, not a content/MPAA certification.
    pub(crate) audience_rating: Option<f64>,
    pub(crate) source_url: String,
    pub(crate) artwork: Vec<ArtworkCandidate>,
    pub(crate) confidence: f32,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvSeasonMetadata {
    pub(crate) number: Option<u32>,
    pub(crate) title: Option<String>,
    pub(crate) year_start: Option<String>,
    pub(crate) year_end: Option<String>,
    #[serde(default)]
    pub(crate) overview: Option<String>,
    /// Audience score from 0 to 10. Absent when this season has no published score.
    #[serde(default)]
    pub(crate) rating: Option<f64>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvEpisodeMetadata {
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) title: String,
    pub(crate) airdate: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) source_url: String,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvSeriesDetails {
    pub(crate) show: TvShowSearchResult,
    pub(crate) seasons: Vec<TvSeasonMetadata>,
    pub(crate) episodes: Vec<TvEpisodeMetadata>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbSearchResult {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) original_title: String,
    pub(crate) year: Option<String>,
    pub(crate) overview: Option<String>,
    pub(crate) original_language: Option<String>,
    pub(crate) rating: Option<f64>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
    pub(crate) source_url: String,
    /// Ranking hint only; never used to apply a match automatically.
    pub(crate) confidence: f32,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbMovieDetails {
    pub(crate) movie: TmdbSearchResult,
    pub(crate) release_date: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) runtime_minutes: Option<u32>,
    pub(crate) genres: Vec<String>,
    pub(crate) tagline: Option<String>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbEpisodeMetadata {
    pub(crate) season: u32,
    pub(crate) episode: u32,
    pub(crate) title: String,
    pub(crate) airdate: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) source_url: String,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbSeasonMetadata {
    pub(crate) number: u32,
    pub(crate) title: String,
    pub(crate) year_start: Option<String>,
    pub(crate) year_end: Option<String>,
    #[serde(default)]
    pub(crate) overview: Option<String>,
    /// Audience score from 0 to 10. Absent when this season has no published score.
    #[serde(default)]
    pub(crate) rating: Option<f64>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmdbTvDetails {
    pub(crate) show: TmdbSearchResult,
    pub(crate) overview: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) genres: Vec<String>,
    pub(crate) seasons: Vec<TmdbSeasonMetadata>,
    pub(crate) episodes: Vec<TmdbEpisodeMetadata>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvdbSearchResult {
    pub(crate) id: u64,
    pub(crate) title: String,
    pub(crate) year: Option<String>,
    pub(crate) overview: Option<String>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
    pub(crate) source_url: String,
    /// A local title similarity hint. TheTVDB's score is intentionally not
    /// presented as a match probability because its meaning is provider-defined.
    pub(crate) confidence: f32,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TvdbMetadataDetails {
    pub(crate) title: String,
    pub(crate) original_title: Option<String>,
    pub(crate) year: Option<String>,
    pub(crate) source_url: String,
    pub(crate) short_description: Option<String>,
    pub(crate) long_description: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) runtime_minutes: Option<u32>,
    pub(crate) genres: Vec<String>,
    pub(crate) rating: Option<f64>,
    pub(crate) artwork: Vec<ArtworkCandidate>,
    pub(crate) seasons: Vec<TvSeasonMetadata>,
    pub(crate) episodes: Vec<TvEpisodeMetadata>,
    pub(crate) episode_order_label: Option<String>,
    pub(crate) attribution: String,
}
