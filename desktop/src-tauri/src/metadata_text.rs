use regex::Regex;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{self, Read};
use std::sync::OnceLock;

const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 1024 * 1024;
const MAX_TEXTS: usize = 10_000;
const MAX_PATTERNS: usize = 100;
const MAX_PATTERN_BYTES: usize = 128;
const RELEASE_SUFFIX_RULE: &str = "release-group suffix";

static RELEASE_MARKER_REGEX: OnceLock<Regex> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanRequest {
    texts: Vec<String>,
    patterns: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CleanResponse {
    results: Vec<CleanResult>,
    suffix_policy: &'static str,
}

#[derive(Debug, Serialize)]
struct CleanResult {
    text: String,
    matched: Vec<String>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(clean_request) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(text) => {
                println!("{text}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize metadata text cleanup response: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn read_request() -> Result<CleanRequest, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read metadata text cleanup request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("Metadata text cleanup request exceeds the 8 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Metadata text cleanup request JSON is invalid: {error}"))
}

fn clean_request(request: CleanRequest) -> Result<CleanResponse, String> {
    if request.texts.len() > MAX_TEXTS {
        return Err("Metadata text cleanup accepts at most 10000 text values.".into());
    }
    if request.patterns.len() > MAX_PATTERNS {
        return Err("Metadata text cleanup accepts at most 100 blacklist patterns.".into());
    }
    let total_text_bytes = request.texts.iter().try_fold(0usize, |total, text| {
        if text.len() > MAX_TEXT_BYTES {
            return None;
        }
        total.checked_add(text.len())
    });
    let Some(total_text_bytes) = total_text_bytes else {
        return Err("Each metadata text cleanup value is limited to 256 KiB.".into());
    };
    if total_text_bytes > MAX_TOTAL_TEXT_BYTES {
        return Err("Metadata text cleanup values exceed the 1 MiB combined limit.".into());
    }
    for pattern in &request.patterns {
        validate_pattern(&pattern)?;
    }
    let results = request
        .texts
        .into_iter()
        .map(|text| clean_text(text, &request.patterns))
        .collect();
    Ok(CleanResponse {
        results,
        suffix_policy: "recognized-release-group-v1",
    })
}

fn clean_text(mut text: String, patterns: &[String]) -> CleanResult {
    let mut matched = Vec::new();
    for pattern in patterns {
        let (cleaned, did_match) = remove_glob_matches(&text, pattern);
        if did_match {
            text = cleaned;
            matched.push(pattern.clone());
        }
    }
    let (cleaned, removed_suffix) = scrub_release_group_suffix(&text);
    text = cleaned;
    if removed_suffix {
        matched.push(RELEASE_SUFFIX_RULE.to_string());
    }
    CleanResult { text, matched }
}

fn scrub_release_group_suffix(value: &str) -> (String, bool) {
    let trimmed = value.trim_end_matches(is_python_whitespace);
    let trailing = &value[trimmed.len()..];
    if trimmed.is_empty() {
        return (value.to_string(), false);
    }

    for (closing, opening) in [(']', '['), (')', '(')] {
        if !trimmed.ends_with(closing) {
            continue;
        }
        let Some(open_index) = trimmed[..trimmed.len() - closing.len_utf8()].rfind(opening) else {
            continue;
        };
        let group = &trimmed[open_index + opening.len_utf8()..trimmed.len() - closing.len_utf8()];
        let prefix = trimmed[..open_index].trim_end_matches(is_python_whitespace);
        if is_release_group(prefix, group) {
            return (
                format!("{}{}", trim_release_group_delimiters(prefix), trailing),
                true,
            );
        }
    }

    for (index, separator) in trimmed.char_indices().rev() {
        if !matches!(separator, '.' | '_' | '-' | '–' | '—') {
            continue;
        }
        let group_start = index + separator.len_utf8();
        let group = &trimmed[group_start..];
        let prefix = trimmed[..index].trim_end_matches(is_python_whitespace);
        if is_release_group(prefix, group) {
            return (
                format!("{}{}", trim_release_group_delimiters(prefix), trailing),
                true,
            );
        }
    }
    (value.to_string(), false)
}

fn is_release_group(prefix: &str, group: &str) -> bool {
    if !is_release_group_token(group) || !release_marker_regex().is_match(prefix) {
        return false;
    }
    let normalized_group = group
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect::<String>();
    let known_lowercase_group = matches!(
        normalized_group.as_str(),
        "amiable"
            | "cakes"
            | "ctrlhd"
            | "d3g"
            | "eureka"
            | "ettv"
            | "evo"
            | "eztv"
            | "fgt"
            | "flux"
            | "framestor"
            | "galaxyrg"
            | "hdt"
            | "ion10"
            | "joy"
            | "megusta"
            | "ntb"
            | "pahe"
            | "psa"
            | "rarbg"
            | "sparks"
            | "tigole"
            | "yify"
            | "yts"
            | "ytsmx"
    );
    let has_uppercase = group.chars().any(char::is_uppercase);
    let has_lowercase = group.chars().any(char::is_lowercase);
    if !(has_uppercase && !has_lowercase) && !known_lowercase_group {
        return false;
    }
    !group
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_uppercase)
        .any(|token| is_technical_release_token(&token))
}

fn is_release_group_token(group: &str) -> bool {
    let mut characters = group.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && group.chars().count() >= 2
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn is_technical_release_token(token: &str) -> bool {
    matches!(
        token,
        "2160P"
            | "1080P"
            | "720P"
            | "576P"
            | "480P"
            | "4320P"
            | "4K"
            | "8K"
            | "WEB"
            | "DL"
            | "RIP"
            | "BLURAY"
            | "BDRIP"
            | "BRRIP"
            | "HDTV"
            | "DVDRIP"
            | "REMUX"
            | "X264"
            | "X265"
            | "H264"
            | "H265"
            | "HEVC"
            | "AV1"
            | "AAC"
            | "AC3"
            | "EAC3"
            | "DDP"
            | "DTS"
            | "DTSHD"
            | "TRUEHD"
            | "ATMOS"
            | "FLAC"
            | "HDR"
            | "HDR10"
            | "HLG"
            | "DV"
            | "UHD"
            | "SDR"
            | "10BIT"
            | "8BIT"
            | "HI10P"
            | "STEREO"
            | "MONO"
            | "HD"
            | "SD"
    )
}

fn release_marker_regex() -> &'static Regex {
    RELEASE_MARKER_REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[^a-z0-9])(?:\d{3,4}p|4k|8k|web[\s._-]?(?:dl|rip)|blu[\s._-]?ray|bdrip|brrip|hdtv|dvd[\s._-]?rip|remux|x26[45]|h\.?26[45]|hevc|av1|hdr10\+?|hlg|uhd)(?:$|[^a-z0-9])",
        )
        .expect("release marker regex is valid")
    })
}

fn trim_release_group_delimiters(value: &str) -> &str {
    value.trim_end_matches(|character| matches!(character, ' ' | '.' | '_' | '-' | '–' | '—'))
}

fn validate_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() || pattern.trim() != pattern {
        return Err(
            "Metadata blacklist patterns must be nonempty and have no surrounding whitespace."
                .into(),
        );
    }
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err("Metadata blacklist patterns are limited to 128 UTF-8 bytes.".into());
    }
    if pattern.chars().any(char::is_control) {
        return Err("Metadata blacklist patterns cannot contain control characters.".into());
    }
    if !pattern.chars().any(|character| character != '*') {
        return Err("Metadata blacklist patterns must include text besides '*'.".into());
    }
    Ok(())
}

/// Apply the user's editable cleanup rules to a single title or other text
/// value. Filename clues and rename planning use the same matcher as output
/// metadata so searches and produced names follow the Settings list.
pub(crate) fn clean_value(value: &str, patterns: &[String]) -> Result<String, String> {
    if value.len() > MAX_TEXT_BYTES {
        return Err("Metadata text cleanup values are limited to 256 KiB.".into());
    }
    if patterns.len() > MAX_PATTERNS {
        return Err("Metadata text cleanup accepts at most 100 blacklist patterns.".into());
    }
    let mut seen = std::collections::HashSet::new();
    for pattern in patterns {
        validate_pattern(pattern)?;
        if !seen.insert(pattern.chars().map(fold_case).collect::<String>()) {
            return Err(
                "Metadata text cleanup patterns cannot contain case-insensitive duplicates.".into(),
            );
        }
    }
    Ok(clean_text(value.to_string(), patterns).text)
}

fn remove_glob_matches(value: &str, pattern: &str) -> (String, bool) {
    let original_chars: Vec<char> = value.chars().collect();
    let folded_chars: Vec<char> = original_chars.iter().copied().map(fold_case).collect();
    let pattern_chars: Vec<char> = pattern.chars().map(fold_case).collect();
    let chunks: Vec<Vec<char>> = pattern_chars
        .split(|character| *character == '*')
        .filter(|chunk| !chunk.is_empty())
        .map(<[char]>::to_vec)
        .collect();
    if chunks.is_empty() {
        return (value.to_string(), false);
    }

    let leading_star = pattern_chars.first() == Some(&'*');
    let first_occurrences = (!leading_star).then(|| occurrence_map(&folded_chars, &chunks[0]));
    let full_end_map = leading_star.then(|| sequence_end_map(&folded_chars, &chunks));
    let suffix_end_map = (chunks.len() > 1).then(|| sequence_end_map(&folded_chars, &chunks[1..]));

    let mut matches = Vec::new();
    let mut search_from = 0;
    while search_from <= folded_chars.len() {
        let span = if let Some(end_map) = full_end_map.as_ref() {
            end_map[search_from].map(|end| (search_from, end))
        } else {
            let mut found = None;
            for start in search_from..folded_chars.len() {
                if !first_occurrences
                    .as_ref()
                    .is_some_and(|occurrences| occurrences[start])
                {
                    continue;
                }
                let first_end = start + chunks[0].len();
                let end = suffix_end_map
                    .as_ref()
                    .map_or(Some(first_end), |map| map[first_end]);
                if let Some(end) = end {
                    found = Some((start, end));
                    break;
                }
            }
            found
        };
        let Some(span) = span else {
            break;
        };
        if span.1 <= span.0 {
            break;
        }
        search_from = span.1;
        matches.push(span);
    }
    if matches.is_empty() {
        return (value.to_string(), false);
    }

    let mut cleaned = original_chars;
    for (start, end) in matches.into_iter().rev() {
        remove_span(&mut cleaned, start, end);
    }
    (cleaned.into_iter().collect(), true)
}

/// For every text position, return the end of the earliest complete sequence
/// of literal chunks beginning at or after that position. Stars between chunks
/// may consume any characters; processing chunks from right to left keeps this
/// bounded to O(text length * pattern length) time and O(text length) memory.
fn sequence_end_map(text: &[char], chunks: &[Vec<char>]) -> Vec<Option<usize>> {
    let mut next_stage: Option<Vec<Option<usize>>> = None;
    for chunk in chunks.iter().rev() {
        let occurrences = occurrence_map(text, chunk);
        let mut stage = vec![None; text.len() + 1];
        for position in (0..=text.len()).rev() {
            if occurrences.get(position).copied().unwrap_or(false) {
                let end = position + chunk.len();
                stage[position] = next_stage.as_ref().map_or(Some(end), |next| next[end]);
            }
            if stage[position].is_none() && position < text.len() {
                stage[position] = stage[position + 1];
            }
        }
        next_stage = Some(stage);
    }
    next_stage.unwrap_or_else(|| vec![None; text.len() + 1])
}

fn occurrence_map(text: &[char], needle: &[char]) -> Vec<bool> {
    let mut occurrences = vec![false; text.len() + 1];
    if needle.is_empty() || needle.len() > text.len() {
        return occurrences;
    }

    let mut prefix = vec![0; needle.len()];
    let mut matched = 0;
    for index in 1..needle.len() {
        while matched > 0 && needle[index] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }

    matched = 0;
    for (index, character) in text.iter().enumerate() {
        while matched > 0 && *character != needle[matched] {
            matched = prefix[matched - 1];
        }
        if *character == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            occurrences[index + 1 - needle.len()] = true;
            matched = prefix[matched - 1];
        }
    }
    occurrences
}

fn remove_span(text: &mut Vec<char>, start: usize, end: usize) {
    let left_whitespace = text[..start]
        .iter()
        .rev()
        .take_while(|character| is_python_whitespace(**character))
        .count();
    let right_whitespace = text[end..]
        .iter()
        .take_while(|character| is_python_whitespace(**character))
        .count();

    if start == 0 {
        text.drain(..end + right_whitespace);
    } else if end == text.len() {
        text.truncate(start - left_whitespace);
    } else if left_whitespace > 0 && right_whitespace > 0 {
        text.splice(start - left_whitespace..end + right_whitespace, [' ']);
    } else {
        text.drain(start..end);
    }
}

fn is_python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{001c}'..='\u{001f}')
}

fn fold_case(character: char) -> char {
    match character {
        // Python's case-insensitive regex matching treats these as their ASCII
        // base letters, alongside ordinary Unicode lowercase pairs.
        '\u{0130}' | '\u{0131}' => 'i',
        '\u{017f}' => 's',
        '\u{212a}' => 'k',
        _ => character.to_lowercase().next().unwrap_or(character),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_case_insensitive_movie_group_globs_without_removing_surrounding_text() {
        let (cleaned, matched) = remove_glob_matches(
            "Keep [movieReleaseGroup] and Movieshowgroup titles",
            "Movie*Group",
        );
        assert_eq!(cleaned, "Keep [] and titles");
        assert!(matched);
    }

    #[test]
    fn wildcard_matches_across_newlines_and_leaves_nonmatching_text_alone() {
        assert_eq!(
            remove_glob_matches("before Movie\nReleaseGroup after", "Movie*Group"),
            ("before after".to_string(), true)
        );
        assert_eq!(
            remove_glob_matches("keep every word", "Movie*Group"),
            ("keep every word".to_string(), false)
        );
    }

    #[test]
    fn cleanup_preserves_existing_spacing_when_no_whitespace_surrounds_a_match() {
        assert_eq!(
            remove_glob_matches("BeforeMovieReleaseGroupAfter", "Movie*Group"),
            ("BeforeAfter".to_string(), true)
        );
    }

    #[test]
    fn sequence_map_uses_the_shortest_successful_wildcard_match() {
        assert_eq!(
            remove_glob_matches("a1b2b tail", "a*b"),
            ("2b tail".to_string(), true)
        );
    }

    #[test]
    fn request_limits_and_invalid_patterns_are_rejected() {
        assert!(clean_request(CleanRequest {
            texts: Vec::new(),
            patterns: vec!["***".into()],
        })
        .is_err());
        assert!(clean_request(CleanRequest {
            texts: vec!["x".repeat(MAX_TEXT_BYTES + 1)],
            patterns: Vec::new(),
        })
        .is_err());
    }

    #[test]
    fn repeated_matches_use_one_bounded_scan_per_pattern() {
        let source = "x".repeat(20_000);
        let (cleaned, did_match) = remove_glob_matches(&source, "x");
        assert_eq!(cleaned, "");
        assert!(did_match);
    }

    #[test]
    fn boundary_whitespace_matches_python_strip_behavior() {
        assert_eq!(
            remove_glob_matches("\u{001c}MovieReleaseGroup\u{001c}x", "Movie*Group"),
            (" x".to_string(), true)
        );
    }

    #[test]
    fn conservative_release_group_suffix_policy_matches_legacy_cases() {
        for (source, expected) in [
            (
                "Film.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HEVC-TEPES",
                "Film.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HEVC",
            ),
            ("Film 1080p WEB-DL x265 [YTS.MX]", "Film 1080p WEB-DL x265"),
            ("Film.1080p.WEB-DL.x265-yify", "Film.1080p.WEB-DL.x265"),
        ] {
            assert_eq!(
                scrub_release_group_suffix(source),
                (expected.to_string(), true)
            );
        }
    }

    #[test]
    fn suffix_policy_preserves_titles_and_technical_tokens() {
        for source in [
            "Spider-Man (2002)",
            "Film.1080p.WEB-DL.x265",
            "A story about WEB-DL distribution",
            "A title - Criterion Collection",
            "Film x265 - Resurrection",
            "Film.1080p.WEB-DL.x265-HEVC",
        ] {
            assert_eq!(
                scrub_release_group_suffix(source),
                (source.to_string(), false)
            );
        }
    }

    #[test]
    fn suffix_policy_preserves_trailing_whitespace_and_reports_its_rule() {
        let response = clean_request(CleanRequest {
            texts: vec!["Film.1080p.WEB-DL.x265-GROUP  \n".into()],
            patterns: Vec::new(),
        })
        .unwrap();
        assert_eq!(response.suffix_policy, "recognized-release-group-v1");
        assert_eq!(response.results[0].text, "Film.1080p.WEB-DL.x265  \n");
        assert_eq!(
            response.results[0].matched,
            vec![RELEASE_SUFFIX_RULE.to_string()]
        );
    }
}
