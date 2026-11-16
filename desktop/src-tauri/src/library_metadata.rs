use regex::Regex;
use reqwest::{Client, Response, StatusCode};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

mod filename_clue;
mod tmdb;
mod tvdb;
mod tvmaze;
mod types;
mod wikimedia;

pub(crate) use filename_clue::filename_metadata_clue_with_patterns;
pub(crate) use tmdb::{
    load_tmdb_movie_details, load_tmdb_tv_details, search_tmdb_movies, search_tmdb_tv,
    tmdb_tv_certification_for_tvdb,
};
pub(crate) use tvdb::{load_tvdb_details, search_tvdb_titles};
pub(crate) use tvmaze::{load_tv_series_details, search_tv_shows};
pub(crate) use types::{
    ArtworkCandidate, FilenameMetadataClue, FilmMetadataDetails, FilmSearchResult,
    MetadataProvider, TmdbEpisodeMetadata, TmdbMovieDetails, TmdbSearchResult, TmdbSeasonMetadata,
    TmdbTvDetails, TvEpisodeMetadata, TvSeasonMetadata, TvSeriesDetails, TvShowSearchResult,
    TvdbMetadataDetails, TvdbSearchResult,
};
use wikimedia::*;
pub(crate) use wikimedia::{
    load_film_details, load_film_details_in_language, search_films, search_films_in_language,
};

const MAX_QUERY_BYTES: usize = 256;
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESULTS: usize = 15;
const MAX_FILENAME_HINT_BYTES: usize = 1024;

fn api_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(format!(
            "HEVC-Rebuild-Studio/{} (opt-in metadata lookup)",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(Duration::from_secs(25))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| format!("Could not initialize the metadata lookup client: {error}"))
}

fn validate_query(query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Enter a title before searching public metadata sources.".into());
    }
    if query.len() > MAX_QUERY_BYTES || query.chars().any(char::is_control) {
        return Err(
            "Metadata search text must be at most 256 bytes and contain no control characters."
                .into(),
        );
    }
    Ok(query.to_string())
}

pub(crate) fn validate_metadata_language(language: &str) -> Result<String, String> {
    let language = language.trim();
    let valid = match language.as_bytes() {
        [a, b] => a.is_ascii_alphabetic() && b.is_ascii_alphabetic(),
        [a, b, b'-', c, d] => {
            a.is_ascii_alphabetic()
                && b.is_ascii_alphabetic()
                && c.is_ascii_alphabetic()
                && d.is_ascii_alphabetic()
        }
        _ => false,
    };
    if !valid {
        return Err("Metadata language must use a two-letter code, optionally followed by a two-letter region (for example en-US or pt-BR).".into());
    }
    let mut normalized = language.to_ascii_lowercase();
    if let Some((_, region)) = normalized.split_once('-') {
        normalized = format!("{}-{}", &normalized[..2], region.to_ascii_uppercase());
    }
    Ok(normalized)
}

fn language_code(language: &str) -> String {
    language
        .split_once('-')
        .map(|(primary, _)| primary)
        .unwrap_or(language)
        .to_ascii_lowercase()
}

fn language_region(language: &str) -> &str {
    language
        .split_once('-')
        .map(|(_, region)| region)
        .unwrap_or("US")
}

async fn bounded_json(response: Response, provider: &str) -> Result<Value, String> {
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok());
        return Err(match retry_after {
            Some(value) => {
                format!("{provider} rate-limited this lookup; try again after {value} seconds.")
            }
            None => format!("{provider} rate-limited this lookup; wait briefly and try again."),
        });
    }
    if !status.is_success() {
        return Err(format!(
            "{provider} lookup failed with HTTP status {status}."
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JSON_BYTES as u64)
    {
        return Err(format!(
            "{provider} returned a response larger than the 8 MiB safety limit."
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Could not read {provider} response: {error}"))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(format!(
            "{provider} returned a response larger than the 8 MiB safety limit."
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{provider} returned invalid JSON: {error}"))
}

async fn get_json(client: &Client, url: &str, provider: &str) -> Result<Value, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not reach {provider}: {error}"))?;
    bounded_json(response, provider).await
}

fn published_audience_score(value: Option<&Value>) -> Option<f64> {
    let score = value.and_then(Value::as_f64)?;
    (score > 0.0 && score <= 10.0).then_some(score)
}

fn published_overview(value: Option<&Value>) -> Option<String> {
    text_value(value).filter(|text| !text.trim().is_empty())
}

fn text_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn language_text(container: Option<&Value>, language: &str) -> Option<String> {
    text_value(container?.get(language)?.get("value"))
}

fn year_prefix(value: Option<&Value>) -> Option<String> {
    let time = value?.get("time")?.as_str()?;
    let digits: String = time
        .trim_start_matches(['+', '-'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.len() == 4 {
        Some(digits)
    } else {
        None
    }
}

fn confidence(query: &str, title: &str, description: &str) -> f32 {
    let normalized = |input: &str| -> String {
        input
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let q = normalized(query);
    let t = normalized(title);
    let lexical: f32 = if q.is_empty() {
        0.0
    } else if q == t {
        0.92
    } else if t.contains(&q) || q.contains(&t) {
        0.78
    } else {
        0.45
    };
    let context = description.to_lowercase();
    (lexical
        + if context.contains("film") || context.contains("movie") {
            0.07
        } else {
            0.0
        })
    .min(0.99)
}

fn year_from_date(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(|date| date.get(0..4))
        .filter(|year| year.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
}

fn unique_artwork(candidates: Vec<ArtworkCandidate>) -> Vec<ArtworkCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests;
