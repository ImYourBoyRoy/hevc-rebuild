use quick_xml::events::Event;
use quick_xml::Reader;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

const MAX_MEDIA_PATHS: usize = 10_000;
const MAX_DIRECTORY_PATHS: usize = 1_000;
const MAX_NFO_BYTES: u64 = 128 * 1024;
const MAX_NFO_DEPTH: usize = 32;
const MAX_NFO_EVENTS: usize = 20_000;
const MAX_METADATA_TEXT_BYTES: usize = 64 * 1024;
const MAX_ARTWORK_PER_ITEM: usize = 12;
const MAX_ARTWORK_BYTES: u64 = 16 * 1024 * 1024;
const MAX_REQUEST_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalAssetsRequest {
    pub(crate) root: String,
    pub(crate) media_paths: Vec<String>,
    pub(crate) directory_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalArtworkFile {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) role: String,
}

#[derive(Clone, Debug, Default, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalNfoMetadata {
    pub(crate) found: bool,
    pub(crate) paths: Vec<String>,
    pub(crate) title: Option<String>,
    pub(crate) show_title: Option<String>,
    pub(crate) year: Option<String>,
    pub(crate) certification: Option<String>,
    pub(crate) short_description: Option<String>,
    pub(crate) plot: Option<String>,
    pub(crate) genres: Vec<String>,
    pub(crate) rating: Option<f64>,
    pub(crate) source: Option<String>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) airdate: Option<String>,
    #[serde(default)]
    pub(crate) season_names: BTreeMap<String, String>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalMediaAssets {
    pub(crate) path: String,
    pub(crate) artwork: Vec<LocalArtworkFile>,
    pub(crate) metadata: LocalNfoMetadata,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalDirectoryAssets {
    pub(crate) path: String,
    pub(crate) artwork: Vec<LocalArtworkFile>,
    pub(crate) metadata: LocalNfoMetadata,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalAssetsScan {
    pub(crate) root: String,
    pub(crate) files: Vec<LocalMediaAssets>,
    pub(crate) directories: Vec<LocalDirectoryAssets>,
}

#[derive(Clone, Debug)]
struct ParsedNfo {
    kind: String,
    fields: BTreeMap<String, Vec<String>>,
}

fn normalized_name(value: &str) -> String {
    value.nfkc().flat_map(char::to_lowercase).collect()
}

fn is_inside(root: &Path, path: &Path, allow_root: bool) -> bool {
    (allow_root && path == root) || path.starts_with(root)
}

fn canonical_scoped_path(
    root: &Path,
    value: &str,
    expect_directory: bool,
) -> Result<PathBuf, String> {
    let input = Path::new(value);
    if !input.is_absolute() {
        return Err("Local metadata paths must be absolute.".into());
    }
    let canonical = input.canonicalize().map_err(|error| {
        format!(
            "Could not open local library path {}: {error}",
            input.display()
        )
    })?;
    if !is_inside(root, &canonical, expect_directory) {
        return Err(format!(
            "Local library path {} is outside the selected folder.",
            input.display()
        ));
    }
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| "Local library path is outside the selected folder.")?;
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("Local library paths cannot contain parent or prefix components.".into());
        };
        cursor.push(name);
        let metadata = fs::symlink_metadata(&cursor).map_err(|error| {
            format!(
                "Could not inspect local library path {}: {error}",
                cursor.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Local library paths cannot pass through symlinks: {}",
                cursor.display()
            ));
        }
    }
    let metadata = fs::metadata(&canonical).map_err(|error| {
        format!(
            "Could not inspect local library path {}: {error}",
            canonical.display()
        )
    })?;
    if expect_directory && !metadata.is_dir() {
        return Err(format!(
            "Local metadata folder {} is not a directory.",
            canonical.display()
        ));
    }
    if !expect_directory && !metadata.is_file() {
        return Err(format!(
            "Local media path {} is not a regular file.",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn image_extension(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv"
                    | "mp4"
                    | "m4v"
                    | "mov"
                    | "avi"
                    | "webm"
                    | "wmv"
                    | "mpg"
                    | "mpeg"
                    | "m2ts"
                    | "mts"
                    | "ts"
                    | "vob"
                    | "flv"
                    | "ogv"
            )
        })
}

pub(crate) fn valid_image_file(path: &Path) -> bool {
    let Some(expected) = image_extension(path) else {
        return false;
    };
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_ARTWORK_BYTES
    {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 12];
    let Ok(read) = file.read(&mut header) else {
        return false;
    };
    if read < 12 {
        return false;
    }
    match expected {
        "jpg" => header.starts_with(&[0xff, 0xd8, 0xff]),
        "png" => header.starts_with(b"\x89PNG\r\n\x1a\n"),
        "webp" => &header[0..4] == b"RIFF" && &header[8..12] == b"WEBP",
        _ => false,
    }
}

fn index_images(directory: &Path) -> HashMap<String, PathBuf> {
    let mut images = HashMap::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return images;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !valid_image_file(&path) {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            images.insert(normalized_name(name), path);
        }
    }
    images
}

fn add_named_images(
    images: &HashMap<String, PathBuf>,
    bases: &[String],
    result: &mut Vec<LocalArtworkFile>,
) {
    let mut seen = result
        .iter()
        .map(|item| item.path.clone())
        .collect::<HashSet<_>>();
    for base in bases {
        for extension in ["jpg", "jpeg", "png", "webp"] {
            let filename = normalized_name(&format!("{base}.{extension}"));
            let Some(path) = images.get(&filename) else {
                continue;
            };
            let path_text = path.to_string_lossy().into_owned();
            if !seen.insert(path_text.clone()) {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Artwork")
                .to_owned();
            let role = if base.to_ascii_lowercase().contains("poster")
                || base.eq_ignore_ascii_case("cover")
            {
                "Poster or cover"
            } else if base.eq_ignore_ascii_case("fanart") || base.eq_ignore_ascii_case("landscape")
            {
                "Background art"
            } else if base.eq_ignore_ascii_case("folder") {
                "Folder art"
            } else {
                "Local image"
            };
            result.push(LocalArtworkFile {
                path: path_text,
                name,
                role: role.into(),
            });
            if result.len() >= MAX_ARTWORK_PER_ITEM {
                return;
            }
        }
    }
}

fn media_artwork(
    path: &Path,
    directory_cache: &mut HashMap<PathBuf, HashMap<String, PathBuf>>,
) -> Vec<LocalArtworkFile> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let images = directory_cache
        .entry(parent.to_path_buf())
        .or_insert_with(|| index_images(parent));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let stem = stem.to_owned();
    let mut result = Vec::new();
    add_named_images(
        images,
        &[
            format!("{stem}-poster"),
            format!("{stem}_poster"),
            format!("{stem}.poster"),
            format!("{stem}-cover"),
            format!("{stem}_cover"),
            format!("{stem}-fanart"),
            format!("{stem}_fanart"),
            format!("{stem}-thumb"),
            stem,
            "poster".into(),
            "folder".into(),
            "cover".into(),
            "fanart".into(),
            "landscape".into(),
        ],
        &mut result,
    );
    result
}

fn directory_artwork(
    path: &Path,
    season_number: Option<u32>,
    directory_cache: &mut HashMap<PathBuf, HashMap<String, PathBuf>>,
) -> Vec<LocalArtworkFile> {
    let images = directory_cache
        .entry(path.to_path_buf())
        .or_insert_with(|| index_images(path));
    let mut names = vec![
        "folder".to_owned(),
        "poster".into(),
        "cover".into(),
        "fanart".into(),
        "landscape".into(),
        "banner".into(),
    ];
    if let Some(season) = season_number {
        names.extend([
            format!("Season{season:02}"),
            format!("season{season}"),
            format!("season{season:02}-poster"),
            format!("season{season}-poster"),
        ]);
    }
    let mut result = Vec::new();
    add_named_images(images, &names, &mut result);
    if let (Some(season), Some(series_root)) = (season_number, path.parent()) {
        let series_images = directory_cache
            .entry(series_root.to_path_buf())
            .or_insert_with(|| index_images(series_root));
        let series_names = [
            format!("season{season:02}-poster"),
            format!("season{season}-poster"),
            format!("Season {season:02}-poster"),
            format!("Season {season}-poster"),
        ];
        add_named_images(series_images, &series_names, &mut result);
    }
    result
}

fn season_number_from_directory(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name == "specials" {
        return Some(0);
    }
    let digits = name
        .strip_prefix("season")?
        .trim_start_matches([' ', '_', '-'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn show_nfo_candidates(directory: &Path) -> [PathBuf; 4] {
    [
        directory.join("tvshow.nfo"),
        directory.join("tvshow.NFO"),
        directory.join("tvshow.hevc-rebuild.nfo"),
        directory.join("tvshow.hevc-rebuild.NFO"),
    ]
}

fn nfo_candidates(root: &Path, source: &Path, directory: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut parents = Vec::new();
    let mut current = if directory {
        source.to_path_buf()
    } else {
        source.parent().unwrap_or(source).to_path_buf()
    };
    for _ in 0..24 {
        if !is_inside(root, &current, true) {
            break;
        }
        parents.push(current.clone());
        if current == root {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    if directory {
        for parent in parents.iter().take(1) {
            found.extend(show_nfo_candidates(parent));
        }
    } else {
        for parent in &parents {
            found.extend(show_nfo_candidates(parent));
            if found.iter().any(|path| path.is_file()) {
                break;
            }
        }
        let stem = source.file_stem().unwrap_or_default();
        found.extend([
            source.with_extension("nfo"),
            source.with_extension("NFO"),
            source.with_file_name(format!("{}.hevc-rebuild.nfo", stem.to_string_lossy())),
            source.with_file_name(format!("{}.hevc-rebuild.NFO", stem.to_string_lossy())),
        ]);
    }
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|path| {
            path.starts_with(root)
                && seen.insert(path.clone())
                && fs::symlink_metadata(path)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        })
        .collect()
}

fn local_tag(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

fn nfo_values(path: &Path) -> Result<ParsedNfo, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect NFO {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_NFO_BYTES {
        return Err(format!(
            "NFO {} is not a regular file within the 128 KiB limit.",
            path.display()
        ));
    }
    let source = fs::read_to_string(path).map_err(|error| {
        format!(
            "Could not read NFO {} as UTF-8 XML: {error}",
            path.display()
        )
    })?;
    let mut reader = Reader::from_str(&source);
    // Keep boundary whitespace while decoding references such as `&amp;`;
    // field values are normalized after all XML text events are combined.
    reader.config_mut().trim_text(false);
    let mut stack = Vec::<String>::new();
    let mut kind = None;
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut field_text = String::new();
    let mut text_bytes = 0_usize;
    let mut capture_default_rating = false;
    let mut pending_named_season: Option<String> = None;
    for event_index in 0..MAX_NFO_EVENTS {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                if stack.len() >= MAX_NFO_DEPTH {
                    return Err(format!(
                        "NFO {} exceeds the safe XML nesting limit.",
                        path.display()
                    ));
                }
                let name = local_tag(element.local_name().as_ref());
                if stack.is_empty() {
                    kind = Some(name.clone());
                }
                if name == "rating" {
                    capture_default_rating = element.attributes().any(|attribute| {
                        attribute.ok().is_some_and(|attribute| {
                            attribute.key.as_ref() == "default"
                                && attribute.value.as_ref().eq_ignore_ascii_case("true")
                        })
                    });
                }
                if name == "value" && capture_default_rating {
                    field_text.clear();
                }
                if name == "namedseason" {
                    pending_named_season = element.attributes().find_map(|attribute| {
                        let attribute = attribute.ok()?;
                        if attribute.key.as_ref() != "number" {
                            return None;
                        }
                        let number = attribute.value.as_ref().trim();
                        number.parse::<u32>().ok().map(|number| number.to_string())
                    });
                }
                stack.push(name);
            }
            Ok(Event::Empty(element)) => {
                if stack.is_empty() {
                    kind = Some(local_tag(element.local_name().as_ref()));
                }
            }
            Ok(Event::End(_)) => {
                if capture_default_rating
                    && stack.len() == 4
                    && stack.get(3).map(String::as_str) == Some("value")
                {
                    let rating = field_text.trim();
                    if !rating.is_empty() {
                        fields
                            .entry("audience".to_owned())
                            .or_default()
                            .push(rating.to_owned());
                    }
                    field_text.clear();
                }
                if stack.last().map(String::as_str) == Some("rating") {
                    capture_default_rating = false;
                }
                if stack.len() == 2 {
                    let key = &stack[1];
                    let value = field_text.trim();
                    if key == "namedseason" {
                        if let Some(number) = pending_named_season.take() {
                            if !value.is_empty() {
                                fields
                                    .entry(format!("namedseason:{number}"))
                                    .or_default()
                                    .push(value.to_owned());
                            }
                        }
                    }
                    if !value.is_empty()
                        && matches!(
                            key.as_str(),
                            "title"
                                | "showtitle"
                                | "year"
                                | "premiered"
                                | "aired"
                                | "mpaa"
                                | "tagline"
                                | "plot"
                                | "genre"
                                | "userrating"
                                | "source"
                                | "season"
                                | "episode"
                        )
                    {
                        fields
                            .entry(key.clone())
                            .or_default()
                            .push(value.to_owned());
                    }
                    field_text.clear();
                }
                stack.pop();
            }
            Ok(Event::Text(text)) => {
                if stack.len() >= 2 {
                    let value = text.xml10_content();
                    text_bytes = text_bytes.saturating_add(value.len());
                    if text_bytes > MAX_METADATA_TEXT_BYTES {
                        return Err(format!(
                            "NFO {} contains too much text for a metadata preview.",
                            path.display()
                        ));
                    }
                    field_text.push_str(&value);
                }
            }
            Ok(Event::CData(text)) => {
                if stack.len() >= 2 {
                    let value = text.xml10_content();
                    text_bytes = text_bytes.saturating_add(value.len());
                    if text_bytes > MAX_METADATA_TEXT_BYTES {
                        return Err(format!(
                            "NFO {} contains too much text for a metadata preview.",
                            path.display()
                        ));
                    }
                    field_text.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if stack.len() >= 2 {
                    let value = if let Some(character) =
                        reference.resolve_char_ref().map_err(|error| {
                            format!(
                                "NFO {} contains an invalid XML character reference: {error}",
                                path.display()
                            )
                        })? {
                        character.to_string()
                    } else {
                        quick_xml::escape::resolve_predefined_entity(reference.as_ref())
                            .ok_or_else(|| {
                                format!("NFO {} uses an undeclared XML entity.", path.display())
                            })?
                            .to_owned()
                    };
                    text_bytes = text_bytes.saturating_add(value.len());
                    if text_bytes > MAX_METADATA_TEXT_BYTES {
                        return Err(format!(
                            "NFO {} contains too much text for a metadata preview.",
                            path.display()
                        ));
                    }
                    field_text.push_str(&value);
                }
            }
            Ok(Event::DocType(_)) => {
                return Err(format!(
                    "NFO {} uses a document type that is not supported for local metadata preview.",
                    path.display()
                ))
            }
            Ok(Event::Eof) => {
                if !stack.is_empty() {
                    return Err(format!(
                        "NFO {} ends before all XML tags close.",
                        path.display()
                    ));
                }
                return Ok(ParsedNfo {
                    kind: kind.unwrap_or_else(|| "unknown".into()),
                    fields,
                });
            }
            Ok(_) => {}
            Err(error) => return Err(format!("NFO {} is not valid XML: {error}", path.display())),
        }
        if event_index + 1 == MAX_NFO_EVENTS {
            return Err(format!(
                "NFO {} exceeds the safe XML element limit.",
                path.display()
            ));
        }
    }
    Err(format!("NFO {} could not be read safely.", path.display()))
}

fn positive_rating(value: Option<String>) -> Option<f64> {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|rating| *rating > 0.0 && *rating <= 10.0)
}

fn value(fields: &BTreeMap<String, Vec<String>>, key: &str) -> Option<String> {
    let found = fields.get(key)?;
    let mut seen = HashSet::new();
    let joined = found
        .iter()
        .map(|item| item.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|item| !item.is_empty() && seen.insert(normalized_name(item)))
        .collect::<Vec<_>>()
        .join(", ");
    (!joined.is_empty()).then_some(joined)
}

fn merge_nfo(target: &mut BTreeMap<String, Vec<String>>, source: BTreeMap<String, Vec<String>>) {
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn canonical_nfo_field(name: &str) -> &str {
    match name {
        "year" | "premiered" | "aired" => "year",
        "mpaa" => "certification",
        "tagline" => "short_description",
        "userrating" => "rating",
        other => other,
    }
}

fn normalized_nfo_values(key: &str, values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .flat_map(|value| {
            if key == "genre" {
                value.split(',').map(str::to_owned).collect::<Vec<_>>()
            } else {
                vec![value.clone()]
            }
        })
        .map(|value| {
            value
                .nfkc()
                .flat_map(char::to_lowercase)
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn conflicting_nfo_fields(
    left: &BTreeMap<String, Vec<String>>,
    right: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut left_values = BTreeMap::<&str, Vec<String>>::new();
    for (key, values) in left {
        let canonical = canonical_nfo_field(key);
        left_values
            .entry(canonical)
            .or_insert_with(|| normalized_nfo_values(key, values));
    }
    let mut conflicts = BTreeSet::new();
    for (key, values) in right {
        let canonical = canonical_nfo_field(key);
        let Some(previous) = left_values.get(canonical) else {
            continue;
        };
        let normalized = normalized_nfo_values(key, values);
        if normalized.is_empty() {
            continue;
        }
        if canonical == "year"
            && year_values_agree(&[previous.as_slice(), normalized.as_slice()].concat())
        {
            continue;
        }
        if previous != &normalized {
            conflicts.insert(canonical.to_owned());
        }
    }
    conflicts.into_iter().collect()
}

fn year_token(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 4 || !bytes[..4].iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if bytes.len() > 4 && bytes[4] != b'-' {
        return None;
    }
    Some(value[..4].to_owned())
}

fn year_values_agree(values: &[String]) -> bool {
    let mut token: Option<String> = None;
    if values.is_empty() {
        return true;
    }
    for value in values {
        let Some(next) = year_token(value) else {
            return false;
        };
        if let Some(current) = &token {
            if current != &next {
                return false;
            }
        } else {
            token = Some(next);
        }
    }
    true
}

fn conflicting_fields_within_nfo(fields: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut values_by_field = BTreeMap::<&str, Vec<String>>::new();
    for (key, values) in fields {
        let canonical = canonical_nfo_field(key);
        let normalized = normalized_nfo_values(key, values);
        let accumulated = values_by_field.entry(canonical).or_default();
        accumulated.extend(normalized);
        accumulated.sort();
        accumulated.dedup();
    }
    values_by_field
        .into_iter()
        .filter(|(field, values)| {
            *field != "genre"
                && values.len() > 1
                && !(*field == "year" && year_values_agree(values))
        })
        .map(|(field, _)| field.to_owned())
        .collect()
}

fn local_metadata(
    root: &Path,
    source: &Path,
    is_directory: bool,
    cache: &mut HashMap<PathBuf, Result<ParsedNfo, String>>,
) -> LocalNfoMetadata {
    let candidates = nfo_candidates(root, source, is_directory);
    let mut result = LocalNfoMetadata {
        found: !candidates.is_empty(),
        paths: candidates
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        ..LocalNfoMetadata::default()
    };
    let mut fields = BTreeMap::new();
    let mut documents_by_kind = HashMap::<String, (PathBuf, BTreeMap<String, Vec<String>>)>::new();
    for path in candidates {
        let parsed = cache
            .entry(path.clone())
            .or_insert_with(|| nfo_values(&path));
        match parsed {
            Ok(document) => {
                let mut conflicts = conflicting_fields_within_nfo(&document.fields);
                if let Some((previous_path, previous_fields)) =
                    documents_by_kind.get(&document.kind)
                {
                    conflicts.extend(conflicting_nfo_fields(previous_fields, &document.fields));
                    conflicts.sort();
                    conflicts.dedup();
                    if !conflicts.is_empty() {
                        let kind = if document.kind == "unknown" {
                            "metadata".to_owned()
                        } else {
                            document.kind.clone()
                        };
                        let first = previous_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("first NFO");
                        let second = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("second NFO");
                        result.error = Some(format!(
                            "Conflicting {kind} NFO fields in {first} and {second}: {}. No local values were applied; review the sidecars or compare a public source.",
                            conflicts.join(", ")
                        ));
                        return result;
                    }
                    if let Some((_, aggregated_fields)) = documents_by_kind.get_mut(&document.kind)
                    {
                        for (key, values) in &document.fields {
                            aggregated_fields
                                .entry(key.clone())
                                .or_default()
                                .extend(values.iter().cloned());
                        }
                    }
                } else {
                    if !conflicts.is_empty() {
                        let kind = if document.kind == "unknown" {
                            "metadata".to_owned()
                        } else {
                            document.kind.clone()
                        };
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("NFO");
                        result.error = Some(format!(
                            "Conflicting {kind} NFO fields in {name}: {}. No local values were applied; review the sidecar or compare a public source.",
                            conflicts.join(", ")
                        ));
                        return result;
                    }
                    documents_by_kind.insert(
                        document.kind.clone(),
                        (path.clone(), document.fields.clone()),
                    );
                }
                merge_nfo(&mut fields, document.fields.clone());
            }
            Err(error) => {
                result.error = Some(error.clone());
                return result;
            }
        }
    }
    result.title = value(&fields, "title");
    result.show_title = value(&fields, "showtitle");
    result.year = value(&fields, "year")
        .or_else(|| value(&fields, "premiered"))
        .or_else(|| value(&fields, "aired"));
    result.certification = value(&fields, "mpaa").map(|value| {
        value
            .strip_prefix("Rated ")
            .or_else(|| value.strip_prefix("rated "))
            .unwrap_or(&value)
            .to_owned()
    });
    result.short_description = value(&fields, "tagline");
    result.plot = value(&fields, "plot");
    result.genres = fields
        .get("genre")
        .map(|items| {
            items
                .iter()
                .flat_map(|item| item.split(','))
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    result.rating = positive_rating(value(&fields, "userrating"))
        .or_else(|| positive_rating(value(&fields, "audience")));
    result.source = value(&fields, "source");
    result.season = value(&fields, "season").and_then(|value| value.parse().ok());
    result.episode = value(&fields, "episode").and_then(|value| value.parse().ok());
    result.airdate = value(&fields, "aired").or_else(|| value(&fields, "premiered"));
    result.season_names = fields
        .iter()
        .filter_map(|(key, values)| {
            let number = key.strip_prefix("namedseason:")?;
            let name = values.first()?.trim();
            (!name.is_empty()).then(|| (number.to_owned(), name.to_owned()))
        })
        .collect();
    result
}

pub(crate) fn scan_local_assets(request: LocalAssetsRequest) -> Result<LocalAssetsScan, String> {
    if request.media_paths.len() > MAX_MEDIA_PATHS
        || request.directory_paths.len() > MAX_DIRECTORY_PATHS
    {
        return Err("Local metadata preview exceeds the supported path count.".into());
    }
    let root = Path::new(&request.root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected library folder: {error}"))?;
    if !root.is_dir() {
        return Err("Local metadata preview requires a library folder.".into());
    }
    let mut directory_cache = HashMap::new();
    let mut nfo_cache = HashMap::new();
    let mut files = Vec::new();
    let mut seen_files = HashSet::new();
    for path in request.media_paths {
        let path = canonical_scoped_path(&root, &path, false)?;
        if !video_file(&path) {
            continue;
        }
        if !seen_files.insert(path.clone()) {
            continue;
        }
        files.push(LocalMediaAssets {
            artwork: media_artwork(&path, &mut directory_cache),
            metadata: local_metadata(&root, &path, false, &mut nfo_cache),
            path: path.to_string_lossy().into_owned(),
        });
    }
    let mut directories = Vec::new();
    let mut seen_directories = HashSet::new();
    for path in request
        .directory_paths
        .into_iter()
        .chain(std::iter::once(root.to_string_lossy().into_owned()))
    {
        let path = canonical_scoped_path(&root, &path, true)?;
        if !seen_directories.insert(path.clone()) {
            continue;
        }
        directories.push(LocalDirectoryAssets {
            artwork: directory_artwork(
                &path,
                season_number_from_directory(&path),
                &mut directory_cache,
            ),
            metadata: local_metadata(&root, &path, true, &mut nfo_cache),
            path: path.to_string_lossy().into_owned(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    directories.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(LocalAssetsScan {
        root: root.to_string_lossy().into_owned(),
        files,
        directories,
    })
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtworkPreviewRequest {
    pub(crate) root: String,
    pub(crate) path: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArtworkPreview {
    pub(crate) path: String,
    pub(crate) mime_type: String,
    pub(crate) data_url: String,
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

pub(crate) fn read_artwork_preview(
    request: ArtworkPreviewRequest,
) -> Result<ArtworkPreview, String> {
    let root = Path::new(&request.root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected library folder: {error}"))?;
    if !root.is_dir() {
        return Err("Artwork preview requires a library folder.".into());
    }
    let requested = Path::new(&request.path)
        .canonicalize()
        .map_err(|error| format!("Could not open local artwork {}: {error}", request.path))?;
    let path = if requested.starts_with(&root) {
        canonical_scoped_path(&root, &request.path, false)?
    } else {
        let cache_root = crate::library_artwork::app_artwork_cache_root()?
            .canonicalize()
            .map_err(|_| {
                "This artwork is outside the selected folder and the private app cache."
            })?;
        if !requested.starts_with(&cache_root) {
            return Err("Artwork previews must come from the selected folder or the private app artwork cache.".into());
        }
        requested
    };
    if image_extension(&path).is_none() || !valid_image_file(&path) {
        return Err("This local artwork is not a supported JPEG, PNG, or WebP image.".into());
    }
    let metadata =
        fs::metadata(&path).map_err(|error| format!("Could not inspect local artwork: {error}"))?;
    if metadata.len() > MAX_ARTWORK_BYTES {
        return Err("Local artwork is larger than the 16 MiB preview limit.".into());
    }
    let mut file =
        fs::File::open(&path).map_err(|error| format!("Could not open local artwork: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Could not read local artwork: {error}"))?;
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read local artwork: {error}"))?;
    let mime_type = match image_extension(&path) {
        Some("jpg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => unreachable!(),
    };
    let data_url = format!("data:{mime_type};base64,{}", encode_base64(&bytes));
    Ok(ArtworkPreview {
        path: path.to_string_lossy().into_owned(),
        mime_type: mime_type.into(),
        data_url,
    })
}

pub(crate) fn cli_mode(args: &[String]) -> i32 {
    if args.len() != 3 || args[1] != "inspect" {
        eprintln!("Usage: hevc-rebuild-studio metadata assets inspect REQUEST.json");
        return 2;
    }
    let request_path = Path::new(&args[2]);
    match fs::metadata(request_path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_REQUEST_BYTES => {}
        Ok(_) => {
            eprintln!("The local asset request must be a JSON file no larger than 2 MiB.");
            return 2;
        }
        Err(error) => {
            eprintln!("Could not inspect local asset request: {error}");
            return 2;
        }
    };
    let bytes = match fs::read(request_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("Could not read local asset request: {error}");
            return 2;
        }
    };
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        eprintln!("The local asset request grew beyond the 2 MiB limit while being read.");
        return 2;
    }
    let request = match serde_json::from_slice::<LocalAssetsRequest>(&bytes) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("Local asset request JSON is invalid: {error}");
            return 2;
        }
    };
    match scan_local_assets(request).and_then(|result| {
        serde_json::to_string(&result)
            .map_err(|error| format!("Could not encode local asset result: {error}"))
    }) {
        Ok(result) => {
            println!("{result}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("hevc-local-assets-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn png_header() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0; 24]);
        bytes
    }

    #[test]
    fn scans_local_nfo_and_nearby_poster_without_network_access() {
        let root = scratch();
        let media = root.join("A Film (2020).mkv");
        fs::write(&media, b"synthetic media path").unwrap();
        fs::write(root.join("A Film (2020).nfo"), "<movie><title>Local &amp; Film</title><year>2020</year><genre>Animation</genre><genre>Family</genre><userrating>8.4</userrating></movie>").unwrap();
        fs::write(root.join("A Film (2020)-poster.png"), png_header()).unwrap();
        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        assert_eq!(result.files.len(), 1);
        let file = &result.files[0];
        assert!(file.metadata.found);
        assert_eq!(file.metadata.title.as_deref(), Some("Local & Film"));
        assert_eq!(file.metadata.year.as_deref(), Some("2020"));
        assert_eq!(file.metadata.genres, ["Animation", "Family"]);
        assert_eq!(file.metadata.rating, Some(8.4));
        assert_eq!(file.artwork.len(), 1);
        assert!(file.artwork[0].name.ends_with("-poster.png"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn show_nfo_year_and_premiered_date_for_the_same_year_keep_the_other_fields() {
        let root = scratch();
        let show = root.join("Faerie");
        fs::create_dir_all(&show).unwrap();
        fs::write(
            show.join("tvshow.nfo"),
            "<tvshow><title>Faerie Tale Theatre</title><year>1982</year><premiered>1982-09-11</premiered><aired></aired><plot>A fairy tale anthology.</plot><genre>Family</genre><userrating>0</userrating><ratings><rating name=\"imdb\" max=\"10\"><value>8.3</value></rating><rating name=\"tmdb\" max=\"10\" default=\"true\"><value>8.8</value></rating></ratings><namedseason number=\"1\">Season 1</namedseason></tvshow>",
        )
        .unwrap();

        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![],
            directory_paths: vec![show.to_string_lossy().into_owned()],
        })
        .unwrap();
        let metadata = &result
            .directories
            .iter()
            .find(|directory| directory.path.ends_with("Faerie"))
            .unwrap()
            .metadata;
        assert!(metadata.error.is_none(), "{:?}", metadata.error);
        assert_eq!(metadata.title.as_deref(), Some("Faerie Tale Theatre"));
        assert_eq!(metadata.year.as_deref(), Some("1982"));
        assert_eq!(metadata.plot.as_deref(), Some("A fairy tale anthology."));
        assert_eq!(metadata.genres, ["Family"]);
        assert_eq!(metadata.rating, Some(8.8));
        assert_eq!(
            metadata.season_names.get("1").map(String::as_str),
            Some("Season 1")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn conflicting_movie_nfo_candidates_are_reported_without_applying_either() {
        let root = scratch();
        let media = root.join("Film.mkv");
        fs::write(&media, b"synthetic media path").unwrap();
        fs::write(
            root.join("Film.nfo"),
            "<movie><title>First Film</title><year>2020</year></movie>",
        )
        .unwrap();
        fs::write(
            root.join("Film.hevc-rebuild.nfo"),
            "<movie><title>Different Film</title><year>2021</year></movie>",
        )
        .unwrap();

        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        let metadata = &result.files[0].metadata;
        assert!(metadata.found);
        assert_eq!(metadata.paths.len(), 2);
        let error = metadata.error.as_deref().unwrap();
        assert!(error.contains("Conflicting movie NFO fields"));
        assert!(error.contains("title"));
        assert!(error.contains("year"));
        assert!(metadata.title.is_none());
        assert!(metadata.year.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn equivalent_movie_nfos_allow_case_spacing_genre_order_and_year_aliases() {
        let root = scratch();
        let media = root.join("Film.mkv");
        fs::write(&media, b"synthetic media path").unwrap();
        fs::write(
            root.join("Film.nfo"),
            "<movie><title> A Film </title><year>2020</year><genre>Drama, Mystery</genre></movie>",
        )
        .unwrap();
        fs::write(
            root.join("Film.hevc-rebuild.nfo"),
            "<movie><title>a   film</title><premiered>2020</premiered><genre>Mystery</genre><genre>Drama</genre></movie>",
        )
        .unwrap();

        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        let metadata = &result.files[0].metadata;
        assert!(metadata.error.is_none());
        assert_eq!(metadata.title.as_deref(), Some("a film"));
        assert_eq!(metadata.year.as_deref(), Some("2020"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_conflicting_scalar_fields_inside_one_movie_nfo_fail_closed() {
        let root = scratch();
        let media = root.join("Film.mkv");
        fs::write(&media, b"synthetic media path").unwrap();
        fs::write(
            root.join("Film.nfo"),
            "<movie><title>First Film</title><title>Second Film</title><year>2020</year></movie>",
        )
        .unwrap();

        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        let metadata = &result.files[0].metadata;
        assert!(metadata
            .error
            .as_deref()
            .unwrap()
            .contains("Conflicting movie NFO fields"));
        assert!(metadata.title.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_show_nfo_is_found_for_nested_episode_and_overrides_with_episode_nfo() {
        let root = scratch();
        let season = root.join("Season 01");
        fs::create_dir_all(&season).unwrap();
        let media = season.join("Show.S01E02.mkv");
        fs::write(&media, b"synthetic").unwrap();
        fs::write(
            root.join("tvshow.nfo"),
            "<tvshow><title>Local Show</title><year>2018</year></tvshow>",
        )
        .unwrap();
        fs::write(root.join("season01-poster.png"), png_header()).unwrap();
        fs::write(season.join("Show.S01E02.nfo"), "<episodedetails><title>Second Episode</title><season>1</season><episode>2</episode><plot>Episode plot</plot></episodedetails>").unwrap();
        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![season.to_string_lossy().into_owned()],
        })
        .unwrap();
        let metadata = &result.files[0].metadata;
        assert_eq!(metadata.title.as_deref(), Some("Second Episode"));
        assert_eq!(metadata.year.as_deref(), Some("2018"));
        assert_eq!(metadata.episode, Some(2));
        assert_eq!(metadata.paths.len(), 2);
        let season_assets = result
            .directories
            .iter()
            .find(|assets| assets.path == season.to_string_lossy())
            .unwrap();
        assert!(season_assets
            .artwork
            .iter()
            .any(|image| image.name == "season01-poster.png"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_local_nfo_is_reported_instead_of_silently_ignored() {
        let root = scratch();
        let media = root.join("Film.mkv");
        fs::write(&media, b"synthetic").unwrap();
        fs::write(root.join("Film.nfo"), "<movie><title>unfinished").unwrap();
        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        assert!(result.files[0].metadata.found);
        assert!(result.files[0].metadata.error.is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artwork_preview_is_scoped_and_returns_a_data_url() {
        let root = scratch();
        let poster = root.join("poster.png");
        fs::write(&poster, png_header()).unwrap();
        let preview = read_artwork_preview(ArtworkPreviewRequest {
            root: root.to_string_lossy().into_owned(),
            path: poster.to_string_lossy().into_owned(),
        })
        .unwrap();
        assert_eq!(preview.mime_type, "image/png");
        assert!(preview
            .data_url
            .starts_with("data:image/png;base64,iVBORw0KGgo"));
        let outside = scratch();
        let outside_poster = outside.join("outside.png");
        fs::write(&outside_poster, png_header()).unwrap();
        let escaped = read_artwork_preview(ArtworkPreviewRequest {
            root: root.to_string_lossy().into_owned(),
            path: outside_poster.to_string_lossy().into_owned(),
        });
        assert!(escaped.unwrap_err().contains("outside the selected folder"));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn image_signature_must_match_the_extension() {
        let root = scratch();
        let wrong = root.join("poster.jpg");
        fs::write(&wrong, png_header()).unwrap();
        assert!(!valid_image_file(&wrong));
        fs::remove_dir_all(root).unwrap();
    }
}
