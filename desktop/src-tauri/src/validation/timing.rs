//! Packet-count, duration/timing and chapter checks.

use super::*;

/// Largest packet-count difference tolerated between a source video stream and
/// its re-encode: a few packets for decoder start-up/open-GOP effects, scaling
/// to 0.01% of the stream so long titles are not failed by a handful of frames.
pub(super) fn packet_count_tolerance(source_packets: u64) -> u64 {
    3.max(source_packets / 10_000)
}

pub(super) fn check_video_packet_counts(
    source_index: i64,
    source_packets: u64,
    output_packets: u64,
) -> Result<(), String> {
    if source_packets == 0 {
        // Nothing to compare against (for example a still-image track).
        return Ok(());
    }
    let tolerance = packet_count_tolerance(source_packets);
    if output_packets.abs_diff(source_packets) > tolerance {
        let missing = source_packets as i128 - output_packets as i128;
        let direction = if missing > 0 {
            format!("is missing {missing} frame(s)")
        } else {
            format!("has {} extra frame(s)", -missing)
        };
        return Err(format!(
            "Re-encoded video stream #{source_index} {direction}: source has {source_packets} packets, output has {output_packets} (tolerance {tolerance}). Refusing to replace the original."
        ));
    }
    Ok(())
}

pub(super) fn validate_timing(
    kind: &str,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    source_probe: &Value,
    output_probe: &Value,
    tolerance: f64,
) -> Result<(), String> {
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
        for key in ["start_time", "duration"] {
            let (Some(left), Some(right)) = (
                a.and_then(|stream| float_value(stream.get(key))),
                b.and_then(|stream| float_value(stream.get(key))),
            ) else {
                continue;
            };
            let drift = (left - right).abs();
            if drift > tolerance {
                return Err(format!(
                    "{kind} stream {ordinal} {key} shifted by {drift:.6}s; refusing possible A/V/subtitle sync change."
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_chapters(request: &Request, output_probe: &Value) -> Result<(), String> {
    let source_chapters = request
        .source_probe
        .get("chapters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let output_chapters = output_probe
        .get("chapters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if output_chapters.len() != request.source_chapters_count {
        return Err(format!(
            "Chapter count changed: {} -> {}",
            request.source_chapters_count,
            output_chapters.len()
        ));
    }
    for (ordinal, (source, output)) in source_chapters.iter().zip(output_chapters).enumerate() {
        for key in ["start_time", "end_time"] {
            let (Some(before), Some(after)) = (
                source.get(key).and_then(|value| float_value(Some(value))),
                output.get(key).and_then(|value| float_value(Some(value))),
            ) else {
                continue;
            };
            let drift = (before - after).abs();
            if drift > 0.002 {
                return Err(format!("Chapter {ordinal} {key} changed by {drift:.6}s."));
            }
        }
        let source_tags = lower_key_map(source.get("tags"));
        let output_tags = lower_key_map(output.get("tags"));
        for (name, before) in source_tags {
            if matches!(name.as_str(), "encoder" | "duration") {
                continue;
            }
            let after = output_tags.get(&name).cloned().unwrap_or_default();
            if before != after
                && !(name == "title"
                    && approved_scrub_change(
                        &request.chapter_text_edits,
                        &["chapter"],
                        &before,
                        &after,
                        Some(ordinal),
                    ))
            {
                return Err(format!(
                    "Chapter {ordinal} metadata {name:?} changed: {before:?} -> {after:?}"
                ));
            }
        }
    }
    Ok(())
}
