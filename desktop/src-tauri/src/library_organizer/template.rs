//! Template shortcodes and portable path-component rules.

use super::*;

fn allowed_shortcodes(kind: &RenameTemplateKind) -> &'static [&'static str] {
    match kind {
        RenameTemplateKind::Movie => &["title", "year", "certification", "ext"],
        RenameTemplateKind::Series => &["series_title", "first_year", "last_year", "year_range"],
        RenameTemplateKind::Season => &["season"],
        RenameTemplateKind::Episode => {
            &["series_title", "season", "episode", "episode_title", "ext"]
        }
    }
}

fn shortcode_value(
    token: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    let (name, width) = match token.split_once(':') {
        Some((name, "02")) => (name, Some(2)),
        Some((name, "03")) => (name, Some(3)),
        Some((name, "04")) => (name, Some(4)),
        Some(_) => {
            return Err(format!(
                "Unsupported format in naming shortcode {{{token}}}."
            ))
        }
        None => (token, None),
    };
    if !allowed_shortcodes(kind).contains(&name) {
        return Err(format!(
            "Unknown naming shortcode {{{token}}} for this item."
        ));
    }
    let value = values.get(name).cloned().unwrap_or_default();
    if let Some(width) = width {
        if !value.chars().all(|character| character.is_ascii_digit()) {
            return Err(format!("Shortcode {{{token}}} requires a numeric value."));
        }
        Ok(format!("{value:0>width$}"))
    } else {
        Ok(value)
    }
}

fn render_chunk(
    pattern: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<(String, bool), String> {
    let mut rendered = String::with_capacity(pattern.len());
    let mut found_nonempty = false;
    let mut rest = pattern;
    while !rest.is_empty() {
        let brace = rest.find('{');
        let close = rest.find('}');
        match (brace, close) {
            (None, None) => {
                rendered.push_str(rest);
                break;
            }
            (None, Some(_)) => {
                return Err("A naming shortcode has a closing brace but no opening brace.".into())
            }
            (Some(open), Some(close)) if close < open => {
                return Err(
                    "A naming shortcode has a closing brace before its opening brace.".into(),
                );
            }
            (Some(open), _) => {
                rendered.push_str(&rest[..open]);
                let after_open = &rest[open + 1..];
                let end = after_open
                    .find('}')
                    .ok_or("A naming shortcode is missing its closing brace.")?;
                if after_open[..end].contains('{') {
                    return Err("Naming shortcodes cannot be nested.".into());
                }
                let token = &after_open[..end];
                if token.is_empty() {
                    return Err("Empty naming shortcodes are not supported.".into());
                }
                let value = shortcode_value(token, kind, values)?;
                found_nonempty |= !value.is_empty();
                rendered.push_str(&value);
                rest = &after_open[end + 1..];
            }
        }
    }
    Ok((rendered, found_nonempty))
}

pub(super) fn render_template(
    template: &str,
    kind: &RenameTemplateKind,
    values: &BTreeMap<String, String>,
) -> Result<String, String> {
    if template.trim().is_empty() || template.len() > MAX_TEMPLATE_BYTES {
        return Err("Naming templates must contain 1 to 1,024 UTF-8 bytes.".into());
    }
    let mut result = String::new();
    let mut rest = template;
    while !rest.is_empty() {
        let open = rest.find("[[");
        let close = rest.find("]]");
        match (open, close) {
            (None, None) => {
                let (text, _) = render_chunk(rest, kind, values)?;
                result.push_str(&text);
                break;
            }
            (None, Some(_)) => {
                return Err("An optional naming block is missing its opening [[ marker.".into())
            }
            (Some(_), Some(close)) if close < open.unwrap_or(usize::MAX) => {
                return Err("An optional naming block closes before it opens.".into());
            }
            (Some(open), _) => {
                let (prefix, _) = render_chunk(&rest[..open], kind, values)?;
                result.push_str(&prefix);
                let body = &rest[open + 2..];
                let end = body
                    .find("]]")
                    .ok_or("An optional naming block is missing its closing ]] marker.")?;
                if body[..end].contains("[[") {
                    return Err("Optional naming blocks cannot be nested.".into());
                }
                let (optional, has_value) = render_chunk(&body[..end], kind, values)?;
                if has_value {
                    result.push_str(&optional);
                }
                rest = &body[end + 2..];
            }
        }
    }

    let cleaned = result
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace("[ ", "[")
        .replace(" ]", "]");
    let mut cleaned = cleaned.trim().to_string();
    for (left, right) in [('(', ')'), ('[', ']')] {
        let empty = format!("{left}{right}");
        cleaned = cleaned.replace(&empty, "");
    }
    cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return Err("The naming template produced an empty name.".into());
    }
    Ok(cleaned)
}

pub(super) fn portable_component_error(name: &str, is_directory: bool) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." {
        return Some("The new name cannot be empty, '.' or '..'.".into());
    }
    if name.len() > MAX_FILENAME_BYTES {
        return Some(format!(
            "The new name exceeds the {MAX_FILENAME_BYTES}-byte portable component limit."
        ));
    }
    if matches!(name.chars().last(), Some(' ' | '.')) {
        return Some("Names cannot end with a space or period on supported filesystems.".into());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Some("The new name contains a path separator, control character, or character unsupported by Windows/macOS.".into());
    }
    let stem = if is_directory {
        name
    } else {
        name.rsplit_once('.').map_or(name, |(stem, _)| stem)
    };
    let device = stem
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or(stem)
        .to_ascii_uppercase();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
    {
        return Some("The new name is reserved by Windows.".into());
    }
    None
}
