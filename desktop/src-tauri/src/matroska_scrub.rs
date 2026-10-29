//! ./desktop/src-tauri/src/matroska_scrub.rs
//! Native output text scrub for Matroska files (migration slice T4.3b).
//!
//! Ports `hevc_rebuild/metadata.py::_scrub_chapter_tree` and
//! `_scrub_output_text_metadata`: after a remux or encode, extract the output's
//! tags and chapters, clean every text value (blacklist globs and recognized
//! release-group suffixes), and write the sanitized XML and track-header names
//! back with `mkvpropedit`. Only metadata is edited, in place, on the file the
//! caller names; no stream payload is touched.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-scrub`.
//! Input: tool paths, absolute `output` MKV, existing `work_dir`, blacklist.
//! Output: change records and chapter-text edits (same shape as Python).
//! Side effects: `mkvpropedit` rewrites tags/chapters/track names of `output`
//! only when cleanup changed something. Python remains authoritative for
//! production runs until parity is proven.

use crate::matroska_tags::{node_at_mut, parse_xml, scrub_tag_tree, serialize_xml, XmlNode};
use crate::metadata_text::clean_batch;
use crate::process_output::spawn_owned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOOL_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const INSPECT_TIMEOUT: Duration = Duration::from_secs(300);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const EDIT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatroskaTools {
    pub mkvmerge: String,
    pub mkvextract: String,
    pub mkvpropedit: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScrubRequest {
    tools: MatroskaTools,
    output: PathBuf,
    work_dir: PathBuf,
    #[serde(default)]
    text_blacklist: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ScrubOutcome {
    pub changes: Vec<Value>,
    pub chapter_edits: Vec<Value>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(|request| {
        scrub_output_text_metadata(
            &request.tools,
            &request.output,
            &request.work_dir,
            &request.text_blacklist,
        )
    }) {
        Ok(outcome) => match serde_json::to_string(&outcome) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize the Matroska scrub response: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<ScrubRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the Matroska scrub request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("The Matroska scrub request exceeds the 8 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The Matroska scrub request JSON is invalid: {error}"))
}

pub(crate) struct ToolRun {
    pub(crate) code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

fn read_capped(mut reader: impl Read) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    let mut overflow = false;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let room = MAX_TOOL_OUTPUT_BYTES.saturating_sub(output.len());
                if count > room {
                    overflow = true;
                }
                output.extend_from_slice(&buffer[..count.min(room)]);
            }
        }
    }
    (output, overflow)
}

/// Run one MKVToolNix command in its own process group with bounded output.
pub(crate) fn run_tool(
    program: &str,
    args: &[OsString],
    timeout: Duration,
) -> Result<ToolRun, String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_owned(&mut command)
        .map_err(|error| format!("Could not start external command {program}: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not capture tool output.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not capture tool errors.")?;
    let stdout_reader = thread::spawn(move || read_capped(stdout));
    let stderr_reader = thread::spawn(move || read_capped(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = crate::process_output::kill_owned_group(child.id());
                let _ = child.wait();
                break None;
            }
            Err(error) => {
                let _ = crate::process_output::kill_owned_group(child.id());
                let _ = child.wait();
                return Err(format!("Could not wait for {program}: {error}"));
            }
        }
    };
    // Descendants holding inherited pipes must not block the reader joins.
    let _ = crate::process_output::kill_owned_group(child.id());
    let (stdout, stdout_overflow) = stdout_reader.join().unwrap_or_default();
    let (stderr, _) = stderr_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(format!(
            "{program} timed out after {} seconds.",
            timeout.as_secs()
        ));
    };
    if stdout_overflow {
        return Err(format!(
            "{program} produced more output than is safe to read."
        ));
    }
    let tail = String::from_utf8_lossy(&stderr);
    let tail: String = tail
        .trim()
        .chars()
        .rev()
        .take(4000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Ok(ToolRun {
        code: status.code(),
        stdout,
        stderr: tail,
    })
}

pub(crate) fn checked(
    program: &str,
    args: Vec<OsString>,
    timeout: Duration,
) -> Result<ToolRun, String> {
    let run = run_tool(program, &args, timeout)?;
    if run.code != Some(0) {
        let detail = if run.stderr.is_empty() {
            String::from_utf8_lossy(&run.stdout).trim().to_string()
        } else {
            run.stderr.clone()
        };
        return Err(format!(
            "External command {program} failed with exit code {}. {detail}",
            run.code
                .map_or_else(|| "unknown".to_string(), |code| code.to_string())
        ));
    }
    Ok(run)
}

/// `mkvmerge -J`, validating the three record lists the scrub depends on.
pub(crate) fn mkv_identify(tools: &MatroskaTools, path: &Path) -> Result<Value, String> {
    let run = checked(
        &tools.mkvmerge,
        vec!["-J".into(), path.as_os_str().to_owned()],
        INSPECT_TIMEOUT,
    )?;
    let value: Value = serde_json::from_slice(&run.stdout).map_err(|error| {
        format!(
            "MKVToolNix {} returned malformed JSON for {}: {error}",
            tools.mkvmerge,
            path.display()
        )
    })?;
    let Some(object) = value.as_object() else {
        return Err(format!(
            "MKVToolNix {} returned an invalid response for {}.",
            tools.mkvmerge,
            path.display()
        ));
    };
    if !object.get("tracks").is_some_and(Value::is_array) {
        return Err(format!(
            "MKVToolNix {} returned no track list for {}.",
            tools.mkvmerge,
            path.display()
        ));
    }
    for field in ["tracks", "attachments", "chapters"] {
        if let Some(records) = object.get(field) {
            let valid = records
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_object));
            if !valid {
                return Err(format!(
                    "MKVToolNix {} returned invalid {field} for {}.",
                    tools.mkvmerge,
                    path.display()
                ));
            }
        }
    }
    Ok(value)
}

/// `mkvextract <mode>`; `None` when the file has no such metadata.
fn extract_xml(
    tools: &MatroskaTools,
    path: &Path,
    mode: &str,
    destination: &Path,
) -> Result<Option<XmlNode>, String> {
    let run = run_tool(
        &tools.mkvextract,
        &[
            path.as_os_str().to_owned(),
            mode.into(),
            destination.as_os_str().to_owned(),
        ],
        EXTRACT_TIMEOUT,
    )?;
    if run.code != Some(0) {
        return Err(format!(
            "Could not safely inspect Matroska {mode} metadata (mkvextract exit {}).",
            run.code
                .map_or_else(|| "unknown".to_string(), |code| code.to_string())
        ));
    }
    if !destination.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(destination)
        .map_err(|error| format!("Could not safely read Matroska {mode} metadata: {error}"))?;
    parse_xml(&text)
        .map(Some)
        .map_err(|error| format!("Could not safely parse Matroska {mode} metadata: {error}"))
}

fn write_xml(root: &XmlNode, destination: &Path) -> Result<(), String> {
    fs::write(destination, serialize_xml(root))
        .map_err(|error| format!("Could not write sanitized Matroska metadata: {error}"))
}

struct ChapterTarget {
    chapter_index: usize,
    start_ns: String,
    string_path: Vec<usize>,
    before: String,
}

/// Collect chapter titles whose nearest `ChapterAtom` ancestor is `atom`.
fn collect_atom_strings(
    node: &XmlNode,
    path: &mut Vec<usize>,
    out: &mut Vec<(Vec<usize>, String)>,
) {
    for (index, child) in node.children.iter().enumerate() {
        if child.name == "ChapterAtom" {
            continue;
        }
        path.push(index);
        if child.name == "ChapterString" {
            if let Some(text) = &child.text {
                out.push((path.clone(), text.clone()));
            }
        }
        collect_atom_strings(child, path, out);
        path.pop();
    }
}

fn collect_atoms(node: &XmlNode, path: &mut Vec<usize>, atoms: &mut Vec<Vec<usize>>) {
    for (index, child) in node.children.iter().enumerate() {
        path.push(index);
        if child.name == "ChapterAtom" {
            atoms.push(path.clone());
        }
        collect_atoms(child, path, atoms);
        path.pop();
    }
}

/// Clean chapter titles in place. Returns (change records, chapter edits) in
/// document order, matching Python `_scrub_chapter_tree`.
pub(crate) fn scrub_chapter_tree(
    root: &mut XmlNode,
    patterns: &[String],
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let mut atoms: Vec<Vec<usize>> = Vec::new();
    collect_atoms(root, &mut Vec::new(), &mut atoms);

    let mut targets: Vec<ChapterTarget> = Vec::new();
    for (chapter_index, atom_path) in atoms.iter().enumerate() {
        let Some(atom) = node_at(root, atom_path) else {
            continue;
        };
        let start_ns = atom.find_text("ChapterTimeStart").unwrap_or_default();
        let mut strings = Vec::new();
        collect_atom_strings(atom, &mut Vec::new(), &mut strings);
        for (relative, before) in strings {
            let mut string_path = atom_path.clone();
            string_path.extend(relative);
            targets.push(ChapterTarget {
                chapter_index,
                start_ns: start_ns.clone(),
                string_path,
                before,
            });
        }
    }

    let cleaned = clean_batch(
        targets.iter().map(|target| target.before.clone()).collect(),
        patterns,
    )?;
    let mut changes = Vec::new();
    let mut edits = Vec::new();
    let mut mutations: Vec<(Vec<usize>, String)> = Vec::new();
    for (target, (after, rules)) in targets.iter().zip(cleaned) {
        if after == target.before {
            continue;
        }
        mutations.push((target.string_path.clone(), after.clone()));
        changes.push(json!({
            "scope": "chapter", "field": "title",
            "chapter_index": target.chapter_index, "chapter_start": target.start_ns,
            "before": target.before, "after": after, "rules": rules,
        }));
        edits.push(json!({
            "scope": "chapter", "chapter_index": target.chapter_index,
            "before": target.before, "after": after,
        }));
    }
    for (path, after) in mutations {
        if let Some(node) = node_at_mut(root, &path) {
            node.text = Some(after);
        }
    }
    Ok((changes, edits))
}

fn node_at<'a>(root: &'a XmlNode, path: &[usize]) -> Option<&'a XmlNode> {
    let mut node = root;
    for index in path {
        node = node.children.get(*index)?;
    }
    Some(node)
}

/// Clean tags, chapters, and track-header names of `output` in place.
pub(crate) fn scrub_output_text_metadata(
    tools: &MatroskaTools,
    output: &Path,
    work_dir: &Path,
    patterns: &[String],
) -> Result<ScrubOutcome, String> {
    if !output.is_absolute() || !output.is_file() {
        return Err("The scrub target must be an existing file with an absolute path.".into());
    }
    if !work_dir.is_dir() {
        return Err("The scrub work folder must be an existing directory.".into());
    }
    let mut changes: Vec<Value> = Vec::new();
    let mut chapter_edits: Vec<Value> = Vec::new();

    if let Some(mut tags) = extract_xml(tools, output, "tags", &work_dir.join("output-tags.xml"))? {
        let tag_changes = scrub_tag_tree(&mut tags, patterns)?;
        if !tag_changes.is_empty() {
            let sanitized = work_dir.join("sanitized-output-tags.xml");
            write_xml(&tags, &sanitized)?;
            let mut argument = OsString::from("all:");
            argument.push(&sanitized);
            checked(
                &tools.mkvpropedit,
                vec![output.as_os_str().to_owned(), "--tags".into(), argument],
                EDIT_TIMEOUT,
            )?;
            changes.extend(tag_changes);
        }
    }

    if let Some(mut chapters) = extract_xml(
        tools,
        output,
        "chapters",
        &work_dir.join("output-chapters.xml"),
    )? {
        let (chapter_changes, edits) = scrub_chapter_tree(&mut chapters, patterns)?;
        chapter_edits = edits;
        if !chapter_changes.is_empty() {
            let sanitized = work_dir.join("sanitized-output-chapters.xml");
            write_xml(&chapters, &sanitized)?;
            checked(
                &tools.mkvpropedit,
                vec![
                    output.as_os_str().to_owned(),
                    "--chapters".into(),
                    sanitized.into_os_string(),
                ],
                EDIT_TIMEOUT,
            )?;
            changes.extend(chapter_changes);
        }
    }

    // Matroska track names live in track headers, not in the XML tag tree.
    let identify = mkv_identify(tools, output)?;
    let tracks = identify
        .get("tracks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut targets: Vec<(usize, String, String)> = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        let name = track
            .get("properties")
            .and_then(|properties| properties.get("track_name"))
            .and_then(Value::as_str);
        if let Some(name) = name {
            let kind = track
                .get("type")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown");
            targets.push((index + 1, name.to_string(), kind.to_string()));
        }
    }
    let cleaned = clean_batch(
        targets.iter().map(|(_, name, _)| name.clone()).collect(),
        patterns,
    )?;
    let mut edit_args: Vec<OsString> = vec![output.as_os_str().to_owned()];
    for ((position, before, kind), (after, rules)) in targets.iter().zip(cleaned) {
        if after == *before {
            continue;
        }
        edit_args.extend([
            "--edit".into(),
            format!("track:{position}").into(),
            "--set".into(),
            format!("name={after}").into(),
        ]);
        changes.push(json!({
            "scope": "track_header", "field": "title", "track_index": position - 1,
            "track_type": kind, "before": before, "after": after, "rules": rules,
        }));
    }
    if edit_args.len() > 1 {
        checked(&tools.mkvpropedit, edit_args, EDIT_TIMEOUT)?;
    }
    Ok(ScrubOutcome {
        changes,
        chapter_edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAPTERS: &str = "<Chapters><EditionEntry>
        <ChapterAtom><ChapterTimeStart>00:00:00.000000000</ChapterTimeStart>
          <ChapterDisplay><ChapterString>Intro MovieReleaseGroup</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay>
          <ChapterAtom><ChapterTimeStart>00:00:05.000000000</ChapterTimeStart>
            <ChapterDisplay><ChapterString>Nested Movieshowgroup</ChapterString></ChapterDisplay>
          </ChapterAtom>
        </ChapterAtom>
        <ChapterAtom><ChapterDisplay><ChapterString>Untouched</ChapterString></ChapterDisplay></ChapterAtom>
      </EditionEntry></Chapters>";

    #[test]
    fn chapter_titles_are_cleaned_with_nearest_atom_attribution() {
        let mut root = parse_xml(CHAPTERS).unwrap();
        let (changes, edits) = scrub_chapter_tree(&mut root, &["Movie*Group".to_string()]).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["chapter_index"], 0);
        assert_eq!(changes[0]["chapter_start"], "00:00:00.000000000");
        assert_eq!(changes[0]["after"], "Intro");
        assert_eq!(changes[1]["chapter_index"], 1);
        assert_eq!(changes[1]["chapter_start"], "00:00:05.000000000");
        assert_eq!(edits[1]["after"], "Nested");
        assert!(edits[0].get("rules").is_none());
        let xml = serialize_xml(&root);
        assert!(xml.contains("<ChapterString>Intro</ChapterString>"));
        assert!(xml.contains("<ChapterString>Untouched</ChapterString>"));
    }

    #[test]
    fn chapter_scrub_without_rules_changes_nothing_for_plain_titles() {
        let mut root = parse_xml(CHAPTERS).unwrap();
        let (changes, edits) = scrub_chapter_tree(&mut root, &[]).unwrap();
        assert!(changes.is_empty() && edits.is_empty());
    }

    #[test]
    fn scrub_rejects_missing_targets_before_running_any_tool() {
        let tools = MatroskaTools {
            mkvmerge: "mkvmerge".into(),
            mkvextract: "mkvextract".into(),
            mkvpropedit: "mkvpropedit".into(),
        };
        let error =
            scrub_output_text_metadata(&tools, Path::new("relative.mkv"), Path::new("/tmp"), &[])
                .unwrap_err();
        assert!(error.contains("absolute path"));
    }

    #[test]
    fn tool_runner_reports_nonzero_exit_and_missing_programs() {
        #[cfg(unix)]
        {
            let error = checked(
                "sh",
                vec!["-c".into(), "echo bad >&2; exit 3".into()],
                INSPECT_TIMEOUT,
            )
            .err()
            .unwrap();
            assert!(error.contains("exit code 3") && error.contains("bad"));
        }
        let error = run_tool("hevc-rebuild-no-such-tool", &[], INSPECT_TIMEOUT)
            .err()
            .unwrap();
        assert!(error.contains("Could not start"));
    }
}
