use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

const MAX_MEDIA_PATHS: usize = 10_000;
const MAX_DIRECTORY_PATHS: usize = 1_000;
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

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalMediaAssets {
    pub(crate) path: String,
    pub(crate) artwork: Vec<LocalArtworkFile>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalDirectoryAssets {
    pub(crate) path: String,
    pub(crate) artwork: Vec<LocalArtworkFile>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalAssetsScan {
    pub(crate) root: String,
    pub(crate) files: Vec<LocalMediaAssets>,
    pub(crate) directories: Vec<LocalDirectoryAssets>,
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
    fn scans_nearby_poster_and_ignores_nfo_sidecars_without_network_access() {
        let root = scratch();
        let media = root.join("A Film (2020).mkv");
        fs::write(&media, b"synthetic media path").unwrap();
        fs::write(
            root.join("A Film (2020).nfo"),
            "<movie><title>Ignored</title></movie>",
        )
        .unwrap();
        fs::write(root.join("A Film (2020)-poster.png"), png_header()).unwrap();
        let result = scan_local_assets(LocalAssetsRequest {
            root: root.to_string_lossy().into_owned(),
            media_paths: vec![media.to_string_lossy().into_owned()],
            directory_paths: vec![],
        })
        .unwrap();
        assert_eq!(result.files.len(), 1);
        let file = &result.files[0];
        assert_eq!(file.artwork.len(), 1);
        assert!(file.artwork[0].name.ends_with("-poster.png"));
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
