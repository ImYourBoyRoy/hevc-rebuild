//! Validated settings-to-engine CLI contract shared by GUI and MCP.
//! Pure translation: no filesystem, preferences, or job lifecycle state.

use serde_json::Value;

fn config_value<'a>(settings: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut value = settings;
    for key in path.split('.') {
        value = value
            .get(key)
            .ok_or_else(|| format!("Setting {path} is missing."))?;
    }
    Ok(value)
}

fn config_string(settings: &Value, path: &str, allowed: &[&str]) -> Result<String, String> {
    let value = config_value(settings, path)?
        .as_str()
        .ok_or_else(|| format!("Setting {path} must be text."))?;
    if !allowed.contains(&value) {
        return Err(format!("Setting {path} has an unsupported value."));
    }
    Ok(value.to_string())
}

fn config_bool(settings: &Value, path: &str) -> Result<bool, String> {
    config_value(settings, path)?
        .as_bool()
        .ok_or_else(|| format!("Setting {path} must be true or false."))
}

fn config_integer(settings: &Value, path: &str, minimum: i64, maximum: i64) -> Result<i64, String> {
    let value = config_value(settings, path)?
        .as_i64()
        .ok_or_else(|| format!("Setting {path} must be a whole number."))?;
    if value < minimum || value > maximum {
        return Err(format!(
            "Setting {path} must be between {minimum} and {maximum}."
        ));
    }
    Ok(value)
}

fn config_number(settings: &Value, path: &str, minimum: f64, maximum: f64) -> Result<f64, String> {
    let value = config_value(settings, path)?
        .as_f64()
        .ok_or_else(|| format!("Setting {path} must be a number."))?;
    if !value.is_finite() || value < minimum || value > maximum {
        return Err(format!(
            "Setting {path} must be between {minimum} and {maximum}."
        ));
    }
    Ok(value)
}

fn add_boolean_option(args: &mut Vec<String>, name: &str, value: bool) {
    args.push(format!("--{}{}", if value { "" } else { "no-" }, name));
}

pub(crate) fn metadata_text_blacklist_arguments(settings: &Value) -> Result<Vec<String>, String> {
    let embed_artwork = settings
        .get("metadata")
        .and_then(|metadata| metadata.get("embed_artwork"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut args = vec![format!(
        "--{}embed-artwork",
        if embed_artwork { "" } else { "no-" }
    )];
    let Some(configured) = settings
        .get("metadata")
        .and_then(|metadata| metadata.get("text_blacklist"))
    else {
        return Ok(args);
    };
    let patterns = configured
        .as_array()
        .ok_or("Metadata text blacklist must be an array of patterns.")?;
    if patterns.len() > 100 {
        return Err("Metadata text blacklist is limited to 100 patterns.".into());
    }
    args.push("--replace-metadata-text-blacklist".into());
    let mut seen = std::collections::HashSet::new();
    for pattern in patterns {
        let value = pattern
            .as_str()
            .ok_or("Metadata text blacklist patterns must be text.")?;
        if value.is_empty() || value.trim() != value {
            return Err(
                "Metadata text blacklist patterns cannot be empty or padded with spaces.".into(),
            );
        }
        if value.len() > 128 {
            return Err("Metadata text blacklist patterns are limited to 128 UTF-8 bytes.".into());
        }
        if value.chars().all(|character| character == '*') {
            return Err("Metadata text blacklist patterns must include text besides '*'.".into());
        }
        if value.chars().any(char::is_control) {
            return Err(
                "Metadata text blacklist patterns cannot contain control characters.".into(),
            );
        }
        if !seen.insert(value.to_lowercase()) {
            return Err("Metadata text blacklist cannot contain duplicate patterns.".into());
        }
        args.extend(["--metadata-text-blacklist".into(), value.to_string()]);
    }
    Ok(args)
}

pub(crate) fn settings_arguments(settings: &Value) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    args.extend([
        "--crf".into(),
        config_integer(settings, "video.crf", 0, 51)?.to_string(),
    ]);
    args.extend([
        "--preset".into(),
        config_string(
            settings,
            "video.preset",
            &[
                "ultrafast",
                "superfast",
                "veryfast",
                "faster",
                "fast",
                "medium",
                "slow",
                "slower",
                "veryslow",
            ],
        )?,
    ]);
    add_boolean_option(
        &mut args,
        "promote-8bit-to-10bit",
        config_bool(settings, "video.promote_8bit_to_10bit")?,
    );
    args.extend([
        "--tune".into(),
        config_string(
            settings,
            "content.tune",
            &["auto", "animation", "grain", "none"],
        )?,
    ]);
    add_boolean_option(
        &mut args,
        "detect-animation",
        config_bool(settings, "content.detect_animation")?,
    );
    add_boolean_option(
        &mut args,
        "preserve-hdr10plus",
        config_bool(settings, "hdr.preserve_hdr10plus")?,
    );
    args.extend([
        "--dolby-vision-policy".into(),
        config_string(
            settings,
            "hdr.dolby_vision",
            &["safe", "skip", "hdr10-fallback"],
        )?,
    ]);
    args.extend([
        "--secondary-video".into(),
        config_string(
            settings,
            "streams.secondary_video",
            &["encode", "copy", "drop"],
        )?,
    ]);
    args.extend([
        "--data-streams".into(),
        config_string(settings, "streams.data_streams", &["preserve", "drop"])?,
    ]);
    add_boolean_option(
        &mut args,
        "verify-copy-hashes",
        config_bool(settings, "safety.verify_copy_hashes")?,
    );

    let comment = config_value(settings, "metadata.comment")?
        .as_str()
        .ok_or("Metadata comment must be text.")?;
    if comment.len() > 500 {
        return Err("Metadata comment is limited to 500 UTF-8 bytes.".into());
    }
    args.extend(["--comment".into(), comment.to_string()]);
    args.extend(metadata_text_blacklist_arguments(settings)?);

    args.extend([
        "--resource-threading".into(),
        config_string(settings, "resources.threading", &["auto", "unlimited"])?,
    ]);
    args.extend([
        "--reserved-cpu-threads".into(),
        config_integer(settings, "resources.reserved_cpu_threads", 0, 64)?.to_string(),
    ]);
    args.extend([
        "--max-worker-threads".into(),
        config_integer(settings, "resources.max_worker_threads", 0, 64)?.to_string(),
    ]);
    args.extend([
        "--frame-threads".into(),
        config_integer(settings, "resources.frame_threads", 0, 16)?.to_string(),
    ]);
    args.extend([
        "--min-free-memory-gib".into(),
        config_number(settings, "resources.min_free_memory_gib", 0.0, 1_000_000.0)?.to_string(),
    ]);
    args.extend([
        "--memory-reserve-percent".into(),
        config_number(settings, "resources.memory_reserve_percent", 0.0, 50.0)?.to_string(),
    ]);
    args.extend([
        "--min-free-disk-gib".into(),
        config_number(settings, "resources.min_free_disk_gib", 0.0, 1_000_000.0)?.to_string(),
    ]);
    args.extend([
        "--preflight-output-ratio".into(),
        config_number(settings, "resources.preflight_output_ratio", 0.25, 4.0)?.to_string(),
    ]);
    args.extend([
        "--disk-projection-margin-percent".into(),
        config_number(
            settings,
            "resources.disk_projection_margin_percent",
            0.0,
            100.0,
        )?
        .to_string(),
    ]);
    args.extend([
        "--calibration-mode".into(),
        config_string(
            settings,
            "calibration.mode",
            &["auto", "ask", "always", "off"],
        )?,
    ]);
    args.extend([
        "--calibration-cache-hours".into(),
        config_number(settings, "calibration.cache_hours", 0.0, 720.0)?.to_string(),
    ]);
    args.extend([
        "--calibration-large-file-gib".into(),
        config_number(settings, "calibration.large_file_gib", 0.0, 1000.0)?.to_string(),
    ]);
    args.extend([
        "--calibration-sample-seconds".into(),
        config_number(settings, "calibration.sample_seconds", 1.0, 15.0)?.to_string(),
    ]);
    args.extend([
        "--calibration-max-candidates".into(),
        config_integer(settings, "calibration.max_candidates", 1, 4)?.to_string(),
    ]);
    args.extend([
        "--calibration-tolerance-percent".into(),
        config_number(
            settings,
            "calibration.throughput_tolerance_percent",
            0.0,
            20.0,
        )?
        .to_string(),
    ]);
    Ok(args)
}
