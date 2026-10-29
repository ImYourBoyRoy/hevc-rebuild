//! ./desktop/src-tauri/src/matroska_poster.rs
//! Native poster selection, JPEG normalization, and cover attachment (T4.3c).
//!
//! Ports `hevc_rebuild/metadata.py::{_poster_candidates, choose_poster,
//! poster_to_jpeg, _poster_attachment_description, attach_cover}`. Library art
//! is only ever read; the single mutation is `mkvpropedit` on `output`, which
//! replaces cover/poster-like image attachments and adds `cover.jpg`.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-poster`.
//! Input: absolute `media_path`, optional `poster_override`, and (to attach) an
//! `attach` block with ffmpeg/mkv tool paths, `output`, and `work_dir`.
//! Output: chosen poster, attachment description, deleted attachment ids.
//! Python remains authoritative for production runs until parity is proven.

use crate::matroska_scrub::{checked, mkv_identify, MatroskaTools, EDIT_TIMEOUT, INSPECT_TIMEOUT};
use crate::metadata_text::is_python_whitespace;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const MAX_ATTRIBUTION_BYTES: u64 = 16 * 1024;
const MAX_DESCRIPTION_CHARS: usize = 2048;
const EXTENSIONS: [&str; 4] = [".jpg", ".jpeg", ".png", ".webp"];
const ATTRIBUTION_FIELDS: [(&str, &str); 6] = [
    ("title", "Artwork"),
    ("creator", "Creator"),
    ("attribution", "Credit"),
    ("license", "License"),
    ("sourceUrl", "Source"),
    ("licenseUrl", "License details"),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachRequest {
    ffmpeg: String,
    tools: MatroskaTools,
    output: PathBuf,
    work_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PosterRequest {
    media_path: PathBuf,
    #[serde(default)]
    poster_override: Option<PathBuf>,
    #[serde(default)]
    attach: Option<AttachRequest>,
}

#[derive(Debug, Serialize)]
struct PosterResponse {
    poster: Option<String>,
    description: Option<String>,
    deleted_attachment_ids: Vec<i64>,
    attached: bool,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(run_request) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize the poster response: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<PosterRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the poster request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("The poster request exceeds the 1 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The poster request JSON is invalid: {error}"))
}

fn run_request(request: PosterRequest) -> Result<PosterResponse, String> {
    let poster = choose_poster(&request.media_path, request.poster_override.as_deref());
    let description = match &poster {
        Some(poster) => Some(attachment_description(poster)?),
        None => None,
    };
    let (mut deleted, mut attached) = (Vec::new(), false);
    if let (Some(poster), Some(description), Some(attach)) =
        (&poster, &description, &request.attach)
    {
        if !attach.output.is_absolute() || !attach.output.is_file() {
            return Err("The attach target must be an existing file with an absolute path.".into());
        }
        if !attach.work_dir.is_dir() {
            return Err("The poster work folder must be an existing directory.".into());
        }
        let cover = poster_to_jpeg(&attach.ffmpeg, poster, &attach.work_dir)?;
        deleted = attach_cover(&attach.tools, &attach.output, &cover, description)?;
        attached = true;
    }
    Ok(PosterResponse {
        poster: poster.map(|path| path.to_string_lossy().into_owned()),
        description,
        deleted_attachment_ids: deleted,
        attached,
    })
}

fn is_season_folder(name: &str) -> bool {
    let lowered = name.to_lowercase();
    let Some(rest) = lowered.strip_prefix("season") else {
        return false;
    };
    rest.trim_start_matches(is_python_whitespace)
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sibling(path: &Path, name: String) -> PathBuf {
    path.with_file_name(name)
}

pub(crate) fn poster_candidates(media: &Path) -> Vec<PathBuf> {
    let stem = media
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidates: Vec<PathBuf> = EXTENSIONS
        .iter()
        .map(|extension| sibling(media, format!("{stem}-poster{extension}")))
        .collect();
    let parent = media.parent().unwrap_or_else(|| Path::new(""));
    let parent_name = name_of(parent);
    if is_season_folder(&parent_name) {
        let series = parent.parent().unwrap_or_else(|| Path::new(""));
        let series_name = name_of(series);
        let normalized: String = parent_name
            .chars()
            .filter(|character| !is_python_whitespace(*character))
            .collect::<String>()
            .to_lowercase();
        for extension in EXTENSIONS {
            candidates.push(parent.join(format!("poster{extension}")));
            candidates.push(parent.join(format!("folder{extension}")));
            candidates.push(series.join(format!("{normalized}-poster{extension}")));
            candidates.push(series.join(format!("{parent_name}-poster{extension}")));
        }
        for extension in EXTENSIONS {
            candidates.push(series.join(format!("folder{extension}")));
            candidates.push(series.join(format!("poster{extension}")));
            candidates.push(series.join(format!("cover{extension}")));
            candidates.push(series.join(format!("{series_name}-poster{extension}")));
        }
    }
    candidates
}

fn inside(path: &Path, directory: &Path) -> bool {
    match (fs::canonicalize(path), fs::canonicalize(directory)) {
        (Ok(path), Ok(directory)) => path.starts_with(directory),
        _ => false,
    }
}

/// Library artwork that explicitly matches the media or season; never a fallback.
pub(crate) fn choose_poster(media: &Path, override_path: Option<&Path>) -> Option<PathBuf> {
    let present: Vec<PathBuf> = poster_candidates(media)
        .into_iter()
        .filter(|path| path.is_file())
        .collect();
    let parent = media.parent().unwrap_or_else(|| Path::new(""));
    if is_season_folder(&name_of(parent)) {
        let season_art = present.iter().find(|path| inside(path, parent));
        if let Some(season_art) = season_art {
            if override_path.is_none_or(|choice| !inside(choice, parent)) {
                return Some(season_art.clone());
            }
        }
    }
    if let Some(choice) = override_path {
        return Some(choice.to_path_buf());
    }
    present.into_iter().next()
}

/// Attribution text embedded with the attachment; "Cover art" without a sidecar.
pub(crate) fn attachment_description(poster: &Path) -> Result<String, String> {
    let attribution = sibling(poster, format!("{}.attribution.json", name_of(poster)));
    if !attribution.exists() {
        return Ok("Cover art".into());
    }
    let shown = attribution.display();
    let metadata = fs::metadata(&attribution).map_err(|error| {
        format!("Selected poster attribution could not be read safely ({shown}): {error}")
    })?;
    if metadata.len() > MAX_ATTRIBUTION_BYTES {
        return Err(format!(
            "Artwork attribution is too large to embed safely ({shown})."
        ));
    }
    let text = fs::read_to_string(&attribution).map_err(|error| {
        format!("Selected poster attribution could not be read safely ({shown}): {error}")
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        format!("Selected poster attribution could not be read safely ({shown}): {error}")
    })?;
    let Some(object) = value.as_object() else {
        return Err(format!(
            "Selected poster attribution has an invalid structure ({shown})."
        ));
    };
    let mut details: Vec<String> = Vec::new();
    for (key, prefix) in ATTRIBUTION_FIELDS {
        let Some(item) = object.get(key).and_then(Value::as_str) else {
            continue;
        };
        if item.trim_matches(is_python_whitespace).is_empty() {
            continue;
        }
        if item.chars().any(|character| (character as u32) < 32) {
            return Err(format!(
                "Selected poster attribution contains control text ({shown})."
            ));
        }
        let clean = item
            .split(is_python_whitespace)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        details.push(format!("{prefix}: {clean}"));
    }
    if details.is_empty() {
        return Err(format!(
            "Selected poster attribution contains no credit or license information ({shown})."
        ));
    }
    let description = format!("Cover art · {}", details.join(" · "));
    Ok(description.chars().take(MAX_DESCRIPTION_CHARS).collect())
}

/// Decode and re-encode so mislabeled art cannot be attached as image/jpeg.
pub(crate) fn poster_to_jpeg(
    ffmpeg: &str,
    poster: &Path,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    let out = work_dir.join("cover.jpg");
    checked(
        ffmpeg,
        vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-i".into(),
            poster.as_os_str().to_owned(),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "2".into(),
            out.as_os_str().to_owned(),
        ],
        INSPECT_TIMEOUT,
    )?;
    match fs::metadata(&out) {
        Ok(metadata) if metadata.len() >= 128 => Ok(out),
        _ => Err(format!(
            "Failed to prepare poster artwork from {}",
            poster.display()
        )),
    }
}

fn attachment_id(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_str()
            .and_then(|text| text.trim_matches(is_python_whitespace).parse().ok())
    })
}

/// Replace cover/poster-like image attachments with `cover`; returns deleted ids.
pub(crate) fn attach_cover(
    tools: &MatroskaTools,
    output: &Path,
    cover: &Path,
    description: &str,
) -> Result<Vec<i64>, String> {
    let identify = mkv_identify(tools, output)?;
    let mut delete_ids: Vec<i64> = Vec::new();
    for attachment in identify
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = attachment
            .get("file_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let mime = attachment
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cover_like = ["cover", "poster", "folder", "front"]
            .iter()
            .any(|word| name.contains(word));
        if mime.starts_with("image/") && cover_like {
            if let Some(id) = attachment.get("id").and_then(attachment_id) {
                delete_ids.push(id);
            }
        }
    }
    if !delete_ids.is_empty() {
        let mut args: Vec<OsString> = vec![output.as_os_str().to_owned()];
        for id in &delete_ids {
            args.extend(["--delete-attachment".into(), id.to_string().into()]);
        }
        checked(&tools.mkvpropedit, args, EDIT_TIMEOUT)?;
    }
    checked(
        &tools.mkvpropedit,
        vec![
            output.as_os_str().to_owned(),
            "--attachment-name".into(),
            "cover.jpg".into(),
            "--attachment-mime-type".into(),
            "image/jpeg".into(),
            "--attachment-description".into(),
            description.into(),
            "--add-attachment".into(),
            cover.as_os_str().to_owned(),
        ],
        EDIT_TIMEOUT,
    )?;
    Ok(delete_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hevc-poster-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"art").unwrap();
    }

    #[test]
    fn season_names_need_a_digit_after_optional_space() {
        assert!(is_season_folder("Season 04"));
        assert!(is_season_folder("SEASON4"));
        assert!(is_season_folder("season\u{a0}2"));
        assert!(!is_season_folder("Seasons 1"));
        assert!(!is_season_folder("Season"));
        assert!(!is_season_folder("Season one"));
    }

    #[test]
    fn season_art_beats_series_art_and_foreign_overrides() {
        let root = TempDir::new();
        let media = root.path().join("Show/Season 02/S02E01.mkv");
        touch(&media);
        touch(&root.path().join("Show/folder.jpg"));
        touch(&root.path().join("Show/season02-poster.png"));
        let series_override = root.path().join("Show/chosen.png");
        touch(&series_override);
        // Series art named for the season is not inside the season folder.
        assert_eq!(
            choose_poster(&media, Some(&series_override)),
            Some(series_override.clone())
        );
        assert_eq!(
            choose_poster(&media, None),
            Some(root.path().join("Show/season02-poster.png"))
        );
        touch(&root.path().join("Show/Season 02/poster.webp"));
        assert_eq!(
            choose_poster(&media, Some(&series_override)),
            Some(root.path().join("Show/Season 02/poster.webp"))
        );
    }

    #[test]
    fn no_art_means_no_poster_and_description_defaults() {
        let root = TempDir::new();
        let media = root.path().join("Movie.mkv");
        touch(&media);
        assert_eq!(choose_poster(&media, None), None);
        let poster = root.path().join("Movie-poster.jpg");
        touch(&poster);
        assert_eq!(attachment_description(&poster).unwrap(), "Cover art");
    }

    #[test]
    fn attribution_is_normalized_and_bad_sidecars_are_rejected() {
        let root = TempDir::new();
        let poster = root.path().join("p.jpg");
        let sidecar = root.path().join("p.jpg.attribution.json");
        touch(&poster);
        fs::write(
            &sidecar,
            r#"{"title":"  A   B ","license":"CC0","sourceUrl":"   ","extra":1}"#,
        )
        .unwrap();
        assert_eq!(
            attachment_description(&poster).unwrap(),
            "Cover art · Artwork: A B · License: CC0"
        );
        fs::write(&sidecar, r#"{"title":"bad\u0007"}"#).unwrap();
        assert!(attachment_description(&poster)
            .unwrap_err()
            .contains("control text"));
        fs::write(&sidecar, "[]").unwrap();
        assert!(attachment_description(&poster)
            .unwrap_err()
            .contains("invalid structure"));
        fs::write(&sidecar, "{}").unwrap();
        assert!(attachment_description(&poster)
            .unwrap_err()
            .contains("no credit"));
        fs::write(&sidecar, vec![b' '; 16 * 1024 + 1]).unwrap();
        assert!(attachment_description(&poster)
            .unwrap_err()
            .contains("too large"));
    }

    #[test]
    fn long_descriptions_are_truncated_by_characters() {
        let root = TempDir::new();
        let poster = root.path().join("p.jpg");
        touch(&poster);
        let title = "é".repeat(3000);
        fs::write(
            root.path().join("p.jpg.attribution.json"),
            format!(r#"{{"title":"{title}"}}"#),
        )
        .unwrap();
        assert_eq!(
            attachment_description(&poster).unwrap().chars().count(),
            MAX_DESCRIPTION_CHARS
        );
    }
}
