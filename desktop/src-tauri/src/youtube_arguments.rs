//! Engine arguments for the YouTube mode and tuning choices shared by the GUI and MCP.

/// Saved-preference keys for the YouTube mode and its tuning choices, with the
/// accepted values. The first value of each list is the default.
pub(crate) const YOUTUBE_CHOICE_PREFERENCES: &[(&str, &str, &[&str])] = &[
    ("youtubeMode", "YouTube mode", &["upscale", "optimize"]),
    ("youtubeSdrCodec", "YouTube SDR codec", &["h264", "hevc"]),
    (
        "youtubePreset",
        "YouTube encoder preset",
        &["slow", "medium", "slower"],
    ),
    (
        "youtubeShrink",
        "YouTube shrink mode",
        &["auto", "off", "aggressive"],
    ),
    (
        "youtubeQualityCheck",
        "YouTube quality check",
        &["standard", "off", "relaxed", "strict"],
    ),
];

/// Engine arguments for the YouTube mode and tuning choices shared by the GUI and MCP.
/// `None` keeps the engine default.
pub(crate) fn youtube_mode_arguments(
    mode: Option<&str>,
    sdr_codec: Option<&str>,
    preset: Option<&str>,
    shrink: Option<&str>,
    quality_check: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for ((_, label, allowed), (flag, value)) in YOUTUBE_CHOICE_PREFERENCES.iter().zip([
        ("--youtube-mode", mode),
        ("--youtube-sdr-codec", sdr_codec),
        ("--youtube-preset", preset),
        ("--youtube-shrink", shrink),
        ("--youtube-quality-check", quality_check),
    ]) {
        if let Some(value) = value {
            if !allowed.contains(&value) {
                return Err(format!("{label} must be one of: {}.", allowed.join(", ")));
            }
            args.extend([flag.to_string(), value.to_string()]);
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_mode_arguments_forward_only_valid_choices_in_flag_order() {
        assert!(youtube_mode_arguments(None, None, None, None, None)
            .unwrap()
            .is_empty());
        assert_eq!(
            youtube_mode_arguments(Some("optimize"), None, Some("slow"), None, Some("strict"))
                .unwrap(),
            vec![
                "--youtube-mode",
                "optimize",
                "--youtube-preset",
                "slow",
                "--youtube-quality-check",
                "strict"
            ]
        );
        let error = youtube_mode_arguments(None, Some("av1"), None, None, None).unwrap_err();
        assert_eq!(error, "YouTube SDR codec must be one of: h264, hevc.");
        for (name, _, allowed) in YOUTUBE_CHOICE_PREFERENCES {
            assert!(!allowed.is_empty(), "{name} has a default choice");
        }
    }
}
