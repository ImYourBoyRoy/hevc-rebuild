//! Small JSON value helpers used across the validation checks.

use super::*;

pub(super) fn flag_value(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_i64() == Some(1)
            || value.as_u64() == Some(1)
            || value.as_str().is_some_and(|text| text == "1")
    })
}

pub(super) fn lower_key_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.to_ascii_lowercase(), value_text(value)))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn float_value(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if text != "N/A" => text.parse().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

pub(super) fn source_value_is_present(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(value)) => !value.is_empty() && value != "0",
        Some(Value::Number(value)) => value.as_f64().is_none_or(|number| number != 0.0),
        _ => true,
    }
}

pub(super) fn value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}
