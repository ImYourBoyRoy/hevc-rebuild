//! Source-versus-output comparison of copied streams: payload type, metadata, dispositions and codec parameters.

use super::*;

pub(super) fn compare_copied_type(
    request: &Request,
    kind: &str,
    source: &[StreamRef],
    output: &[StreamRef],
) -> Result<(), String> {
    if source.len() != output.len() {
        return Err(format!(
            "{kind} stream count changed: input={} output={}",
            source.len(),
            output.len()
        ));
    }
    let short = match kind {
        "audio" => "a",
        "subtitle" => "s",
        "video" => "v",
        "attachment" => "t",
        "data" => "d",
        _ => return Err(format!("Unsupported copied stream type {kind:?}.")),
    };
    if request.verify_copy_hashes {
        if kind == "attachment" {
            for (ordinal, (left, right)) in source.iter().zip(output).enumerate() {
                let source_hash = packet_verify::stream_copy_hash(
                    Path::new(&request.ffmpeg),
                    &request.source_path,
                    &format!("{short}:{}", left.type_ordinal),
                )?;
                let output_hash = packet_verify::stream_copy_hash(
                    Path::new(&request.ffmpeg),
                    &request.output_path,
                    &format!("{short}:{}", right.type_ordinal),
                )?;
                compare_streams(
                    request,
                    kind,
                    ordinal,
                    left,
                    right,
                    &source_hash,
                    &output_hash,
                )?;
            }
        } else {
            let source_hashes = packet_verify::packet_fingerprints(
                Path::new(&request.ffmpeg),
                &request.source_path,
                &source
                    .iter()
                    .map(|stream| format!("{short}:{}", stream.type_ordinal))
                    .collect::<Vec<_>>(),
            )?;
            let output_hashes = packet_verify::packet_fingerprints(
                Path::new(&request.ffmpeg),
                &request.output_path,
                &output
                    .iter()
                    .map(|stream| format!("{short}:{}", stream.type_ordinal))
                    .collect::<Vec<_>>(),
            )?;
            for (ordinal, ((left, right), (source_hash, output_hash))) in source
                .iter()
                .zip(output)
                .zip(source_hashes.iter().zip(&output_hashes))
                .enumerate()
            {
                compare_streams(
                    request,
                    kind,
                    ordinal,
                    left,
                    right,
                    source_hash,
                    output_hash,
                )?;
            }
        }
    } else {
        for (ordinal, (left, right)) in source.iter().zip(output).enumerate() {
            compare_stream_metadata(request, kind, ordinal, left, right)?;
        }
    }
    Ok(())
}

pub(super) fn is_replaceable_cover(stream: &StreamRef) -> bool {
    let is_attachment = stream.codec_type == "attachment";
    let is_attached_picture = stream.codec_type == "video" && stream.attached_pic;
    if !is_attachment && !is_attached_picture {
        return false;
    }
    let filename = stream
        .tags
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("filename"))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    let mime = stream
        .tags
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("mimetype"))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    mime.to_ascii_lowercase().starts_with("image/")
        && ["cover", "poster", "folder", "front"]
            .iter()
            .any(|marker| filename.to_ascii_lowercase().contains(marker))
}

fn compare_streams<T: PartialEq>(
    request: &Request,
    kind: &str,
    ordinal: usize,
    source: &StreamRef,
    output: &StreamRef,
    source_hash: &T,
    output_hash: &T,
) -> Result<(), String> {
    compare_stream_metadata(request, kind, ordinal, source, output)?;
    if source_hash != output_hash {
        return Err(format!(
            "{kind} stream {ordinal} compressed packet sequence/extradata differs. The output is rejected; no audio/subtitle transcoding or packet mutation is allowed."
        ));
    }
    Ok(())
}

fn compare_stream_metadata(
    request: &Request,
    kind: &str,
    ordinal: usize,
    source: &StreamRef,
    output: &StreamRef,
) -> Result<(), String> {
    if source.codec_name != output.codec_name {
        return Err(format!(
            "{kind} stream {ordinal} codec changed: {:?} -> {:?}",
            source.codec_name, output.codec_name
        ));
    }
    if matches!(kind, "audio" | "subtitle" | "attachment" | "data") {
        for key in ["language", "title"] {
            let before = source.tags.get(key).map(value_text).unwrap_or_default();
            let after = output.tags.get(key).map(value_text).unwrap_or_default();
            if before != after
                && !(key == "title"
                    && approved_scrub_change(
                        &request.scrubbed_text_values,
                        &["track_tag", "track_header"],
                        &before,
                        &after,
                        None,
                    ))
            {
                return Err(format!("{kind} stream {ordinal} {key} metadata changed."));
            }
        }
        if kind == "attachment" {
            for key in ["filename", "mimetype"] {
                let before = source.tags.get(key).map(value_text).unwrap_or_default();
                let after = output.tags.get(key).map(value_text).unwrap_or_default();
                if before != after {
                    return Err(format!(
                        "attachment stream {ordinal} {key} metadata changed."
                    ));
                }
            }
        }
        let before = active_dispositions(&source.disposition);
        let after = active_dispositions(&output.disposition);
        if before != after {
            return Err(format!(
                "{kind} stream {ordinal} disposition flags changed: {before:?} -> {after:?}"
            ));
        }
    }
    Ok(())
}

fn active_dispositions(values: &Map<String, Value>) -> BTreeSet<String> {
    values
        .iter()
        .filter(|(_, value)| integerish(value) == Some(1))
        .map(|(name, _)| name.clone())
        .collect()
}

pub(super) fn integerish(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_bool().map(i64::from))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

pub(super) fn compare_codec_parameters(
    kind: &str,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    source_probe: &Value,
    output_probe: &Value,
) -> Result<(), String> {
    let keys: &[&str] = match kind {
        "audio" => &[
            "profile",
            "sample_rate",
            "channels",
            "channel_layout",
            "bits_per_sample",
            "bits_per_raw_sample",
            "extradata_hash",
        ],
        "subtitle" => &["profile", "width", "height", "extradata_hash"],
        _ => return Ok(()),
    };
    let source: Vec<_> = selected
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let output: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == kind)
        .collect();
    let source_raw = raw_stream_map(source_probe)?;
    let output_raw = raw_stream_map(output_probe)?;
    for (ordinal, (source_ref, output_ref)) in source.iter().zip(&output).enumerate() {
        let a = source_raw.get(&source_ref.index);
        let b = output_raw.get(&output_ref.index);
        for key in keys {
            let left = a.and_then(|stream| stream.get(*key));
            let right = b.and_then(|stream| stream.get(*key));
            if source_value_is_present(left)
                && value_text(left.unwrap()) != value_text(right.unwrap_or(&Value::Null))
            {
                return Err(format!(
                    "{kind} stream {ordinal} codec parameter {key} changed: {} -> {}",
                    value_text(left.unwrap()),
                    value_text(right.unwrap_or(&Value::Null))
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn approved_scrub_change(
    edits: &[Value],
    scopes: &[&str],
    before: &str,
    after: &str,
    chapter_index: Option<usize>,
) -> bool {
    edits.iter().any(|edit| {
        let scope = edit
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let field = edit
            .get("field")
            .and_then(Value::as_str)
            .unwrap_or("title")
            .trim()
            .to_ascii_lowercase();
        let edit_before = edit.get("before").map(value_text).unwrap_or_default();
        let edit_after = edit.get("after").map(value_text).unwrap_or_default();
        let index_matches = chapter_index.is_none_or(|index| {
            edit.get("chapter_index").and_then(Value::as_u64) == Some(index as u64)
        });
        scopes.contains(&scope)
            && matches!(field.as_str(), "title" | "name")
            && edit_before == before
            && edit_after == after
            && index_matches
    })
}
