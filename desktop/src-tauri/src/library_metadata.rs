use regex::Regex;
use reqwest::{Client, Response, StatusCode};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_QUERY_BYTES: usize = 256;
const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESULTS: usize = 15;
const MAX_FILENAME_HINT_BYTES: usize = 1024;

static FILENAME_YEAR_REGEX: OnceLock<Regex> = OnceLock::new();
static FILENAME_RELEASE_MARKER_REGEX: OnceLock<Regex> = OnceLock::new();
static FILENAME_CERTIFICATION_REGEX: OnceLock<Regex> = OnceLock::new();

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

/// Build a search hint from a filename or series-folder name without accepting
/// or returning a filesystem path. The hint is local until the user starts a
/// provider lookup; only `title` is sent to a metadata provider.
// Compatibility delegate retained; canonical callers/tests use the pattern-aware path.
#[allow(dead_code)]
pub(crate) fn filename_metadata_clue(filename: &str) -> Result<FilenameMetadataClue, String> {
    filename_metadata_clue_with_patterns(filename, &[])
}

pub(crate) fn filename_metadata_clue_with_patterns(
    filename: &str,
    text_blacklist_patterns: &[String],
) -> Result<FilenameMetadataClue, String> {
    let filename = filename.trim();
    if filename.is_empty()
        || filename.len() > MAX_FILENAME_HINT_BYTES
        || filename.chars().any(char::is_control)
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err("Provide only a filename or series-folder name up to 1024 bytes.".into());
    }

    let stem = filename_stem(filename);
    let year_regex = FILENAME_YEAR_REGEX.get_or_init(|| {
        Regex::new(
            r"(?:^|[^0-9])(?P<year>(?:19|20)\d{2})(?:[-–](?P<year_end>(?:19|20)\d{2}))?(?:$|[^0-9])",
        )
        .expect("filename year expression is valid")
    });
    let release_regex = FILENAME_RELEASE_MARKER_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._-])(?:\d{3,4}p|4k|8k|web[\s._-]?(?:dl|rip)|blu[\s._-]?ray|bdrip|brrip|hdtv|dvd[\s._-]?rip|remux|x26[45]|h\.?26[45]|hevc|av1|hdr10\+?|hlg|uhd|aac|ac3|eac3|ddp|dts|truehd|atmos|flac|10bit|8bit)(?:$|[\s._-])",
        )
        .expect("filename release expression is valid")
    });
    let certification_regex = FILENAME_CERTIFICATION_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._(\[])(?P<cert>NC-17|PG-13|TV-MA|TV-14|TV-PG|TV-G|TV-Y7?|UNRATED|NR|PG|G|R|X)(?:$|[\s)._\-\]])",
        )
        .expect("filename certification expression is valid")
    });

    let year_capture = year_regex.captures(stem);
    let year = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year"))
        .map(|value| value.as_str().to_string());
    let year_end = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year_end"))
        .map(|value| value.as_str().to_string());
    let year_position = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year"))
        .map(|value| value.start());
    let release_position = release_regex.find(stem).map(|matched| matched.start());
    let title_end = [year_position, release_position]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(stem.len());
    let title_fragment = &stem[..title_end];
    let title = title_fragment
        .chars()
        .map(|character| match character {
            '.' | '_' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '(' | ')' | '[' | ']' | '{' | '}' | '.' | '_' | '-'
                )
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = crate::metadata_text::clean_value(&title, text_blacklist_patterns)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        return Err("Could not find a title before the filename's year or release markers.".into());
    }
    if title.len() > MAX_QUERY_BYTES {
        return Err(
            "The filename-derived title exceeds the 256-byte metadata search limit.".into(),
        );
    }

    let certification = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year_end").or_else(|| capture.name("year")))
        .and_then(|year_match| stem.get(year_match.end()..))
        .and_then(|tail| certification_regex.captures(tail))
        .and_then(|capture| capture.name("cert"))
        .map(|value| value.as_str().to_ascii_uppercase());

    Ok(FilenameMetadataClue {
        title,
        year,
        year_end,
        certification,
    })
}

fn filename_stem(filename: &str) -> &str {
    let Some((stem, extension)) = filename.rsplit_once('.') else {
        return filename;
    };
    if matches!(
        extension.to_ascii_lowercase().as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "mov"
            | "avi"
            | "webm"
            | "ts"
            | "m2ts"
            | "mts"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "flv"
            | "ogv"
            | "vob"
    ) {
        stem
    } else {
        filename
    }
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

fn wikidata_year(entity: &Value) -> Option<String> {
    entity
        .get("claims")?
        .get("P577")?
        .as_array()?
        .iter()
        .find_map(|claim| year_prefix(claim.pointer("/mainsnak/datavalue/value")))
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

pub(crate) async fn search_films(query: &str) -> Result<Vec<FilmSearchResult>, String> {
    search_films_in_language(query, "en-US").await
}

fn trailing_year_fallback_title(query: &str) -> Option<&str> {
    let trimmed = query.trim_end_matches(|character: char| {
        character.is_whitespace() || matches!(character, ')' | ']')
    });
    let split = trimmed.len().checked_sub(4)?;
    let (title_with_separator, year) = trimmed.split_at(split);
    if !(year.starts_with("19") || year.starts_with("20"))
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || !title_with_separator
            .chars()
            .last()
            .is_some_and(|character| {
                character.is_whitespace() || matches!(character, '.' | '_' | '-' | '(' | '[')
            })
    {
        return None;
    }
    let title = title_with_separator.trim_end_matches(|character: char| {
        character.is_whitespace() || matches!(character, '.' | '_' | '-' | '(' | '[')
    });
    (!title.is_empty()).then_some(title)
}

async fn wikidata_film_search(
    client: &Client,
    query: &str,
    language_code: &str,
) -> Result<Vec<Value>, String> {
    let response = client
        .get("https://www.wikidata.org/w/api.php")
        .query(&[
            ("action", "wbsearchentities"),
            ("search", query),
            ("language", language_code),
            ("type", "item"),
            ("limit", "15"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|error| format!("Could not reach Wikidata: {error}"))?;
    let value = bounded_json(response, "Wikidata").await?;
    value
        .get("search")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "Wikidata returned no search list.".into())
}

pub(crate) async fn search_films_in_language(
    query: &str,
    language: &str,
) -> Result<Vec<FilmSearchResult>, String> {
    let query = validate_query(query)?;
    let language = validate_metadata_language(language)?;
    let language_code = language_code(&language);
    let client = api_client()?;
    let mut search_query = query.clone();
    let mut results = wikidata_film_search(&client, &search_query, &language_code).await?;
    if results.is_empty() {
        if let Some(title_only) = trailing_year_fallback_title(&query) {
            let retry = wikidata_film_search(&client, title_only, &language_code).await?;
            if !retry.is_empty() {
                search_query = title_only.to_string();
                results = retry;
            }
        }
    }
    let ids = results
        .iter()
        .take(MAX_RESULTS)
        .filter_map(|result| result.get("id").and_then(Value::as_str))
        .filter(|id| valid_wikidata_id(id))
        .collect::<Vec<_>>();
    let entity_values = if ids.is_empty() {
        Value::Null
    } else {
        let ids = ids.join("|");
        let response = client
            .get("https://www.wikidata.org/w/api.php")
            .query(&[
                ("action", "wbgetentities"),
                ("ids", ids.as_str()),
                ("props", "claims"),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(|error| format!("Could not read candidate years from Wikidata: {error}"))?;
        bounded_json(response, "Wikidata").await?
    };
    Ok(results
        .iter()
        .take(MAX_RESULTS)
        .filter_map(|result| {
            let id = result.get("id")?.as_str()?.to_string();
            if !valid_wikidata_id(&id) {
                return None;
            }
            let title = text_value(result.get("label"))?;
            let description = text_value(result.get("description")).unwrap_or_default();
            let year = entity_values
                .pointer(&format!("/entities/{id}"))
                .and_then(wikidata_year);
            Some(FilmSearchResult {
                entity_id: id.clone(),
                confidence: confidence(&search_query, &title, &description),
                title,
                description,
                year,
                source_url: format!("https://www.wikidata.org/wiki/{id}"),
            })
        })
        .collect())
}

fn valid_wikidata_id(id: &str) -> bool {
    id.strip_prefix('Q').is_some_and(|digits| {
        !digits.is_empty() && digits.len() <= 12 && digits.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn claim_label_or_string(entity: &Value, property: &str) -> Option<String> {
    let claim = entity.get("claims")?.get(property)?.as_array()?.first()?;
    let value = claim.pointer("/mainsnak/datavalue/value")?;
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.get("id").and_then(Value::as_str).map(str::to_string))
}

async fn wikidata_label(
    client: &Client,
    id: &str,
    language: &str,
) -> Result<Option<String>, String> {
    if !valid_wikidata_id(id) {
        return Ok(None);
    }
    let response = client
        .get("https://www.wikidata.org/w/api.php")
        .query(&[
            ("action", "wbgetentities"),
            ("ids", id),
            ("languages", language),
            ("props", "labels"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|error| format!("Could not read a Wikidata label: {error}"))?;
    let value = bounded_json(response, "Wikidata").await?;
    Ok(language_text(
        value.pointer(&format!("/entities/{id}/labels")),
        language,
    ))
}

fn decode_html_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extmetadata(info: &Value, key: &str) -> Option<String> {
    text_value(info.get("extmetadata")?.get(key)?.get("value"))
}

fn commons_artwork(page: &Value) -> Option<ArtworkCandidate> {
    let info = page.get("imageinfo")?.as_array()?.first()?;
    let image_url = text_value(info.get("url"))?;
    let thumbnail_url = text_value(info.get("thumburl")).unwrap_or_else(|| image_url.clone());
    let source_url = text_value(info.get("descriptionurl"))?;
    Some(ArtworkCandidate {
        id: text_value(page.get("title")).unwrap_or_else(|| source_url.clone()),
        provider: MetadataProvider::WikimediaCommons,
        title: text_value(page.get("title")).unwrap_or_else(|| "Wikimedia Commons image".into()),
        image_url,
        thumbnail_url,
        source_url,
        creator: extmetadata(info, "Artist").map(|value| decode_html_text(&value)),
        license: extmetadata(info, "LicenseShortName"),
        license_url: extmetadata(info, "LicenseUrl"),
        attribution: extmetadata(info, "Credit").map(|value| decode_html_text(&value)),
    })
}

async fn search_commons_artwork(
    client: &Client,
    query: &str,
) -> Result<Vec<ArtworkCandidate>, String> {
    let response = client
        .get("https://commons.wikimedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("generator", "search"),
            ("gsrnamespace", "6"),
            ("gsrlimit", "12"),
            ("gsrsearch", query),
            ("prop", "imageinfo"),
            ("iiprop", "url|extmetadata"),
            (
                "iiextmetadatafilter",
                "Artist|Credit|LicenseShortName|LicenseUrl",
            ),
            ("iiurlwidth", "360"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|error| format!("Could not reach Wikimedia Commons: {error}"))?;
    let value = bounded_json(response, "Wikimedia Commons").await?;
    let mut artwork: Vec<_> = value
        .pointer("/query/pages")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|pages| pages.values())
        .filter_map(commons_artwork)
        .collect();
    artwork.sort_by(|a, b| a.title.cmp(&b.title));
    artwork.truncate(12);
    Ok(artwork)
}

pub(crate) async fn load_film_details(entity_id: &str) -> Result<FilmMetadataDetails, String> {
    load_film_details_in_language(entity_id, "en-US").await
}

pub(crate) async fn load_film_details_in_language(
    entity_id: &str,
    language: &str,
) -> Result<FilmMetadataDetails, String> {
    if !valid_wikidata_id(entity_id) {
        return Err("Film metadata selection must be a Wikidata item ID.".into());
    }
    let language = validate_metadata_language(language)?;
    let language_code = language_code(&language);
    let client = api_client()?;
    let response = client
        .get("https://www.wikidata.org/w/api.php")
        .query(&[
            ("action", "wbgetentities"),
            ("ids", entity_id),
            ("languages", language_code.as_str()),
            ("props", "labels|descriptions|claims|sitelinks"),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|error| format!("Could not reach Wikidata: {error}"))?;
    let value = bounded_json(response, "Wikidata").await?;
    let entity = value
        .pointer(&format!("/entities/{entity_id}"))
        .ok_or("Wikidata returned no details for this item.")?;
    let title = language_text(entity.get("labels"), &language_code)
        .or_else(|| language_text(entity.get("labels"), "en"))
        .ok_or("Wikidata item has no title in the selected or fallback language.")?;
    let short_description = language_text(entity.get("descriptions"), &language_code)
        .or_else(|| language_text(entity.get("descriptions"), "en"));
    let wiki_language = if language_code == "en" {
        "en".to_string()
    } else {
        language_code.clone()
    };
    let wiki_key = format!("{wiki_language}wiki");
    let wiki_title = text_value(entity.pointer(&format!("/sitelinks/{wiki_key}/title")));
    let (long_description, description_source_url) = if let Some(wiki_title) = wiki_title {
        let endpoint = format!("https://{wiki_language}.wikipedia.org/w/api.php");
        let response = client
            .get(endpoint)
            .query(&[
                ("action", "query"),
                ("prop", "extracts"),
                ("explaintext", "1"),
                ("exchars", "8000"),
                ("redirects", "1"),
                ("format", "json"),
                ("formatversion", "2"),
                ("titles", wiki_title.as_str()),
            ])
            .send()
            .await
            .map_err(|error| format!("Could not reach Wikipedia: {error}"))?;
        let value = bounded_json(response, "Wikipedia").await?;
        let page = value.pointer("/query/pages/0");
        let extract = page.and_then(|page| text_value(page.get("extract")));
        let actual_title = page
            .and_then(|page| text_value(page.get("title")))
            .unwrap_or(wiki_title);
        (
            extract,
            Some(format!(
                "https://{}.wikipedia.org/wiki/{}",
                wiki_language,
                actual_title.replace(' ', "_")
            )),
        )
    } else {
        (None, None)
    };
    let year = wikidata_year(entity);
    let certification = match claim_label_or_string(entity, "P1657") {
        Some(value) if valid_wikidata_id(&value) => {
            wikidata_label(&client, &value, &language_code).await?
        }
        value => value,
    };
    let artwork = search_commons_artwork(&client, &format!("{} film poster", title)).await?;
    Ok(FilmMetadataDetails {
        entity_id: entity_id.into(),
        title,
        year,
        certification,
        short_description,
        long_description,
        description_source_url,
        source_url: format!("https://www.wikidata.org/wiki/{entity_id}"),
        artwork,
    })
}

fn year_from_date(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(|date| date.get(0..4))
        .filter(|year| year.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
}

fn tvmaze_artwork(
    id: u64,
    title: &str,
    image: Option<&Value>,
    art_kind: &str,
    source_url: &str,
) -> Vec<ArtworkCandidate> {
    let Some(image) = image else {
        return Vec::new();
    };
    let Some(original) = text_value(image.get("original")) else {
        return Vec::new();
    };
    let medium = text_value(image.get("medium")).unwrap_or_else(|| original.clone());
    vec![ArtworkCandidate {
        id: format!("tvmaze-{id}-{art_kind}"),
        provider: MetadataProvider::TvMaze,
        title: format!("{title} ({art_kind})"),
        image_url: original,
        thumbnail_url: medium,
        source_url: source_url.into(),
        creator: None,
        license: Some("CC BY-SA (TVmaze API content)".into()),
        license_url: Some("https://www.tvmaze.com/api".into()),
        attribution: Some(format!("TVmaze: {source_url}")),
    }]
}

pub(crate) async fn search_tv_shows(query: &str) -> Result<Vec<TvShowSearchResult>, String> {
    let query = validate_query(query)?;
    let client = api_client()?;
    let response = client
        .get("https://api.tvmaze.com/search/shows")
        .query(&[("q", query.as_str())])
        .send()
        .await
        .map_err(|error| format!("Could not reach TVmaze: {error}"))?;
    let value = bounded_json(response, "TVmaze").await?;
    let results = value.as_array().ok_or("TVmaze returned no show list.")?;
    Ok(results
        .iter()
        .take(MAX_RESULTS)
        .filter_map(|result| {
            let show = result.get("show")?;
            let id = show.get("id")?.as_u64()?;
            let title = text_value(show.get("name"))?;
            let source_url = text_value(show.get("url"))
                .unwrap_or_else(|| format!("https://www.tvmaze.com/shows/{id}"));
            let summary = text_value(show.get("summary")).map(|html| decode_html_text(&html));
            let supplied_score = result.get("score").and_then(Value::as_f64);
            let score = supplied_score
                .unwrap_or_else(|| confidence(&query, &title, "tv series") as f64)
                .clamp(0.0, 1.0) as f32;
            Some(TvShowSearchResult {
                id,
                title: title.clone(),
                year_start: year_from_date(show.get("premiered")),
                year_end: year_from_date(show.get("ended")),
                summary,
                audience_rating: show.pointer("/rating/average").and_then(Value::as_f64),
                artwork: tvmaze_artwork(
                    id,
                    &title,
                    show.get("image"),
                    "series poster",
                    &source_url,
                ),
                source_url,
                confidence: score,
            })
        })
        .collect())
}

fn parse_tv_season(id: u64, value: &Value) -> TvSeasonMetadata {
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let title = text_value(value.get("name"));
    let source_url = text_value(value.get("url"))
        .unwrap_or_else(|| format!("https://www.tvmaze.com/seasons/{id}"));
    let artwork = tvmaze_artwork(
        id,
        title.as_deref().unwrap_or("Season"),
        value.get("image"),
        "season poster",
        &source_url,
    );
    TvSeasonMetadata {
        number,
        title,
        year_start: year_from_date(value.get("premiereDate")),
        year_end: year_from_date(value.get("endDate")),
        overview: text_value(value.get("summary"))
            .map(|html| decode_html_text(&html))
            .filter(|text| !text.trim().is_empty()),
        rating: published_audience_score(value.pointer("/rating/average")),
        artwork,
    }
}

pub(crate) async fn load_tv_series_details(show_id: u64) -> Result<TvSeriesDetails, String> {
    if show_id == 0 {
        return Err("TVmaze show ID must be greater than zero.".into());
    }
    let client = api_client()?;
    let show_url = format!("https://api.tvmaze.com/shows/{show_id}");
    let seasons_url = format!("https://api.tvmaze.com/shows/{show_id}/seasons");
    let episodes_url = format!("https://api.tvmaze.com/shows/{show_id}/episodes?specials=1");
    let images_url = format!("https://api.tvmaze.com/shows/{show_id}/images?type=poster");
    let (show_v, seasons_v, episodes_v, images_v) = tokio::try_join!(
        get_json(&client, &show_url, "TVmaze"),
        get_json(&client, &seasons_url, "TVmaze"),
        get_json(&client, &episodes_url, "TVmaze"),
        get_json(&client, &images_url, "TVmaze"),
    )?;
    let title = text_value(show_v.get("name")).ok_or("TVmaze show has no title.")?;
    let source_url = text_value(show_v.get("url"))
        .unwrap_or_else(|| format!("https://www.tvmaze.com/shows/{show_id}"));
    let all_images: Vec<ArtworkCandidate> = images_v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|image| {
            let image_id = image.get("id").and_then(Value::as_u64)?;
            let resolutions = image.get("resolutions")?;
            let original = text_value(resolutions.pointer("/original/url"))?;
            let medium =
                text_value(resolutions.pointer("/medium/url")).unwrap_or_else(|| original.clone());
            Some(ArtworkCandidate {
                id: format!("tvmaze-{show_id}-image-{image_id}"),
                provider: MetadataProvider::TvMaze,
                title: format!(
                    "{} ({})",
                    title,
                    text_value(image.get("type")).unwrap_or_else(|| "poster".into())
                ),
                image_url: original,
                thumbnail_url: medium,
                source_url: source_url.clone(),
                creator: None,
                license: Some("CC BY-SA (TVmaze API content)".into()),
                license_url: Some("https://www.tvmaze.com/api".into()),
                attribution: Some(format!("TVmaze: {source_url}")),
            })
        })
        .take(40)
        .collect();
    let series_art = if all_images.is_empty() {
        tvmaze_artwork(
            show_id,
            &title,
            show_v.get("image"),
            "series poster",
            &source_url,
        )
    } else {
        all_images
    };
    let seasons = seasons_v
        .as_array()
        .ok_or("TVmaze returned invalid season data.")?
        .iter()
        .map(|season| parse_tv_season(show_id, season))
        .collect();
    let episodes = episodes_v
        .as_array()
        .ok_or("TVmaze returned invalid episode data.")?
        .iter()
        .filter_map(|episode| {
            let id = episode.get("id")?.as_u64()?;
            let name = text_value(episode.get("name"))?;
            let episode_url = text_value(episode.get("url"))
                .unwrap_or_else(|| format!("https://www.tvmaze.com/episodes/{id}"));
            let summary = text_value(episode.get("summary")).map(|html| decode_html_text(&html));
            let art = tvmaze_artwork(
                id,
                &name,
                episode.get("image"),
                "episode image",
                &episode_url,
            );
            Some(TvEpisodeMetadata {
                season: episode
                    .get("season")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok()),
                episode: episode
                    .get("number")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok()),
                title: name,
                airdate: text_value(episode.get("airdate")),
                summary,
                source_url: episode_url,
                artwork: art,
            })
        })
        .take(5000)
        .collect();
    Ok(TvSeriesDetails {
        show: TvShowSearchResult {
            id: show_id,
            title: title.clone(),
            year_start: year_from_date(show_v.get("premiered")),
            year_end: year_from_date(show_v.get("ended")),
            summary: text_value(show_v.get("summary")).map(|html| decode_html_text(&html)),
            audience_rating: show_v.pointer("/rating/average").and_then(Value::as_f64),
            source_url: source_url.clone(),
            artwork: series_art,
            confidence: 1.0,
        },
        seasons,
        episodes,
    })
}

fn validate_tmdb_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Enter your TMDB API read access token to use this optional provider.".into());
    }
    if token.len() > 4096 || token.chars().any(char::is_control) {
        return Err("The TMDB token is invalid or too long.".into());
    }
    Ok(token.to_string())
}

async fn tmdb_json(
    client: &Client,
    path: &str,
    token: &str,
    query: &[(&str, &str)],
) -> Result<Value, String> {
    // Callers construct paths from fixed endpoints and validated numeric IDs.
    let url = format!("https://api.themoviedb.org/3{}", path);
    let response = client
        .get(url)
        .bearer_auth(token)
        .query(query)
        .send()
        .await
        .map_err(|error| format!("Could not reach TMDB: {error}"))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err("TMDB rejected this read access token. Check the key and try again.".into());
    }
    bounded_json(response, "TMDB").await
}

fn tvdb_language_code(language: &str) -> Option<&'static str> {
    let primary = language
        .split_once('-')
        .map_or(language, |(primary, _)| primary);
    Some(match primary.to_ascii_lowercase().as_str() {
        "en" => "eng",
        "es" => "spa",
        "fr" => "fra",
        "de" => "deu",
        "it" => "ita",
        "pt" => "por",
        "ja" => "jpn",
        "ko" => "kor",
        "zh" => "zho",
        "ru" => "rus",
        "nl" => "nld",
        "ar" => "ara",
        "hi" => "hin",
        "tr" => "tur",
        "pl" => "pol",
        "sv" => "swe",
        "da" => "dan",
        "fi" => "fin",
        "no" => "nor",
        "cs" => "ces",
        "el" => "ell",
        "he" => "heb",
        "th" => "tha",
        "id" => "ind",
        "vi" => "vie",
        "uk" => "ukr",
        _ => return None,
    })
}

fn tvdb_source_url(kind: &str, id: u64, slug: Option<&Value>) -> String {
    let route = if kind == "movie" { "movies" } else { "series" };
    let slug = slug.and_then(Value::as_str).filter(|slug| {
        !slug.is_empty()
            && slug.len() <= 240
            && slug
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    });
    match slug {
        Some(slug) => format!("https://www.thetvdb.com/{route}/{slug}"),
        None => format!("https://www.thetvdb.com/{route}/{id}"),
    }
}

fn tvdb_id(value: &Value) -> Option<u64> {
    value
        .get("tvdb_id")
        .and_then(Value::as_u64)
        .or_else(|| value.get("id").and_then(Value::as_u64))
        .or_else(|| {
            value
                .get("tvdb_id")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str)
                .and_then(|id| id.rsplit('-').next())
                .and_then(|id| id.parse::<u64>().ok())
        })
        .filter(|id| *id > 0)
}

fn tvdb_image_url(value: Option<&Value>) -> Option<String> {
    let raw = value.and_then(Value::as_str)?.trim();
    let url = reqwest::Url::parse(raw).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("artworks.thetvdb.com") {
        return None;
    }
    Some(url.into())
}

fn tvdb_artwork_candidate(
    id: &str,
    title: &str,
    image: Option<&Value>,
    source_url: &str,
) -> Option<ArtworkCandidate> {
    let image_url = tvdb_image_url(image)?;
    Some(ArtworkCandidate {
        id: id.into(),
        provider: MetadataProvider::Tvdb,
        title: title.into(),
        image_url: image_url.clone(),
        thumbnail_url: image_url,
        source_url: source_url.into(),
        creator: None,
        license: None,
        license_url: Some("https://thetvdb.com/api-information".into()),
        attribution: Some(format!("TheTVDB: {source_url}")),
    })
}

fn tvdb_artwork_for_season(
    data: &Value,
    season_id: u64,
    season_number: u32,
    title: &str,
    source_url: &str,
) -> Vec<ArtworkCandidate> {
    let marker = format!("/season/{season_id}/posters/");
    let mut artwork: Vec<ArtworkCandidate> = Vec::new();
    for (index, item) in data
        .get("artworks")
        .or_else(|| data.get("artwork"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(200)
        .enumerate()
    {
        let Some(image) = tvdb_image_url(
            item.get("image")
                .or_else(|| item.get("image_url"))
                .or_else(|| item.get("url")),
        ) else {
            continue;
        };
        let linked = item
            .get("seasonId")
            .or_else(|| item.get("season_id"))
            .and_then(Value::as_u64)
            == Some(season_id);
        if !linked && !image.contains(&marker) {
            continue;
        }
        if !image.contains("/posters/") {
            continue;
        }
        let id = item
            .get("id")
            .and_then(Value::as_u64)
            .map(|value| format!("tvdb-season-{season_number}-art-{value}"))
            .unwrap_or_else(|| format!("tvdb-season-{season_number}-art-{index}"));
        if artwork.iter().any(|current| current.image_url == image) {
            continue;
        }
        artwork.push(ArtworkCandidate {
            id,
            provider: MetadataProvider::Tvdb,
            title: format!("{title} season {season_number} poster"),
            image_url: image.clone(),
            thumbnail_url: image,
            source_url: source_url.into(),
            creator: None,
            license: None,
            license_url: Some("https://thetvdb.com/api-information".into()),
            attribution: Some(format!("TheTVDB: {source_url}")),
        });
    }
    artwork
}

fn tvdb_recorded_seasons(data: &Value, title: &str, source_url: &str) -> Vec<TvSeasonMetadata> {
    data.get("seasons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|season| {
            let number = season
                .get("number")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())?;
            let season_title = text_value(season.get("name"))
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("Season {number}"));
            let mut artwork = tvdb_artwork_candidate(
                &format!("tvdb-season-{number}-poster"),
                &format!("{title} season {number} poster"),
                season.get("image").or_else(|| season.get("imageUrl")),
                source_url,
            )
            .into_iter()
            .collect::<Vec<_>>();
            if let Some(season_id) = season.get("id").and_then(Value::as_u64) {
                for extra in tvdb_artwork_for_season(data, season_id, number, title, source_url) {
                    if !artwork
                        .iter()
                        .any(|current| current.image_url == extra.image_url)
                    {
                        artwork.push(extra);
                    }
                }
            }
            Some(TvSeasonMetadata {
                number: Some(number),
                title: Some(season_title),
                year_start: text_value(season.get("year")),
                year_end: None,
                overview: text_value(season.get("overview"))
                    .map(|text| decode_html_text(&text))
                    .filter(|text| !text.trim().is_empty()),
                rating: published_audience_score(season.get("score")),
                artwork,
            })
        })
        .collect()
}

fn tvdb_artworks(
    value: &Value,
    entity_id: u64,
    title: &str,
    source_url: &str,
) -> Vec<ArtworkCandidate> {
    let mut artwork = Vec::new();
    for key in ["image", "image_url", "poster"] {
        if let Some(candidate) = tvdb_artwork_candidate(
            &format!("tvdb-{entity_id}-{key}"),
            &format!("{title} poster"),
            value.get(key),
            source_url,
        ) {
            artwork.push(candidate);
        }
    }
    for (index, item) in value
        .get("artworks")
        .or_else(|| value.get("artwork"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(30)
        .enumerate()
    {
        let Some(image) = tvdb_image_url(
            item.get("image")
                .or_else(|| item.get("image_url"))
                .or_else(|| item.get("url")),
        ) else {
            continue;
        };
        let art_title = text_value(item.get("type_name"))
            .or_else(|| text_value(item.get("type")))
            .map(|kind| format!("{title} ({kind})"))
            .unwrap_or_else(|| format!("{title} artwork"));
        let id = item
            .get("id")
            .and_then(Value::as_u64)
            .map(|id| format!("tvdb-{entity_id}-art-{id}"))
            .unwrap_or_else(|| format!("tvdb-{entity_id}-art-{index}"));
        if !artwork.iter().any(|current| current.image_url == image) {
            artwork.push(ArtworkCandidate {
                id,
                provider: MetadataProvider::Tvdb,
                title: art_title,
                image_url: image.clone(),
                thumbnail_url: image,
                source_url: source_url.into(),
                creator: None,
                license: None,
                license_url: Some("https://thetvdb.com/api-information".into()),
                attribution: Some(format!("TheTVDB: {source_url}")),
            });
        }
    }
    artwork.truncate(30);
    artwork
}

async fn tvdb_bearer_token(
    client: &Client,
    api_key: &str,
    subscriber_pin: Option<&str>,
) -> Result<String, String> {
    let api_key = api_key.trim();
    let pin = subscriber_pin.map(str::trim).filter(|pin| !pin.is_empty());
    if api_key.is_empty() || api_key.len() > 512 || api_key.chars().any(char::is_control) {
        return Err("The saved TheTVDB project API key is invalid. Replace it in Settings.".into());
    }
    let mut payload = serde_json::Map::new();
    payload.insert("apikey".into(), Value::String(api_key.into()));
    if let Some(pin) = pin {
        if pin.len() > 128 || pin.chars().any(char::is_control) {
            return Err(
                "The saved TheTVDB subscriber PIN is invalid. Replace it in Settings.".into(),
            );
        }
        payload.insert("pin".into(), Value::String(pin.into()));
    }
    let response = client
        .post("https://api4.thetvdb.com/v4/login")
        .json(&Value::Object(payload))
        .send()
        .await
        .map_err(|error| format!("Could not reach TheTVDB authentication: {error}"))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err("TheTVDB rejected this project key or subscriber PIN. Check the credentials and access model in Settings.".into());
    }
    let value = bounded_json(response, "TheTVDB authentication").await?;
    text_value(value.pointer("/data/token")).ok_or_else(|| {
        "TheTVDB returned no authentication token. Check the project key and access model.".into()
    })
}

async fn tvdb_json(
    client: &Client,
    path: &str,
    token: &str,
    query: &[(&str, &str)],
) -> Result<Value, String> {
    let url = format!("https://api4.thetvdb.com/v4{path}");
    let response = client
        .get(url)
        .bearer_auth(token)
        .query(query)
        .send()
        .await
        .map_err(|error| format!("Could not reach TheTVDB: {error}"))?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err("TheTVDB authentication was rejected. Confirm the key, PIN, and project access model in Settings.".into());
    }
    bounded_json(response, "TheTVDB").await
}

pub(crate) async fn search_tvdb_titles(
    query: &str,
    kind: &str,
    language: &str,
    api_key: &str,
    subscriber_pin: Option<&str>,
) -> Result<Vec<TvdbSearchResult>, String> {
    let query = validate_query(query)?;
    let language = validate_metadata_language(language)?;
    if !matches!(kind, "movie" | "series") {
        return Err("TheTVDB search type must be movie or series.".into());
    }
    let client = api_client()?;
    let token = tvdb_bearer_token(&client, api_key, subscriber_pin).await?;
    let language_code = tvdb_language_code(&language);
    let mut params = vec![("query", query.as_str()), ("type", kind), ("limit", "15")];
    if let Some(language) = language_code {
        params.push(("language", language));
    }
    let value = tvdb_json(&client, "/search", &token, &params).await?;
    let results = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or("TheTVDB returned no search results list.")?;
    let locale = language_code.unwrap_or("eng");
    Ok(results
        .iter()
        .filter(|item| {
            item.get("type")
                .and_then(Value::as_str)
                .is_none_or(|item_kind| item_kind == kind)
        })
        .take(MAX_RESULTS)
        .filter_map(|item| {
            let id = tvdb_id(item)?;
            let title = text_value(item.get("name_translated"))
                .or_else(|| text_value(item.get("name")))
                .or_else(|| text_value(item.pointer(&format!("/translations/{locale}"))))?;
            let source_url = tvdb_source_url(kind, id, item.get("slug"));
            let overview = text_value(item.pointer(&format!("/overviews/{locale}")))
                .or_else(|| text_value(item.get("overview")))
                .map(|text| decode_html_text(&text));
            let artwork = tvdb_artwork_candidate(
                &format!("tvdb-{id}-search-poster"),
                &format!("{title} poster"),
                item.get("image_url").or_else(|| item.get("image")),
                &source_url,
            )
            .into_iter()
            .collect();
            Some(TvdbSearchResult {
                id,
                title: title.clone(),
                year: text_value(item.get("year")).or_else(|| {
                    year_from_date(
                        item.get("first_air_time")
                            .or_else(|| item.get("firstAired")),
                    )
                }),
                overview,
                artwork,
                source_url,
                confidence: confidence(&query, &title, "TVDB title search"),
            })
        })
        .collect())
}

fn tvdb_genres(value: &Value) -> Vec<String> {
    value
        .get("genres")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|genre| {
            genre
                .as_str()
                .map(str::to_string)
                .or_else(|| text_value(genre.get("name")))
        })
        .filter(|genre| !genre.is_empty())
        .take(40)
        .collect()
}

fn tvdb_certification(value: &Value) -> Option<String> {
    let mut ratings = value
        .get("contentRatings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rating| {
            let name = text_value(rating.get("name"))
                .filter(|name| !name.is_empty() && name.len() <= 32)
                .or_else(|| {
                    text_value(rating.get("fullName"))
                        .filter(|name| !name.is_empty() && name.len() <= 32)
                })?;
            let country = text_value(rating.get("country"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let rank = if matches!(
                country.as_str(),
                "usa" | "us" | "united states" | "united states of america"
            ) {
                0
            } else {
                1
            };
            Some((rank, name))
        })
        .collect::<Vec<_>>();
    ratings.sort_by_key(|(rank, _)| *rank);
    ratings.into_iter().next().map(|(_, name)| name)
}

fn tvdb_runtime(value: &Value) -> Option<u32> {
    value
        .get("runtime")
        .or_else(|| value.get("averageRuntime"))
        .and_then(Value::as_u64)
        .and_then(|runtime| u32::try_from(runtime).ok())
        .filter(|runtime| *runtime > 0 && *runtime <= 24 * 60)
}

fn tvdb_season_episode(value: &Value, key: &[&str]) -> Option<u32> {
    key.iter().find_map(|field| {
        value
            .get(*field)
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok())
    })
}

async fn tvdb_series_episodes(
    client: &Client,
    series_id: u64,
    token: &str,
    language: &str,
) -> Result<Vec<TvEpisodeMetadata>, String> {
    const MAX_TVDB_EPISODES: usize = 5000;
    const MAX_TVDB_PAGES: u32 = 50;
    let language_path = tvdb_language_code(language);
    let route = match language_path {
        Some(language) => format!("/series/{series_id}/episodes/default/{language}"),
        None => format!("/series/{series_id}/episodes/default"),
    };
    let mut episodes = Vec::new();
    for page in 0..MAX_TVDB_PAGES {
        let page_text = page.to_string();
        let result = tvdb_json(client, &route, token, &[("page", page_text.as_str())]).await?;
        let episode_values = result
            .pointer("/data/episodes")
            .and_then(Value::as_array)
            .ok_or("TheTVDB returned an invalid episode page.")?;
        for value in episode_values {
            let id = tvdb_id(value);
            let season =
                tvdb_season_episode(value, &["seasonNumber", "season_number", "airedSeason"]);
            let number = tvdb_season_episode(
                value,
                &[
                    "number",
                    "episodeNumber",
                    "episode_number",
                    "airedEpisodeNumber",
                ],
            );
            let (Some(id), Some(season), Some(number)) = (id, season, number) else {
                continue;
            };
            let title = text_value(value.get("name"))
                .or_else(|| text_value(value.get("nameTranslated")))
                .unwrap_or_else(|| format!("Episode {number}"));
            let source_url = format!("https://www.thetvdb.com/episodes/{id}");
            let art = tvdb_artwork_candidate(
                &format!("tvdb-episode-{id}-image"),
                &format!("{title} still"),
                value.get("image"),
                &source_url,
            )
            .into_iter()
            .collect();
            episodes.push(TvEpisodeMetadata {
                season: Some(season),
                episode: Some(number),
                title,
                airdate: text_value(value.get("aired")),
                summary: text_value(value.get("overview")).map(|text| decode_html_text(&text)),
                source_url,
                artwork: art,
            });
            if episodes.len() >= MAX_TVDB_EPISODES {
                return Ok(episodes);
            }
        }
        if result
            .pointer("/links/next")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            break;
        }
    }
    Ok(episodes)
}

pub(crate) async fn load_tvdb_details(
    id: u64,
    kind: &str,
    language: &str,
    api_key: &str,
    subscriber_pin: Option<&str>,
) -> Result<TvdbMetadataDetails, String> {
    if id == 0 || !matches!(kind, "movie" | "series") {
        return Err("Choose a valid TheTVDB movie or series result first.".into());
    }
    let language = validate_metadata_language(language)?;
    let client = api_client()?;
    let token = tvdb_bearer_token(&client, api_key, subscriber_pin).await?;
    let route = if kind == "movie" { "movies" } else { "series" };
    let path = format!("/{route}/{id}/extended");
    let value = tvdb_json(&client, &path, &token, &[("meta", "translations")]).await?;
    let data = value
        .get("data")
        .ok_or("TheTVDB returned no extended title record.")?;
    let title = text_value(data.get("name"))
        .or_else(|| text_value(data.get("nameTranslated")))
        .ok_or("TheTVDB title record has no title.")?;
    let source_url = tvdb_source_url(kind, id, data.get("slug"));
    let mut episodes = Vec::new();
    let mut seasons = Vec::new();
    let mut episode_order_label = None;
    if kind == "series" {
        episodes = tvdb_series_episodes(&client, id, &token, &language).await?;
        let mut season_numbers = std::collections::BTreeSet::new();
        for episode in &episodes {
            if let Some(number) = episode.season.filter(|number| *number > 0) {
                season_numbers.insert(number);
            }
        }
        let recorded = tvdb_recorded_seasons(data, &title, &source_url);
        seasons = if recorded.is_empty() {
            season_numbers
                .into_iter()
                .map(|number| TvSeasonMetadata {
                    number: Some(number),
                    title: Some(format!("Season {number}")),
                    year_start: None,
                    year_end: None,
                    overview: None,
                    rating: None,
                    artwork: Vec::new(),
                })
                .collect()
        } else {
            recorded
        };
        episode_order_label = Some("TheTVDB default season order".into());
    }
    Ok(TvdbMetadataDetails {
        title: title.clone(),
        original_title: text_value(data.get("originalName")),
        year: year_from_date(data.get("firstAired").or_else(|| data.get("firstRelease")))
            .or_else(|| text_value(data.get("year"))),
        source_url: source_url.clone(),
        short_description: text_value(data.get("tagline")),
        long_description: text_value(data.get("overview")).map(|text| decode_html_text(&text)),
        certification: tvdb_certification(data),
        runtime_minutes: tvdb_runtime(data),
        genres: tvdb_genres(data),
        rating: None,
        artwork: unique_artwork(tvdb_artworks(data, id, &title, &source_url)),
        seasons,
        episodes,
        episode_order_label,
        attribution: "TheTVDB metadata and artwork. Review the project’s access model, attribution requirements and reuse terms before saving or distributing provider data.".into(),
    })
}

fn tmdb_image_path(path: Option<&Value>) -> Option<String> {
    let path = path.and_then(Value::as_str)?;
    let name = path.strip_prefix('/')?;
    if name.is_empty()
        || name.contains("..")
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.'))
    {
        return None;
    }
    Some(name.to_string())
}

fn tmdb_artwork(
    id: u64,
    title: &str,
    path: Option<&Value>,
    kind: &str,
) -> Option<ArtworkCandidate> {
    let image_path = tmdb_image_path(path)?;
    let source_url = format!("https://www.themoviedb.org/{kind}/{id}");
    Some(ArtworkCandidate {
        id: format!(
            "tmdb-{id}-{kind}-{}",
            image_path.rsplit('/').next().unwrap_or("image")
        ),
        provider: MetadataProvider::Tmdb,
        title: format!("{title} ({kind} artwork)"),
        image_url: format!("https://image.tmdb.org/t/p/original/{image_path}"),
        thumbnail_url: format!("https://image.tmdb.org/t/p/w342/{image_path}"),
        source_url: source_url.clone(),
        creator: None,
        // TMDB does not claim ownership of API artwork; the UI must link to source and show attribution.
        license: None,
        license_url: None,
        attribution: Some(format!("Source: The Movie Database (TMDB) — {source_url}")),
    })
}

fn unique_artwork(candidates: Vec<ArtworkCandidate>) -> Vec<ArtworkCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.id.clone()))
        .collect()
}

fn tmdb_image_list(
    value: Option<&Value>,
    id: u64,
    title: &str,
    kind: &str,
) -> Vec<ArtworkCandidate> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|image| tmdb_artwork(id, title, image.get("file_path"), kind))
        .take(30)
        .collect()
}

fn parse_tmdb_search_result(
    result: &Value,
    media_kind: &str,
    confidence_query: &str,
) -> Option<TmdbSearchResult> {
    let id = result.get("id")?.as_u64()?;
    let title = text_value(result.get(if media_kind == "movie" {
        "title"
    } else {
        "name"
    }))?;
    let original_title = text_value(result.get(if media_kind == "movie" {
        "original_title"
    } else {
        "original_name"
    }))
    .unwrap_or_else(|| title.clone());
    let date = if media_kind == "movie" {
        result.get("release_date")
    } else {
        result.get("first_air_date")
    };
    let overview = text_value(result.get("overview"));
    let source_url = format!("https://www.themoviedb.org/{media_kind}/{id}");
    let artwork = tmdb_artwork(id, &title, result.get("poster_path"), media_kind)
        .into_iter()
        .collect();
    Some(TmdbSearchResult {
        id,
        confidence: confidence(confidence_query, &title, media_kind),
        year: year_from_date(date),
        overview,
        original_language: text_value(result.get("original_language")),
        rating: result.get("vote_average").and_then(Value::as_f64),
        artwork,
        source_url,
        title,
        original_title,
    })
}

async fn search_tmdb(
    query: &str,
    token: &str,
    media_kind: &str,
    language: &str,
) -> Result<Vec<TmdbSearchResult>, String> {
    let query = validate_query(query)?;
    let token = validate_tmdb_token(token)?;
    let language = validate_metadata_language(language)?;
    let endpoint = if media_kind == "movie" {
        "/search/movie"
    } else {
        "/search/tv"
    };
    let client = api_client()?;
    let value = tmdb_json(
        &client,
        endpoint,
        &token,
        &[
            ("query", query.as_str()),
            ("include_adult", "false"),
            ("language", language.as_str()),
            ("page", "1"),
        ],
    )
    .await?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or("TMDB returned no search results list.")?;
    Ok(results
        .iter()
        .filter(|item| item.get("adult").and_then(Value::as_bool) != Some(true))
        .take(MAX_RESULTS)
        .filter_map(|item| parse_tmdb_search_result(item, media_kind, &query))
        .collect())
}

pub(crate) async fn search_tmdb_movies(
    query: &str,
    token: &str,
    language: &str,
) -> Result<Vec<TmdbSearchResult>, String> {
    search_tmdb(query, token, "movie", language).await
}

pub(crate) async fn search_tmdb_tv(
    query: &str,
    token: &str,
    language: &str,
) -> Result<Vec<TmdbSearchResult>, String> {
    search_tmdb(query, token, "tv", language).await
}

pub(crate) async fn load_tmdb_movie_details(
    id: u64,
    token: &str,
    language: &str,
) -> Result<TmdbMovieDetails, String> {
    if id == 0 {
        return Err("TMDB movie ID must be greater than zero.".into());
    }
    let token = validate_tmdb_token(token)?;
    let language = validate_metadata_language(language)?;
    let language_code = language_code(&language);
    let image_languages = format!("{language_code},null");
    let region = language_region(&language);
    let client = api_client()?;
    let path = format!("/movie/{id}");
    let value = tmdb_json(
        &client,
        &path,
        &token,
        &[
            ("append_to_response", "images,release_dates"),
            ("include_image_language", image_languages.as_str()),
            ("language", language.as_str()),
        ],
    )
    .await?;
    let title = text_value(value.get("title")).ok_or("TMDB movie details have no title.")?;
    let release_date = text_value(value.get("release_date"));
    let movie = TmdbSearchResult {
        id,
        title: title.clone(),
        original_title: text_value(value.get("original_title")).unwrap_or_else(|| title.clone()),
        year: year_from_date(value.get("release_date")),
        overview: text_value(value.get("overview")),
        original_language: text_value(value.get("original_language")),
        rating: value.get("vote_average").and_then(Value::as_f64),
        artwork: tmdb_artwork(id, &title, value.get("poster_path"), "movie")
            .into_iter()
            .collect(),
        source_url: format!("https://www.themoviedb.org/movie/{id}"),
        confidence: 1.0,
    };
    let mut artwork = tmdb_artwork(id, &title, value.get("poster_path"), "movie")
        .into_iter()
        .collect::<Vec<_>>();
    artwork.extend(tmdb_image_list(
        value.pointer("/images/posters"),
        id,
        &title,
        "movie",
    ));
    let artwork = unique_artwork(artwork);
    let releases = value
        .pointer("/release_dates/results")
        .and_then(Value::as_array);
    let mut certifications = releases
        .into_iter()
        .flatten()
        .flat_map(|country| {
            let region_rank = if country
                .get("iso_3166_1")
                .and_then(Value::as_str)
                .is_some_and(|code| code.eq_ignore_ascii_case(region))
            {
                0
            } else {
                1
            };
            country
                .get("release_dates")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(move |release| {
                    text_value(release.get("certification")).map(|label| (region_rank, label))
                })
        })
        .collect::<Vec<_>>();
    certifications.sort_by_key(|(rank, _)| *rank);
    let certification = certifications.into_iter().next().map(|(_, label)| label);
    let genres = value
        .get("genres")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|genre| text_value(genre.get("name")))
        .collect();
    Ok(TmdbMovieDetails {
        movie,
        release_date,
        certification,
        runtime_minutes: value
            .get("runtime")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        genres,
        tagline: text_value(value.get("tagline")),
        artwork,
    })
}

fn tmdb_region_certificate(value: &Value, region: &str) -> Option<String> {
    let mut content_ratings = value
        .pointer("/content_ratings/results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rating| {
            let region_rank = if rating
                .get("iso_3166_1")
                .and_then(Value::as_str)
                .is_some_and(|code| code.eq_ignore_ascii_case(region))
            {
                0
            } else {
                1
            };
            text_value(rating.get("rating")).map(|label| (region_rank, label))
        })
        .collect::<Vec<_>>();
    content_ratings.sort_by_key(|(rank, _)| *rank);
    content_ratings.into_iter().next().map(|(_, label)| label)
}

pub(crate) async fn tmdb_tv_certification_for_tvdb(
    tvdb_id: u64,
    token: &str,
    language: &str,
) -> Result<Option<String>, String> {
    if tvdb_id == 0 {
        return Ok(None);
    }
    let token = validate_tmdb_token(token)?;
    let language = validate_metadata_language(language)?;
    let region = language_region(&language);
    let client = api_client()?;
    let found = tmdb_json(
        &client,
        &format!("/find/{tvdb_id}"),
        &token,
        &[
            ("external_source", "tvdb_id"),
            ("language", language.as_str()),
        ],
    )
    .await?;
    let Some(tmdb_id) = found
        .pointer("/tv_results/0/id")
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
    else {
        return Ok(None);
    };
    let value = tmdb_json(
        &client,
        &format!("/tv/{tmdb_id}"),
        &token,
        &[
            ("append_to_response", "content_ratings"),
            ("language", language.as_str()),
        ],
    )
    .await?;
    Ok(tmdb_region_certificate(&value, &region))
}

pub(crate) async fn load_tmdb_tv_details(
    id: u64,
    token: &str,
    language: &str,
) -> Result<TmdbTvDetails, String> {
    if id == 0 {
        return Err("TMDB TV ID must be greater than zero.".into());
    }
    let token = validate_tmdb_token(token)?;
    let language = validate_metadata_language(language)?;
    let language_code = language_code(&language);
    let image_languages = format!("{language_code},null");
    let region = language_region(&language);
    let client = api_client()?;
    let path = format!("/tv/{id}");
    let value = tmdb_json(
        &client,
        &path,
        &token,
        &[
            ("append_to_response", "images,content_ratings"),
            ("include_image_language", image_languages.as_str()),
            ("language", language.as_str()),
        ],
    )
    .await?;
    let title = text_value(value.get("name")).ok_or("TMDB TV show details have no title.")?;
    let source_url = format!("https://www.themoviedb.org/tv/{id}");
    let mut show_art = tmdb_artwork(id, &title, value.get("poster_path"), "tv")
        .into_iter()
        .collect::<Vec<_>>();
    show_art.extend(tmdb_image_list(
        value.pointer("/images/posters"),
        id,
        &title,
        "tv",
    ));
    let show_art = unique_artwork(show_art);
    let show = TmdbSearchResult {
        id,
        title: title.clone(),
        original_title: text_value(value.get("original_name")).unwrap_or_else(|| title.clone()),
        year: year_from_date(value.get("first_air_date")),
        overview: text_value(value.get("overview")),
        original_language: text_value(value.get("original_language")),
        rating: value.get("vote_average").and_then(Value::as_f64),
        artwork: show_art,
        source_url: source_url.clone(),
        confidence: 1.0,
    };
    let season_values = value
        .get("seasons")
        .and_then(Value::as_array)
        .ok_or("TMDB returned no season list.")?;
    let mut seasons: Vec<TmdbSeasonMetadata> = season_values
        .iter()
        .take(50)
        .filter_map(|season| {
            let number = season
                .get("season_number")?
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())?;
            let name = text_value(season.get("name")).unwrap_or_else(|| format!("Season {number}"));
            let art = tmdb_artwork(id, &title, season.get("poster_path"), "tv")
                .into_iter()
                .collect();
            let date = text_value(season.get("air_date"));
            Some(TmdbSeasonMetadata {
                number,
                title: name,
                year_start: date
                    .as_deref()
                    .and_then(|d| d.get(0..4))
                    .map(str::to_string),
                year_end: None,
                overview: None,
                rating: None,
                artwork: art,
            })
        })
        .collect();
    let certification = tmdb_region_certificate(&value, &region);

    // Query season detail endpoints with a strict concurrency and episode-count bound.
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for season in &seasons {
        let client = client.clone();
        let token = token.clone();
        let language = language.clone();
        let semaphore = Arc::clone(&semaphore);
        let season_number = season.number;
        let season_path = format!("/tv/{id}/season/{season_number}");
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .map_err(|_| "TMDB season request was cancelled.".to_string())?;
            let detail = tmdb_json(
                &client,
                &season_path,
                &token,
                &[("language", language.as_str())],
            )
            .await?;
            Ok::<(u32, Value), String>((season_number, detail))
        });
    }
    let mut episode_rows = Vec::new();
    let mut season_copy = std::collections::HashMap::<u32, (Option<String>, Option<f64>)>::new();
    while let Some(task) = tasks.join_next().await {
        let (season_number, season_detail) =
            task.map_err(|error| format!("TMDB season request failed: {error}"))??;
        season_copy.insert(
            season_number,
            (
                published_overview(season_detail.get("overview")),
                published_audience_score(season_detail.get("vote_average")),
            ),
        );
        for episode in season_detail
            .get("episodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(number) = episode
                .get("episode_number")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
            else {
                continue;
            };
            let season_number = episode
                .get("season_number")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let episode_title =
                text_value(episode.get("name")).unwrap_or_else(|| format!("Episode {number}"));
            let episode_url = format!(
                "https://www.themoviedb.org/tv/{id}/season/{season_number}/episode/{number}"
            );
            let episode_art = tmdb_artwork(id, &episode_title, episode.get("still_path"), "tv")
                .into_iter()
                .collect();
            episode_rows.push(TmdbEpisodeMetadata {
                season: season_number,
                episode: number,
                title: episode_title,
                airdate: text_value(episode.get("air_date")),
                summary: text_value(episode.get("overview")),
                source_url: episode_url,
                artwork: episode_art,
            });
            if episode_rows.len() >= 5000 {
                break;
            }
        }
        if episode_rows.len() >= 5000 {
            break;
        }
    }
    for season in &mut seasons {
        if let Some((overview, rating)) = season_copy.remove(&season.number) {
            season.overview = overview;
            season.rating = rating;
        }
    }
    episode_rows.sort_by_key(|episode| (episode.season, episode.episode));
    let genres = value
        .get("genres")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|genre| text_value(genre.get("name")))
        .collect();
    Ok(TmdbTvDetails {
        show,
        overview: text_value(value.get("overview")),
        certification,
        genres,
        seasons,
        episodes: episode_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_query_is_bounded_and_trimmed_without_file_path_input() {
        assert_eq!(
            validate_query("  Gnomeo & Juliet  ").unwrap(),
            "Gnomeo & Juliet"
        );
        assert!(validate_query(" ").is_err());
        assert!(validate_query(&"x".repeat(257)).is_err());
        assert!(validate_query("Movie\nFile").is_err());
    }

    #[test]
    fn metadata_locale_accepts_language_and_region_and_normalizes_case() {
        assert_eq!(validate_metadata_language("EN-us").unwrap(), "en-US");
        assert_eq!(validate_metadata_language("ja").unwrap(), "ja");
        assert!(validate_metadata_language("en-US-extra").is_err());
        assert!(validate_metadata_language("e-US").is_err());
    }

    #[test]
    fn filename_clue_extracts_title_year_and_certification_without_release_labels() {
        let clue =
            filename_metadata_clue_with_patterns("Gnomeo & Juliet (2011 G).mp4", &[]).unwrap();
        assert_eq!(clue.title, "Gnomeo & Juliet");
        assert_eq!(clue.year.as_deref(), Some("2011"));
        assert_eq!(clue.year_end, None);
        assert_eq!(clue.certification.as_deref(), Some("G"));
    }

    #[test]
    fn filename_clue_cleans_release_tokens_and_preserves_title_punctuation() {
        let clue = filename_metadata_clue_with_patterns(
            "Spider-Man.2.The.New.Chapter.2024.1080p.WEB-DL.x265-GROUP.mkv",
            &[],
        )
        .unwrap();
        assert_eq!(clue.title, "Spider-Man 2 The New Chapter");
        assert_eq!(clue.year.as_deref(), Some("2024"));
        assert_eq!(clue.certification, None);
    }

    #[test]
    fn filename_clue_applies_configured_blacklist_before_import_lookup() {
        let patterns = crate::settings::default_settings()["metadata"]["text_blacklist"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pattern| pattern.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(patterns, ["Movie*Group"]);

        for filename in [
            "The.MovieReleaseGroup.Film.2011.1080p.WEB-DL.mkv",
            "The.Movieshowgroup.Film.2011.1080p.WEB-DL.mkv",
        ] {
            let clue = filename_metadata_clue_with_patterns(filename, &patterns).unwrap();
            assert_eq!(clue.title, "The Film");
            assert_eq!(clue.year.as_deref(), Some("2011"));
        }
    }

    #[test]
    fn filename_clue_supports_series_folder_year_ranges_and_rejects_paths() {
        let clue =
            filename_metadata_clue_with_patterns("Faerie Tale Theatre (1982-1987)", &[]).unwrap();
        assert_eq!(clue.title, "Faerie Tale Theatre");
        assert_eq!(clue.year.as_deref(), Some("1982"));
        assert_eq!(clue.year_end.as_deref(), Some("1987"));
        assert!(
            filename_metadata_clue_with_patterns("/library/Gnomeo & Juliet (2011 G).mp4", &[])
                .is_err()
        );
        assert!(filename_metadata_clue_with_patterns("1080p.WEB-DL.x265.mkv", &[]).is_err());
    }

    #[test]
    fn wikidata_ids_and_years_are_validated_conservatively() {
        assert!(valid_wikidata_id("Q123"));
        assert!(!valid_wikidata_id("Q0;rm -rf"));
        assert!(!valid_wikidata_id("Q"));
        let time = serde_json::json!({"time":"+2011-01-01T00:00:00Z"});
        assert_eq!(year_prefix(Some(&time)), Some("2011".into()));
    }

    #[test]
    fn confidence_is_a_review_hint_not_an_auto_match() {
        assert!(
            confidence("A Movie", "A Movie", "2011 film")
                > confidence("A Movie", "A Muvie", "series")
        );
        assert!(confidence("A Movie", "A Movie", "2011 film") < 1.0);
    }

    #[test]
    fn trailing_year_fallback_keeps_only_a_nonempty_title() {
        assert_eq!(
            trailing_year_fallback_title("Gnomeo & Juliet 2011"),
            Some("Gnomeo & Juliet")
        );
        assert_eq!(
            trailing_year_fallback_title("Gnomeo & Juliet (2011)"),
            Some("Gnomeo & Juliet")
        );
        assert_eq!(trailing_year_fallback_title("Gnomeo & Juliet"), None);
        assert_eq!(trailing_year_fallback_title("1917"), None);
    }

    #[test]
    fn tvdb_content_rating_prefers_the_united_states_certificate() {
        let value = serde_json::json!({
            "contentRatings": [
                { "name": "PG", "country": "aus", "fullName": "Parental guidance" },
                { "name": "TV-PG", "country": "usa", "fullName": "TV Parental Guidelines" }
            ]
        });
        assert_eq!(tvdb_certification(&value).as_deref(), Some("TV-PG"));
        assert_eq!(tvdb_certification(&serde_json::json!({})), None);
    }

    #[test]
    fn tmdb_certificate_prefers_the_requested_region() {
        let value = serde_json::json!({
            "content_ratings": {"results": [
                {"iso_3166_1": "BR", "rating": "L"},
                {"iso_3166_1": "US", "rating": "NR"}
            ]}
        });
        assert_eq!(tmdb_region_certificate(&value, "US").as_deref(), Some("NR"));
    }

    #[test]
    fn season_publication_keeps_a_real_plot_and_score() {
        let detail = serde_json::json!({
            "overview": "Six stage tales.",
            "vote_average": 0.0
        });
        assert_eq!(
            published_overview(detail.get("overview")).as_deref(),
            Some("Six stage tales.")
        );
        assert_eq!(published_audience_score(detail.get("vote_average")), None);
        let scored = serde_json::json!({ "overview": "  ", "vote_average": 8.2 });
        assert_eq!(published_overview(scored.get("overview")), None);
        assert_eq!(
            published_audience_score(scored.get("vote_average")),
            Some(8.2)
        );
        assert_eq!(
            published_audience_score(Some(&serde_json::json!(11.0))),
            None
        );
    }

    #[test]
    fn repeated_provider_poster_keeps_the_first_id() {
        let poster = |id: &str| ArtworkCandidate {
            id: id.into(),
            provider: MetadataProvider::Tmdb,
            title: "Faerie Tale Theatre".into(),
            image_url: "https://image.tmdb.org/t/p/original/poster.jpg".into(),
            thumbnail_url: "https://image.tmdb.org/t/p/w342/poster.jpg".into(),
            source_url: "https://www.themoviedb.org/tv/4603".into(),
            creator: None,
            license: None,
            license_url: None,
            attribution: None,
        };
        let unique = unique_artwork(vec![
            poster("tmdb-4603-tv-poster.jpg"),
            poster("tmdb-4603-tv-poster.jpg"),
            poster("tmdb-4603-tv-other.jpg"),
        ]);
        assert_eq!(
            unique
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb-4603-tv-poster.jpg", "tmdb-4603-tv-other.jpg"]
        );
    }

    #[test]
    fn tvmaze_summary_is_flattened_to_plain_text() {
        assert_eq!(
            decode_html_text("<p>A <b>long</b> tale &amp; more.</p>"),
            "A long tale & more."
        );
    }
}
