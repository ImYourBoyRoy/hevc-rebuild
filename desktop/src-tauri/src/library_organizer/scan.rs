//! Names-only folder scan: season folders, episode ids and episode-number gaps.

use super::*;

fn parse_number(chars: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut end = start;
    while end < chars.len() && chars[end].is_ascii_digit() && end - start < 4 {
        end += 1;
    }
    if end == start || (end < chars.len() && chars[end].is_ascii_digit()) {
        return None;
    }
    let number = std::str::from_utf8(&chars[start..end]).ok()?.parse().ok()?;
    Some((number, end))
}

pub(super) fn parse_episode_id(stem: &str) -> (Option<u32>, Option<u32>, bool) {
    let upper = stem.to_ascii_uppercase();
    let upper = upper.as_bytes();
    for index in 0..upper.len() {
        if upper[index] != b'S' || (index > 0 && upper[index - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let Some((season, after_season)) = parse_number(upper, index + 1) else {
            continue;
        };
        let mut episode_marker = after_season;
        while episode_marker < upper.len()
            && matches!(upper[episode_marker], b' ' | b'.' | b'_' | b'-')
        {
            episode_marker += 1;
        }
        if episode_marker >= upper.len() || upper[episode_marker] != b'E' {
            continue;
        }
        let Some((episode, after_episode)) = parse_number(upper, episode_marker + 1) else {
            continue;
        };
        let mut next = after_episode;
        while next < upper.len() && matches!(upper[next], b' ' | b'.' | b'_' | b'-') {
            next += 1;
        }
        let multiple =
            next < upper.len() && upper[next] == b'E' && parse_number(upper, next + 1).is_some();
        return (Some(season), Some(episode), multiple);
    }
    (None, None, false)
}

pub(super) fn episode_number_gaps(files: &[OrganizerMediaFile]) -> Vec<OrganizerEpisodeGap> {
    const MAX_EXAMPLES_PER_SEASON: usize = 8;
    let mut by_season: BTreeMap<u32, (BTreeSet<u32>, bool)> = BTreeMap::new();
    for file in files {
        let (Some(season), Some(episode)) = (file.season, file.episode) else {
            continue;
        };
        let (episodes, has_multi_episode_file) = by_season.entry(season).or_default();
        *has_multi_episode_file |= file.multiple_episodes;
        episodes.insert(episode);
    }

    by_season
        .into_iter()
        .filter_map(|(season, (episodes, has_multi_episode_file))| {
            // Multi-episode files encode more than one number in one filename;
            // avoid presenting an incomplete gap list as authoritative.
            if has_multi_episode_file || episodes.len() < 2 {
                return None;
            }
            let mut missing_count = 0u32;
            let mut examples = Vec::new();
            let first = *episodes.first()?;
            let last = *episodes.last()?;
            for episode in first.saturating_add(1)..last {
                if !episodes.contains(&episode) {
                    missing_count = missing_count.saturating_add(1);
                    if examples.len() < MAX_EXAMPLES_PER_SEASON {
                        examples.push(episode);
                    }
                }
            }
            (missing_count > 0).then_some(OrganizerEpisodeGap {
                season,
                missing_count,
                examples,
            })
        })
        .collect()
}

pub(super) fn is_video_file(path: &Path) -> bool {
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

pub(super) fn season_number(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if lower == "specials" {
        return Some(0);
    }
    let rest = lower
        .strip_prefix("season")?
        .trim_start_matches([' ', '_', '-']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

pub(crate) fn scan_organizer_folder(root: String) -> Result<OrganizerFolderScan, String> {
    let root = Path::new(&root)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected media folder: {error}"))?;
    if !root.is_dir() {
        return Err("Choose a directory to scan for movies or episodes.".into());
    }
    let mut files = Vec::new();
    let mut seasons = Vec::new();
    let mut errors = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    let mut truncated = false;
    while let Some((directory, depth)) = stack.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                errors.push(format!("{}: {error}", directory.display()));
                if errors.len() >= 200 {
                    break;
                }
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("{}: {error}", directory.display()));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    errors.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.')
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "old" | "output" | "converted" | "youtube uploads"
                    )
                {
                    continue;
                }
                if depth == 0 {
                    if let Some(number) = season_number(&name) {
                        seasons.push(OrganizerSeasonDirectory {
                            path: path.to_string_lossy().into_owned(),
                            name,
                            season: number,
                        });
                    }
                }
                if depth < MAX_WALK_DEPTH {
                    children.push((path, depth + 1));
                }
                continue;
            }
            if metadata.is_file() && is_video_file(&path) {
                if files.len() >= MAX_SCAN_FILES {
                    truncated = true;
                    break;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&name);
                let (season, episode, multiple_episodes) = parse_episode_id(stem);
                files.push(OrganizerMediaFile {
                    path: path.to_string_lossy().into_owned(),
                    name,
                    parent_path: directory.to_string_lossy().into_owned(),
                    extension: path
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or("")
                        .to_string(),
                    season,
                    episode,
                    multiple_episodes,
                });
            }
        }
        children.sort_by(|left, right| left.0.cmp(&right.0));
        stack.extend(children.into_iter().rev());
        if truncated {
            break;
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    seasons.sort_by_key(|season| (season.season, season.path.clone()));
    let episode_gaps = episode_number_gaps(&files);
    Ok(OrganizerFolderScan {
        root: root.to_string_lossy().into_owned(),
        rename_root: root
            .parent()
            .unwrap_or(&root)
            .to_string_lossy()
            .into_owned(),
        files,
        season_directories: seasons,
        episode_gaps,
        errors,
        truncated,
    })
}
