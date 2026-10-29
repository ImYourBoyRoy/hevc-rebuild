//! ./desktop/src-tauri/src/matroska_apply.rs
//! Native `apply_metadata`: reviewed tag merge, poster, and text scrub (T4.3e).
//!
//! Composes `matroska_tags` (tag builder), `matroska_poster` (artwork), and
//! `matroska_scrub` (post-write cleanup) in the same order as
//! `hevc_rebuild/metadata.py::apply_metadata`, and ports the existing-global-tag
//! extraction. Tag values come only from the reviewed `metadata_override`, the
//! run comment, and the source's own global tags; sidecar `.nfo` files are not
//! read. The only mutation is on `output`; the source media and library art are
//! read-only.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-apply`.
//! Input: absolute `media_path` (source naming and artwork anchor), absolute `output`
//! (the file to edit in place), `work_dir`, tool paths, comment, text blacklist,
//! optional reviewed `metadata_override`, `poster_override`, `embed_artwork`.
//! Output: JSON mirroring Python's `MetadataResult` plus `warnings`.
//! Python adapts to this helper when `HEVC_REBUILD_NATIVE_HELPER` is set and
//! keeps its own implementation as the fallback.

use crate::matroska_poster::{attach_cover, attachment_description, choose_poster, poster_to_jpeg};
use crate::matroska_scrub::{checked, scrub_output_text_metadata, MatroskaTools, EDIT_TIMEOUT};
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
    let poster = if request.embed_artwork {
        choose_poster(&request.media_path, request.poster_override.as_deref())
    } else {
        None
    };
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
        "warnings": built.warnings,
    }))
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
        };
        assert!(apply_metadata(request)
            .unwrap_err()
            .contains("absolute path"));
    }
}
