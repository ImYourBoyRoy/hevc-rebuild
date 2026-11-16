//! Tests for queue persistence, validation, partial saves and interrupted-run recovery.

use super::*;

fn save_synthetic_scan(path: &Path, directory: &Path, media_paths: &[PathBuf]) {
    let entries = media_paths
        .iter()
        .map(|media_path| {
            json!({
                "path": media_path,
                "relative_path": media_path.file_name().and_then(|name| name.to_str()).unwrap_or("sample.mkv"),
                "name": media_path.file_name().and_then(|name| name.to_str()).unwrap_or("sample.mkv"),
                "size_bytes": 1234,
                "status": "Ready",
                "error": null,
                "rawProbePayload": {"privateTestMarker": true},
                "profile": {
                    "video": {"codec": "hevc", "width": 1920, "height": 1080},
                    "hdr": {"hdr10": true, "hdr10plus": false, "dolby_vision": false}
                }
            })
        })
        .collect::<Vec<_>>();
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: directory.to_string_lossy().into_owned(),
        status: QueueSourceStatus::Ready,
        scan_result: Some(json!({"entries": entries})),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    save(path, queue).expect("save synthetic queue fixture");
}

#[test]
fn queue_summary_is_paged_and_omits_raw_scan_payloads() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-summary-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let first = directory.join("one.mkv");
    let second = directory.join("two.mkv");
    fs::write(&first, []).expect("create synthetic path");
    fs::write(&second, []).expect("create synthetic path");
    let queue_path = directory.join("library-queue.json");
    save_synthetic_scan(&queue_path, &directory, &[first.clone(), second.clone()]);

    let page = summarize(&queue_path, 1, 1).expect("read bounded queue summary");
    assert_eq!(page["fileCount"], 2);
    assert_eq!(page["offset"], 1);
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        page["entries"][0]["path"],
        second.to_string_lossy().as_ref()
    );
    assert_eq!(page["entries"][0]["videoCodec"], "hevc");
    assert_eq!(page["entries"][0]["hdr10"], true);
    assert!(page["entries"][0].get("rawProbePayload").is_none());
    assert!(summarize(&queue_path, 0, MAX_QUEUE_PAGE_SIZE + 1).is_err());
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn queue_selection_update_preserves_other_queue_decisions_and_rejects_unknown_paths() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-selection-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let media_path = directory.join("sample.mkv");
    fs::write(&media_path, []).expect("create synthetic path");
    let queue_path = directory.join("library-queue.json");
    save_synthetic_scan(&queue_path, &directory, std::slice::from_ref(&media_path));
    let mut queue = load(&queue_path).expect("load fixture").queue;
    queue.file_tune_overrides.insert(
        media_path.to_string_lossy().into_owned(),
        "animation".into(),
    );
    save(&queue_path, queue).expect("save existing tune decision");

    save_selected_paths(&queue_path, vec![media_path.to_string_lossy().into_owned()])
        .expect("select analyzed file");
    let selected = load(&queue_path).expect("reload selected queue").queue;
    assert_eq!(
        selected.selected_media_paths,
        vec![media_path.to_string_lossy()]
    );
    assert_eq!(
        selected.file_tune_overrides[media_path.to_string_lossy().as_ref()],
        "animation"
    );

    let unknown = directory.join("outside.mkv");
    let error = save_selected_paths(&queue_path, vec![unknown.to_string_lossy().into_owned()])
        .expect_err("reject a path not present in the queue");
    assert!(error.contains("belong to a saved queue source"));
    let unchanged = load(&queue_path)
        .expect("selection remains unchanged")
        .queue;
    assert_eq!(
        unchanged.selected_media_paths,
        selected.selected_media_paths
    );
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn metadata_suggestions_persist_for_analyzed_paths_without_replacing_queue_state() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-suggestions-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let media_path = directory.join("Gnomeo & Juliet (2011).mkv");
    fs::write(&media_path, []).expect("create synthetic media path");
    let media_path_string = media_path.to_string_lossy().into_owned();
    let queue_path = directory.join("library-queue.json");
    save_synthetic_scan(&queue_path, &directory, std::slice::from_ref(&media_path));
    save_selected_paths(&queue_path, vec![media_path_string.clone()])
        .expect("save global queue selection");

    save_metadata_suggestions(
        &queue_path,
        BTreeMap::from([(
            media_path_string.clone(),
            QueueMetadataSuggestion {
                kind: "movie".into(),
                status: "ready".into(),
                title: Some("Gnomeo & Juliet".into()),
                year: Some("2011".into()),
                provider: Some("tmdb".into()),
                record_id: Some("43".into()),
                source_url: Some("https://www.themoviedb.org/movie/43".into()),
                description: None,
                auto_associated: false,
                artwork: None,
                error: String::new(),
            },
        )]),
    )
    .expect("save queue title suggestion");

    let queue = load(&queue_path).expect("reload queue").queue;
    let suggestion = queue.match_suggestions.get(&media_path_string).unwrap();
    assert_eq!(suggestion.title.as_deref(), Some("Gnomeo & Juliet"));
    assert_eq!(suggestion.provider.as_deref(), Some("tmdb"));
    assert_eq!(queue.selected_media_paths, vec![media_path_string]);
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn metadata_suggestions_reject_unknown_paths_and_unapproved_providers() {
    let unknown = QueueMetadataSuggestion {
        kind: "movie".into(),
        status: "ready".into(),
        title: Some("Unknown Film".into()),
        year: None,
        provider: Some("tmdb".into()),
        record_id: None,
        source_url: None,
        description: None,
        auto_associated: false,
        artwork: None,
        error: String::new(),
    };
    let mut queue = QueueDocument::default();
    queue
        .match_suggestions
        .insert(media_path("not-scanned.mkv"), unknown.clone());
    assert!(validate(&queue)
        .unwrap_err()
        .contains("analyzed queue file"));

    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-suggestion-validation-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let media_path = directory.join("sample.mkv");
    fs::write(&media_path, []).expect("create synthetic media path");
    let mut scanned = QueueDocument::default();
    scanned.sources.push(QueueSource {
        root: directory.to_string_lossy().into_owned(),
        status: QueueSourceStatus::Ready,
        scan_result: Some(json!({"entries": [{"path": media_path}]})),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    let mut unsupported = unknown;
    unsupported.provider = Some("untrusted-provider".into());
    scanned
        .match_suggestions
        .insert(media_path.to_string_lossy().into_owned(), unsupported);
    assert!(validate(&scanned)
        .unwrap_err()
        .contains("unsupported metadata provider"));
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

/// A path that is absolute on whatever platform the tests run on (`/media/..` is
/// not absolute on Windows, and the queue validator rightly insists on absolute
/// artwork paths).
fn media_path(rest: &str) -> String {
    if cfg!(windows) {
        format!(r"C:\media\{}", rest.replace('/', r"\"))
    } else {
        format!("/media/{rest}")
    }
}

fn sample_review() -> QueueMovieReview {
    QueueMovieReview {
        title: "Gnomeo & Juliet".into(),
        year: "2011".into(),
        certification: "G".into(),
        record_id: "tmdb:41733".into(),
        short_description: String::new(),
        plot: String::new(),
        genres_text: "Animation, Family".into(),
        rating_text: String::new(),
        source_url: "https://www.themoviedb.org/movie/41733".into(),
        comment: String::new(),
        artwork_candidates: vec![QueueArtworkCandidate {
            id: "tmdb:poster-1".into(),
            provider: "TMDB".into(),
            title: "Gnomeo & Juliet poster".into(),
            image_url: "https://image.tmdb.org/t/p/original/poster.jpg".into(),
            thumbnail_url: "https://image.tmdb.org/t/p/w342/poster.jpg".into(),
            source_url: "https://www.themoviedb.org/movie/41733".into(),
            creator: None,
            license: Some("TMDB terms".into()),
            license_url: Some("https://www.themoviedb.org/api-terms-of-use".into()),
            attribution: Some("Metadata provided by TMDB.".into()),
        }],
        poster_id: "tmdb:poster-1".into(),
        local_poster_path: media_path("Gnomeo & Juliet-poster.jpg"),
    }
}

fn sample_series_review() -> QueueSeriesReview {
    QueueSeriesReview {
        title: "Example Series".into(),
        year: "2019–2021".into(),
        certification: "TV-14".into(),
        record_id: "tvmaze:42".into(),
        short_description: String::new(),
        plot: "A reviewed series summary.".into(),
        genres_text: "Drama, Mystery".into(),
        rating_text: "8.4".into(),
        source_url: "https://www.tvmaze.com/shows/42/example-series".into(),
        comment: String::new(),
        artwork_candidates: sample_review().artwork_candidates,
        poster_id: "tmdb:poster-1".into(),
        local_poster_path: media_path("Example Series/poster.jpg"),
        local_season_poster_paths: BTreeMap::from([(
            "1".into(),
            media_path("Example Series/Season 01/poster.jpg"),
        )]),
        season_records: BTreeMap::new(),
        episodes: vec![QueueTvEpisode {
            season: 1,
            episode: 2,
            title: "The Second Hour".into(),
            airdate: Some("2019-05-14".into()),
            summary: Some("A reviewed episode summary.".into()),
            source_url: "https://www.tvmaze.com/episodes/2002/example-series-1x02".into(),
            comment: String::new(),
            certification: String::new(),
        }],
        episode_assignments: BTreeMap::from([(
            media_path("Example Series/Season 01/episode.mkv"),
            QueueTvEpisode {
                season: 1,
                episode: 2,
                title: "The Second Hour".into(),
                airdate: Some("2019-05-14".into()),
                summary: Some("Edited episode summary.".into()),
                source_url: "https://www.tvmaze.com/episodes/2002/example-series-1x02".into(),
                comment: String::new(),
                certification: String::new(),
            },
        )]),
    }
}

#[test]
fn older_queue_documents_default_to_no_movie_reviews() {
    let queue = serde_json::from_value::<QueueDocument>(serde_json::json!({
        "version": QUEUE_VERSION,
        "sources": [],
        "selectedMediaPaths": [],
        "fileTuneOverrides": {},
        "fileStreamOverrides": {},
        "activeRoot": null,
        "updatedAtMs": 0
    }))
    .expect("older queue schema remains readable");
    assert!(queue.movie_reviews.is_empty());
    assert!(queue.series_reviews.is_empty());
    assert!(queue.match_suggestions.is_empty());
    assert!(queue.run_states.is_empty());
    assert!(validate(&queue).is_ok());
}

#[test]
fn queue_load_turns_only_active_run_states_into_interrupted() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-run-state-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let active_path = directory.join("active.mkv");
    let pending_path = directory.join("pending.mkv");
    fs::write(&active_path, b"synthetic active media").expect("create active fixture");
    fs::write(&pending_path, b"synthetic pending media").expect("create pending fixture");
    let active = active_path.to_string_lossy().into_owned();
    let pending = pending_path.to_string_lossy().into_owned();
    let queue_path = directory.join("library-queue.json");
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: directory.to_string_lossy().into_owned(),
        status: QueueSourceStatus::Ready,
        scan_result: Some(serde_json::json!({
            "entries": [
                {"path": active, "size_bytes": fs::metadata(&active_path).unwrap().len()},
                {"path": pending, "size_bytes": fs::metadata(&pending_path).unwrap().len()}
            ]
        })),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    queue.run_states.insert(
        active.clone(),
        QueueRunState {
            status: "rebuilding".into(),
            mode: Some("full".into()),
            message: None,
            output_path: None,
            report_path: None,
            report_status: None,
            updated_at_ms: now_ms(),
        },
    );
    queue.run_states.insert(
        pending.clone(),
        QueueRunState {
            status: "queued".into(),
            mode: Some("full".into()),
            message: None,
            output_path: None,
            report_path: None,
            report_status: None,
            updated_at_ms: now_ms(),
        },
    );
    save(&queue_path, queue).expect("save interrupted-run fixture");

    let loaded = load(&queue_path).expect("load and recover queue");
    let states = loaded.queue.run_states;
    assert_eq!(states[&active].status, "interrupted");
    assert!(states[&active]
        .message
        .as_deref()
        .unwrap()
        .contains("while this item was active"));
    assert_eq!(states[&pending].status, "queued");
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn interrupted_session_keeps_the_original_and_removes_only_its_temporary_encode() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-recovery-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated recovery directory");
    let source = directory.join("Chaos Theory.mkv");
    let finished = directory.join("Finished.mkv");
    let output = directory.join("Finished.hevc.mkv");
    let missing = directory.join("Missing.mkv");
    fs::write(&source, b"original-video").expect("create original");
    fs::write(&finished, b"finished-source").expect("create finished source");
    fs::write(&output, b"validated-output").expect("create finished output");
    let temp =
        directory.join(".Chaos Theory.hevc-rebuild.abcdef0123456789abcdef0123456789.tmp.mkv");
    fs::write(&temp, b"partial-encode").expect("create temporary encode");
    fs::create_dir(directory.join("old")).expect("create backup folder");
    fs::write(
        directory.join("old").join("Chaos Theory.mkv"),
        b"older-backup",
    )
    .expect("create backup");
    fs::write(directory.join("Chaos Theory.hevc-rebuild.nfo"), b"nfo").expect("create nfo");
    let queue_path = directory.join("library-queue.json");
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: directory.to_string_lossy().into_owned(),
        status: QueueSourceStatus::Ready,
        scan_result: Some(serde_json::json!({
            "entries": [
                {"path": source.to_string_lossy(), "size_bytes": 15},
                {"path": finished.to_string_lossy(), "size_bytes": 16},
                {"path": missing.to_string_lossy(), "size_bytes": 1}
            ]
        })),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    queue.run_states.insert(
        source.to_string_lossy().into_owned(),
        QueueRunState {
            status: "rebuilding".into(),
            mode: Some("full".into()),
            message: None,
            output_path: None,
            report_path: None,
            report_status: None,
            updated_at_ms: now_ms(),
        },
    );
    queue.run_states.insert(
        finished.to_string_lossy().into_owned(),
        QueueRunState {
            status: "validating".into(),
            mode: Some("full".into()),
            message: None,
            output_path: Some(output.to_string_lossy().into_owned()),
            report_path: None,
            report_status: Some("OK".into()),
            updated_at_ms: now_ms(),
        },
    );
    queue.run_states.insert(
        missing.to_string_lossy().into_owned(),
        QueueRunState {
            status: "running".into(),
            mode: Some("full".into()),
            message: None,
            output_path: None,
            report_path: None,
            report_status: None,
            updated_at_ms: now_ms(),
        },
    );
    save(&queue_path, queue).expect("save recovery fixture");

    let recovery = reconcile_interrupted(&queue_path).expect("recover interrupted files");
    assert!(!recovery.skipped);
    assert_eq!(recovery.cleared, 1);
    assert_eq!(recovery.completed, 1);
    assert_eq!(recovery.held, 1);
    assert_eq!(recovery.removed_temporary_files, 1);
    assert!(!temp.exists());
    assert_eq!(fs::read(&source).unwrap(), b"original-video");
    assert_eq!(
        fs::read(directory.join("old").join("Chaos Theory.mkv")).unwrap(),
        b"older-backup"
    );
    assert_eq!(
        fs::read(directory.join("Chaos Theory.hevc-rebuild.nfo")).unwrap(),
        b"nfo"
    );
    assert_eq!(fs::read(&output).unwrap(), b"validated-output");
    assert!(!recovery
        .run_states
        .contains_key(source.to_string_lossy().as_ref()));
    assert_eq!(
        recovery.run_states[&finished.to_string_lossy().into_owned()].status,
        "completed"
    );
    assert_eq!(
        recovery.run_states[&missing.to_string_lossy().into_owned()].status,
        "needs_review"
    );
    fs::remove_dir_all(directory).expect("remove isolated recovery directory");
}

#[test]
fn operation_stage_states_are_active_until_they_reach_a_terminal_state() {
    for status in [
        "running",
        "scanning",
        "renaming",
        "remuxing",
        "rebuilding",
        "validating",
    ] {
        assert!(
            is_active_run_status(status),
            "{status} should be recovered as interrupted"
        );
    }
    for status in [
        "queued",
        "renamed",
        "completed",
        "failed",
        "skipped",
        "interrupted",
        "needs_review",
    ] {
        assert!(
            !is_active_run_status(status),
            "{status} should remain intact after reload"
        );
    }
}

#[test]
fn run_states_require_known_paths_and_supported_values() {
    let mut queue = QueueDocument::default();
    queue.run_states.insert(
        media_path("missing.mkv"),
        QueueRunState {
            status: "completed".into(),
            mode: Some("full".into()),
            message: None,
            output_path: None,
            report_path: None,
            report_status: None,
            updated_at_ms: now_ms(),
        },
    );
    assert!(validate(&queue)
        .unwrap_err()
        .contains("analyzed queue file"));
}

#[test]
fn movie_review_keeps_edited_fields_and_selected_attributed_poster() {
    let mut queue = QueueDocument::default();
    queue
        .movie_reviews
        .insert(media_path("film.mkv"), sample_review());
    assert!(validate(&queue).is_ok());
    let encoded = serde_json::to_value(&queue).expect("queue serializes");
    let restored = serde_json::from_value::<QueueDocument>(encoded).expect("queue restores");
    let restored_review = restored.movie_reviews.get(&media_path("film.mkv")).unwrap();
    assert_eq!(restored_review.title, "Gnomeo & Juliet");
    assert_eq!(restored_review.poster_id, "tmdb:poster-1");
    assert_eq!(
        restored_review.local_poster_path,
        media_path("Gnomeo & Juliet-poster.jpg")
    );
    assert_eq!(
        restored_review.artwork_candidates[0].license.as_deref(),
        Some("TMDB terms")
    );
}

#[test]
fn older_movie_and_series_reviews_default_local_artwork_choices() {
    let mut movie = serde_json::to_value(sample_review()).expect("movie review serializes");
    movie.as_object_mut().unwrap().remove("localPosterPath");
    let restored_movie = serde_json::from_value::<QueueMovieReview>(movie)
        .expect("older movie review remains readable");
    assert!(restored_movie.local_poster_path.is_empty());

    let mut series =
        serde_json::to_value(sample_series_review()).expect("series review serializes");
    series.as_object_mut().unwrap().remove("localPosterPath");
    series
        .as_object_mut()
        .unwrap()
        .remove("localSeasonPosterPaths");
    let restored_series = serde_json::from_value::<QueueSeriesReview>(series)
        .expect("older series review remains readable");
    assert!(restored_series.local_poster_path.is_empty());
    assert!(restored_series.local_season_poster_paths.is_empty());
}

#[test]
fn review_local_artwork_requires_absolute_paths_and_canonical_season_keys() {
    let mut queue = QueueDocument::default();
    let mut movie = sample_review();
    movie.local_poster_path = "posters/film.jpg".into();
    queue.movie_reviews.insert(media_path("film.mkv"), movie);
    assert!(validate(&queue).unwrap_err().contains("absolute"));

    queue.movie_reviews.clear();
    let mut series = sample_series_review();
    series
        .local_season_poster_paths
        .insert("01".into(), media_path("show/Season 01/poster.jpg"));
    queue.series_reviews.insert(media_path("show"), series);
    assert!(validate(&queue)
        .unwrap_err()
        .contains("bounded season numbers"));

    let mut series = sample_series_review();
    series.local_poster_path = "https://image.tmdb.org/t/p/original/series.jpg".into();
    series.local_season_poster_paths.insert(
        "1".into(),
        "https://artworks.thetvdb.com/banners/season.jpg".into(),
    );
    queue.series_reviews.insert(media_path("show"), series);
    assert!(validate(&queue).is_ok());
}

#[test]
fn saving_movie_reviews_preserves_existing_queue_sources() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let queue_path = directory.join("library-queue.json");
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: media_path("library"),
        status: QueueSourceStatus::Ready,
        scan_result: None,
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    queue.active_root = Some(media_path("library"));
    save(&queue_path, queue).expect("save source queue");

    let mut reviews = BTreeMap::new();
    reviews.insert(media_path("film.mkv"), sample_review());
    save_movie_reviews(&queue_path, reviews).expect("save movie review map");

    let bytes = fs::read(&queue_path).expect("read saved queue");
    let restored = serde_json::from_slice::<QueueDocument>(&bytes).expect("restore saved queue");
    assert_eq!(restored.sources.len(), 1);
    assert_eq!(
        restored.active_root.as_deref(),
        Some(media_path("library").as_str())
    );
    assert_eq!(
        restored.movie_reviews[&media_path("film.mkv")].title,
        "Gnomeo & Juliet"
    );
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn movie_review_rejects_unsafe_poster_urls_and_unmatched_selection() {
    let mut queue = QueueDocument::default();
    let mut review = sample_review();
    review.artwork_candidates[0].image_url = "http://example.invalid/poster.jpg".into();
    queue.movie_reviews.insert(media_path("film.mkv"), review);
    assert!(validate(&queue).is_err());

    let mut review = sample_review();
    review.poster_id = "tmdb:unknown-poster".into();
    queue.movie_reviews.insert(media_path("film.mkv"), review);
    assert!(validate(&queue).is_err());
}

#[test]
fn series_review_round_trips_episode_matches_and_edits() {
    let mut queue = QueueDocument::default();
    queue
        .series_reviews
        .insert(media_path("Example Series"), sample_series_review());
    assert!(validate(&queue).is_ok());
    let encoded = serde_json::to_value(&queue).expect("queue serializes");
    let restored = serde_json::from_value::<QueueDocument>(encoded).expect("queue restores");
    let review = restored
        .series_reviews
        .get(&media_path("Example Series"))
        .unwrap();
    assert_eq!(review.title, "Example Series");
    assert_eq!(review.poster_id, "tmdb:poster-1");
    assert_eq!(
        review.local_poster_path,
        media_path("Example Series/poster.jpg")
    );
    assert_eq!(
        review.local_season_poster_paths["1"],
        media_path("Example Series/Season 01/poster.jpg")
    );
    assert_eq!(review.episodes[0].title, "The Second Hour");
    assert_eq!(
        review.episode_assignments[&media_path("Example Series/Season 01/episode.mkv")]
            .summary
            .as_deref(),
        Some("Edited episode summary.")
    );
}

#[test]
fn saving_series_reviews_preserves_queue_sources_and_movie_reviews() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-series-queue-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let queue_path = directory.join("library-queue.json");
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: media_path("TV"),
        status: QueueSourceStatus::Ready,
        scan_result: None,
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: Vec::new(),
    });
    queue
        .movie_reviews
        .insert(media_path("film.mkv"), sample_review());
    save(&queue_path, queue).expect("save existing queue");

    let reviews = BTreeMap::from([(media_path("TV/Example Series"), sample_series_review())]);
    save_series_reviews(&queue_path, reviews).expect("save series reviews");
    let restored =
        serde_json::from_slice::<QueueDocument>(&fs::read(&queue_path).expect("read saved queue"))
            .expect("restore saved queue");
    assert_eq!(restored.sources.len(), 1);
    assert_eq!(
        restored.movie_reviews[&media_path("film.mkv")].title,
        "Gnomeo & Juliet"
    );
    assert_eq!(
        restored.series_reviews[&media_path("TV/Example Series")].title,
        "Example Series"
    );
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}

#[test]
fn series_review_rejects_duplicate_episodes_and_non_https_sources() {
    let mut queue = QueueDocument::default();
    let mut review = sample_series_review();
    review.episodes.push(review.episodes[0].clone());
    queue
        .series_reviews
        .insert(media_path("Example Series"), review);
    assert!(validate(&queue).is_err());

    let mut review = sample_series_review();
    review.episodes[0].source_url = "http://example.invalid/episode".into();
    queue
        .series_reviews
        .insert(media_path("Example Series"), review);
    assert!(validate(&queue).is_err());
}

#[test]
fn queue_preserves_source_order_entry_order_and_hidden_files() {
    let directory = std::env::temp_dir().join(format!(
        "hevc-rebuild-queue-order-test-{}-{}",
        std::process::id(),
        now_ms()
    ));
    fs::create_dir(&directory).expect("create isolated queue test directory");
    let first = directory.join("first.mkv");
    let second = directory.join("second.mkv");
    fs::write(&first, []).expect("create first fixture");
    fs::write(&second, []).expect("create second fixture");
    let queue_path = directory.join("library-queue.json");
    let mut queue = QueueDocument::default();
    queue.sources.push(QueueSource {
        root: media_path("later"),
        status: QueueSourceStatus::Ready,
        scan_result: Some(json!({"entries": [{"path": first}]})),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: vec![second.to_string_lossy().into_owned()],
        entry_order: vec![first.to_string_lossy().into_owned()],
    });
    queue.sources.push(QueueSource {
        root: media_path("earlier"),
        status: QueueSourceStatus::Ready,
        scan_result: Some(json!({"entries": [{"path": second}]})),
        error: None,
        updated_at_ms: now_ms(),
        changed_files: Vec::new(),
        missing_files: Vec::new(),
        excluded_paths: Vec::new(),
        entry_order: vec![second.to_string_lossy().into_owned()],
    });
    queue.sources.swap(0, 1);
    save(&queue_path, queue).expect("save reordered queue");
    let restored = load(&queue_path).expect("reload ordered queue").queue;
    assert_eq!(
        restored
            .sources
            .iter()
            .map(|source| source.root.clone())
            .collect::<Vec<_>>(),
        vec![media_path("earlier"), media_path("later")]
    );
    assert_eq!(
        restored.sources[1].excluded_paths,
        vec![second.to_string_lossy()]
    );
    assert_eq!(
        restored.sources[1].entry_order,
        vec![first.to_string_lossy()]
    );
    fs::remove_dir_all(directory).expect("remove isolated queue test directory");
}
