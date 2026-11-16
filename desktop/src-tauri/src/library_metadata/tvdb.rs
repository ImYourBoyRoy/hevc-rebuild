//! TheTVDB search, series and episode details (needs the user's own credentials).

use super::*;

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

pub(super) fn tvdb_id(value: &Value) -> Option<u64> {
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

pub(super) fn tvdb_certification(value: &Value) -> Option<String> {
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
