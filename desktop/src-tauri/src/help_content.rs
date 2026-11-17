//! The plain-language help shared with the GUI. `desktop/src/lib/help-content.json` is the single source:
//! the frontend (`explanations.ts`) renders it and the MCP tools `explain_settings` and `app_guide` return it,
//! so the app and a model are always told the same thing.

use serde_json::{json, Value};
use std::sync::OnceLock;

const RAW: &str = include_str!("../../src/lib/help-content.json");

/// The explanation topics `explain_settings` accepts, besides any individual setting name.
pub(crate) const TOPICS: &[&str] = &["all", "crf", "tune", "preset", "settings", "short_test"];

fn content() -> &'static Value {
    static CONTENT: OnceLock<Value> = OnceLock::new();
    CONTENT.get_or_init(|| serde_json::from_str(RAW).expect("help-content.json is valid JSON"))
}

fn bands() -> &'static Vec<Value> {
    content()["crf"]["bands"].as_array().expect("CRF bands")
}

/// The band a CRF number falls in (clamped to 0..=51): label, plain summary and whether it is recommended.
pub(crate) fn crf_band(crf: i64) -> Value {
    let value = crf.clamp(0, 51);
    let band = bands()
        .iter()
        .find(|band| band["max"].as_i64().is_some_and(|max| value <= max))
        .or_else(|| bands().last())
        .expect("at least one band");
    json!({"label": band["label"], "summary": band["summary"], "recommended": band["recommended"]})
}

/// Rough file size relative to the default CRF: every `stepsToHalveSize` steps halves (or doubles) it.
pub(crate) fn crf_size_ratio(crf: i64) -> f64 {
    let default = content()["crf"]["default"].as_f64().unwrap_or(21.0);
    let steps = content()["crf"]["stepsToHalveSize"].as_f64().unwrap_or(6.0);
    2f64.powf((default - crf.clamp(0, 51) as f64) / steps)
}

fn setting_names() -> Vec<String> {
    content()["settings"]
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

/// Explain a topic. `topic` is one of [`TOPICS`] or a setting name (for example `hdr10plus`); `crf` adds the
/// band and rough size for that number to the `crf` topic.
pub(crate) fn explain(topic: Option<&str>, crf: Option<i64>) -> Result<Value, String> {
    let c = content();
    let crf_topic = || {
        let mut value = json!({
            "default": c["crf"]["default"],
            "explanation": c["crf"]["explanation"],
            "bands": c["crf"]["bands"],
            "size_rule": format!("Every {} steps of CRF roughly halves (higher) or doubles (lower) the file size. A rough guide only; real results depend on the video.", c["crf"]["stepsToHalveSize"]),
        });
        if let Some(number) = crf {
            value["for_value"] = json!({
                "crf": number.clamp(0, 51),
                "band": crf_band(number),
                "size_vs_default": (crf_size_ratio(number) * 100.0).round() / 100.0,
            });
        }
        value
    };
    match topic
        .map(str::trim)
        .filter(|topic| !topic.is_empty())
        .unwrap_or("all")
    {
        "all" => Ok(json!({
            "crf": crf_topic(),
            "tunes": c["tunes"],
            "presets": c["presets"],
            "settings": c["settings"],
            "short_test": c["shortTest"],
        })),
        "crf" => Ok(crf_topic()),
        "tune" => Ok(c["tunes"].clone()),
        "preset" => Ok(c["presets"].clone()),
        "settings" => Ok(c["settings"].clone()),
        "short_test" => Ok(json!({"text": c["shortTest"]})),
        name => c["settings"].get(name).cloned().ok_or_else(|| {
            format!(
                "Unknown topic `{name}`. Topics: {}. Settings: {}.",
                TOPICS.join(", "),
                setting_names().join(", ")
            )
        }),
    }
}

/// The application guide. Without a section it lists them; with one it returns its items.
pub(crate) fn guide(section: Option<&str>) -> Result<Value, String> {
    let sections = content()["guide"].as_object().expect("guide sections");
    match section.map(str::trim).filter(|section| !section.is_empty()) {
        None => Ok(json!({
            "sections": sections.iter().map(|(key, value)| json!({"section": key, "title": value["title"], "summary": value["summary"]})).collect::<Vec<_>>(),
            "next": "Call app_guide again with one of these section names.",
        })),
        Some(name) => sections.get(name).cloned().ok_or_else(|| {
            format!(
                "Unknown section `{name}`. Sections: {}.",
                sections.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_file_parses_and_has_every_part_the_tools_need() {
        let c = content();
        for key in ["crf", "tunes", "presets", "settings", "shortTest", "guide"] {
            assert!(!c[key].is_null(), "{key}");
        }
        assert_eq!(c["crf"]["default"], 21);
        assert!(setting_names().len() >= 14);
        let sections: Vec<_> = c["guide"].as_object().unwrap().keys().cloned().collect();
        for expected in ["workflow", "modes", "safety", "headless", "mcp"] {
            assert!(sections.contains(&expected.to_string()), "{expected}");
        }
    }

    #[test]
    fn every_crf_has_one_band_and_only_the_default_band_is_recommended() {
        for crf in -5..=60 {
            let band = crf_band(crf);
            assert!(
                band["label"]
                    .as_str()
                    .is_some_and(|label| !label.is_empty()),
                "{crf}"
            );
        }
        assert_eq!(crf_band(21)["recommended"], true);
        assert_eq!(crf_band(40)["recommended"], false);
        assert_eq!(crf_band(40)["label"], "Low quality");
        assert_eq!(
            crf_band(-3)["label"],
            crf_band(0)["label"],
            "out-of-range values clamp"
        );
        let recommended = bands()
            .iter()
            .filter(|band| band["recommended"] == true)
            .count();
        assert_eq!(recommended, 1);
    }

    #[test]
    fn the_size_rule_matches_the_gui_and_falls_as_crf_rises() {
        assert_eq!(crf_size_ratio(21), 1.0);
        assert!((crf_size_ratio(27) - 0.5).abs() < 0.001);
        assert!((crf_size_ratio(15) - 2.0).abs() < 0.001);
        assert!((1..=51).all(|crf| crf_size_ratio(crf) < crf_size_ratio(crf - 1)));
    }

    #[test]
    fn explaining_crf_with_a_number_adds_its_band_and_rough_size() {
        let value = explain(Some("crf"), Some(27)).unwrap();
        assert_eq!(value["for_value"]["crf"], 27);
        assert_eq!(value["for_value"]["band"]["label"], "Space saver");
        assert_eq!(value["for_value"]["size_vs_default"], 0.5);
        assert!(value["size_rule"].as_str().unwrap().contains("rough guide"));
        assert!(explain(Some("crf"), None)
            .unwrap()
            .get("for_value")
            .is_none());
        assert_eq!(
            explain(Some("crf"), Some(99)).unwrap()["for_value"]["crf"],
            51,
            "out-of-range numbers clamp"
        );
    }

    #[test]
    fn topics_settings_and_the_overview_all_answer() {
        assert!(
            explain(None, None).unwrap()["settings"]["hdr10plus"]["text"]
                .as_str()
                .unwrap()
                .contains("brightness")
        );
        assert_eq!(
            explain(Some("all"), None).unwrap()["short_test"],
            content()["shortTest"]
        );
        assert!(explain(Some("tune"), None).unwrap()["grain"]["detail"]
            .as_str()
            .unwrap()
            .contains("grain"));
        assert!(explain(Some("preset"), None).unwrap()["slow"]
            .as_str()
            .unwrap()
            .contains("Recommended"));
        assert_eq!(
            explain(Some("dolbyVision"), None).unwrap()["title"],
            "Dolby Vision policy"
        );
        assert!(explain(Some("short_test"), None).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("5-second"));
    }

    #[test]
    fn an_unknown_topic_lists_what_is_valid_so_a_model_can_recover() {
        let error = explain(Some("sharpness"), None).unwrap_err();
        assert!(error.contains("Unknown topic `sharpness`"), "{error}");
        for needle in ["crf", "preset", "hdr10plus", "minSsim"] {
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn the_guide_lists_its_sections_then_returns_one() {
        let index = guide(None).unwrap();
        let names: Vec<_> = index["sections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["section"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names.len(), 5);
        assert!(index["sections"][0]["title"]
            .as_str()
            .is_some_and(|title| !title.is_empty()));
        let headless = guide(Some("headless")).unwrap();
        let text = headless["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["text"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        for needle in ["--read-only", "--root", "/mcp", "Bearer", "reverse proxy"] {
            assert!(text.contains(needle), "{needle}");
        }
        let error = guide(Some("nope")).unwrap_err();
        assert!(
            error.contains("workflow") && error.contains("safety"),
            "{error}"
        );
    }
}
