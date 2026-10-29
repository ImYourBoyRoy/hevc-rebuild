//! ./desktop/src-tauri/src/matroska_tags.rs
//! Native Matroska global-tag builder (migration slice T4.3a).
//!
//! Ports the pure decision logic of `hevc_rebuild/metadata.py::build_tags` and
//! `_scrub_tag_tree`: reviewed overrides, release-group and
//! blacklist cleanup, merge with the source's existing global tags, and the
//! merged `Tags` XML handed to `mkvpropedit`. It performs no media or file I/O,
//! so the future mutating run coordinator can call it directly, and
//! `internal metadata-tags` exposes it for Python/Rust parity fixtures.
//!
//! Run: `echo '<request json>' | hevc-rebuild-studio internal metadata-tags`.
//! Input: source stem, comment, blacklist, reviewed
//! override object, and optional `mkvextract tags` XML. Output: title, expected
//! tag values, change records, merged tag XML, and warnings.
//! Python remains authoritative for production runs until parity is proven.

use crate::metadata_text::{clean_batch, is_python_whitespace};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{self, Read};

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_XML_NODES: usize = 100_000;
const MAX_XML_DEPTH: usize = 64;

const OVERRIDE_TEXT_TAGS: [(&str, &str); 8] = [
    ("title", "TITLE"),
    ("show_title", "SHOWTITLE"),
    ("certification", "LAW_RATING"),
    ("short_description", "SUMMARY"),
    ("plot", "SYNOPSIS"),
    ("season", "SEASON"),
    ("episode", "EPISODE"),
    ("source_url", "SOURCE"),
];

const RELEASE_GROUP_TAG_NAMES: [&str; 3] = ["RELEASE_GROUP", "RELEASEGROUP", "RLSGROUP"];
const TARGET_UID_SCOPES: [(&str, &str); 4] = [
    ("TrackUID", "track_tag"),
    ("ChapterUID", "chapter_tag"),
    ("AttachmentUID", "attachment_tag"),
    ("EditionUID", "edition_tag"),
];

/// Minimal element tree with ElementTree semantics: `text` is the character
/// data before the first child; text after a child (the "tail") is dropped.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct XmlNode {
    pub name: String,
    pub text: Option<String>,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    fn with_text(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            text: Some(text.to_string()),
            children: Vec::new(),
        }
    }

    pub(crate) fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|child| child.name == name)
    }

    pub(crate) fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlNode> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// ElementTree `findtext(name)`: `None` when absent, empty text for an
    /// empty element.
    pub(crate) fn find_text(&self, name: &str) -> Option<String> {
        self.child(name)
            .map(|child| child.text.clone().unwrap_or_default())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagBuildRequest {
    pub source_stem: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub text_blacklist: Vec<String>,
    #[serde(default)]
    pub metadata_override: Option<Map<String, Value>>,
    #[serde(default)]
    pub existing_tags_xml: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TagBuildResult {
    pub title: String,
    pub expected_tags: Map<String, Value>,
    pub changes: Vec<Value>,
    pub tags_xml: String,
    pub warnings: Vec<String>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(build_tags) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize the Matroska tag response: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<TagBuildRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the Matroska tag request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("The Matroska tag request exceeds the 8 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("The Matroska tag request JSON is invalid: {error}"))
}

/// Parse well-formed XML into an [`XmlNode`] tree. DTDs are rejected so
/// entity expansion can never be abused.
pub(crate) fn parse_xml(xml: &str) -> Result<XmlNode, String> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;
    let mut node_count = 0usize;

    fn attach(
        stack: &mut [XmlNode],
        root: &mut Option<XmlNode>,
        node: XmlNode,
    ) -> Result<(), String> {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
            Ok(())
        } else if root.is_none() {
            *root = Some(node);
            Ok(())
        } else {
            Err("XML document has more than one root element.".into())
        }
    }
    fn append_text(stack: &mut [XmlNode], value: &str) {
        if value.is_empty() {
            return;
        }
        if let Some(top) = stack.last_mut() {
            if top.children.is_empty() {
                top.text.get_or_insert_with(String::new).push_str(value);
            }
        }
    }

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                node_count += 1;
                if node_count > MAX_XML_NODES || stack.len() >= MAX_XML_DEPTH {
                    return Err("XML document exceeds the safe size or depth limit.".into());
                }
                stack.push(XmlNode::new(element.name().as_ref()));
            }
            Ok(Event::Empty(element)) => {
                node_count += 1;
                if node_count > MAX_XML_NODES {
                    return Err("XML document exceeds the safe size limit.".into());
                }
                let node = XmlNode::new(element.name().as_ref());
                attach(&mut stack, &mut root, node)?;
            }
            Ok(Event::End(_)) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| "XML document has an unexpected closing tag.".to_string())?;
                attach(&mut stack, &mut root, node)?;
            }
            Ok(Event::Text(text)) => append_text(&mut stack, &text.xml10_content()),
            Ok(Event::CData(text)) => append_text(&mut stack, &text.xml10_content()),
            Ok(Event::GeneralRef(reference)) => {
                let value = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|error| format!("Invalid XML character reference: {error}"))?
                {
                    character.to_string()
                } else {
                    quick_xml::escape::resolve_predefined_entity(reference.as_ref())
                        .ok_or_else(|| "XML uses an undeclared entity.".to_string())?
                        .to_owned()
                };
                append_text(&mut stack, &value);
            }
            Ok(Event::DocType(_)) => {
                return Err("XML document types are not supported.".into());
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("XML is not well formed: {error}")),
        }
    }
    if !stack.is_empty() {
        return Err("XML ends before all tags close.".into());
    }
    root.ok_or_else(|| "XML document has no root element.".to_string())
}

fn escape_text(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

fn write_node(node: &XmlNode, out: &mut String) {
    let text = if node.children.is_empty() {
        node.text.as_deref()
    } else {
        node.text
            .as_deref()
            .filter(|value| !value.chars().all(is_python_whitespace))
    };
    out.push('<');
    out.push_str(&node.name);
    if text.is_none() && node.children.is_empty() {
        out.push_str(" />");
        return;
    }
    out.push('>');
    if let Some(value) = text {
        escape_text(value, out);
    }
    for child in &node.children {
        write_node(child, out);
    }
    out.push_str("</");
    out.push_str(&node.name);
    out.push('>');
}

/// Serialize with an XML declaration, as `mkvpropedit --tags` expects.
pub(crate) fn serialize_xml(root: &XmlNode) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    write_node(root, &mut out);
    out
}

pub(crate) fn py_strip(value: &str) -> &str {
    value.trim_matches(is_python_whitespace)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

/// Python `float(str)` for the finite forms rating fields can contain.
fn python_float(value: &str) -> Option<f64> {
    let trimmed = py_strip(value);
    if trimmed.is_empty() {
        return None;
    }
    let characters: Vec<char> = trimmed.chars().collect();
    let mut cleaned = String::with_capacity(trimmed.len());
    for (index, character) in characters.iter().enumerate() {
        if *character == '_' {
            let digit_before = index > 0 && characters[index - 1].is_ascii_digit();
            let digit_after = characters
                .get(index + 1)
                .is_some_and(|next| next.is_ascii_digit());
            if !(digit_before && digit_after) {
                return None;
            }
        } else {
            cleaned.push(*character);
        }
    }
    cleaned.parse::<f64>().ok()
}

/// Matroska RATING is 0–5; reviewed ratings are 0–10.
fn matroska_rating(rating: f64) -> String {
    format!("{:.2}", rating / 2.0)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn upsert(values: &mut Vec<(String, String)>, name: &str, value: String) {
    if let Some(entry) = values.iter_mut().find(|(key, _)| key == name) {
        entry.1 = value;
    } else {
        values.push((name.to_string(), value));
    }
}

fn override_text(overrides: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match overrides.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(py_strip(text).to_string())),
        Some(_) => Err(format!("Reviewed metadata field '{key}' must be text.")),
    }
}

fn apply_overrides(
    values: &mut Vec<(String, String)>,
    overrides: &Map<String, Value>,
    comment: &str,
) -> Result<(), String> {
    for (source, target) in OVERRIDE_TEXT_TAGS {
        if let Some(text) = override_text(overrides, source)? {
            upsert(values, target, text);
        }
    }
    if overrides.contains_key("year") || overrides.contains_key("airdate") {
        let chosen = ["airdate", "year"]
            .iter()
            .filter_map(|key| overrides.get(*key))
            .find(|value| is_truthy(value));
        let date = match chosen {
            None => String::new(),
            Some(Value::String(text)) => py_strip(text).to_string(),
            Some(_) => return Err("Reviewed year and air date values must be text.".into()),
        };
        upsert(values, "DATE_RELEASED", date);
    }
    if let Some(genres) = overrides.get("genres") {
        let Value::Array(items) = genres else {
            return Err("Reviewed genres must be a list of text values.".into());
        };
        let mut names = Vec::new();
        for item in items {
            let Value::String(text) = item else {
                return Err("Reviewed genres must be a list of text values.".into());
            };
            let stripped = py_strip(text);
            if !stripped.is_empty() {
                names.push(stripped.to_string());
            }
        }
        let mut seen = HashSet::new();
        names.retain(|name| seen.insert(name.clone()));
        upsert(values, "GENRE", names.join(", "));
    }
    if let Some(rating) = overrides.get("rating") {
        let Value::String(text) = rating else {
            return Err("Reviewed rating must be text.".into());
        };
        if py_strip(text).is_empty() {
            upsert(values, "RATING", String::new());
        } else {
            let number = python_float(text)
                .filter(|number| (0.0..=10.0).contains(number))
                .ok_or_else(|| "Reviewed rating must be a number from 0 to 10.".to_string())?;
            upsert(values, "RATING", matroska_rating(number));
        }
    }
    if let Some(comment_value) = overrides.get("comment") {
        let Value::String(text) = comment_value else {
            return Err("Reviewed comment must be text.".into());
        };
        upsert(values, "COMMENT", py_strip(text).to_string());
    } else if !comment.is_empty() {
        upsert(values, "COMMENT", comment.to_string());
    }
    Ok(())
}

pub(crate) fn is_global_tag(tag: &XmlNode) -> bool {
    let Some(targets) = tag.child("Targets") else {
        return true;
    };
    !targets.children.iter().any(|child| {
        TARGET_UID_SCOPES
            .iter()
            .any(|(field, _)| child.name == *field)
            && child
                .text
                .as_deref()
                .is_some_and(|text| !py_strip(text).is_empty())
    })
}

fn tag_scope(tag: &XmlNode) -> &'static str {
    let Some(targets) = tag.child("Targets") else {
        return "global";
    };
    for (field, scope) in TARGET_UID_SCOPES {
        if targets.children_named(field).any(|child| {
            child
                .text
                .as_deref()
                .is_some_and(|text| !py_strip(text).is_empty())
        }) {
            return scope;
        }
    }
    "global"
}

fn read_existing_global_tags(xml: Option<&str>, warnings: &mut Vec<String>) -> XmlNode {
    let mut root = XmlNode::new("Tags");
    let Some(xml) = xml.filter(|text| !text.trim().is_empty()) else {
        return root;
    };
    match parse_xml(xml) {
        Ok(parsed) => {
            for tag in parsed
                .children_named("Tag")
                .filter(|tag| is_global_tag(tag))
            {
                root.children.push(tag.clone());
            }
        }
        Err(error) => warnings.push(format!(
            "Existing Matroska tags could not be parsed and will not be merged: {error}"
        )),
    }
    root
}

fn remove_conflicting_simple_tags(root: &mut XmlNode, names: &HashSet<String>) {
    root.children.retain_mut(|tag| {
        if tag.name != "Tag" || !is_global_tag(tag) {
            return true;
        }
        tag.children.retain(|simple| {
            if simple.name != "Simple" {
                return true;
            }
            let name = simple.find_text("Name").unwrap_or_default();
            !names.contains(&py_strip(&name).to_uppercase())
        });
        tag.child("Simple").is_some()
    });
}

fn normalized_tag_name(value: &str) -> String {
    let mut out = String::new();
    let mut pending_separator = false;
    for character in value.to_uppercase().chars() {
        if character.is_ascii_uppercase() || character.is_ascii_digit() {
            if pending_separator && !out.is_empty() {
                out.push('_');
            }
            pending_separator = false;
            out.push(character);
        } else {
            pending_separator = true;
        }
    }
    out
}

struct TextEvent {
    simple_path: Vec<usize>,
    string_index: usize,
    scope: &'static str,
    field: String,
    before: String,
    remove: bool,
}

fn collect_text_events(
    parent: &XmlNode,
    path: &mut Vec<usize>,
    scope: &'static str,
    events: &mut Vec<TextEvent>,
) {
    for (index, simple) in parent.children.iter().enumerate() {
        if simple.name != "Simple" {
            continue;
        }
        let name = py_strip(&simple.find_text("Name").unwrap_or_default()).to_string();
        let string_index = simple
            .children
            .iter()
            .position(|child| child.name == "String");
        path.push(index);
        if let Some(string_index) = string_index {
            if RELEASE_GROUP_TAG_NAMES.contains(&normalized_tag_name(&name).as_str()) {
                events.push(TextEvent {
                    simple_path: path.clone(),
                    string_index,
                    scope,
                    field: if name.is_empty() {
                        "RELEASE_GROUP".into()
                    } else {
                        name
                    },
                    before: simple.children[string_index]
                        .text
                        .clone()
                        .unwrap_or_default(),
                    remove: true,
                });
                path.pop();
                continue;
            }
            if let Some(text) = &simple.children[string_index].text {
                events.push(TextEvent {
                    simple_path: path.clone(),
                    string_index,
                    scope,
                    field: if name.is_empty() { "text".into() } else { name },
                    before: text.clone(),
                    remove: false,
                });
            }
        }
        collect_text_events(simple, path, scope, events);
        path.pop();
    }
}

pub(crate) fn node_at_mut<'a>(root: &'a mut XmlNode, path: &[usize]) -> Option<&'a mut XmlNode> {
    let mut node = root;
    for index in path {
        node = node.children.get_mut(*index)?;
    }
    Some(node)
}

/// Remove release-group tags and clean every text value in the tag tree.
/// Mirrors Python `_scrub_tag_tree`, returning change records in event order.
pub(crate) fn scrub_tag_tree(
    root: &mut XmlNode,
    patterns: &[String],
) -> Result<Vec<Value>, String> {
    let mut events: Vec<TextEvent> = Vec::new();
    let tag_indices: Vec<usize> = root
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.name == "Tag")
        .map(|(index, _)| index)
        .collect();
    for index in tag_indices {
        let scope = tag_scope(&root.children[index]);
        let mut path = vec![index];
        collect_text_events(&root.children[index], &mut path, scope, &mut events);
    }

    let editable: Vec<String> = events
        .iter()
        .filter(|event| !event.remove)
        .map(|event| event.before.clone())
        .collect();
    let mut cleaned = clean_batch(editable, patterns)?.into_iter();

    let mut changes = Vec::new();
    let mut edits: Vec<(Vec<usize>, String)> = Vec::new();
    let mut removals: Vec<Vec<usize>> = Vec::new();
    for event in &events {
        if event.remove {
            removals.push(event.simple_path.clone());
            changes.push(json!({
                "scope": event.scope, "field": event.field,
                "before": event.before, "after": "",
            }));
            continue;
        }
        let (after, rules) = cleaned
            .next()
            .ok_or_else(|| "Text cleanup returned fewer results than requested.".to_string())?;
        if after != event.before {
            let mut string_path = event.simple_path.clone();
            string_path.push(event.string_index);
            edits.push((string_path, after.clone()));
            changes.push(json!({
                "scope": event.scope, "field": event.field,
                "before": event.before, "after": after, "rules": rules,
            }));
        }
    }
    for (path, after) in edits {
        if let Some(node) = node_at_mut(root, &path) {
            node.text = Some(after);
        }
    }
    removals.sort_by(|left, right| right.cmp(left));
    for path in removals {
        if let Some((last, parent_path)) = path.split_last() {
            if let Some(parent) = node_at_mut(root, parent_path) {
                if *last < parent.children.len() {
                    parent.children.remove(*last);
                }
            }
        }
    }
    Ok(changes)
}

/// Build the merged global-tag document and expected validation values.
pub(crate) fn build_tags(request: TagBuildRequest) -> Result<TagBuildResult, String> {
    let mut values: Vec<(String, String)> = Vec::new();
    let empty = Map::new();
    apply_overrides(
        &mut values,
        request.metadata_override.as_ref().unwrap_or(&empty),
        &request.comment,
    )?;

    let raw_title = values
        .iter()
        .find(|(name, _)| name == "TITLE")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| request.source_stem.clone());
    let non_title: Vec<(String, String)> = values
        .iter()
        .filter(|(name, _)| name != "TITLE")
        .cloned()
        .collect();
    let mut texts: Vec<String> = non_title.iter().map(|(_, value)| value.clone()).collect();
    texts.push(raw_title.clone());
    let mut cleaned = clean_batch(texts, &request.text_blacklist)?;
    let (title, title_rules) = cleaned
        .pop()
        .ok_or_else(|| "Text cleanup returned no title.".to_string())?;

    let mut changes: Vec<Value> = Vec::new();
    for ((name, value), (after, rules)) in non_title.iter().zip(cleaned) {
        if after != *value {
            upsert(&mut values, name, after.clone());
            changes.push(json!({
                "scope": "global", "field": name,
                "before": value, "after": after, "rules": rules,
            }));
        }
    }
    if title != raw_title {
        changes.push(json!({
            "scope": "global", "field": "TITLE",
            "before": raw_title, "after": title, "rules": title_rules,
        }));
        changes.push(json!({
            "scope": "segment", "field": "title",
            "before": raw_title, "after": title, "rules": title_rules,
        }));
    }
    upsert(&mut values, "TITLE", title.clone());

    let mut warnings = Vec::new();
    let mut tags = read_existing_global_tags(request.existing_tags_xml.as_deref(), &mut warnings);
    let names: HashSet<String> = values.iter().map(|(name, _)| name.to_uppercase()).collect();
    remove_conflicting_simple_tags(&mut tags, &names);
    let mut tag = XmlNode::new("Tag");
    tag.children.push(XmlNode::new("Targets"));
    for (name, value) in values.iter().filter(|(_, value)| !value.is_empty()) {
        let mut simple = XmlNode::new("Simple");
        simple.children.push(XmlNode::with_text("Name", name));
        simple.children.push(XmlNode::with_text("String", value));
        tag.children.push(simple);
    }
    tags.children.push(tag);

    changes.extend(scrub_tag_tree(&mut tags, &request.text_blacklist)?);

    let expected_tags: Map<String, Value> = values
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    Ok(TagBuildResult {
        title,
        expected_tags,
        changes,
        tags_xml: serialize_xml(&tags),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(stem: &str) -> TagBuildRequest {
        TagBuildRequest {
            source_stem: stem.into(),
            comment: String::new(),
            text_blacklist: Vec::new(),
            metadata_override: None,
            existing_tags_xml: None,
        }
    }

    fn strings(xml: &str) -> Vec<String> {
        fn walk(node: &XmlNode, out: &mut Vec<String>) {
            if node.name == "String" {
                out.push(node.text.clone().unwrap_or_default());
            }
            node.children.iter().for_each(|child| walk(child, out));
        }
        let mut out = Vec::new();
        walk(&parse_xml(xml).unwrap(), &mut out);
        out
    }

    fn override_of(value: Value) -> Option<Map<String, Value>> {
        value.as_object().cloned()
    }

    #[test]
    fn glob_cleanup_applies_to_comment_and_filename_title() {
        let mut input = request("Example MovieReleaseGroup");
        input.comment = "Keep Movieshowgroup metadata".into();
        input.text_blacklist = vec!["Movie*Group".into()];
        let result = build_tags(input).unwrap();
        assert_eq!(result.title, "Example");
        assert_eq!(result.expected_tags["COMMENT"], "Keep metadata");
        assert_eq!(result.expected_tags["TITLE"], "Example");
        let fields: Vec<&str> = result
            .changes
            .iter()
            .map(|change| change["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["COMMENT", "TITLE", "title"]);
        assert_eq!(strings(&result.tags_xml), ["Keep metadata", "Example"]);
    }

    #[test]
    fn file_comment_override_replaces_the_global_comment() {
        let mut input = request("Example");
        input.comment = "Global comment".into();
        input.metadata_override = override_of(json!({"comment": "Episode comment"}));
        assert_eq!(
            build_tags(input).unwrap().expected_tags["COMMENT"],
            "Episode comment"
        );
    }

    #[test]
    fn replaced_source_tags_do_not_leave_empty_tag_elements() {
        let mut input = request("Chaos Theory (2008 PG-13)");
        input.comment = "Brought To you By V1x0r".into();
        input.metadata_override = override_of(json!({"title": "Chaos Theory"}));
        input.existing_tags_xml = Some(
            "<Tags>
              <Tag><Simple><Name>TITLE</Name><String>Chaos Theory</String></Simple><Targets><TargetTypeValue>50</TargetTypeValue></Targets></Tag>
              <Tag><Simple><Name>GENRE</Name><String>Comedy</String></Simple><Targets><TargetTypeValue>50</TargetTypeValue></Targets></Tag>
            </Tags>"
                .into(),
        );
        let result = build_tags(input).unwrap();
        let root = parse_xml(&result.tags_xml).unwrap();
        assert!(root
            .children
            .iter()
            .all(|tag| tag.child("Simple").is_some()));
        assert_eq!(result.expected_tags["TITLE"], "Chaos Theory");
        assert!(strings(&result.tags_xml).contains(&"Comedy".to_string()));
    }

    #[test]
    fn targeted_source_tags_are_not_merged_but_global_ones_are_kept() {
        let mut input = request("Film");
        input.existing_tags_xml = Some(
            "<Tags>
              <Tag><Targets><TrackUID>7</TrackUID></Targets><Simple><Name>BPS</Name><String>1</String></Simple></Tag>
              <Tag><Targets/><Simple><Name>ENCODER</Name><String>mkvmerge</String></Simple></Tag>
            </Tags>"
                .into(),
        );
        let xml = build_tags(input).unwrap().tags_xml;
        let values = strings(&xml);
        assert!(values.contains(&"mkvmerge".to_string()));
        assert!(!values.contains(&"1".to_string()));
    }

    #[test]
    fn unparseable_source_tags_warn_and_are_not_merged() {
        let mut input = request("Film");
        input.existing_tags_xml = Some("<Tags><Tag>".into());
        let result = build_tags(input).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.expected_tags["TITLE"], "Film");
    }

    #[test]
    fn invalid_reviewed_values_are_rejected() {
        let mut input = request("Film");
        input.metadata_override = override_of(json!({"rating": "nan"}));
        assert!(build_tags(input).unwrap_err().contains("0 to 10"));
        let mut input = request("Film");
        input.metadata_override = override_of(json!({"season": 2}));
        assert!(build_tags(input).unwrap_err().contains("must be text"));
    }

    #[test]
    fn rating_conversion_matches_python_formatting() {
        for (input, expected) in [
            ("0", "0"),
            ("10", "5"),
            ("0.25", "0.12"),
            ("9.99", "5"),
            ("7", "3.5"),
        ] {
            assert_eq!(
                matroska_rating(python_float(input).unwrap()),
                expected,
                "{input}"
            );
        }
        assert_eq!(python_float("1_0"), Some(10.0));
        assert_eq!(python_float("1__0"), None);
        assert_eq!(python_float(" 3 "), Some(3.0));
    }

    #[test]
    fn tag_scrub_removes_release_group_tags_and_cleans_text() {
        let mut root = parse_xml(
            "<Tags>
              <Tag><Targets/><Simple><Name>COMMENT</Name><String>Film.1080p.WEB-DL.x265-GROUP</String></Simple>
                <Simple><Name>Release Group</Name><String>GROUP</String></Simple></Tag>
              <Tag><Targets><TrackUID>1</TrackUID></Targets><Simple><Name>TITLE</Name><String>Keep</String></Simple></Tag>
            </Tags>",
        )
        .unwrap();
        let changes = scrub_tag_tree(&mut root, &[]).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0]["after"], "Film.1080p.WEB-DL.x265");
        assert_eq!(changes[1]["field"], "Release Group");
        assert_eq!(changes[1]["after"], "");
        assert!(changes[1].get("rules").is_none());
        assert_eq!(root.children[0].children.len(), 2);
        assert_eq!(
            root.children[0].children[1].children[1].text.as_deref(),
            Some("Film.1080p.WEB-DL.x265")
        );
    }

    #[test]
    fn serialized_xml_escapes_text_and_round_trips() {
        let mut input = request("A & B <C>");
        input.comment = "x".into();
        let xml = build_tags(input).unwrap().tags_xml;
        assert!(xml.contains("A &amp; B &lt;C&gt;"));
        assert!(strings(&xml).contains(&"A & B <C>".to_string()));
    }
}
