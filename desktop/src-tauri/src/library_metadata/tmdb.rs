//! TMDb search and details (needs the user's own read token).

use super::*;

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

pub(super) fn tmdb_region_certificate(value: &Value, region: &str) -> Option<String> {
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
    Ok(tmdb_region_certificate(&value, region))
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
    let certification = tmdb_region_certificate(&value, region);

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
