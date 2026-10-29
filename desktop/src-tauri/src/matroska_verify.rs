//! ./desktop/src-tauri/src/matroska_verify.rs
//! Native post-write expectation checks for a metadata-tagged Matroska output (T4.3d).
//!
//! Ports the tail of `hevc_rebuild/validate.py::validate_output`
//! (`expect_poster` and `expected_global_tags`) plus
//! `metadata.py::read_global_simple_tags`. Read-only: it never edits `output`.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-verify`.
//! Input: tool paths, absolute `output`, `expect_poster`, and ordered
//! `expected_global_tags` as `[[name, value], ...]` pairs (order decides which
//! failure is reported first, as in Python dict order).
//! Output: `{"ok":true,"global_tags":{...}}` on success; when an expectation is not
//! met, `{"ok":false,"message":"<Python-identical text>"}` with exit 0 so the Python
//! adapter can re-raise the exact user-facing error; unusable input or a failing
//! MKVToolNix call writes stderr and exits non-zero.
//! Python adapts to this helper when `HEVC_REBUILD_NATIVE_HELPER` is set.

use crate::matroska_scrub::{mkv_identify, run_tool, MatroskaTools};
use crate::matroska_tags::{is_global_tag, parse_xml, py_strip, XmlNode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_REQUEST_BYTES: u64 = 4 * 1024 * 1024;
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyRequest {
    tools: MatroskaTools,
    output: PathBuf,
    work_dir: PathBuf,
    #[serde(default)]
    expect_poster: bool,
    #[serde(default)]
    expected_global_tags: Vec<(String, String)>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request()
        .map_err(VerifyError::Operational)
        .and_then(|request| {
            verify_expected_output(
                &request.tools,
                &request.output,
                &request.work_dir,
                request.expect_poster,
                &request.expected_global_tags,
            )
        }) {
        Ok(tags) => {
            let tags: Map<String, Value> = tags
                .into_iter()
                .map(|(name, values)| (name, json!(values)))
                .collect();
            println!("{}", json!({"ok": true, "global_tags": tags}));
            0
        }
        Err(VerifyError::Mismatch(message)) => {
            println!("{}", json!({"ok": false, "message": message}));
            0
        }
        Err(VerifyError::Operational(error)) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<VerifyRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the verify request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("The verify request exceeds the 4 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The verify request JSON is invalid: {error}"))
}

/// Global Simple tags as `NAME -> values`, ignoring track/chapter/etc. targeted tags.
pub(crate) fn global_simple_tags(root: &XmlNode) -> BTreeMap<String, Vec<String>> {
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for tag in root.children_named("Tag") {
        if !is_global_tag(tag) {
            continue;
        }
        for simple in tag.children_named("Simple") {
            let name = py_strip(&simple.find_text("Name").unwrap_or_default()).to_uppercase();
            let value = py_strip(&simple.find_text("String").unwrap_or_default()).to_string();
            if !name.is_empty() && !value.is_empty() {
                result.entry(name).or_default().push(value);
            }
        }
    }
    result
}

/// Extract and read global tags. Any extraction or parse problem reads as "no tags".
pub(crate) fn read_global_simple_tags(
    tools: &MatroskaTools,
    output: &Path,
    work_dir: &Path,
) -> BTreeMap<String, Vec<String>> {
    let destination = work_dir.join("verify-tags.xml");
    let _ = fs::remove_file(&destination);
    let run = run_tool(
        &tools.mkvextract,
        &[
            output.as_os_str().to_owned(),
            "tags".into(),
            destination.as_os_str().to_owned(),
        ],
        EXTRACT_TIMEOUT,
    );
    if !run.is_ok_and(|run| run.code == Some(0)) || !destination.exists() {
        return BTreeMap::new();
    }
    let parsed = fs::read_to_string(&destination)
        .ok()
        .and_then(|text| parse_xml(&text).ok());
    let _ = fs::remove_file(&destination);
    parsed.map_or_else(BTreeMap::new, |root| global_simple_tags(&root))
}

/// Python `repr()` of a string, for error messages that must match byte for byte.
pub(crate) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other if other == quote => {
                out.push('\\');
                out.push(other);
            }
            other if is_python_printable(other) => out.push(other),
            other => {
                let code = other as u32;
                if code <= 0xff {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
        }
    }
    out.push(quote);
    out
}

/// Approximates `str.isprintable` for common non-printing categories (Cc, Cf, Z*).
fn is_python_printable(character: char) -> bool {
    if character == ' ' {
        return true;
    }
    if character.is_control() || character.is_whitespace() {
        return false;
    }
    !matches!(character as u32,
        0xad | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x180e
        | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f
        | 0xfeff | 0xfff9..=0xfffb | 0xe000..=0xf8ff | 0xf0000..=0x10ffff
        | 0xd800..=0xdfff)
}

/// Why a verification did not pass.
#[derive(Debug)]
pub(crate) enum VerifyError {
    /// The output lacks an expected cover or tag; text matches Python's `RebuildError`.
    Mismatch(String),
    /// The request or an MKVToolNix call was unusable.
    Operational(String),
}

/// Check the cover attachment and expected global tags; mismatches match Python wording.
pub(crate) fn verify_expected_output(
    tools: &MatroskaTools,
    output: &Path,
    work_dir: &Path,
    expect_poster: bool,
    expected_global_tags: &[(String, String)],
) -> Result<BTreeMap<String, Vec<String>>, VerifyError> {
    if !output.is_absolute() || !output.is_file() {
        return Err(VerifyError::Operational(
            "The verify target must be an existing file with an absolute path.".into(),
        ));
    }
    if !work_dir.is_dir() {
        return Err(VerifyError::Operational(
            "The verify work folder must be an existing directory.".into(),
        ));
    }
    if expect_poster {
        let identify = mkv_identify(tools, output).map_err(VerifyError::Operational)?;
        let has_cover = identify
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|attachment| {
                text_of(attachment.get("file_name")).to_lowercase() == "cover.jpg"
                    && text_of(attachment.get("content_type")).to_lowercase() == "image/jpeg"
            });
        if !has_cover {
            return Err(VerifyError::Mismatch(
                "Expected cover.jpg attachment is missing from output.".into(),
            ));
        }
    }
    let mut actual = BTreeMap::new();
    if !expected_global_tags.is_empty() {
        actual = read_global_simple_tags(tools, output, work_dir);
        for (name, value) in expected_global_tags {
            let verified = actual
                .get(&name.to_uppercase())
                .is_some_and(|values| values.contains(value));
            if !verified {
                return Err(VerifyError::Mismatch(format!(
                    "Expected embedded Matroska tag {name}={} was not verified in the output.",
                    python_repr(value)
                )));
            }
        }
    }
    Ok(actual)
}

/// Python `str(value)` for the JSON scalars mkvmerge can emit; missing reads as "".
fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python_quote_and_escape_rules() {
        assert_eq!(python_repr("plain"), "'plain'");
        assert_eq!(python_repr("it's"), "\"it's\"");
        assert_eq!(python_repr("say \"hi\" it's"), "'say \"hi\" it\\'s'");
        assert_eq!(python_repr("a\\b\n\t\r"), "'a\\\\b\\n\\t\\r'");
        assert_eq!(python_repr("é“”"), "'é“”'");
        assert_eq!(
            python_repr("\u{7}\u{a0}\u{200b}\u{1f600}"),
            "'\\x07\\xa0\\u200b😀'"
        );
    }

    #[test]
    fn only_global_simple_tags_are_collected_and_normalized() {
        let root = parse_xml(
            "<Tags><Tag><Targets/><Simple><Name> title </Name><String> A </String></Simple>
               <Simple><Name>TITLE</Name><String>B</String></Simple>
               <Simple><Name>EMPTY</Name><String>  </String></Simple></Tag>
             <Tag><Targets><TrackUID>7</TrackUID></Targets>
               <Simple><Name>TRACKONLY</Name><String>x</String></Simple></Tag>
             <Tag><Targets><TrackUID> </TrackUID></Targets>
               <Simple><Name>BLANKUID</Name><String>y</String></Simple></Tag></Tags>",
        )
        .unwrap();
        let tags = global_simple_tags(&root);
        assert_eq!(tags["TITLE"], ["A", "B"]);
        assert_eq!(tags["BLANKUID"], ["y"]);
        assert!(!tags.contains_key("TRACKONLY") && !tags.contains_key("EMPTY"));
    }

    #[test]
    fn invalid_targets_are_rejected_before_any_tool_runs() {
        let tools = MatroskaTools {
            mkvmerge: "mkvmerge".into(),
            mkvextract: "mkvextract".into(),
            mkvpropedit: "mkvpropedit".into(),
        };
        let error =
            verify_expected_output(&tools, Path::new("x.mkv"), Path::new("/tmp"), true, &[])
                .unwrap_err();
        assert!(matches!(error, VerifyError::Operational(text) if text.contains("absolute path")));
    }
}
