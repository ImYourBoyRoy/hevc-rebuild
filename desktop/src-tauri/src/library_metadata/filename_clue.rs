//! Title, year and certification clues parsed from a media filename.

use super::*;

static FILENAME_YEAR_REGEX: OnceLock<Regex> = OnceLock::new();

static FILENAME_RELEASE_MARKER_REGEX: OnceLock<Regex> = OnceLock::new();

static FILENAME_CERTIFICATION_REGEX: OnceLock<Regex> = OnceLock::new();

/// Build a search hint from a filename or series-folder name without accepting
/// or returning a filesystem path. The hint is local until the user starts a
/// provider lookup; only `title` is sent to a metadata provider.
// Compatibility delegate retained; canonical callers/tests use the pattern-aware path.
#[allow(dead_code)]
pub(crate) fn filename_metadata_clue(filename: &str) -> Result<FilenameMetadataClue, String> {
    filename_metadata_clue_with_patterns(filename, &[])
}

pub(crate) fn filename_metadata_clue_with_patterns(
    filename: &str,
    text_blacklist_patterns: &[String],
) -> Result<FilenameMetadataClue, String> {
    let filename = filename.trim();
    if filename.is_empty()
        || filename.len() > MAX_FILENAME_HINT_BYTES
        || filename.chars().any(char::is_control)
        || filename.contains('/')
        || filename.contains('\\')
    {
        return Err("Provide only a filename or series-folder name up to 1024 bytes.".into());
    }

    let stem = filename_stem(filename);
    let year_regex = FILENAME_YEAR_REGEX.get_or_init(|| {
        Regex::new(
            r"(?:^|[^0-9])(?P<year>(?:19|20)\d{2})(?:[-–](?P<year_end>(?:19|20)\d{2}))?(?:$|[^0-9])",
        )
        .expect("filename year expression is valid")
    });
    let release_regex = FILENAME_RELEASE_MARKER_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._-])(?:\d{3,4}p|4k|8k|web[\s._-]?(?:dl|rip)|blu[\s._-]?ray|bdrip|brrip|hdtv|dvd[\s._-]?rip|remux|x26[45]|h\.?26[45]|hevc|av1|hdr10\+?|hlg|uhd|aac|ac3|eac3|ddp|dts|truehd|atmos|flac|10bit|8bit)(?:$|[\s._-])",
        )
        .expect("filename release expression is valid")
    });
    let certification_regex = FILENAME_CERTIFICATION_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._(\[])(?P<cert>NC-17|PG-13|TV-MA|TV-14|TV-PG|TV-G|TV-Y7?|UNRATED|NR|PG|G|R|X)(?:$|[\s)._\-\]])",
        )
        .expect("filename certification expression is valid")
    });

    let year_capture = year_regex.captures(stem);
    let year = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year"))
        .map(|value| value.as_str().to_string());
    let year_end = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year_end"))
        .map(|value| value.as_str().to_string());
    let year_position = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year"))
        .map(|value| value.start());
    let release_position = release_regex.find(stem).map(|matched| matched.start());
    let title_end = [year_position, release_position]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(stem.len());
    let title_fragment = &stem[..title_end];
    let title = title_fragment
        .chars()
        .map(|character| match character {
            '.' | '_' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '(' | ')' | '[' | ']' | '{' | '}' | '.' | '_' | '-'
                )
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = crate::metadata_text::clean_value(&title, text_blacklist_patterns)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        return Err("Could not find a title before the filename's year or release markers.".into());
    }
    if title.len() > MAX_QUERY_BYTES {
        return Err(
            "The filename-derived title exceeds the 256-byte metadata search limit.".into(),
        );
    }

    let certification = year_capture
        .as_ref()
        .and_then(|capture| capture.name("year_end").or_else(|| capture.name("year")))
        .and_then(|year_match| stem.get(year_match.end()..))
        .and_then(|tail| certification_regex.captures(tail))
        .and_then(|capture| capture.name("cert"))
        .map(|value| value.as_str().to_ascii_uppercase());

    Ok(FilenameMetadataClue {
        title,
        year,
        year_end,
        certification,
    })
}

fn filename_stem(filename: &str) -> &str {
    let Some((stem, extension)) = filename.rsplit_once('.') else {
        return filename;
    };
    if matches!(
        extension.to_ascii_lowercase().as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "mov"
            | "avi"
            | "webm"
            | "ts"
            | "m2ts"
            | "mts"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "flv"
            | "ogv"
            | "vob"
    ) {
        stem
    } else {
        filename
    }
}
