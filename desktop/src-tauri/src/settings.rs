use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub(crate) struct EffectiveSettings {
    settings: Value,
    sources: Vec<String>,
    user_config: String,
}

pub(crate) fn load_effective_settings(root: &str) -> Result<EffectiveSettings, String> {
    let user_config = user_config_path()?;
    let preferences = crate::app_preferences_file()?;
    let root = resolve_settings_root(root);
    load_effective_settings_at(&root, &user_config, &preferences)
}

pub(crate) fn metadata_text_blacklist_for_root(root: &str) -> Result<Vec<String>, String> {
    let effective = load_effective_settings(root)?;
    effective
        .settings
        .get("metadata")
        .and_then(|metadata| metadata.get("text_blacklist"))
        .and_then(Value::as_array)
        .map(|patterns| {
            patterns
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| "Effective metadata blacklist settings are unavailable.".into())
}

fn load_effective_settings_at(
    root: &Path,
    user_config: &Path,
    preferences: &Path,
) -> Result<EffectiveSettings, String> {
    let mut settings = default_settings();
    let mut sources = Vec::new();

    if user_config.exists() {
        merge_toml_file(&mut settings, user_config)?;
        sources.push(display_path(user_config));
    }

    if preferences.exists() {
        let bytes = fs::read(preferences).map_err(|error| {
            format!(
                "Could not read application preferences at {}: {error}",
                display_path(preferences)
            )
        })?;
        let parsed: Value = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "Could not read application preferences at {}: {error}",
                display_path(preferences)
            )
        })?;
        if let Some(profile) = parsed.as_object().and_then(|object| object.get("settings")) {
            if !profile.is_null() {
                if !profile.is_object() {
                    return Err(format!(
                        "Saved application settings in {} must be an object or null.",
                        display_path(preferences)
                    ));
                }
                merge_overlay(&mut settings, profile.clone(), &display_path(preferences))?;
                sources.push(display_path(preferences));
            }
        }
    }

    let local_config = root.join(".hevc-rebuild.toml");
    if local_config.exists() {
        merge_toml_file(&mut settings, &local_config)?;
        sources.push(display_path(&local_config));
    }

    validate_settings(&settings)?;
    Ok(EffectiveSettings {
        settings,
        sources,
        user_config: display_path(user_config),
    })
}

pub(crate) fn default_settings() -> Value {
    json!({
        "video": {
            "crf": 21,
            "preset": "slow",
            "promote_8bit_to_10bit": false
        },
        "content": {
            "tune": "auto",
            "detect_animation": true
        },
        "hdr": {
            "preserve_hdr10plus": true,
            "dolby_vision": "safe"
        },
        "streams": {
            "secondary_video": "encode",
            "data_streams": "preserve"
        },
        "metadata": {
            "comment": "Brought To you By V1x0r",
            "text_blacklist": ["Movie*Group"],
            "embed_artwork": true
        },
        "safety": {
            "verify_copy_hashes": true
        },
        "calibration": {
            "mode": "auto",
            "cache_hours": 24.0,
            "large_file_gib": 20.0,
            "sample_seconds": 3.0,
            "max_candidates": 2,
            "throughput_tolerance_percent": 3.0
        },
        "resources": {
            "threading": "auto",
            "reserved_cpu_threads": 1,
            "max_worker_threads": 0,
            "frame_threads": 0,
            "min_free_memory_gib": 3.0,
            "memory_reserve_percent": 8.0,
            "min_free_disk_gib": 0.0,
            "preflight_output_ratio": 1.30,
            "disk_projection_margin_percent": 15.0
        }
    })
}

fn user_config_path() -> Result<PathBuf, String> {
    if let Some(portable) = std::env::var_os("HEVC_REBUILD_PORTABLE_DATA_DIR") {
        return Ok(PathBuf::from(portable).join("engine-config.toml"));
    }
    #[cfg(target_os = "linux")]
    {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_directory().map(|home| home.join(".config")))
            .ok_or("Could not locate the current user's configuration folder.")?;
        return Ok(config_home.join("hevc-rebuild/config.toml"));
    }
    #[cfg(not(target_os = "linux"))]
    {
        return Ok(crate::app_config_directory()?.join("engine-config.toml"));
    }
}

fn resolve_settings_root(value: &str) -> PathBuf {
    let mut root = expand_home(PathBuf::from(value));
    if root.is_file() {
        if let Some(parent) = root.parent() {
            root = parent.to_path_buf();
        }
    }
    if !root.is_dir() {
        root = home_directory().unwrap_or_else(|| PathBuf::from("."));
    }
    root
}

fn home_directory() -> Option<PathBuf> {
    crate::user_home_directory()
}

fn expand_home(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    let Some(rest) = text.strip_prefix('~') else {
        return path;
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return path;
    }
    home_directory()
        .map(|home| home.join(rest.strip_prefix('/').unwrap_or(rest)))
        .unwrap_or(path)
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn merge_toml_file(settings: &mut Value, path: &Path) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Cannot read settings file {}: {error}", display_path(path)))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("Cannot read settings file {}: {error}", display_path(path)))?;
    let overlay: toml::Value = toml::from_str(text)
        .map_err(|error| format!("Cannot read settings file {}: {error}", display_path(path)))?;
    let overlay = serde_json::to_value(overlay)
        .map_err(|error| format!("Cannot read settings file {}: {error}", display_path(path)))?;
    merge_overlay(settings, overlay, &display_path(path))
}

fn merge_overlay(settings: &mut Value, overlay: Value, source: &str) -> Result<(), String> {
    let values = overlay
        .as_object()
        .ok_or_else(|| format!("Settings in {source} must be an object."))?;
    let base = settings
        .as_object_mut()
        .expect("built-in settings are a JSON object");

    for (section_name, section_values) in values {
        let Some(base_section) = base.get_mut(section_name) else {
            eprintln!("WARNING: Ignoring unknown config section [{section_name}].");
            continue;
        };
        let section_values = section_values
            .as_object()
            .ok_or_else(|| format!("Config section [{section_name}] must be a table."))?;
        let base_section = base_section
            .as_object_mut()
            .expect("built-in config sections are JSON objects");
        for (key, value) in section_values {
            if let Some(target) = base_section.get_mut(key) {
                *target = value.clone();
            } else {
                eprintln!("WARNING: Ignoring unknown config option {section_name}.{key}.");
            }
        }
    }
    Ok(())
}

fn field<'a>(settings: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut value = settings;
    for key in path.split('.') {
        value = value
            .get(key)
            .ok_or_else(|| format!("Setting {path} is missing."))?;
    }
    Ok(value)
}

fn text<'a>(
    settings: &'a Value,
    path: &str,
    allowed: &[&str],
    invalid_message: &str,
) -> Result<&'a str, String> {
    let value = field(settings, path)?
        .as_str()
        .ok_or_else(|| invalid_message.to_string())?;
    if !allowed.contains(&value) {
        return Err(invalid_message.to_string());
    }
    Ok(value)
}

fn boolean(settings: &Value, path: &str) -> Result<(), String> {
    field(settings, path)?
        .as_bool()
        .map(|_| ())
        .ok_or_else(|| format!("Config {path} must be true or false."))
}

fn integer(settings: &Value, path: &str, min: i64, max: i64) -> Result<i64, String> {
    let value = field(settings, path)?
        .as_i64()
        .ok_or_else(|| format!("Config {path} must be an integer from {min} to {max}."))?;
    if !(min..=max).contains(&value) {
        return Err(format!(
            "Config {path} must be an integer from {min} to {max}."
        ));
    }
    Ok(value)
}

fn number(
    settings: &Value,
    path: &str,
    min: f64,
    max: Option<f64>,
    range_message: &str,
) -> Result<f64, String> {
    let value = field(settings, path)?
        .as_f64()
        .ok_or_else(|| format!("Config {path} must be a number."))?;
    if !value.is_finite() || value < min || max.is_some_and(|max| value > max) {
        return Err(range_message.to_string());
    }
    Ok(value)
}

fn validate_blacklist(settings: &Value) -> Result<(), String> {
    let patterns = field(settings, "metadata.text_blacklist")?
        .as_array()
        .ok_or("Config metadata.text_blacklist must be a list of text patterns.")?;
    if patterns.len() > 100 {
        return Err("Config metadata.text_blacklist can contain at most 100 patterns.".into());
    }
    let mut seen = HashSet::new();
    for (index, pattern) in patterns.iter().enumerate() {
        let number = index + 1;
        let pattern = pattern.as_str().ok_or_else(|| {
            format!("Config metadata.text_blacklist entry {number} must be text.")
        })?;
        if pattern.is_empty() || pattern.trim() != pattern {
            return Err(format!("Config metadata.text_blacklist entry {number} must not be empty or padded with spaces."));
        }
        if pattern.len() > 128 {
            return Err(format!(
                "Config metadata.text_blacklist entry {number} is limited to 128 UTF-8 bytes."
            ));
        }
        if pattern.chars().all(|character| character == '*') {
            return Err(format!(
                "Config metadata.text_blacklist entry {number} must include text besides '*'."
            ));
        }
        if pattern.chars().any(char::is_control) {
            return Err(format!(
                "Config metadata.text_blacklist entry {number} cannot contain control characters."
            ));
        }
        if !seen.insert(pattern.to_lowercase()) {
            return Err("Config metadata.text_blacklist cannot contain duplicate patterns.".into());
        }
    }
    Ok(())
}

fn validate_settings(settings: &Value) -> Result<(), String> {
    integer(settings, "video.crf", 0, 51)?;
    text(
        settings,
        "video.preset",
        &[
            "ultrafast",
            "superfast",
            "veryfast",
            "faster",
            "fast",
            "medium",
            "slow",
            "slower",
            "veryslow",
        ],
        "Config video.preset must be one of: fast, faster, medium, slow, slower, superfast, ultrafast, veryfast, veryslow.",
    )?;
    boolean(settings, "video.promote_8bit_to_10bit")?;
    text(
        settings,
        "content.tune",
        &["auto", "animation", "grain", "none"],
        "Config content.tune must be auto, animation, grain, or none.",
    )?;
    boolean(settings, "content.detect_animation")?;
    boolean(settings, "hdr.preserve_hdr10plus")?;
    text(
        settings,
        "hdr.dolby_vision",
        &["safe", "skip", "hdr10-fallback"],
        "Config hdr.dolby_vision must be safe, skip, or hdr10-fallback.",
    )?;
    text(
        settings,
        "streams.secondary_video",
        &["encode", "copy", "drop"],
        "Config streams.secondary_video must be encode, copy, or drop.",
    )?;
    text(
        settings,
        "streams.data_streams",
        &["preserve", "drop"],
        "Config streams.data_streams must be preserve or drop.",
    )?;
    field(settings, "metadata.comment")?
        .as_str()
        .ok_or("Config metadata.comment must be text.")?;
    field(settings, "metadata.embed_artwork")?
        .as_bool()
        .ok_or("Config metadata.embed_artwork must be true or false.")?;
    validate_blacklist(settings)?;
    boolean(settings, "safety.verify_copy_hashes")?;
    text(
        settings,
        "calibration.mode",
        &["auto", "ask", "always", "off"],
        "Config calibration.mode must be auto, ask, always, or off.",
    )?;
    number(
        settings,
        "calibration.cache_hours",
        0.0,
        Some(720.0),
        "Config calibration.cache_hours must be from 0 to 720.",
    )?;
    number(
        settings,
        "calibration.large_file_gib",
        0.0,
        Some(1000.0),
        "Config calibration.large_file_gib must be from 0 to 1000.",
    )?;
    number(
        settings,
        "calibration.sample_seconds",
        1.0,
        Some(15.0),
        "Config calibration.sample_seconds must be from 1 to 15.",
    )?;
    integer(settings, "calibration.max_candidates", 1, 4)?;
    number(
        settings,
        "calibration.throughput_tolerance_percent",
        0.0,
        Some(20.0),
        "Config calibration.throughput_tolerance_percent must be from 0 to 20.",
    )?;
    text(
        settings,
        "resources.threading",
        &["auto", "unlimited"],
        "Config resources.threading must be auto or unlimited.",
    )?;
    integer(settings, "resources.reserved_cpu_threads", 0, 64)?;
    integer(settings, "resources.max_worker_threads", 0, 64)?;
    integer(settings, "resources.frame_threads", 0, 16)?;
    number(
        settings,
        "resources.min_free_memory_gib",
        0.0,
        None,
        "Config resources.min_free_memory_gib must be 0 or greater.",
    )?;
    number(
        settings,
        "resources.memory_reserve_percent",
        0.0,
        Some(50.0),
        "Config resources.memory_reserve_percent must be from 0 to 50.",
    )?;
    number(
        settings,
        "resources.min_free_disk_gib",
        0.0,
        None,
        "Config resources.min_free_disk_gib must be 0 or greater.",
    )?;
    number(
        settings,
        "resources.preflight_output_ratio",
        0.25,
        Some(4.0),
        "Config resources.preflight_output_ratio must be from 0.25 to 4.0.",
    )?;
    number(
        settings,
        "resources.disk_projection_margin_percent",
        0.0,
        Some(100.0),
        "Config resources.disk_projection_margin_percent must be from 0 to 100.",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("hevc-settings-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("scratch folder should be created");
            Self(path)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn effective_precedence_is_builtin_then_user_then_app_then_folder() {
        let temp = ScratchDir::new();
        let user = temp.0.join("user.toml");
        let app = temp.0.join("settings.json");
        fs::write(&user, "[video]\ncrf = 23\n").unwrap();
        fs::write(&app, r#"{"settings":{"video":{"crf":25}}}"#).unwrap();
        fs::write(temp.0.join(".hevc-rebuild.toml"), "[video]\ncrf = 27\n").unwrap();

        let result = load_effective_settings_at(&temp.0, &user, &app).unwrap();
        assert_eq!(result.settings["video"]["crf"], 27);
        assert_eq!(result.sources.len(), 3);
        assert_eq!(
            result.settings["metadata"]["comment"],
            "Brought To you By V1x0r"
        );
    }

    #[test]
    fn application_profile_can_be_null_and_folder_overrides_user_config() {
        let temp = ScratchDir::new();
        let user = temp.0.join("user.toml");
        let app = temp.0.join("settings.json");
        fs::write(&user, "[resources]\nreserved_cpu_threads = 2\n").unwrap();
        fs::write(&app, r#"{"settings":null}"#).unwrap();
        fs::write(
            temp.0.join(".hevc-rebuild.toml"),
            "[resources]\nreserved_cpu_threads = 3\n",
        )
        .unwrap();

        let result = load_effective_settings_at(&temp.0, &user, &app).unwrap();
        assert_eq!(result.settings["resources"]["reserved_cpu_threads"], 3);
        assert_eq!(result.sources.len(), 2);
    }

    #[test]
    fn invalid_ranges_and_blacklist_patterns_are_rejected() {
        let mut settings = default_settings();
        settings["video"]["crf"] = json!(52);
        assert!(validate_settings(&settings)
            .unwrap_err()
            .contains("video.crf"));

        settings = default_settings();
        settings["metadata"]["text_blacklist"] = json!(["Movie*Group", "mOvIe*gRoUp"]);
        assert!(validate_settings(&settings)
            .unwrap_err()
            .contains("duplicate patterns"));
    }

    #[test]
    fn invalid_toml_and_wrong_table_types_report_the_source() {
        let temp = ScratchDir::new();
        let user = temp.0.join("user.toml");
        let app = temp.0.join("settings.json");
        fs::write(&user, "[video]\ncrf = [broken\n").unwrap();
        assert!(load_effective_settings_at(&temp.0, &user, &app)
            .unwrap_err()
            .contains("user.toml"));

        fs::write(&user, "video = 21\n").unwrap();
        assert!(load_effective_settings_at(&temp.0, &user, &app)
            .unwrap_err()
            .contains("must be a table"));
    }
}
