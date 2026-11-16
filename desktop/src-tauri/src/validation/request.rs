//! The validation request, its parsing from stdin or JSON, path checks and stream references.

use super::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) ffmpeg: String,
    pub(super) ffprobe: String,
    #[serde(default)]
    pub(super) mediainfo: Option<String>,
    #[serde(default)]
    pub(super) hdr10plus_tool: Option<String>,
    pub(super) source_path: PathBuf,
    pub(super) output_path: PathBuf,
    pub(super) source_probe: Value,
    pub(super) selected_stream_indices: Vec<i64>,
    #[serde(default)]
    pub(super) encode_video_indices: Vec<i64>,
    #[serde(default)]
    pub(super) video_target_formats: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) video_hdr_expectations: BTreeMap<String, HdrTrackExpectation>,
    #[serde(default)]
    pub(super) verify_copy_hashes: bool,
    #[serde(default)]
    pub(super) test_mode: bool,
    #[serde(default)]
    pub(super) copy_only: bool,
    #[serde(default)]
    pub(super) expect_poster: bool,
    pub(super) source_duration: f64,
    pub(super) source_chapters_count: usize,
    #[serde(default)]
    pub(super) scrubbed_text_values: Vec<Value>,
    #[serde(default)]
    pub(super) chapter_text_edits: Vec<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HdrTrackExpectation {
    pub(super) color_primaries: Option<String>,
    pub(super) transfer: Option<String>,
    pub(super) color_matrix: Option<String>,
    pub(super) master_display: Option<String>,
    pub(super) max_cll: Option<i64>,
    pub(super) max_fall: Option<i64>,
    #[serde(default)]
    pub(super) hdr10: bool,
    #[serde(default)]
    pub(super) hdr10plus: bool,
    #[serde(default)]
    pub(super) hlg: bool,
    #[serde(default)]
    pub(super) dolby_vision: bool,
    pub(super) dovi_profile: Option<i64>,
    #[serde(default)]
    pub(super) dovi_rpu_present: bool,
    #[serde(default)]
    pub(super) dovi_el_present: bool,
    pub(super) dovi_compatibility_id: Option<i64>,
    #[serde(default)]
    pub(super) preserve_dolbyvision: bool,
    #[serde(default)]
    pub(super) strip_dolbyvision: bool,
}

#[derive(Debug, Clone)]
pub(super) struct StreamRef {
    pub(super) index: i64,
    pub(super) type_ordinal: usize,
    pub(super) codec_type: String,
    pub(super) codec_name: Option<String>,
    pub(super) width: i64,
    pub(super) height: i64,
    pub(super) pix_fmt: Option<String>,
    pub(super) default: bool,
    pub(super) attached_pic: bool,
    pub(super) tags: Map<String, Value>,
    pub(super) disposition: Map<String, Value>,
}

pub(crate) fn cli_mode(_args: Vec<OsString>) -> i32 {
    match read_request().and_then(validate_request) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(error) => {
                eprintln!("Could not serialize Rust stream-validation result: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub(super) fn read_request() -> Result<Request, String> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read Rust stream-validation request: {error}"))?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("Rust stream-validation request exceeds the 16 MiB limit.".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Rust stream-validation request JSON is invalid: {error}"))
}

/// In-process entry for native runs: same checks as `internal validate-streams`.
pub(crate) fn validate_streams_value(request: Value) -> Result<Value, String> {
    let request: Request = serde_json::from_value(request)
        .map_err(|error| format!("Rust stream-validation request JSON is invalid: {error}"))?;
    validate_request(request)
}

pub(super) fn validate_request(request: Request) -> Result<Value, String> {
    validate_paths(&request)?;
    if !request.source_duration.is_finite() || request.source_duration < 0.0 {
        return Err("Source duration must be a finite non-negative value.".into());
    }
    if request.selected_stream_indices.len() > MAX_STREAMS {
        return Err(format!(
            "Output validation is limited to {MAX_STREAMS} selected streams."
        ));
    }
    let source_refs = stream_refs(&request.source_probe)?;
    let source_by_index: HashMap<_, _> = source_refs
        .iter()
        .map(|stream| (stream.index, stream.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut selected = Vec::with_capacity(request.selected_stream_indices.len());
    for index in &request.selected_stream_indices {
        if !seen.insert(*index) {
            return Err(format!(
                "Selected stream index #{index} occurs more than once."
            ));
        }
        selected.push(source_by_index.get(index).cloned().ok_or_else(|| {
            format!("Selected source stream #{index} is missing from FFprobe data.")
        })?);
    }
    let encoded: BTreeSet<_> = request.encode_video_indices.iter().copied().collect();
    if encoded.len() != request.encode_video_indices.len() {
        return Err("Encoded video stream indices must be unique.".into());
    }
    if encoded.iter().any(|index| {
        source_by_index
            .get(index)
            .is_none_or(|stream| stream.codec_type != "video" || stream.attached_pic)
    }) {
        return Err(
            "An encoded video stream index is absent or is not a real video stream.".into(),
        );
    }
    if encoded.iter().any(|index| !seen.contains(index)) {
        return Err("Every encoded video stream must also be selected for output.".into());
    }

    let output_probe = media_scan::probe_output(&request.ffprobe, &request.output_path)?;
    let output_refs = stream_refs(&output_probe)?;
    validate_streams(&request, &selected, &output_refs, &output_probe, &encoded)?;

    Ok(json!({
        "output_probe": output_probe,
        "copied_hashes_verified": request.verify_copy_hashes && !request.test_mode,
        "hdr_validation_complete": true,
    }))
}

fn validate_paths(request: &Request) -> Result<(), String> {
    if request.ffmpeg.trim().is_empty() || request.ffmpeg.len() > 8192 {
        return Err("FFmpeg path is empty or exceeds the supported limit.".into());
    }
    if request.ffprobe.trim().is_empty() || request.ffprobe.len() > 8192 {
        return Err("FFprobe path is empty or exceeds the supported limit.".into());
    }
    if request.source_path.as_os_str().len() > 32 * 1024
        || request.output_path.as_os_str().len() > 32 * 1024
    {
        return Err("Media paths exceed the supported validation limit.".into());
    }
    if !request.source_path.is_file() {
        return Err(format!(
            "Validation source is not a readable file: {}",
            request.source_path.display()
        ));
    }
    let metadata = request.output_path.metadata().map_err(|_| {
        format!(
            "Output is missing or empty: {}",
            request.output_path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() < MIN_OUTPUT_BYTES {
        return Err(format!(
            "Output is missing or empty: {}",
            request.output_path.display()
        ));
    }
    Ok(())
}

pub(super) fn stream_refs(probe: &Value) -> Result<Vec<StreamRef>, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| "FFprobe result has no stream list.".to_string())?;
    if streams.len() > MAX_STREAMS {
        return Err(format!("FFprobe returned more than {MAX_STREAMS} streams."));
    }
    let mut ordinals: BTreeMap<String, usize> = BTreeMap::new();
    let mut result = Vec::with_capacity(streams.len());
    let mut indexes = BTreeSet::new();
    for raw in streams {
        let codec_type = raw
            .get("codec_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let ordinal = *ordinals.get(&codec_type).unwrap_or(&0);
        ordinals.insert(codec_type.clone(), ordinal + 1);
        let index = raw
            .get("index")
            .and_then(Value::as_i64)
            .ok_or_else(|| "FFprobe returned a stream without an integer index.".to_string())?;
        if index < 0 || !indexes.insert(index) {
            return Err("FFprobe returned a negative or duplicate stream index.".into());
        }
        let disposition = raw
            .get("disposition")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let tags = raw
            .get("tags")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        result.push(StreamRef {
            index,
            type_ordinal: ordinal,
            codec_type,
            codec_name: raw
                .get("codec_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            width: raw.get("width").and_then(Value::as_i64).unwrap_or(0),
            height: raw.get("height").and_then(Value::as_i64).unwrap_or(0),
            pix_fmt: raw
                .get("pix_fmt")
                .and_then(Value::as_str)
                .map(str::to_string),
            default: flag_value(disposition.get("default")),
            attached_pic: flag_value(disposition.get("attached_pic")),
            tags,
            disposition,
        });
    }
    Ok(result)
}

pub(super) fn raw_stream_map(probe: &Value) -> Result<HashMap<i64, Value>, String> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .ok_or_else(|| "FFprobe result has no stream list.".to_string())?;
    streams
        .iter()
        .map(|stream| {
            let index = stream
                .get("index")
                .and_then(Value::as_i64)
                .ok_or_else(|| "FFprobe returned a stream without an integer index.".to_string())?;
            Ok((index, stream.clone()))
        })
        .collect()
}
