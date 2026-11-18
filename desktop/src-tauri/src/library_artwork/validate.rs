//! Provider host, image type, credit text and target validation before any artwork is fetched or written.

use super::*;

fn expected_host(provider: &MetadataProvider) -> Option<&'static str> {
    match provider {
        MetadataProvider::WikimediaCommons => Some("upload.wikimedia.org"),
        MetadataProvider::TvMaze => Some("static.tvmaze.com"),
        MetadataProvider::Tmdb => Some("image.tmdb.org"),
        MetadataProvider::Tvdb => Some("artworks.thetvdb.com"),
        MetadataProvider::Wikidata | MetadataProvider::Wikipedia => None,
    }
}

/// Every host artwork may be downloaded from; the browser page's content-security policy allows the same ones.
pub(crate) fn image_hosts() -> Vec<&'static str> {
    [
        MetadataProvider::WikimediaCommons,
        MetadataProvider::TvMaze,
        MetadataProvider::Tmdb,
        MetadataProvider::Tvdb,
    ]
    .iter()
    .filter_map(expected_host)
    .collect()
}

pub(super) fn expected_source_host(provider: &MetadataProvider) -> &'static str {
    match provider {
        MetadataProvider::WikimediaCommons => "commons.wikimedia.org",
        MetadataProvider::TvMaze => "www.tvmaze.com",
        MetadataProvider::Tmdb => "www.themoviedb.org",
        MetadataProvider::Tvdb => "www.thetvdb.com",
        MetadataProvider::Wikidata => "www.wikidata.org",
        MetadataProvider::Wikipedia => "en.wikipedia.org",
    }
}

fn normalize_credit_text(
    field: &mut Option<String>,
    label: &str,
    limit: usize,
) -> Result<(), String> {
    let Some(value) = field.take() else {
        return Ok(());
    };
    if value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(format!("Artwork {label} details are invalid or too long."));
    }
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    *field = if value.is_empty() { None } else { Some(value) };
    Ok(())
}

pub(super) fn normalize_credit_fields(candidate: &mut ArtworkCandidate) -> Result<(), String> {
    normalize_credit_text(&mut candidate.creator, "creator", 2048)?;
    normalize_credit_text(&mut candidate.license, "license", 2048)?;
    normalize_credit_text(&mut candidate.attribution, "attribution", 4096)?;
    Ok(())
}

fn validate_image_url(provider: &MetadataProvider, value: &str) -> Result<Url, String> {
    let expected =
        expected_host(provider).ok_or("This provider does not supply downloadable artwork.")?;
    let url = Url::parse(value).map_err(|_| "Artwork URL is invalid.")?;
    if url.scheme() != "https"
        || url.host_str() != Some(expected)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(
            "Artwork must use HTTPS from the selected provider's approved image host.".into(),
        );
    }
    let extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp") {
        return Err("Only JPEG, PNG, and WebP poster images can be saved.".into());
    }
    Ok(url)
}

fn valid_redirect(url: &Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
}

pub(super) fn detect_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() >= 32 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.len() >= 32 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.len() >= 32 && bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

pub(super) fn validate_candidate(candidate: &ArtworkCandidate) -> Result<(Url, String), String> {
    if candidate.id.trim().is_empty()
        || candidate.id.len() > 512
        || candidate.title.trim().is_empty()
        || candidate.title.len() > 1024
    {
        return Err("Artwork details are incomplete or too long.".into());
    }
    for value in [candidate.title.as_str(), candidate.id.as_str()] {
        if value.chars().any(char::is_control) {
            return Err("Artwork details cannot contain control characters.".into());
        }
    }
    for (label, value, limit) in [
        ("creator", candidate.creator.as_deref(), 2048),
        ("license", candidate.license.as_deref(), 2048),
        ("attribution", candidate.attribution.as_deref(), 4096),
    ] {
        if value.is_some_and(|value| value.len() > limit || value.chars().any(char::is_control)) {
            return Err(format!("Artwork {label} details are invalid or too long."));
        }
    }
    for value in [
        candidate.source_url.as_str(),
        candidate.thumbnail_url.as_str(),
    ] {
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err("Artwork source links are invalid or too long.".into());
        }
    }
    if candidate.image_url.len() > 8192 || candidate.image_url.chars().any(char::is_control) {
        return Err("Artwork image links are invalid or too long.".into());
    }
    let source =
        Url::parse(&candidate.source_url).map_err(|_| "Artwork source link is invalid.")?;
    if source.scheme() != "https"
        || source.host_str() != Some(expected_source_host(&candidate.provider))
        || !source.username().is_empty()
        || source.password().is_some()
        || source.port().is_some_and(|port| port != 443)
    {
        return Err(
            "Artwork credit must link to the selected provider's HTTPS source page.".into(),
        );
    }
    if let Some(value) = candidate.license_url.as_deref() {
        if value.len() > 8192 || value.chars().any(char::is_control) {
            return Err("Artwork license links are invalid or too long.".into());
        }
        let license = Url::parse(value).map_err(|_| "Artwork license link is invalid.")?;
        if !matches!(license.scheme(), "https" | "http")
            || license.host_str().is_none()
            || !license.username().is_empty()
            || license.password().is_some()
            || license.port().is_some_and(|port| {
                !matches!((license.scheme(), port), ("https", 443) | ("http", 80))
            })
        {
            return Err("Artwork license links must use a public HTTP or HTTPS URL.".into());
        }
    }
    let url = validate_image_url(&candidate.provider, &candidate.image_url)?;
    let advertised_extension = url
        .path()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok((url, advertised_extension))
}

pub(super) fn validate_targets(
    root: &str,
    targets: &[ArtworkTarget],
) -> Result<(PathBuf, Vec<(PathBuf, String)>), String> {
    if targets.is_empty() || targets.len() > MAX_ARTWORK_TARGETS {
        return Err(format!(
            "Choose between 1 and {MAX_ARTWORK_TARGETS} artwork destinations."
        ));
    }
    let root = PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media folder: {error}"))?;
    if !root.is_dir() {
        return Err("The artwork root must be an existing folder.".into());
    }
    // A single reviewed image may be written under several server-specific
    // names in one folder. Uniqueness is therefore per normalized output stem,
    // rather than per destination directory.
    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(targets.len());
    for target in targets {
        if !Path::new(&target.directory).is_absolute() {
            return Err("Artwork destination folders must be absolute paths.".into());
        }
        if target.basename.is_empty()
            || target.basename.len() > 220
            || target.basename == "."
            || target.basename == ".."
            || Path::new(&target.basename)
                .file_name()
                .and_then(|name| name.to_str())
                != Some(target.basename.as_str())
            || target.basename.chars().any(char::is_control)
            || target.basename.contains('/')
            || target.basename.contains('\\')
        {
            return Err("Artwork filenames must be a single safe filename stem.".into());
        }
        let directory = PathBuf::from(&target.directory)
            .canonicalize()
            .map_err(|error| {
                format!(
                    "Could not open artwork destination {}: {error}",
                    target.directory
                )
            })?;
        if !directory.is_dir() || !directory.starts_with(&root) {
            return Err(
                "Artwork destinations must be existing folders under the selected library root."
                    .into(),
            );
        }
        let output_stem = directory.join(&target.basename);
        let normalized_stem = output_stem
            .to_string_lossy()
            .nfkc()
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if !seen.insert(normalized_stem) {
            return Err(
                "Artwork targets must have unique filenames after Unicode and case normalization."
                    .into(),
            );
        }
        resolved.push((directory, target.basename.clone()));
    }
    Ok((root, resolved))
}

pub(super) async fn fetch_image(
    provider: &MetadataProvider,
    url: Url,
) -> Result<(Vec<u8>, &'static str), String> {
    let host = expected_host(provider)
        .ok_or("This provider does not supply downloadable artwork.")?
        .to_string();
    let redirect_host = host.clone();
    let client = Client::builder()
        .timeout(Duration::from_secs(35))
        .connect_timeout(Duration::from_secs(8))
        .redirect(redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 3 || !valid_redirect(attempt.url(), &redirect_host) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(format!(
            "HEVC-Rebuild-Studio/{} (user-requested artwork download)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| format!("Could not initialize the artwork download: {error}"))?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not download the selected artwork: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Artwork provider returned HTTP {}.",
            response.status()
        ));
    }
    if response.url().scheme() != "https" || response.url().host_str() != Some(&host) {
        return Err("Artwork provider redirected to an unapproved image host.".into());
    }
    if response
        .content_length()
        .is_some_and(|length| usize::try_from(length).unwrap_or(usize::MAX) > MAX_ARTWORK_BYTES)
    {
        return Err("The selected artwork exceeds the 16 MiB download limit.".into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("application/octet-stream")
        .trim()
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Artwork download failed while receiving data: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_ARTWORK_BYTES {
            return Err("The selected artwork exceeds the 16 MiB download limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let Some((extension, detected_type)) = detect_image(&bytes) else {
        return Err("The downloaded file is not a supported JPEG, PNG, or WebP image.".into());
    };
    if content_type != "application/octet-stream"
        && content_type != detected_type
        && !(content_type == "image/jpg" && detected_type == "image/jpeg")
    {
        return Err("The artwork's declared image type does not match its file contents.".into());
    }
    Ok((bytes, extension))
}
