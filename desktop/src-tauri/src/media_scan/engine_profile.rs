//! `engine-profile` and `engine-profile-tracks` requests that build source profiles for the encode engine.

use super::*;

const MAX_ENGINE_PROFILE_REQUEST_BYTES: u64 = 64 * 1024;

const MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES: u64 = 68 * 1024 * 1024;

const MAX_ENGINE_PROFILE_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineProfileRequest {
    pub(crate) path: String,
    pub(crate) ffprobe: String,
    pub(crate) mediainfo: String,
    pub(crate) promote_8bit_to_10bit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineTrackProfilesRequest {
    pub(crate) path: String,
    pub(crate) ffprobe: String,
    pub(crate) promote_8bit_to_10bit: bool,
    pub(crate) video_stream_indices: Vec<i64>,
    pub(crate) probe: Value,
    pub(crate) mediainfo: Value,
}

/// Profile the source once in Rust for app-launched engine jobs. The response
/// carries the raw probe payload for remaining compatibility stages and a full
/// profile for the selected primary stream.
pub(crate) fn engine_profile_cli_mode() -> i32 {
    let result = (|| {
        let mut input = Vec::new();
        std::io::stdin()
            .take(MAX_ENGINE_PROFILE_REQUEST_BYTES + 1)
            .read_to_end(&mut input)
            .map_err(|error| format!("Could not read the native profile request: {error}"))?;
        if input.len() as u64 > MAX_ENGINE_PROFILE_REQUEST_BYTES {
            return Err("Native profile request exceeds 64 KiB.".to_owned());
        }
        let request: EngineProfileRequest = serde_json::from_slice(&input)
            .map_err(|error| format!("Invalid native profile request: {error}"))?;
        let payload = engine_profile(request)?;
        let encoded = serde_json::to_vec(&payload)
            .map_err(|error| format!("Could not serialize the native source profile: {error}"))?;
        if encoded.len() > MAX_ENGINE_PROFILE_RESPONSE_BYTES {
            return Err("Native source profile exceeds the 64 MiB response limit.".into());
        }
        Ok(encoded)
    })();
    match result {
        Ok(encoded) => {
            let mut stdout = std::io::stdout().lock();
            if let Err(error) = std::io::Write::write_all(&mut stdout, &encoded)
                .and_then(|()| std::io::Write::write_all(&mut stdout, b"\n"))
            {
                eprintln!("Could not write native source profile: {error}");
                1
            } else {
                0
            }
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Profile one source in-process: raw FFprobe/MediaInfo JSON plus the primary
/// video profile. Shared by the compatibility-engine helper and native runs.
pub(crate) fn engine_profile(request: EngineProfileRequest) -> Result<Value, String> {
    {
        if request.path.is_empty() || request.path.len() > 8192 {
            return Err("Profile request path must be between 1 and 8192 bytes.".into());
        }
        for (name, tool) in [
            ("FFprobe", &request.ffprobe),
            ("MediaInfo", &request.mediainfo),
        ] {
            if tool.is_empty() || tool.len() > 8192 || tool.contains('\0') {
                return Err(format!("The configured {name} path is invalid."));
            }
        }
        let path = expand_home(Path::new(&request.path))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !path.is_file() {
            return Err("Media input must be a regular file.".into());
        }
        let parent = path.parent().unwrap_or(&path);
        let settings_payload = serde_json::to_value(settings::load_effective_settings(
            &parent.to_string_lossy(),
        )?)
        .map_err(|error| format!("Could not encode effective settings: {error}"))?;
        let mut effective_settings = settings_payload
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(video) = effective_settings
            .get_mut("video")
            .and_then(Value::as_object_mut)
        {
            video.insert(
                "promote_8bit_to_10bit".into(),
                Value::Bool(request.promote_8bit_to_10bit),
            );
        }

        let probe = run_json_tool(
            &request.ffprobe,
            &[
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-show_chapters",
                "-show_data_hash",
                "sha256",
                "-of",
                "json",
            ],
            &path,
            Duration::from_secs(120),
            MAX_TOOL_JSON_BYTES,
        )?
        .value;
        let media_info = run_json_tool(
            &request.mediainfo,
            &["--Output=JSON"],
            &path,
            Duration::from_secs(45),
            MAX_TOOL_JSON_BYTES,
        )?
        .value;
        let primary = build_profile(
            &probe,
            &media_info,
            &path,
            &effective_settings,
            &request.ffprobe,
        )?;
        Ok(json!({
            "path": path.to_string_lossy(),
            "probe": probe,
            "mediainfo": media_info,
            "profile": primary.payload
        }))
    }
}

/// Build profile fields for the video streams the engine actually selected.
/// FFprobe and MediaInfo JSON are sent back from the preceding Rust call so this
/// operation only needs the per-video frame metadata probe.
pub(crate) fn engine_track_profiles_cli_mode() -> i32 {
    let result = (|| {
        let mut input = Vec::new();
        std::io::stdin()
            .take(MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES + 1)
            .read_to_end(&mut input)
            .map_err(|error| {
                format!("Could not read the native selected-track profile request: {error}")
            })?;
        if input.len() as u64 > MAX_ENGINE_TRACK_PROFILE_REQUEST_BYTES {
            return Err("Native selected-track profile request exceeds 68 MiB.".into());
        }
        let request: EngineTrackProfilesRequest = serde_json::from_slice(&input)
            .map_err(|error| format!("Invalid native selected-track profile request: {error}"))?;
        engine_track_profiles(request)
    })();
    match result {
        Ok(payload) => match serde_json::to_vec(&payload) {
            Ok(encoded) => {
                let mut stdout = std::io::stdout().lock();
                if let Err(error) = std::io::Write::write_all(&mut stdout, &encoded)
                    .and_then(|()| std::io::Write::write_all(&mut stdout, b"\n"))
                {
                    eprintln!("Could not write selected-track profiles: {error}");
                    1
                } else {
                    0
                }
            }
            Err(error) => {
                eprintln!("Could not serialize selected-track profiles: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Compact per-video-track profiles for the selected stream indices.
pub(crate) fn engine_track_profiles(request: EngineTrackProfilesRequest) -> Result<Value, String> {
    {
        if request.path.is_empty() || request.path.len() > 8192 {
            return Err("Selected-track profile path must be between 1 and 8192 bytes.".into());
        }
        if request.ffprobe.is_empty()
            || request.ffprobe.len() > 8192
            || request.ffprobe.contains('\0')
        {
            return Err("The configured FFprobe path is invalid.".into());
        }
        if request.video_stream_indices.is_empty()
            || request.video_stream_indices.len() > MAX_VIDEO_PROFILES
        {
            return Err(format!(
                "Selected-track profile count must be between 1 and {MAX_VIDEO_PROFILES}."
            ));
        }
        let wanted = request
            .video_stream_indices
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if wanted.len() != request.video_stream_indices.len()
            || wanted.iter().any(|index| *index < 0)
        {
            return Err("Selected video stream indices must be unique and nonnegative.".into());
        }
        if !request.probe.is_object() || !request.mediainfo.is_object() {
            return Err(
                "Selected-track profiling requires FFprobe and MediaInfo JSON objects.".into(),
            );
        }
        let path = expand_home(Path::new(&request.path))
            .canonicalize()
            .map_err(|error| format!("Media input is unavailable: {error}"))?;
        if !path.is_file() {
            return Err("Media input must be a regular file.".into());
        }
        let parent = path.parent().unwrap_or(&path);
        let settings_payload = serde_json::to_value(settings::load_effective_settings(
            &parent.to_string_lossy(),
        )?)
        .map_err(|error| format!("Could not encode effective settings: {error}"))?;
        let mut effective_settings = settings_payload
            .get("settings")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(video) = effective_settings
            .get_mut("video")
            .and_then(Value::as_object_mut)
        {
            video.insert(
                "promote_8bit_to_10bit".into(),
                Value::Bool(request.promote_8bit_to_10bit),
            );
        }
        let streams = request
            .probe
            .get("streams")
            .and_then(Value::as_array)
            .ok_or("FFprobe JSON did not include a stream list.")?;
        let mut video_ordinal = 0_usize;
        let mut profiles = Map::new();
        for stream in streams {
            if !stream
                .get("codec_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("video"))
            {
                continue;
            }
            let ordinal = video_ordinal;
            video_ordinal += 1;
            let index = value_i64(stream.get("index"))
                .ok_or("FFprobe returned a video stream without an index.")?;
            if !wanted.contains(&index) {
                continue;
            }
            if disposition(stream, "attached_pic")
                || value_i64(stream.get("width")).unwrap_or(0) <= 0
                || value_i64(stream.get("height")).unwrap_or(0) <= 0
            {
                return Err(format!(
                    "Selected stream #{index} is not a usable video track."
                ));
            }
            let profile = build_profile_for_video(
                &request.probe,
                &request.mediainfo,
                &path,
                &effective_settings,
                &request.ffprobe,
                Some(ordinal),
            )?;
            let compact = [
                "video",
                "hdr",
                "duration_seconds",
                "interlaced",
                "color_primaries",
                "color_transfer",
                "color_matrix",
                "color_range_x265",
                "color_range_ffmpeg",
                "mastering_display",
                "mastering_display_source",
                "max_cll",
                "max_fall",
                "chroma_location",
                "format_level",
                "primary_stream_index",
            ]
            .into_iter()
            .filter_map(|key| {
                profile
                    .payload
                    .get(key)
                    .cloned()
                    .map(|value| (key.to_owned(), value))
            })
            .collect::<Map<_, _>>();
            profiles.insert(index.to_string(), Value::Object(compact));
        }
        if profiles.len() != wanted.len() {
            let missing = wanted
                .iter()
                .filter(|index| !profiles.contains_key(&index.to_string()))
                .map(|index| format!("#{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Selected video stream(s) were not found: {missing}."
            ));
        }
        Ok(json!({ "video_profiles": profiles }))
    }
}
