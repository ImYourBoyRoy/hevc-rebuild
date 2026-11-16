//! Film search and details from Wikidata, Wikipedia and Wikimedia Commons.

use super::*;

fn wikidata_year(entity: &Value) -> Option<String> {
    entity
        .get("claims")?
        .get("P577")?
        .as_array()?
        .iter()
        .find_map(|claim| year_prefix(claim.pointer("/mainsnak/datavalue/value")))
}

pub(crate) async fn search_films(query: &str) -> Result<Vec<FilmSearchResult>, String> {
    search_films_in_language(query, "en-US").await
}

pub(super) fn trailing_year_fallback_title(query: &str) -> Option<&str> {
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

pub(super) fn valid_wikidata_id(id: &str) -> bool {
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

pub(super) fn decode_html_text(input: &str) -> String {
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
