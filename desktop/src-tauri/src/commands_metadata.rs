//! Tauri commands for filename clues, public film/TV lookups and TMDb/TVDB credentials.

use super::*;

#[tauri::command]
pub(crate) fn metadata_filename_clue(
    filename: String,
    patterns: Vec<String>,
) -> Result<library_metadata::FilenameMetadataClue, String> {
    library_metadata::filename_metadata_clue_with_patterns(&filename, &patterns)
}

#[tauri::command]
pub(crate) fn metadata_filename_clues(
    filenames: Vec<String>,
    patterns: Vec<String>,
) -> Result<Vec<library_metadata::FilenameMetadataClue>, String> {
    if filenames.len() > 10_000 {
        return Err("Filename clue requests are limited to 10,000 media items.".into());
    }
    filenames
        .iter()
        .map(|filename| library_metadata::filename_metadata_clue_with_patterns(filename, &patterns))
        .collect()
}

// Online metadata lookups are explicit GUI operations. TMDB credentials are
// retrieved only from the operating-system keyring and stay out of CLI/MCP.
#[tauri::command]
pub(crate) async fn search_public_films(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::FilmSearchResult>, String> {
    library_metadata::search_films_in_language(&query, &language).await
}

#[tauri::command]
pub(crate) async fn load_public_film_details(
    entity_id: String,
    language: String,
) -> Result<library_metadata::FilmMetadataDetails, String> {
    library_metadata::load_film_details_in_language(&entity_id, &language).await
}

#[tauri::command]
pub(crate) async fn search_public_tv_shows(
    query: String,
) -> Result<Vec<library_metadata::TvShowSearchResult>, String> {
    library_metadata::search_tv_shows(&query).await
}

#[tauri::command]
pub(crate) async fn load_public_tv_details(
    show_id: u64,
) -> Result<library_metadata::TvSeriesDetails, String> {
    library_metadata::load_tv_series_details(show_id).await
}

#[tauri::command]
pub(crate) async fn search_tmdb_movies(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::TmdbSearchResult>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::search_tmdb_movies(&query, &token, &language).await
}

#[tauri::command]
pub(crate) async fn search_tmdb_tv(
    query: String,
    language: String,
) -> Result<Vec<library_metadata::TmdbSearchResult>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::search_tmdb_tv(&query, &token, &language).await
}

#[tauri::command]
pub(crate) async fn load_tmdb_movie_details(
    id: u64,
    language: String,
) -> Result<library_metadata::TmdbMovieDetails, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::load_tmdb_movie_details(id, &token, &language).await
}

#[tauri::command]
pub(crate) async fn load_tmdb_tv_details(
    id: u64,
    language: String,
) -> Result<library_metadata::TmdbTvDetails, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::load_tmdb_tv_details(id, &token, &language).await
}

#[tauri::command]
pub(crate) async fn tmdb_tv_certification_for_tvdb(
    tvdb_id: u64,
    language: String,
) -> Result<Option<String>, String> {
    let token = read_tmdb_read_token().await?;
    library_metadata::tmdb_tv_certification_for_tvdb(tvdb_id, &token, &language).await
}

async fn read_tmdb_read_token() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::read_tmdb_token)
        .await
        .map_err(|error| format!("TMDB credential lookup task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn tmdb_credential_status(
) -> Result<metadata_credentials::TmdbCredentialStatus, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::tmdb_status)
        .await
        .map_err(|error| format!("Credential store status task failed: {error}"))
}

#[tauri::command]
pub(crate) async fn save_tmdb_read_token(token: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || metadata_credentials::save_tmdb_token(&token))
        .await
        .map_err(|error| format!("Credential save task failed: {error}"))?
}

/// Reveal the saved token only after an explicit request from the native UI.
/// This command is intentionally absent from CLI and MCP interfaces.
#[tauri::command]
pub(crate) async fn reveal_tmdb_read_token() -> Result<String, String> {
    read_tmdb_read_token().await
}

#[tauri::command]
pub(crate) async fn remove_tmdb_read_token() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::remove_tmdb_token)
        .await
        .map_err(|error| format!("Credential removal task failed: {error}"))?
}

pub(crate) async fn read_tvdb_credentials() -> Result<metadata_credentials::TvdbCredentials, String>
{
    tauri::async_runtime::spawn_blocking(metadata_credentials::read_tvdb_credentials)
        .await
        .map_err(|error| format!("TheTVDB credential lookup task failed: {error}"))?
}

/// Reveal TheTVDB credentials only after an explicit request from the native UI.
/// This command is intentionally absent from CLI and MCP interfaces.
#[tauri::command]
pub(crate) async fn reveal_tvdb_credentials(
) -> Result<metadata_credentials::TvdbCredentials, String> {
    read_tvdb_credentials().await
}

#[tauri::command]
pub(crate) async fn tvdb_credential_status(
) -> Result<metadata_credentials::TvdbCredentialStatus, String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::tvdb_status)
        .await
        .map_err(|error| format!("Credential store status task failed: {error}"))
}

#[tauri::command]
pub(crate) async fn save_tvdb_credentials(
    api_key: String,
    subscriber_pin: Option<String>,
    access_confirmed: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        metadata_credentials::save_tvdb_credentials(
            &api_key,
            subscriber_pin.as_deref().unwrap_or_default(),
            access_confirmed,
        )
    })
    .await
    .map_err(|error| format!("Credential save task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn remove_tvdb_credentials() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(metadata_credentials::remove_tvdb_credentials)
        .await
        .map_err(|error| format!("Credential removal task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn search_tvdb_titles(
    query: String,
    kind: String,
    language: String,
) -> Result<Vec<library_metadata::TvdbSearchResult>, String> {
    let credentials = read_tvdb_credentials().await?;
    library_metadata::search_tvdb_titles(
        &query,
        &kind,
        &language,
        &credentials.api_key,
        credentials.subscriber_pin.as_deref(),
    )
    .await
}

#[tauri::command]
pub(crate) async fn load_tvdb_details(
    id: u64,
    kind: String,
    language: String,
) -> Result<library_metadata::TvdbMetadataDetails, String> {
    let credentials = read_tvdb_credentials().await?;
    library_metadata::load_tvdb_details(
        id,
        &kind,
        &language,
        &credentials.api_key,
        credentials.subscriber_pin.as_deref(),
    )
    .await
}
