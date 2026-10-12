//! Read-only content genre and x265 tune decisions shared with the scanner and
//! Rust-launched encoder adapter. The Python compatibility engine retains the
//! direct-run fallback while the shared policy is parity-checked.

use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_NFO_BYTES: u64 = 1024 * 1024;
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
    let decision = profile_content(&request.path, &request.mediainfo, &settings);
    Ok(json!({
        "tune": decision.get("resolved_tune").cloned().unwrap_or(Value::Null),
        "genres": decision.get("genres").cloned().unwrap_or_else(|| json!([])),
        "sources": decision.get("sources").cloned().unwrap_or_else(|| json!([])),
        "animation_detected": decision.get("animation_detected").cloned().unwrap_or(Value::Bool(false)),
        "reason": decision.get("reason").cloned().unwrap_or(Value::Null),
    }))
}

pub(crate) fn profile_content(path: &Path, mediainfo: &Value, settings: &Value) -> Value {
    let mut genres = Vec::<String>::new();
    let mut sources = Vec::<String>::new();

    for nfo in nfo_paths_for(path) {
        let Ok(metadata) = fs::metadata(&nfo) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_NFO_BYTES {
            continue;
        }
        let Ok(xml) = fs::read_to_string(&nfo) else {
            continue;
        };
        let found = genres_from_nfo(&xml);
        if !found.is_empty() {
            append_unique(&mut genres, found);
            push_unique(&mut sources, nfo_source_name(path, &nfo));
        }
    }

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

fn nfo_paths_for(path: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut parent = path.parent().unwrap_or(path);
    for _ in 0..24 {
        let series = [
            parent.join("tvshow.nfo"),
            parent.join("tvshow.hevc-rebuild.nfo"),
        ];
        let found = series
            .iter()
            .filter(|candidate| candidate.is_file())
            .cloned()
            .collect::<Vec<_>>();
        if !found.is_empty() {
            candidates.extend(found);
            break;
        }
        let Some(next) = parent.parent() else {
            break;
        };
        if next == parent {
            break;
        }
        parent = next;
    }

    candidates.push(path.with_extension("nfo"));
    if let Some(stem) = path.file_stem() {
        let mut managed = stem.to_os_string();
        managed.push(".hevc-rebuild.nfo");
        candidates.push(path.with_file_name(managed));
    }

    let mut unique = Vec::new();
    for candidate in candidates {
        if candidate.is_file() && !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    unique
}

fn nfo_source_name(media_path: &Path, nfo_path: &Path) -> String {
    if nfo_path.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("tvshow.nfo")
            || name.eq_ignore_ascii_case("tvshow.hevc-rebuild.nfo")
    }) {
        return "TV-show NFO".to_string();
    }
    if nfo_path == media_path.with_extension("nfo") {
        "Adjacent NFO".to_string()
    } else {
        "Managed adjacent NFO".to_string()
    }
}

fn genres_from_nfo(xml: &str) -> Vec<String> {
    let bytes = xml.as_bytes();
    let mut genres = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cursor = 0;

    while cursor < bytes.len() {
        let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') else {
            break;
        };
        let start = cursor + relative;
        if bytes[start..].starts_with(b"<!--") {
            cursor = after_marker(bytes, start + 4, b"-->").unwrap_or(bytes.len());
            continue;
        }
        if bytes[start..].starts_with(b"<![CDATA[") {
            cursor = after_marker(bytes, start + 9, b"]]>").unwrap_or(bytes.len());
            continue;
        }
        if bytes[start..].starts_with(b"<?") {
            cursor = after_marker(bytes, start + 2, b">").unwrap_or(bytes.len());
            continue;
        }
        if bytes[start..].starts_with(b"<!") {
            cursor = after_markup_declaration(bytes, start + 2).unwrap_or(bytes.len());
            continue;
        }
        let Some(end_relative) = bytes[start..].iter().position(|byte| *byte == b'>') else {
            break;
        };
        let end = start + end_relative;
        let tag = String::from_utf8_lossy(&bytes[start + 1..end]);
        let trimmed = tag.trim();
        let closing = trimmed.starts_with('/');
        let self_closing = trimmed.ends_with('/');
        let name = trimmed
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split(|character: char| character.is_ascii_whitespace())
            .next()
            .unwrap_or_default();

        if closing {
            if stack.last().is_some_and(|open| open == name) {
                stack.pop();
            }
        } else if !name.is_empty() {
            // Python's ElementTree `findall("genre")` looks only at direct
            // children of the NFO document root.
            if stack.len() == 1 && name == "genre" && !self_closing {
                if let Some((text, after)) = element_text(bytes, end + 1, "genre") {
                    for genre in split_genres(&text) {
                        push_unique(&mut genres, genre);
                    }
                    cursor = after;
                    continue;
                }
            }
            if !self_closing {
                stack.push(name.to_string());
            }
        }
        cursor = end + 1;
    }
    genres
}

fn element_text(bytes: &[u8], mut cursor: usize, name: &str) -> Option<(String, usize)> {
    let mut depth = 1usize;
    let mut text = String::new();
    while cursor < bytes.len() {
        let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') else {
            return None;
        };
        let start = cursor + relative;
        text.push_str(&String::from_utf8_lossy(&bytes[cursor..start]));
        if bytes[start..].starts_with(b"<!--") {
            cursor = after_marker(bytes, start + 4, b"-->")?;
            continue;
        }
        if bytes[start..].starts_with(b"<![CDATA[") {
            let content = start + 9;
            let end = find_bytes(bytes, content, b"]]>")?;
            text.push_str(&String::from_utf8_lossy(&bytes[content..end]));
            cursor = end + 3;
            continue;
        }
        if bytes[start..].starts_with(b"<?") {
            cursor = after_marker(bytes, start + 2, b">")?;
            continue;
        }
        if bytes[start..].starts_with(b"<!") {
            cursor = after_markup_declaration(bytes, start + 2)?;
            continue;
        }
        let end = start + bytes[start..].iter().position(|byte| *byte == b'>')?;
        let tag = String::from_utf8_lossy(&bytes[start + 1..end]);
        let trimmed = tag.trim();
        let closing = trimmed.starts_with('/');
        let self_closing = trimmed.ends_with('/');
        let child_name = trimmed
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split(|character: char| character.is_ascii_whitespace())
            .next()
            .unwrap_or_default();
        if child_name == name {
            if closing {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((decode_xml_entities(&text), end + 1));
                }
            } else if !self_closing {
                depth += 1;
            }
        }
        cursor = end + 1;
    }
    None
}

fn after_marker(bytes: &[u8], start: usize, marker: &[u8]) -> Option<usize> {
    find_bytes(bytes, start, marker).map(|end| end + marker.len())
}

fn after_markup_declaration(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = None;
    let mut subset_depth = 0usize;
    for (offset, byte) in bytes.get(start..)?.iter().enumerate() {
        if let Some(delimiter) = quote {
            if *byte == delimiter {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(*byte),
            b'[' => subset_depth += 1,
            b']' => subset_depth = subset_depth.saturating_sub(1),
            b'>' if subset_depth == 0 => return Some(start + offset + 1),
            _ => {}
        }
    }
    None
}

fn find_bytes(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|relative| start + relative)
}

fn decode_xml_entities(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative) = value[cursor..].find('&') {
        let ampersand = cursor + relative;
        result.push_str(&value[cursor..ampersand]);
        let Some(relative_end) = value[ampersand..].find(';') else {
            result.push_str(&value[ampersand..]);
            return result;
        };
        let end = ampersand + relative_end;
        let entity = &value[ampersand + 1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };
        if let Some(character) = decoded {
            result.push(character);
        } else {
            result.push_str(&value[ampersand..=end]);
        }
        cursor = end + 1;
    }
    result.push_str(&value[cursor..]);
    result
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
    use super::{genres_from_nfo, resolve_tune, run_tune_request, RunTuneRequest};
    use serde_json::{json, Value};
    use std::fs;
    use std::path::Path;

    #[test]
    fn reads_direct_nfo_genres_and_decodes_xml_text() {
        let xml = r#"<!DOCTYPE movie [ <!ENTITY fake "<genre>Ignore</genre>"> ]><movie><!-- <genre>Ignore</genre> --><genre>Animation / Family &amp; Adventure</genre><details><genre>Drama</genre></details><genre>Animation</genre></movie>"#;
        assert_eq!(
            genres_from_nfo(xml),
            vec!["animation", "family & adventure"]
        );
    }

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
        fs::write(
            root.join("sample.nfo"),
            "<movie><genre>Animation</genre></movie>",
        )
        .unwrap();
        let base = RunTuneRequest {
            path: media,
            mediainfo: json!({"media":{"track":[{"@type":"General","Genre":"Drama"}]}}),
            requested_tune: "auto".into(),
            detect_animation: true,
            file_override: None,
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
        }
    }
}
