//! ./desktop/src-tauri/src/matroska_apply.rs
//! Native `apply_metadata`: reviewed tag merge, poster, and text scrub.
//!
//! Composes `matroska_tags` (tag builder), `matroska_poster` (artwork), and
//! `matroska_scrub` (post-write cleanup), and extracts the existing global tags. Tag values come only from the reviewed `metadata_override`, the
//! run comment, and the source's own global tags; sidecar `.nfo` files are not
//! read. The only mutation is on `output`; the source media and library art are
//! read-only.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-apply`.
//! Input: absolute `media_path` (source naming and artwork anchor), absolute `output`
//! (the file to edit in place), `work_dir`, tool paths, comment, text blacklist,
//! optional reviewed `metadata_override`, `poster_override`, `embed_artwork`.
//! Output: JSON describing the applied metadata plus `warnings`.

use crate::matroska_poster::{attach_cover, attachment_description, choose_poster, poster_to_jpeg};
use crate::matroska_scrub::{
    checked, mkv_identify, scrub_output_text_metadata, MatroskaTools, EDIT_TIMEOUT,
};
use crate::matroska_tags::{build_tags, TagBuildRequest};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyRequest {
    media_path: PathBuf,
    output: PathBuf,
    work_dir: PathBuf,
    ffmpeg: String,
    tools: MatroskaTools,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    text_blacklist: Vec<String>,
    #[serde(default)]
    metadata_override: Option<Map<String, Value>>,
    #[serde(default)]
    poster_override: Option<PathBuf>,
    #[serde(default = "default_true")]
    embed_artwork: bool,
    /// Re-attach the source's own cover when no other poster was chosen. A
    /// re-encode does not carry an existing Matroska cover across (FFmpeg exposes
    /// it as an attached-picture video stream and stream-copying that creates a
    /// second video track), so rebuilds ask for it here. Remuxes that already copy
    /// every attachment leave this off.
    #[serde(default)]
    carry_source_cover: bool,
    /// The source's attached-picture video stream, when it has one. Used when the
    /// source is not Matroska (an MP4 `covr` cover), where `mkvextract` cannot read it.
    #[serde(default)]
    source_cover_stream: Option<SourceCoverStream>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceCoverStream {
    /// FFprobe stream index of the attached picture.
    index: i64,
    /// File extension for the extracted image: `jpg`, `png`, or `webp`.
    extension: String,
}

fn default_true() -> bool {
    true
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(apply_metadata) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<ApplyRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the metadata request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("The metadata request exceeds the 8 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The metadata request JSON is invalid: {error}"))
}

/// Existing true-global tags of an MKV source, so a rebuild keeps them; "" otherwise.
fn existing_tags_xml(tools: &MatroskaTools, media: &Path, work_dir: &Path) -> Option<String> {
    let is_mkv = media
        .extension()
        .is_some_and(|extension| extension.to_string_lossy().to_lowercase() == "mkv");
    if !is_mkv {
        return None;
    }
    let destination = work_dir.join("source-tags.xml");
    let _ = fs::remove_file(&destination);
    let run = crate::matroska_scrub::run_tool(
        &tools.mkvextract,
        &[
            media.as_os_str().to_owned(),
            "tags".into(),
            destination.as_os_str().to_owned(),
        ],
        std::time::Duration::from_secs(24 * 60 * 60),
    )
    .ok()?;
    if run.code != Some(0) || fs::metadata(&destination).map_or(true, |meta| meta.len() == 0) {
        return None;
    }
    fs::read_to_string(&destination).ok()
}

/// In-process entry for native runs: same validation and order as the helper.
pub(crate) fn apply_metadata_value(request: Value) -> Result<Value, String> {
    let request: ApplyRequest = serde_json::from_value(request)
        .map_err(|error| format!("The metadata request JSON is invalid: {error}"))?;
    apply_metadata(request)
}

fn apply_metadata(request: ApplyRequest) -> Result<Value, String> {
    for (label, path) in [("media", &request.media_path), ("output", &request.output)] {
        if !path.is_absolute() || !path.is_file() {
            return Err(format!(
                "The metadata {label} must be an existing file with an absolute path."
            ));
        }
    }
    if !request.work_dir.is_dir() {
        return Err("The metadata work folder must be an existing directory.".into());
    }

    let built = build_tags(TagBuildRequest {
        source_stem: request
            .media_path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        comment: request.comment.clone(),
        text_blacklist: request.text_blacklist.clone(),
        metadata_override: request.metadata_override.clone(),
        existing_tags_xml: existing_tags_xml(
            &request.tools,
            &request.media_path,
            &request.work_dir,
        ),
    })?;
    let xml_path = request.work_dir.join("merged-global-tags.xml");
    fs::write(&xml_path, &built.tags_xml)
        .map_err(|error| format!("Could not write {}: {error}", xml_path.display()))?;

    // Everything that can fail on input is prepared before the output is touched.
    let mut warnings = built.warnings.clone();
    let mut poster = if request.embed_artwork {
        choose_poster(&request.media_path, request.poster_override.as_deref())
    } else {
        None
    };
    if poster.is_none() && request.carry_source_cover {
        match extract_source_cover(&request.tools, &request.media_path, &request.work_dir) {
            Ok(Some(carried)) => poster = Some(carried),
            Ok(None) => {
                if let Some(stream) = &request.source_cover_stream {
                    match extract_cover_stream(
                        &request.ffmpeg,
                        &request.media_path,
                        stream,
                        &request.work_dir,
                    ) {
                        Ok(carried) => poster = Some(carried),
                        Err(error) => warnings.push(format!(
                            "The source's embedded cover could not be carried over: {error}"
                        )),
                    }
                }
            }
            Err(error) => warnings.push(format!(
                "The source's embedded cover could not be carried over: {error}"
            )),
        }
    }
    let cover = match &poster {
        Some(poster) => {
            let description = attachment_description(poster)?;
            let cover = poster_to_jpeg(&request.ffmpeg, poster, &request.work_dir)?;
            Some((cover, description))
        }
        None => None,
    };

    // Replace only global tags; track-specific tags/statistics are preserved.
    let mut tags_argument = OsString::from("global:");
    tags_argument.push(&xml_path);
    checked(
        &request.tools.mkvpropedit,
        vec![
            request.output.as_os_str().to_owned(),
            "--edit".into(),
            "info".into(),
            "--set".into(),
            format!("title={}", built.title).into(),
            "--tags".into(),
            tags_argument,
        ],
        EDIT_TIMEOUT,
    )?;
    if let Some((cover, description)) = &cover {
        attach_cover(&request.tools, &request.output, cover, description)?;
    }
    let scrub = scrub_output_text_metadata(
        &request.tools,
        &request.output,
        &request.work_dir,
        &request.text_blacklist,
    )?;
    let mut changes = built.changes;
    changes.extend(scrub.changes);
    Ok(json!({
        "xml_path": xml_path.to_string_lossy(),
        "title": built.title,
        "poster_path": poster.map(|path| path.to_string_lossy().into_owned()),
        "expected_global_tags": built.expected_tags,
        "scrubbed_text_values": changes,
        "chapter_text_edits": scrub.chapter_edits,
        "warnings": warnings,
    }))
}

/// Copy a source's attached-picture stream out to an image file in the work folder.
fn extract_cover_stream(
    ffmpeg: &str,
    media: &Path,
    stream: &SourceCoverStream,
    work_dir: &Path,
) -> Result<PathBuf, String> {
    if !matches!(stream.extension.as_str(), "jpg" | "png" | "webp") {
        return Err(format!(
            "unsupported cover image type .{}",
            stream.extension
        ));
    }
    let target = work_dir.join(format!("source-cover.{}", stream.extension));
    checked(
        ffmpeg,
        vec![
            "-v".into(),
            "error".into(),
            "-y".into(),
            "-i".into(),
            media.as_os_str().to_owned(),
            "-map".into(),
            format!("0:{}", stream.index).into(),
            "-c".into(),
            "copy".into(),
            "-frames:v".into(),
            "1".into(),
            target.as_os_str().to_owned(),
        ],
        EDIT_TIMEOUT,
    )?;
    if target.is_file() {
        Ok(target)
    } else {
        Err("FFmpeg wrote no cover image".into())
    }
}

/// Extract the source's own cover image (a cover-like image attachment) into the
/// work folder so it can be re-attached to the rebuilt output. `Ok(None)` means
/// the source has no such cover or is not a Matroska file.
fn extract_source_cover(
    tools: &MatroskaTools,
    media: &Path,
    work_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let identify = mkv_identify(tools, media)?;
    // mkvextract only reads Matroska; an MP4 cover is extracted through FFmpeg.
    let is_matroska = identify
        .pointer("/container/type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("matroska"));
    if !is_matroska {
        return Ok(None);
    }
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
        let extension = match attachment
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            "image/webp" => "webp",
            _ => continue,
        };
        if !["cover", "poster", "folder", "front"]
            .iter()
            .any(|word| name.contains(word))
        {
            continue;
        }
        let Some(id) = attachment.get("id").and_then(|id| match id {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.parse().ok(),
            _ => None,
        }) else {
            continue;
        };
        let target = work_dir.join(format!("source-cover.{extension}"));
        let mut spec = OsString::from(format!("{id}:"));
        spec.push(&target);
        checked(
            &tools.mkvextract,
            vec![media.as_os_str().to_owned(), "attachments".into(), spec],
            EDIT_TIMEOUT,
        )?;
        if target.is_file() {
            return Ok(Some(target));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_or_missing_targets_are_rejected_first() {
        let request = ApplyRequest {
            media_path: PathBuf::from("movie.mkv"),
            output: PathBuf::from("/nonexistent/out.mkv"),
            work_dir: PathBuf::from("/tmp"),
            ffmpeg: "ffmpeg".into(),
            tools: MatroskaTools {
                mkvmerge: "mkvmerge".into(),
                mkvextract: "mkvextract".into(),
                mkvpropedit: "mkvpropedit".into(),
            },
            comment: String::new(),
            text_blacklist: Vec::new(),
            metadata_override: None,
            poster_override: None,
            embed_artwork: true,
            carry_source_cover: false,
            source_cover_stream: None,
        };
        assert!(apply_metadata(request)
            .unwrap_err()
            .contains("absolute path"));
    }
}
