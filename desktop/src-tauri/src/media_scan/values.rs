//! Lenient JSON number parsing used when reading tool output.

use super::*;

pub(super) fn value_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

pub(super) fn value_i64_ref(value: &Value) -> Option<i64> {
    value_i64(Some(value))
}

pub(super) fn value_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

pub(super) fn value_f64_ref(value: &Value) -> Option<f64> {
    value_f64(Some(value))
}

pub(super) fn parse_numeric(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(parse_first_integer))
}

pub(super) fn parse_numeric_string(value: &Value) -> Option<i64> {
    parse_numeric(value)
}

fn parse_first_integer(value: &str) -> Option<i64> {
    value
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}
