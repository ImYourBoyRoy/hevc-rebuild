//! Bounds and consistency checks applied to a queue document before it is saved or loaded.

use super::*;

fn saved_poster_reference(path: &str) -> bool {
    !path.trim().is_empty()
        && path.len() <= 8192
        && (path.starts_with("https://") || Path::new(path).is_absolute())
}

pub(super) fn validate(queue: &QueueDocument) -> Result<(), String> {
    if queue.version != QUEUE_VERSION {
        return Err(format!(
            "Queue format version {} is not supported by this application.",
            queue.version
        ));
    }
    if queue.sources.len() > MAX_QUEUE_SOURCES {
        return Err(format!(
            "The media queue is limited to {MAX_QUEUE_SOURCES} source folders or files."
        ));
    }
    if queue.selected_media_paths.len() > MAX_SELECTED_PATHS {
        return Err(format!(
            "The media queue can select at most {MAX_SELECTED_PATHS} files at once."
        ));
    }
    if queue
        .active_root
        .as_ref()
        .is_some_and(|root| !queue.sources.iter().any(|source| &source.root == root))
    {
        return Err("The active queue source must exist in the source list.".into());
    }
    let mut roots = std::collections::BTreeSet::new();
    let mut entry_count = 0_usize;
    let mut known_paths = std::collections::BTreeSet::new();
    for source in &queue.sources {
        if source.root.trim().is_empty() || source.root.len() > 8192 {
            return Err("A queue source path is empty or too long.".into());
        }
        if !roots.insert(source.root.as_str()) {
            return Err("The media queue contains a duplicate source path.".into());
        }
        if source.excluded_paths.len() > MAX_QUEUE_ENTRIES
            || source.entry_order.len() > MAX_QUEUE_ENTRIES
        {
            return Err(format!(
                "A queue source can retain at most {MAX_QUEUE_ENTRIES} ordered or hidden file paths."
            ));
        }
        for path in source
            .excluded_paths
            .iter()
            .chain(source.entry_order.iter())
        {
            if path.is_empty() || path.len() > 8192 {
                return Err("A queue file path is empty or too long.".into());
            }
        }
        if let Some(result) = &source.scan_result {
            let entries = result
                .get("entries")
                .and_then(Value::as_array)
                .ok_or("A saved scan result has no valid media entry list.")?;
            entry_count = entry_count.saturating_add(entries.len());
            for entry in entries {
                let path = entry
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("A saved media entry has no valid path.")?;
                if path.is_empty() || path.len() > 8192 {
                    return Err("A saved media entry path is empty or too long.".into());
                }
                known_paths.insert(path.to_owned());
            }
        }
    }
    if entry_count > MAX_QUEUE_ENTRIES {
        return Err(format!(
            "The saved media queue is limited to {MAX_QUEUE_ENTRIES} analyzed files."
        ));
    }
    let mut selected = std::collections::BTreeSet::new();
    for path in &queue.selected_media_paths {
        if path.is_empty() || path.len() > 8192 {
            return Err("A selected media path is empty or too long.".into());
        }
        if !selected.insert(path) {
            return Err("The selected media list contains duplicate paths.".into());
        }
        if !known_paths.contains(path) {
            return Err("Every selected path must belong to a saved queue source.".into());
        }
    }
    for (path, tune) in &queue.file_tune_overrides {
        if !known_paths.contains(path) || !["animation", "none"].contains(&tune.as_str()) {
            return Err("A saved animation choice is not attached to a queued file.".into());
        }
    }
    for (path, streams) in &queue.file_stream_overrides {
        if !known_paths.contains(path) || streams.is_empty() || streams.len() > 1024 {
            return Err("A saved stream selection is not attached to a queued file.".into());
        }
        let unique: std::collections::BTreeSet<_> = streams.iter().collect();
        if unique.len() != streams.len() {
            return Err("A saved stream selection contains duplicate track indices.".into());
        }
    }
    if queue.run_states.len() > MAX_RUN_STATES {
        return Err(format!(
            "The media queue can retain at most {MAX_RUN_STATES} per-file run states."
        ));
    }
    for (path, state) in &queue.run_states {
        if !known_paths.contains(path) {
            return Err("A saved run state must belong to an analyzed queue file.".into());
        }
        if ![
            "queued",
            "running",
            "scanning",
            "renaming",
            "remuxing",
            "rebuilding",
            "validating",
            "renamed",
            "completed",
            "failed",
            "skipped",
            "interrupted",
            "needs_review",
        ]
        .contains(&state.status.as_str())
        {
            return Err("A saved per-file run state has an unsupported status.".into());
        }
        if state.mode.as_deref().is_some_and(|mode| {
            ![
                "check",
                "metadata",
                "test",
                "full",
                "youtube",
                "organize",
                "integrity",
            ]
            .contains(&mode)
        }) {
            return Err("A saved per-file run mode is unsupported.".into());
        }
        for (label, value, maximum, must_be_absolute) in [
            ("run message", state.message.as_deref(), 4096, false),
            ("run output path", state.output_path.as_deref(), 8192, true),
            ("run report path", state.report_path.as_deref(), 8192, true),
            (
                "run report status",
                state.report_status.as_deref(),
                128,
                false,
            ),
        ] {
            if let Some(value) = value {
                if value.len() > maximum {
                    return Err(format!("A saved {label} is too long."));
                }
                if must_be_absolute && !value.is_empty() && !Path::new(value).is_absolute() {
                    return Err(format!("A saved {label} must be an absolute path."));
                }
            }
        }
    }
    if queue.movie_reviews.len() > MAX_MOVIE_REVIEWS {
        return Err(format!(
            "The media queue can retain at most {MAX_MOVIE_REVIEWS} reviewed movie records."
        ));
    }
    for (path, review) in &queue.movie_reviews {
        if path.trim().is_empty() || path.len() > 8192 || !Path::new(path).is_absolute() {
            return Err("A saved movie review must use an absolute media path.".into());
        }
        for (label, value, maximum) in [
            ("title", review.title.as_str(), 1024),
            ("year", review.year.as_str(), 64),
            ("certification", review.certification.as_str(), 128),
            ("provider record", review.record_id.as_str(), 1024),
            (
                "short description",
                review.short_description.as_str(),
                16_384,
            ),
            ("plot", review.plot.as_str(), 65_536),
            ("genres", review.genres_text.as_str(), 4096),
            ("rating", review.rating_text.as_str(), 64),
            ("source URL", review.source_url.as_str(), 4096),
            ("comment", review.comment.as_str(), 500),
            ("poster selection", review.poster_id.as_str(), 1024),
            ("local poster path", review.local_poster_path.as_str(), 8192),
        ] {
            if value.len() > maximum {
                return Err(format!("A saved movie review {label} is too long."));
            }
        }
        if !review.source_url.is_empty() && !review.source_url.starts_with("https://") {
            return Err("A saved movie review source URL must use HTTPS.".into());
        }
        if !review.local_poster_path.is_empty()
            && !saved_poster_reference(&review.local_poster_path)
        {
            return Err("A saved local movie poster must use an absolute path.".into());
        }
        if review.artwork_candidates.len() > MAX_ARTWORK_CANDIDATES {
            return Err(format!(
                "A movie review can retain at most {MAX_ARTWORK_CANDIDATES} poster choices."
            ));
        }
        let mut artwork_ids = std::collections::BTreeSet::new();
        for artwork in &review.artwork_candidates {
            if artwork.id.is_empty()
                || artwork.id.len() > 1024
                || !artwork_ids.insert(artwork.id.as_str())
            {
                return Err("Saved poster choices must have unique, bounded IDs.".into());
            }
            for (label, value) in [
                ("provider", artwork.provider.as_str()),
                ("title", artwork.title.as_str()),
                ("image URL", artwork.image_url.as_str()),
                ("thumbnail URL", artwork.thumbnail_url.as_str()),
                ("source URL", artwork.source_url.as_str()),
            ] {
                if value.len() > 4096 {
                    return Err(format!("A saved poster {label} is too long."));
                }
            }
            for (label, value) in [
                ("creator", artwork.creator.as_deref()),
                ("license", artwork.license.as_deref()),
                ("license URL", artwork.license_url.as_deref()),
                ("attribution", artwork.attribution.as_deref()),
            ] {
                if value.is_some_and(|text| text.len() > 4096) {
                    return Err(format!("A saved poster {label} is too long."));
                }
            }
            for url in [
                artwork.image_url.as_str(),
                artwork.thumbnail_url.as_str(),
                artwork.source_url.as_str(),
                artwork.license_url.as_deref().unwrap_or_default(),
            ] {
                if !url.is_empty() && !url.starts_with("https://") {
                    return Err("Saved poster links must use HTTPS.".into());
                }
            }
        }
        if !review.poster_id.is_empty() && !artwork_ids.contains(review.poster_id.as_str()) {
            return Err(
                "A saved poster selection must match one of its reviewed poster choices.".into(),
            );
        }
    }
    if queue.series_reviews.len() > MAX_SERIES_REVIEWS {
        return Err(format!(
            "The media queue can retain at most {MAX_SERIES_REVIEWS} reviewed TV series."
        ));
    }
    for (root, review) in &queue.series_reviews {
        if root.trim().is_empty() || root.len() > 8192 || !Path::new(root).is_absolute() {
            return Err("A saved series review must use an absolute series folder path.".into());
        }
        for (label, value, maximum) in [
            ("title", review.title.as_str(), 1024),
            ("year", review.year.as_str(), 64),
            ("certification", review.certification.as_str(), 128),
            ("provider record", review.record_id.as_str(), 1024),
            (
                "short description",
                review.short_description.as_str(),
                16_384,
            ),
            ("plot", review.plot.as_str(), 65_536),
            ("genres", review.genres_text.as_str(), 4096),
            ("rating", review.rating_text.as_str(), 64),
            ("source URL", review.source_url.as_str(), 4096),
            ("comment", review.comment.as_str(), 500),
            ("poster selection", review.poster_id.as_str(), 1024),
            ("local poster path", review.local_poster_path.as_str(), 8192),
        ] {
            if value.len() > maximum {
                return Err(format!("A saved series review {label} is too long."));
            }
        }
        if !review.source_url.is_empty() && !review.source_url.starts_with("https://") {
            return Err("A saved series review source URL must use HTTPS.".into());
        }
        if !review.local_poster_path.is_empty()
            && !saved_poster_reference(&review.local_poster_path)
        {
            return Err("A saved local series poster must use an absolute path.".into());
        }
        if review.local_season_poster_paths.len() > 200
            || review
                .local_season_poster_paths
                .iter()
                .any(|(season, path)| {
                    !season
                        .parse::<u32>()
                        .is_ok_and(|number| number.to_string() == *season)
                        || !saved_poster_reference(path)
                })
        {
            return Err(
                "Saved local season posters must use bounded season numbers and absolute paths."
                    .into(),
            );
        }
        if review.season_records.len() > 200 {
            return Err("A saved series review has too many season records.".into());
        }
        for (season, record) in &review.season_records {
            if !season
                .parse::<u32>()
                .is_ok_and(|number| number.to_string() == *season)
            {
                return Err("A saved season record must use its season number as the key.".into());
            }
            for (label, value, maximum) in [
                ("title", record.title.as_str(), 1024),
                ("year", record.year.as_str(), 64),
                ("certification", record.certification.as_str(), 128),
                ("plot", record.plot.as_str(), 65_536),
                ("genres", record.genres_text.as_str(), 4096),
                ("rating", record.rating_text.as_str(), 64),
                ("comment", record.comment.as_str(), 500),
            ] {
                if value.len() > maximum {
                    return Err(format!("A saved season {label} is too long."));
                }
            }
        }
        validate_artwork_candidates(&review.artwork_candidates, &review.poster_id, "series")?;
        if review.episodes.len() > MAX_TV_EPISODES
            || review.episode_assignments.len() > MAX_SELECTED_PATHS
        {
            return Err("Saved TV episode metadata exceeds its queue safety limit.".into());
        }
        let mut episode_keys = std::collections::BTreeSet::new();
        for episode in &review.episodes {
            validate_tv_episode(episode)?;
            if !episode_keys.insert((episode.season, episode.episode)) {
                return Err("A saved series review contains duplicate episode numbers.".into());
            }
        }
        for (path, episode) in &review.episode_assignments {
            if path.trim().is_empty() || path.len() > 8192 || !Path::new(path).is_absolute() {
                return Err("A saved episode assignment must use an absolute media path.".into());
            }
            validate_tv_episode(episode)?;
        }
    }
    if queue.match_suggestions.len() > MAX_METADATA_SUGGESTIONS {
        return Err(format!(
            "The media queue can retain at most {MAX_METADATA_SUGGESTIONS} title suggestions."
        ));
    }
    for (path, suggestion) in &queue.match_suggestions {
        if !known_paths.contains(path) {
            return Err("A title suggestion must belong to an analyzed queue file.".into());
        }
        if !["movie", "tv"].contains(&suggestion.kind.as_str())
            || !["searching", "ready", "error"].contains(&suggestion.status.as_str())
        {
            return Err("A saved title suggestion has an unsupported kind or status.".into());
        }
        for (label, value, maximum) in [
            ("suggested title", suggestion.title.as_deref(), 1024),
            ("suggested year", suggestion.year.as_deref(), 64),
            ("suggestion provider", suggestion.provider.as_deref(), 64),
            ("suggested record ID", suggestion.record_id.as_deref(), 1024),
            (
                "suggestion source URL",
                suggestion.source_url.as_deref(),
                4096,
            ),
            (
                "suggested description",
                suggestion.description.as_deref(),
                16_384,
            ),
            ("suggestion error", Some(suggestion.error.as_str()), 4096),
        ] {
            if value.is_some_and(|text| text.len() > maximum) {
                return Err(format!("A saved {label} is too long."));
            }
        }
        if suggestion
            .provider
            .as_deref()
            .is_some_and(|provider| !["wikidata", "tvmaze", "tvdb", "tmdb"].contains(&provider))
        {
            return Err("A title suggestion uses an unsupported metadata provider.".into());
        }
        if suggestion
            .source_url
            .as_deref()
            .is_some_and(|url| !url.is_empty() && !url.starts_with("https://"))
        {
            return Err("A title suggestion source URL must use HTTPS.".into());
        }
        if suggestion.auto_associated
            && (suggestion.status != "ready"
                || suggestion.title.as_deref().is_none_or(str::is_empty)
                || suggestion.year.as_deref().is_none_or(str::is_empty)
                || suggestion.record_id.as_deref().is_none_or(str::is_empty)
                || suggestion.provider.is_none())
        {
            return Err("An automatic title association requires a ready, exact-title/year provider record.".into());
        }
        if let Some(artwork) = &suggestion.artwork {
            validate_artwork_candidates(std::slice::from_ref(artwork), "", "metadata suggestion")?;
        }
    }
    Ok(())
}

fn validate_tv_episode(episode: &QueueTvEpisode) -> Result<(), String> {
    for (label, value, maximum) in [
        ("title", episode.title.as_str(), 1024),
        (
            "air date",
            episode.airdate.as_deref().unwrap_or_default(),
            64,
        ),
        (
            "summary",
            episode.summary.as_deref().unwrap_or_default(),
            16_384,
        ),
        ("source URL", episode.source_url.as_str(), 4096),
        ("comment", episode.comment.as_str(), 500),
        ("certification", episode.certification.as_str(), 64),
    ] {
        if value.len() > maximum {
            return Err(format!("A saved episode {label} is too long."));
        }
    }
    if episode.episode == 0 {
        return Err("A saved episode number must be greater than zero.".into());
    }
    if !episode.source_url.is_empty() && !episode.source_url.starts_with("https://") {
        return Err("A saved episode source URL must use HTTPS.".into());
    }
    Ok(())
}

fn validate_artwork_candidates(
    candidates: &[QueueArtworkCandidate],
    selected_id: &str,
    record_kind: &str,
) -> Result<(), String> {
    if candidates.len() > MAX_ARTWORK_CANDIDATES {
        return Err(format!(
            "A {record_kind} review can retain at most {MAX_ARTWORK_CANDIDATES} poster choices."
        ));
    }
    let mut artwork_ids = std::collections::BTreeSet::new();
    for artwork in candidates {
        if artwork.id.is_empty()
            || artwork.id.len() > 1024
            || !artwork_ids.insert(artwork.id.as_str())
        {
            return Err("Saved poster choices must have unique, bounded IDs.".into());
        }
        for (label, value) in [
            ("provider", artwork.provider.as_str()),
            ("title", artwork.title.as_str()),
            ("image URL", artwork.image_url.as_str()),
            ("thumbnail URL", artwork.thumbnail_url.as_str()),
            ("source URL", artwork.source_url.as_str()),
        ] {
            if value.len() > 4096 {
                return Err(format!("A saved poster {label} is too long."));
            }
        }
        for (label, value) in [
            ("creator", artwork.creator.as_deref()),
            ("license", artwork.license.as_deref()),
            ("license URL", artwork.license_url.as_deref()),
            ("attribution", artwork.attribution.as_deref()),
        ] {
            if value.is_some_and(|text| text.len() > 4096) {
                return Err(format!("A saved poster {label} is too long."));
            }
        }
        for url in [
            artwork.image_url.as_str(),
            artwork.thumbnail_url.as_str(),
            artwork.source_url.as_str(),
            artwork.license_url.as_deref().unwrap_or_default(),
        ] {
            if !url.is_empty() && !url.starts_with("https://") {
                return Err("Saved poster links must use HTTPS.".into());
            }
        }
    }
    if !selected_id.is_empty() && !artwork_ids.contains(selected_id) {
        return Err(
            "A saved poster selection must match one of its reviewed poster choices.".into(),
        );
    }
    Ok(())
}
