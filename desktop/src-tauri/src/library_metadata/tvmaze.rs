//! TV show search and series details from TVmaze.

use super::*;

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
