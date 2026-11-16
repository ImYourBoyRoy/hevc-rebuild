//! Tests for filename clues, provider parsing and metadata confidence.

use super::tmdb::*;
use super::tvdb::*;
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
    let clue = filename_metadata_clue_with_patterns("Gnomeo & Juliet (2011 G).mp4", &[]).unwrap();
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
        filename_metadata_clue_with_patterns("/library/Gnomeo & Juliet (2011 G).mp4", &[]).is_err()
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
        confidence("A Movie", "A Movie", "2011 film") > confidence("A Movie", "A Muvie", "series")
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
