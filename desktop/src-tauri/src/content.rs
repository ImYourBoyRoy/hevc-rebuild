//! Read-only content genre and x265 tune decisions shared with the scanner and
//! Rust-launched encoder adapter. The Python compatibility engine retains the
//! direct-run fallback while the shared policy is parity-checked.

use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;

const MAX_RUN_TUNE_REQUEST_BYTES: u64 = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunTuneRequest {
    path: PathBuf,
    mediainfo: Value,
    requested_tune: String,
    detect_animation: bool,
    #[serde(default)]
    file_override: Option<String>,
    /// Genres from the user's reviewed metadata for this file (GUI/CLI/MCP override).
    #[serde(default)]
    reviewed_genres: Vec<String>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    let result = (|| {
        let mut bytes = Vec::new();
        io::stdin()
            .take(MAX_RUN_TUNE_REQUEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("Could not read content-tune request: {error}"))?;
        if bytes.len() as u64 > MAX_RUN_TUNE_REQUEST_BYTES {
            return Err("Content-tune request exceeds the 256 KiB limit.".to_string());
        }
        let request: RunTuneRequest = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Content-tune request JSON is invalid: {error}"))?;
        let response = run_tune_request(request)?;
        serde_json::to_string(&response)
            .map_err(|error| format!("Could not serialize content-tune response: {error}"))
    })();
    match result {
        Ok(output) => {
            println!("{output}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn run_tune_request(request: RunTuneRequest) -> Result<Value, String> {
    if !request.path.is_absolute() || !request.path.is_file() {
        return Err("Content-tune source must be an existing absolute media-file path.".into());
    }
    let (requested, detect_animation) = match request.file_override {
        Some(value) if matches!(value.as_str(), "animation" | "none") => (value, false),
        Some(_) => return Err("Per-file content tune must be 'animation' or 'none'.".into()),
        None => {
            let value = request.requested_tune.to_ascii_lowercase();
            if !matches!(value.as_str(), "auto" | "animation" | "grain" | "none") {
                return Err("Global content tune must be auto, animation, grain, or none.".into());
            }
            (value, request.detect_animation)
        }
    };
    let settings = json!({
        "content": {
            "tune": requested,
            "detect_animation": detect_animation,
        }
    });
    let decision =
        profile_content_with_reviewed(&request.mediainfo, &request.reviewed_genres, &settings);
    Ok(json!({
        "tune": decision.get("resolved_tune").cloned().unwrap_or(Value::Null),
        "genres": decision.get("genres").cloned().unwrap_or_else(|| json!([])),
        "sources": decision.get("sources").cloned().unwrap_or_else(|| json!([])),
        "animation_detected": decision.get("animation_detected").cloned().unwrap_or(Value::Bool(false)),
        "reason": decision.get("reason").cloned().unwrap_or(Value::Null),
    }))
}

pub(crate) fn profile_content(mediainfo: &Value, settings: &Value) -> Value {
    profile_content_with_reviewed(mediainfo, &[], settings)
}

/// Genres come from the container's own metadata (MediaInfo) and, for
/// app-launched runs, the user's reviewed genres. Sidecar files are not read.
fn profile_content_with_reviewed(
    mediainfo: &Value,
    reviewed_genres: &[String],
    settings: &Value,
) -> Value {
    let mut genres = Vec::<String>::new();
    let mut sources = Vec::<String>::new();

    if let Some(general) = mediainfo
        .pointer("/media/track")
        .and_then(Value::as_array)
        .and_then(|tracks| {
            tracks.iter().find(|track| {
                track
                    .get("@type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("general"))
            })
        })
    {
        let before = genres.len();
        for key in ["Genre", "Genres"] {
            if let Some(value) = general.get(key) {
                append_unique(&mut genres, genre_values(value));
            }
        }
        if genres.len() > before {
            push_unique(&mut sources, "MediaInfo".to_string());
        }
    }

    let before = genres.len();
    append_unique(
        &mut genres,
        reviewed_genres
            .iter()
            .flat_map(|genre| split_genres(genre))
            .collect(),
    );
    if genres.len() > before {
        push_unique(&mut sources, "Reviewed metadata".to_string());
    }

    let animation_detected = genres.iter().any(|genre| genre == "animation");
    let requested = settings
        .pointer("/content/tune")
        .and_then(Value::as_str)
        .unwrap_or("auto")
        .to_lowercase();
    let detect_animation = settings
        .pointer("/content/detect_animation")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let resolved_tune = resolve_tune(&requested, detect_animation, animation_detected);
    let reason = match requested.as_str() {
        "animation" => "Global content tune is Animation",
        "grain" => "Global content tune is Film grain",
        "none" => "Global content tune is Normal",
        "auto" if detect_animation && animation_detected => {
            "Animation genre found in source metadata"
        }
        "auto" if !detect_animation => "Automatic animation detection is disabled",
        "auto" => "No Animation genre found in available metadata",
        _ => "No content tune selected",
    };

    json!({
        "genres": genres,
        "sources": sources,
        "animation_detected": animation_detected,
        "resolved_tune": resolved_tune,
        "reason": reason
    })
}

/// Resolve the x265 content tune for one media file using MediaInfo genres and
/// optional reviewed-metadata genres. Returns `None` for a normal (untuned) encode.
pub(crate) fn resolve_run_tune(
    mediainfo: &Value,
    requested_tune: &str,
    detect_animation: bool,
    file_override: Option<&str>,
    reviewed_genres: &[String],
) -> Result<Option<String>, String> {
    let (requested, detect) = match file_override {
        Some(value) if matches!(value, "animation" | "none") => (value.to_string(), false),
        Some(_) => return Err("Per-file content tune must be 'animation' or 'none'.".into()),
        None => {
            let value = requested_tune.to_ascii_lowercase();
            if !matches!(value.as_str(), "auto" | "animation" | "grain" | "none") {
                return Err("Global content tune must be auto, animation, grain, or none.".into());
            }
            (value, detect_animation)
        }
    };
    let settings = json!({
        "content": {
            "tune": requested,
            "detect_animation": detect,
        }
    });
    let decision = profile_content_with_reviewed(mediainfo, reviewed_genres, &settings);
    Ok(decision
        .get("resolved_tune")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// Mirrors `hevc_rebuild.content.resolve_tune` for supported content settings.
fn resolve_tune(
    requested: &str,
    detect_animation: bool,
    animation_detected: bool,
) -> Option<&'static str> {
    match requested {
        "none" => None,
        "animation" => Some("animation"),
        "grain" => Some("grain"),
        "auto" if detect_animation && animation_detected => Some("animation"),
        _ => None,
    }
}

fn genre_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => split_genres(text),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .flat_map(split_genres)
            .collect(),
        _ => Vec::new(),
    }
}

fn split_genres(value: &str) -> Vec<String> {
    value
        .split([',', ';', '/', '|'])
        .map(str::trim)
        .filter(|genre| !genre.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn append_unique(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        push_unique(target, value);
    }
}

fn push_unique(target: &mut Vec<String>, value: String) {
    if !target.contains(&value) {
        target.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_tune, run_tune_request, RunTuneRequest};
    use serde_json::{json, Value};
    use std::fs;
    use std::path::Path;

    #[test]
    fn global_tune_and_automatic_animation_match_python_policy() {
        assert_eq!(resolve_tune("auto", true, true), Some("animation"));
        assert_eq!(resolve_tune("auto", true, false), None);
        assert_eq!(resolve_tune("auto", false, true), None);
        assert_eq!(resolve_tune("animation", false, false), Some("animation"));
        assert_eq!(resolve_tune("grain", true, true), Some("grain"));
        assert_eq!(resolve_tune("none", true, true), None);
    }

    #[test]
    fn run_tune_request_applies_file_override_before_global_automatic_policy() {
        let root = std::env::temp_dir().join(format!(
            "hevc-rebuild-content-tune-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let media = root.join("sample.mkv");
        fs::write(&media, b"fixture").unwrap();
        let base = RunTuneRequest {
            path: media,
            mediainfo: json!({"media":{"track":[{"@type":"General","Genre":"Drama / Animation"}]}}),
            requested_tune: "auto".into(),
            detect_animation: true,
            file_override: None,
            reviewed_genres: Vec::new(),
        };
        assert_eq!(run_tune_request(base).unwrap()["tune"], "animation");

        let suppress = sample_run_tune_request(&root, "auto", Some("none"));
        assert_eq!(run_tune_request(suppress).unwrap()["tune"], Value::Null);
        let suppress = sample_run_tune_request(&root, "none", Some("animation"));
        assert_eq!(run_tune_request(suppress).unwrap()["tune"], "animation");
        let global_grain = sample_run_tune_request(&root, "grain", None);
        assert_eq!(run_tune_request(global_grain).unwrap()["tune"], "grain");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reviewed_animation_genre_enables_auto_tune_without_overriding_global_choices() {
        let root = std::env::temp_dir().join(format!(
            "hevc-rebuild-reviewed-tune-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("sample.mkv"), b"fixture").unwrap();
        let mut request = sample_run_tune_request(&root, "auto", None);
        request.reviewed_genres = vec!["Family, Animation".into()];
        let decision = run_tune_request(request).unwrap();
        assert_eq!(decision["tune"], "animation");
        assert_eq!(decision["sources"], json!(["Reviewed metadata"]));

        let mut off = sample_run_tune_request(&root, "none", None);
        off.reviewed_genres = vec!["Animation".into()];
        assert_eq!(run_tune_request(off).unwrap()["tune"], Value::Null);
        fs::remove_dir_all(root).unwrap();
    }

    fn sample_run_tune_request(
        root: &Path,
        requested_tune: &str,
        file_override: Option<&str>,
    ) -> RunTuneRequest {
        RunTuneRequest {
            path: root.join("sample.mkv"),
            mediainfo: json!({"media":{"track":[]}}),
            requested_tune: requested_tune.into(),
            detect_animation: true,
            file_override: file_override.map(str::to_string),
            reviewed_genres: Vec::new(),
        }
    }
}
