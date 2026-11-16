//! Bit depth, chroma, color-description, mastering-display and Dolby Vision helpers for profiles.

use super::*;

pub(super) fn frame_side_data(frame: Option<&Value>) -> Vec<Value> {
    let mut side_data = Vec::new();
    if let Some(frames) = frame
        .and_then(|value| value.get("frames"))
        .and_then(Value::as_array)
    {
        for entry in frames {
            if let Some(items) = entry.get("side_data_list").and_then(Value::as_array) {
                side_data.extend(items.iter().cloned());
            }
        }
    }
    side_data
}

pub(super) fn frames_are_interlaced(frame: Option<&Value>) -> bool {
    frame
        .and_then(|value| value.get("frames"))
        .and_then(Value::as_array)
        .is_some_and(|frames| {
            frames
                .iter()
                .any(|entry| entry.get("interlaced_frame").and_then(value_i64_ref) == Some(1))
        })
}

pub(super) fn source_depth(stream: &Value) -> i64 {
    stream
        .get("bits_per_raw_sample")
        .and_then(parse_numeric)
        .or_else(|| value_i64(stream.get("bits_per_sample")))
        .filter(|value| matches!(value, 8 | 10 | 12 | 14 | 16))
        .or_else(|| {
            let pixel = stream
                .get("pix_fmt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            [16, 14, 12, 10, 8]
                .into_iter()
                .find(|depth| pixel.contains(&depth.to_string()))
        })
        .unwrap_or(8)
}

pub(super) fn chroma_from_pixel_format(pixel: &str) -> String {
    if pixel.contains("444") {
        "4:4:4".into()
    } else if pixel.contains("422") {
        "4:2:2".into()
    } else {
        "4:2:0".into()
    }
}

pub(super) fn target_pixel_format(
    depth: i64,
    chroma: &str,
    promote_8bit: bool,
) -> Result<String, String> {
    if ![8, 10, 12].contains(&depth) {
        return Err(format!(
            "Source bit depth {depth} cannot be reproduced exactly by this x265 workflow."
        ));
    }
    let depth = if depth == 8 && promote_8bit {
        10
    } else {
        depth
    };
    let chroma = if chroma.contains("4:4:4") {
        "444"
    } else if chroma.contains("4:2:2") {
        "422"
    } else {
        "420"
    };
    Ok(if depth == 8 {
        format!("yuv{chroma}p")
    } else {
        format!("yuv{chroma}p{depth}le")
    })
}

pub(super) fn disposition(stream: &Value, name: &str) -> bool {
    stream
        .pointer(&format!("/disposition/{name}"))
        .and_then(value_i64_ref)
        .unwrap_or(0)
        != 0
}

pub(super) fn first_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| {
        !text.trim().is_empty()
            && !text.eq_ignore_ascii_case("unknown")
            && !text.eq_ignore_ascii_case("unspecified")
    })
}

pub(super) fn normalize_primaries(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    Some(
        match value.as_str() {
            "bt.709" | "bt709" => "bt709",
            "bt.2020" | "bt2020" => "bt2020",
            "bt.601 ntsc" | "smpte170m" => "smpte170m",
            "bt.601 pal" | "bt470bg" => "bt470bg",
            "bt.470 system m" | "bt470m" => "bt470m",
            "generic film" | "film" => "film",
            "display p3" => "smpte432",
            "dci p3" => "smpte431",
            _ => return Some(value),
        }
        .into(),
    )
}

pub(super) fn normalize_transfer(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    Some(
        match value.as_str() {
            "pq" | "smpte2084" | "smpte st 2084" => "smpte2084",
            "hlg" | "arib-std-b67" => "arib-std-b67",
            "bt.709" | "bt709" => "bt709",
            "bt.470 system b/g" | "bt470bg" => "bt470bg",
            "bt.470 system m" | "bt470m" => "bt470m",
            "srgb" => "iec61966-2-1",
            _ => return Some(value),
        }
        .into(),
    )
}

pub(super) fn normalize_matrix(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() || matches!(value.as_str(), "unknown" | "unspecified") {
        return None;
    }
    if value.contains("bt.2020") && value.contains("non-constant") {
        return Some("bt2020nc".into());
    }
    if value.contains("bt.2020") && value.contains("constant") {
        return Some("bt2020c".into());
    }
    Some(
        match value.as_str() {
            "bt2020nc" | "bt2020c" | "bt.709" | "bt709" | "fcc" | "ycgco" | "gbr" => &value,
            "bt.601" | "smpte170m" => "smpte170m",
            "bt.470 system b/g" | "bt470bg" => "bt470bg",
            "rgb" => "gbr",
            _ => return Some(value),
        }
        .into(),
    )
}

pub(super) fn side_kind(value: &Value) -> String {
    value
        .get("side_data_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

pub(super) fn mastering_display_from_side(side: &Value) -> Option<String> {
    let names = [
        "red_x",
        "red_y",
        "green_x",
        "green_y",
        "blue_x",
        "blue_y",
        "white_point_x",
        "white_point_y",
        "min_luminance",
        "max_luminance",
    ];
    let mut values = BTreeMap::new();
    for name in names {
        values.insert(name, value_f64(side.get(name))?);
    }
    let xy = |x: &str, y: &str| {
        format!(
            "{},{}",
            (values[x] * 50000.0).round_ties_even() as i64,
            (values[y] * 50000.0).round_ties_even() as i64
        )
    };
    Some(format!(
        "G({})B({})R({})WP({})L({},{})",
        xy("green_x", "green_y"),
        xy("blue_x", "blue_y"),
        xy("red_x", "red_y"),
        xy("white_point_x", "white_point_y"),
        (values["max_luminance"] * 10000.0).round_ties_even() as i64,
        (values["min_luminance"] * 10000.0).round_ties_even() as i64
    ))
}

pub(super) fn mastering_display_from_mediainfo(video: &Value) -> Option<String> {
    let primaries = video
        .get("MasteringDisplay_ColorPrimaries")
        .and_then(Value::as_str)?
        .trim();
    if primaries.is_empty() {
        return None;
    }
    let luminance = video
        .get("MasteringDisplay_Luminance")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let min_luminance = decimal_after_label(luminance, "min:").unwrap_or(0.005);
    let max_luminance = decimal_after_label(luminance, "max:").unwrap_or(1000.0);
    let light = format!(
        "L({},{})",
        (max_luminance * 10000.0).round_ties_even() as i64,
        (min_luminance * 10000.0).round_ties_even() as i64
    );
    let normalized = primaries.to_ascii_lowercase();
    if normalized.contains("bt.2020") || normalized.contains("bt2020") {
        Some(format!(
            "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450){light}"
        ))
    } else if normalized.contains("display p3") {
        Some(format!(
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450){light}"
        ))
    } else if normalized.contains("bt.709") || normalized.contains("bt709") {
        Some(format!(
            "G(15000,30000)B(7500,3000)R(32000,16500)WP(15635,16450){light}"
        ))
    } else {
        None
    }
}

fn decimal_after_label(value: &str, label: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    let offset = lower.find(label)? + label.len();
    parse_first_decimal(&value[offset..])
}

fn parse_first_decimal(value: &str) -> Option<f64> {
    let start = value
        .char_indices()
        .find(|(_, character)| character.is_ascii_digit())?
        .0;
    let tail = &value[start..];
    let whole_digits = tail.chars().take_while(char::is_ascii_digit).count();
    let mut length = whole_digits;
    let after_whole = &tail[whole_digits..];
    if after_whole.starts_with('.')
        && after_whole
            .chars()
            .nth(1)
            .is_some_and(|character| character.is_ascii_digit())
    {
        length += 1 + after_whole[1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
    }
    tail[..length].parse().ok()
}

pub(super) fn rounded_integer(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(parse_first_decimal))?;
    Some(number.round_ties_even() as i64)
}

pub(super) fn chroma_location(
    ffprobe_value: Option<&str>,
    mediainfo_value: Option<&str>,
) -> Option<i64> {
    if let Some(value) = ffprobe_value {
        let normalized = value.to_ascii_lowercase();
        let location = match normalized.as_str() {
            "left" => Some(0),
            "center" => Some(1),
            "topleft" | "top-left" => Some(2),
            "top" => Some(3),
            "bottomleft" | "bottom-left" => Some(4),
            "bottom" => Some(5),
            _ => None,
        };
        if location.is_some() {
            return location;
        }
    }
    let value = mediainfo_value?;
    for (index, character) in value.char_indices() {
        if !matches!(character, '0'..='5') {
            continue;
        }
        let end = index + character.len_utf8();
        let word = |character: char| character.is_ascii_alphanumeric() || character == '_';
        let has_word_before = value[..index].chars().next_back().is_some_and(word);
        let has_word_after = value[end..].chars().next().is_some_and(word);
        if !has_word_before && !has_word_after {
            return character.to_digit(10).map(i64::from);
        }
    }
    None
}

pub(super) fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_lowercase();
    needles
        .iter()
        .any(|needle| lower.contains(&needle.to_lowercase()))
}

pub(super) fn dovi_profile_from_text(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    for marker in ["dvhe.", "dvh1."] {
        if let Some(index) = lower.find(marker) {
            let digits = lower[index + marker.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            return digits.trim_start_matches('0').parse().ok();
        }
    }
    lower
        .split("profile")
        .nth(1)?
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

pub(super) fn dovi_level_from_text(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    for marker in ["dvhe.", "dvh1."] {
        if let Some(index) = lower.find(marker) {
            let tail = &lower[index + marker.len()..];
            let profile_len = tail.chars().take_while(char::is_ascii_digit).count();
            let rest = tail[profile_len..].trim_start_matches('.');
            let digits = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            return digits.trim_start_matches('0').parse().ok();
        }
    }
    None
}
