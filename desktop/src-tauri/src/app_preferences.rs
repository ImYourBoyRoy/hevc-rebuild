//! Saved application preferences: defaults, normalization and the load/save commands.

use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppPreferencesPayload {
    preferences: Value,
    settings_file: String,
    has_saved_settings: bool,
    portable: bool,
    storage_warning: Option<String>,
}

static PREFERENCES_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn app_preferences_file() -> Result<PathBuf, String> {
    Ok(app_config_directory()?.join("settings.json"))
}

fn default_app_preferences() -> Value {
    json!({
        "theme": "light",
        "windowState": {"width": 1600, "height": 1000, "maximized": false},
        "lastPage": "home",
        "root": null,
        "toolPaths": {
            "ffmpeg": null,
            "ffprobe": null,
            "mediainfo": null,
            "mkvpropedit": null,
            "mkvmerge": null,
            "mkvextract": null
        },
        "recursive": true,
        "minimumSizeMib": "5",
        "stopOnError": false,
        "youtubeResolution": "1440p",
        "youtubeQualityMode": "balanced",
        "youtubeSourceMultiplier": 2.0,
        "youtubeMode": "upscale",
        "youtubeSdrCodec": "h264",
        "youtubePreset": "slow",
        "youtubeShrink": "auto",
        "youtubeQualityCheck": "standard",
        "youtubeOutputDir": null,
        "rebuildOutputDir": null,
        "fileOutputDirs": {},
        "filenameIncludePatterns": [],
        "filenameExcludePatterns": [
            "old", ".*", ".renamethethings", "@eadir", "extras", "extra", "trailers", "trailer",
            "samples", "sample", "behind the scenes", "behindthescenes", "deleted scenes",
            "deleted", "interviews", "interview", "scenes", "clips", "clip", "featurettes",
            "featurette", "shorts", "short", "other", "theme-music", "thememusic",
            "backdrops", "extrafanart"
        ],
        "filenameExcludeDefaultsVersion": 2,
        "savePosterBesideMedia": true,
        "autoUpdateTools": false,
        "metadataAutoLookup": true,
        "metadataLanguage": "en-US",
        "metadataSources": {
            "tmdb": true,
            "wikidata": true,
            "tvmaze": true,
            "tvdb": false
        },
        "organizerTemplates": {
            "movie": "{title} [[({year} {certification})]]{ext}",
            "series": "{series_title}",
            "season": "Season {season:02}",
            "episode": "S{season:02}E{episode:02}.{episode_title}{ext}"
        },
        "settings": null
    })
}

/// Every saved appearance, in the order the picker shows them. Keep in step with
/// `desktop/src/lib/themes.ts` (the frontend registry).
const APP_THEMES: [&str; 8] = [
    "light", "dark", "cinema", "oled", "dos", "phosphor", "ion", "breach",
];

fn appearance_error() -> String {
    let (last, rest) = APP_THEMES.split_last().expect("themes are listed");
    format!("Appearance must be {}, or {last}.", rest.join(", "))
}

fn normalize_app_preferences(value: Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or("Application preferences must be a JSON object.")?;
    let mut normalized = default_app_preferences();
    let result = normalized
        .as_object_mut()
        .expect("default preferences are an object");

    if let Some(theme) = object.get("theme") {
        let theme = theme.as_str().ok_or_else(appearance_error)?;
        let theme = match theme {
            "hacker" => "phosphor",
            // The short-lived high-contrast theme was replaced by DOS; fall back to the calm light theme.
            "contrast" => "light",
            known if APP_THEMES.contains(&known) => known,
            _ => return Err(appearance_error()),
        };
        result.insert("theme".into(), json!(theme));
    }
    if let Some(last_page) = object.get("lastPage") {
        let last_page = last_page
            .as_str()
            .ok_or("The saved workspace tab must be text.")?;
        if ![
            "home", "library", "metadata", "youtube", "run", "settings", "reports", "help",
        ]
        .contains(&last_page)
        {
            return Err("The saved workspace tab is not one of the app pages.".into());
        }
        result.insert("lastPage".into(), json!(last_page));
    }
    if let Some(window_state) = object.get("windowState") {
        let window_state = window_state
            .as_object()
            .ok_or("Window state must be an object.")?;
        let width = window_state
            .get("width")
            .and_then(Value::as_u64)
            .filter(|width| (1120..=8192).contains(width))
            .ok_or("Saved window width must be between 1120 and 8192 logical pixels.")?;
        let height = window_state
            .get("height")
            .and_then(Value::as_u64)
            .filter(|height| (720..=8192).contains(height))
            .ok_or("Saved window height must be between 720 and 8192 logical pixels.")?;
        let maximized = window_state
            .get("maximized")
            .and_then(Value::as_bool)
            .ok_or("Saved maximized state must be true or false.")?;
        result.insert(
            "windowState".into(),
            json!({"width": width, "height": height, "maximized": maximized}),
        );
    }
    if let Some(value) = object.get("root") {
        if !value.is_null() && !value.as_str().is_some_and(|text| text.len() <= 8192) {
            return Err("root must be a path or null.".into());
        }
        result.insert("root".into(), value.clone());
    }
    if let Some(output_dir) = object.get("rebuildOutputDir") {
        if !output_dir.is_null()
            && !output_dir
                .as_str()
                .is_some_and(|text| text.len() <= 8192 && Path::new(text).is_absolute())
        {
            return Err("Default rebuild output folder must be an absolute path or null.".into());
        }
        result.insert("rebuildOutputDir".into(), output_dir.clone());
    }
    if let Some(output_dirs) = object.get("fileOutputDirs") {
        let output_dirs = output_dirs
            .as_object()
            .ok_or("Per-file output folders must be a JSON object.")?;
        if output_dirs.len() > 512 {
            return Err("Per-file output folders are limited to 512 media files.".into());
        }
        let mut normalized_dirs = serde_json::Map::new();
        for (source, destination) in output_dirs {
            if source.len() > 8192
                || !Path::new(source).is_absolute()
                || !destination
                    .as_str()
                    .is_some_and(|text| text.len() <= 8192 && Path::new(text).is_absolute())
            {
                return Err("Per-file output source and folder paths must be absolute and no longer than 8192 characters.".into());
            }
            normalized_dirs.insert(source.clone(), destination.clone());
        }
        result.insert("fileOutputDirs".into(), Value::Object(normalized_dirs));
    }
    for (key, label) in [
        ("filenameIncludePatterns", "include"),
        ("filenameExcludePatterns", "exclude"),
    ] {
        if let Some(value) = object.get(key) {
            let mut patterns = value
                .as_array()
                .ok_or_else(|| format!("{key} must be a list of filename patterns."))?
                .iter()
                .map(|pattern| {
                    pattern
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| format!("{key} entries must be text."))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if label == "exclude"
                && object
                    .get("filenameExcludeDefaultsVersion")
                    .and_then(Value::as_u64)
                    .is_none_or(|version| version < 2)
            {
                let defaults_value = default_app_preferences();
                let defaults = defaults_value["filenameExcludePatterns"]
                    .as_array()
                    .expect("default filename exclusions are an array")
                    .iter()
                    .filter_map(Value::as_str);
                let mut merged = defaults.map(str::to_owned).collect::<Vec<_>>();
                for pattern in patterns {
                    if !merged
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(&pattern))
                    {
                        merged.push(pattern);
                    }
                }
                patterns = merged;
            }
            media_scan::validate_filename_patterns(&patterns, label)?;
            result.insert(key.into(), json!(patterns));
        }
    }
    result.insert("filenameExcludeDefaultsVersion".into(), json!(2));
    if let Some(auto_lookup) = object.get("metadataAutoLookup") {
        if !auto_lookup.is_boolean() {
            return Err("Automatic metadata suggestions must be true or false.".into());
        }
        result.insert("metadataAutoLookup".into(), auto_lookup.clone());
    }
    if let Some(language) = object.get("metadataLanguage") {
        let language = language.as_str().ok_or("Metadata language must be text.")?;
        let normalized_language = library_metadata::validate_metadata_language(language)?;
        result.insert("metadataLanguage".into(), json!(normalized_language));
    }
    if let Some(sources) = object.get("metadataSources") {
        let sources = sources
            .as_object()
            .ok_or("Metadata source preferences must be an object.")?;
        let mut normalized_sources = default_app_preferences()["metadataSources"].clone();
        let normalized_source_object = normalized_sources
            .as_object_mut()
            .expect("default metadata sources are an object");
        for source in ["tmdb", "wikidata", "tvmaze", "tvdb"] {
            if let Some(enabled) = sources.get(source) {
                if !enabled.is_boolean() {
                    return Err(format!(
                        "The {source} source setting must be true or false."
                    ));
                }
                normalized_source_object.insert(source.into(), enabled.clone());
            }
        }
        result.insert("metadataSources".into(), normalized_sources);
    }
    let mut tool_paths = object
        .get("toolPaths")
        .cloned()
        .map(serde_json::from_value::<ToolPaths>)
        .transpose()
        .map_err(|error| format!("Tool executable settings are invalid: {error}"))?
        .unwrap_or_default();
    if let Some(legacy_ffmpeg) = object.get("ffmpegPath") {
        if !legacy_ffmpeg.is_null()
            && !legacy_ffmpeg
                .as_str()
                .is_some_and(|text| text.len() <= 8192)
        {
            return Err("ffmpegPath must be a path or null.".into());
        }
        if tool_paths.ffmpeg.is_none() {
            tool_paths.ffmpeg = legacy_ffmpeg.as_str().map(str::to_string);
        }
    }
    for (name, value) in [
        ("FFmpeg", &tool_paths.ffmpeg),
        ("FFprobe", &tool_paths.ffprobe),
        ("MediaInfo", &tool_paths.mediainfo),
        ("MKVToolNix mkvpropedit", &tool_paths.mkvpropedit),
        ("MKVToolNix mkvmerge", &tool_paths.mkvmerge),
        ("MKVToolNix mkvextract", &tool_paths.mkvextract),
    ] {
        if value.as_ref().is_some_and(|path| path.len() > 8192) {
            return Err(format!("{name} path must be 8192 characters or fewer."));
        }
    }
    result.insert(
        "toolPaths".into(),
        serde_json::to_value(tool_paths)
            .map_err(|error| format!("Could not normalize tool paths: {error}"))?,
    );
    if let Some(recursive) = object.get("recursive") {
        if !recursive.is_boolean() {
            return Err("Folder recursion must be true or false.".into());
        }
        result.insert("recursive".into(), recursive.clone());
    }
    if let Some(stop_on_error) = object.get("stopOnError") {
        if !stop_on_error.is_boolean() {
            return Err("Stop on first error must be true or false.".into());
        }
        result.insert("stopOnError".into(), stop_on_error.clone());
    }
    if let Some(auto_update) = object.get("autoUpdateTools") {
        if !auto_update.is_boolean() {
            return Err("Keep tools up to date automatically must be true or false.".into());
        }
        result.insert("autoUpdateTools".into(), auto_update.clone());
    }
    if let Some(save_poster_beside_media) = object.get("savePosterBesideMedia") {
        if !save_poster_beside_media.is_boolean() {
            return Err("Save poster beside media must be true or false.".into());
        }
        result.insert(
            "savePosterBesideMedia".into(),
            save_poster_beside_media.clone(),
        );
    }
    if let Some(resolution) = object.get("youtubeResolution") {
        let resolution = resolution
            .as_str()
            .ok_or("YouTube resolution must be source, 1440p, or 4k.")?;
        if !["source", "1440p", "4k"].contains(&resolution) {
            return Err("YouTube resolution must be source, 1440p, or 4k.".into());
        }
        result.insert("youtubeResolution".into(), json!(resolution));
    }
    // Older profiles saved the old `source` default whenever any preference
    // changed. Treat that pre-quality-mode value as the old default once; a
    // current profile with youtubeQualityMode set can still explicitly choose source.
    if object.get("youtubeQualityMode").is_none()
        && object.get("youtubeResolution").and_then(Value::as_str) == Some("source")
    {
        result.insert("youtubeResolution".into(), json!("1440p"));
    }
    if let Some(quality_mode) = object.get("youtubeQualityMode") {
        let quality_mode = quality_mode
            .as_str()
            .ok_or("YouTube size profile must be balanced or recommended.")?;
        if !["balanced", "recommended"].contains(&quality_mode) {
            return Err("YouTube size profile must be balanced or recommended.".into());
        }
        result.insert("youtubeQualityMode".into(), json!(quality_mode));
    }
    if let Some(multiplier) = object.get("youtubeSourceMultiplier") {
        let multiplier = multiplier
            .as_f64()
            .ok_or("YouTube source-size multiplier must be a number between 0.5 and 4.0.")?;
        if !multiplier.is_finite() || !(0.5..=4.0).contains(&multiplier) {
            return Err("YouTube source-size multiplier must be between 0.5 and 4.0.".into());
        }
        result.insert("youtubeSourceMultiplier".into(), json!(multiplier));
    }
    for (key, label, allowed) in YOUTUBE_CHOICE_PREFERENCES {
        if let Some(value) = object.get(*key) {
            let text = value
                .as_str()
                .filter(|text| allowed.contains(text))
                .ok_or_else(|| format!("{label} must be one of: {}.", allowed.join(", ")))?;
            result.insert((*key).into(), json!(text));
        }
    }
    if let Some(output_dir) = object.get("youtubeOutputDir") {
        if !output_dir.is_null() && !output_dir.as_str().is_some_and(|text| text.len() <= 8192) {
            return Err("YouTube output folder must be a path or null.".into());
        }
        result.insert("youtubeOutputDir".into(), output_dir.clone());
    }
    if let Some(templates) = object.get("organizerTemplates") {
        let templates = templates
            .as_object()
            .ok_or("Organizer naming patterns must be an object.")?;
        let mut normalized_templates = default_app_preferences()["organizerTemplates"].clone();
        let normalized_object = normalized_templates
            .as_object_mut()
            .expect("default organizer templates are an object");
        for name in ["movie", "series", "season", "episode"] {
            if let Some(value) = templates.get(name) {
                let pattern = value
                    .as_str()
                    .ok_or_else(|| format!("The {name} naming pattern must be text."))?;
                if pattern.trim().is_empty() || pattern.len() > 1_024 {
                    return Err(format!(
                        "The {name} naming pattern must contain 1 to 1,024 UTF-8 bytes."
                    ));
                }
                normalized_object.insert(name.into(), json!(pattern));
            }
        }
        result.insert("organizerTemplates".into(), normalized_templates);
    }
    if let Some(minimum) = object.get("minimumSizeMib") {
        let text = minimum.as_str().ok_or("Minimum file size must be text.")?;
        let number = text
            .parse::<f64>()
            .map_err(|_| "Minimum file size must be a number.")?;
        if !number.is_finite() || !(0.0..=4096.0).contains(&number) {
            return Err("Minimum file size must be between 0 and 4096 MiB.".into());
        }
        result.insert("minimumSizeMib".into(), json!(text));
    }
    if let Some(settings) = object.get("settings") {
        if !settings.is_null() {
            settings_arguments(settings)?;
        }
        result.insert("settings".into(), settings.clone());
    }
    Ok(normalized)
}

fn preference_payload(
    preferences: Value,
    has_saved_settings: bool,
) -> Result<AppPreferencesPayload, String> {
    Ok(AppPreferencesPayload {
        preferences,
        settings_file: app_preferences_file()?.display().to_string(),
        has_saved_settings,
        portable: std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR").is_some(),
        storage_warning: std::env::var("HEVC_REBUILD_PORTABLE_STORAGE_WARNING").ok(),
    })
}

#[tauri::command]
pub(crate) fn load_app_preferences() -> Result<AppPreferencesPayload, String> {
    let path = app_preferences_file()?;
    let has_saved_settings = path.is_file();
    let preferences = if has_saved_settings {
        let bytes = fs::read(&path)
            .map_err(|error| format!("Could not read application settings: {error}"))?;
        let parsed = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Application settings are not valid JSON: {error}"))?;
        normalize_app_preferences(parsed)?
    } else {
        default_app_preferences()
    };
    preference_payload(preferences, has_saved_settings)
}

#[tauri::command]
pub(crate) fn save_app_preferences(preferences: Value) -> Result<AppPreferencesPayload, String> {
    let normalized = normalize_app_preferences(preferences)?;
    let _guard = PREFERENCES_WRITE_LOCK
        .lock()
        .map_err(|_| "Could not lock application settings.")?;
    let path = app_preferences_file()?;
    let parent = path
        .parent()
        .ok_or("Could not locate the application settings folder.")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create the application settings folder: {error}"))?;
    let temp = Path::new(parent).join(format!("settings.json.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&normalized)
        .map_err(|error| format!("Could not encode application settings: {error}"))?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let write_result = (|| {
        let mut file = options
            .open(&temp)
            .map_err(|error| format!("Could not write application settings: {error}"))?;
        file.write_all(&bytes)
            .map_err(|error| format!("Could not write application settings: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush application settings: {error}"))
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("Could not save application settings: {error}")
    })?;
    preference_payload(normalized, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_tool_updates_default_off_and_accept_only_booleans() {
        let defaults = normalize_app_preferences(json!({})).unwrap();
        assert_eq!(
            defaults["autoUpdateTools"], false,
            "never on unless the owner opts in"
        );
        for value in [true, false] {
            let saved = normalize_app_preferences(json!({ "autoUpdateTools": value })).unwrap();
            assert_eq!(saved["autoUpdateTools"], value);
        }
        for bad in [json!("yes"), json!(1), json!(null)] {
            let error = normalize_app_preferences(json!({ "autoUpdateTools": bad })).unwrap_err();
            assert!(error.contains("Keep tools up to date"), "{error}");
        }
    }

    #[test]
    fn every_theme_is_accepted_and_the_legacy_hacker_skin_upgrades() {
        for theme in [
            "light", "dark", "cinema", "oled", "dos", "phosphor", "ion", "breach",
        ] {
            let preferences = normalize_app_preferences(json!({ "theme": theme }))
                .unwrap_or_else(|error| panic!("{theme} must be accepted: {error}"));
            assert_eq!(preferences["theme"], theme);
        }
        let legacy = normalize_app_preferences(json!({ "theme": "hacker" })).expect("legacy skin");
        assert_eq!(legacy["theme"], "phosphor");
        let retired =
            normalize_app_preferences(json!({ "theme": "contrast" })).expect("retired theme");
        assert_eq!(retired["theme"], "light");
    }

    #[test]
    fn unknown_or_non_text_themes_are_rejected_with_the_full_list() {
        for bad in [
            json!("neon"),
            json!("Dark"),
            json!(""),
            json!(3),
            json!(null),
        ] {
            let error = normalize_app_preferences(json!({ "theme": bad }))
                .expect_err("unknown theme must be rejected");
            for theme in [
                "light", "dark", "cinema", "oled", "dos", "phosphor", "ion", "breach",
            ] {
                assert!(error.contains(theme), "{error:?} must list {theme}");
            }
        }
    }

    #[test]
    fn organizer_templates_are_saved_and_missing_values_keep_defaults() {
        let preferences = normalize_app_preferences(json!({
            "organizerTemplates": {"movie": "{title}{ext}"}
        }))
        .expect("accept valid organizer preferences");
        assert_eq!(preferences["organizerTemplates"]["movie"], "{title}{ext}");
        assert_eq!(
            preferences["organizerTemplates"]["series"],
            "{series_title}"
        );
        assert_eq!(
            preferences["organizerTemplates"]["season"],
            "Season {season:02}"
        );
    }

    #[test]
    fn organizer_templates_reject_empty_or_oversized_patterns() {
        assert!(normalize_app_preferences(json!({
            "organizerTemplates": {"movie": "  "}
        }))
        .is_err());
        assert!(normalize_app_preferences(json!({
            "organizerTemplates": {"episode": "x".repeat(1025)}
        }))
        .is_err());
    }

    #[test]
    fn rebuild_output_preferences_default_to_in_place_and_normalize_folder_maps() {
        let defaults = default_app_preferences();
        assert!(defaults["rebuildOutputDir"].is_null());
        assert_eq!(defaults["fileOutputDirs"], json!({}));
        // Absolute on whatever platform the tests run on.
        let base = crate::test_temp_base();
        let output_dir = base.join("rebuild-output").to_string_lossy().into_owned();
        let one_file_dir = base.join("one-file").to_string_lossy().into_owned();
        let source_file = base.join("source.mkv").to_string_lossy().into_owned();

        let normalized = normalize_app_preferences(json!({
            "rebuildOutputDir": output_dir,
            "fileOutputDirs": {source_file.clone(): one_file_dir}
        }))
        .expect("accept absolute output destinations");
        assert_eq!(normalized["rebuildOutputDir"], output_dir);
        assert_eq!(
            normalized["fileOutputDirs"][source_file.as_str()],
            one_file_dir
        );
        assert!(normalize_app_preferences(json!({"rebuildOutputDir": "relative"})).is_err());
        assert!(normalize_app_preferences(json!({
            "fileOutputDirs": {"relative.mkv": output_dir}
        }))
        .is_err());
    }

    #[test]
    fn saved_workspace_tab_defaults_to_home_and_rejects_unknown_pages() {
        let defaults = default_app_preferences();
        assert_eq!(defaults["lastPage"], "home");
        let restored = normalize_app_preferences(json!({"lastPage": "library"}))
            .expect("restore a known workspace tab");
        assert_eq!(restored["lastPage"], "library");
        assert!(normalize_app_preferences(json!({"lastPage": "dashboard"})).is_err());
    }

    #[test]
    fn window_preferences_default_wide_and_reject_unusable_dimensions() {
        let defaults = default_app_preferences();
        assert_eq!(defaults["windowState"]["width"], 1600);
        assert_eq!(defaults["windowState"]["height"], 1000);

        let restored = normalize_app_preferences(json!({
            "windowState": {"width": 1720, "height": 1080, "maximized": true}
        }))
        .expect("restore a previously saved window state");
        assert_eq!(
            restored["windowState"],
            json!({"width": 1720, "height": 1080, "maximized": true})
        );
        assert!(normalize_app_preferences(json!({
            "windowState": {"width": 900, "height": 900, "maximized": false}
        }))
        .is_err());
        assert!(normalize_app_preferences(json!({
            "windowState": {"width": 1600, "height": 500, "maximized": false}
        }))
        .is_err());
    }

    #[test]
    fn legacy_profiles_receive_removable_renamer_exclusions_once() {
        let migrated = normalize_app_preferences(json!({
            "filenameExcludePatterns": ["my-custom-extra"]
        }))
        .expect("migrate a legacy preference profile");
        let exclusions = migrated["filenameExcludePatterns"]
            .as_array()
            .expect("migrated exclusions are an array");
        assert!(exclusions.iter().any(|pattern| pattern == "old"));
        assert!(exclusions.iter().any(|pattern| pattern == ".*"));
        assert!(exclusions
            .iter()
            .any(|pattern| pattern == "behindthescenes"));
        assert!(exclusions
            .iter()
            .any(|pattern| pattern == "my-custom-extra"));
        assert_eq!(migrated["filenameExcludeDefaultsVersion"], 2);

        let cleared = normalize_app_preferences(json!({
            "filenameExcludePatterns": [],
            "filenameExcludeDefaultsVersion": 2
        }))
        .expect("keep a user's explicit clear-all choice");
        assert_eq!(cleared["filenameExcludePatterns"], json!([]));
    }

    #[test]
    fn app_preferences_round_trip_through_the_saved_settings_file() {
        let _guard = TEST_STATE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR");
        let root = test_dir("app-preferences");
        std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &root);

        let fresh = load_app_preferences().unwrap();
        assert!(!fresh.has_saved_settings);
        assert!(fresh.portable);
        assert_eq!(fresh.preferences, default_app_preferences());

        let saved = save_app_preferences(json!({"lastPage": "library"})).unwrap();
        assert!(saved.has_saved_settings);
        assert_eq!(saved.preferences["lastPage"], "library");
        assert!(root.join("settings.json").is_file());
        let leftovers = fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(leftovers, 0, "the temporary settings file is renamed away");
        #[cfg(unix)]
        {
            let mode = fs::metadata(root.join("settings.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let reloaded = load_app_preferences().unwrap();
        assert!(reloaded.has_saved_settings);
        assert_eq!(reloaded.preferences, saved.preferences);
        assert!(save_app_preferences(json!({"lastPage": "dashboard"})).is_err());

        match previous {
            Some(value) => std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", value),
            None => std::env::remove_var("HEVC_REBUILD_PORTABLE_DATA_DIR"),
        }
        fs::remove_dir_all(root).unwrap();
    }
}
