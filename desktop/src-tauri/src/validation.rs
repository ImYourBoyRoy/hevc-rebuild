use crate::{media_scan, packet_verify};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod compare;
mod hdr;
mod request;
mod timing;
mod values;

use compare::*;
use hdr::*;
use request::*;
pub(crate) use request::{cli_mode, validate_streams_value};
use timing::*;
use values::*;

const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STREAMS: usize = 1024;
// A valid short test can be smaller than 1 MiB. FFprobe and the stream/HDR
// checks below are the authoritative output validation, so this guard only
// needs to reject an empty file before those checks run.
const MIN_OUTPUT_BYTES: u64 = 1;

fn validate_streams(
    request: &Request,
    selected: &[StreamRef],
    output_refs: &[StreamRef],
    output_probe: &Value,
    encoded: &BTreeSet<i64>,
) -> Result<(), String> {
    let selected_videos: Vec<_> = selected
        .iter()
        .filter(|stream| stream.codec_type == "video")
        .cloned()
        .collect();
    let mut real_videos: Vec<_> = selected_videos
        .iter()
        .filter(|stream| !stream.attached_pic && stream.width > 0 && stream.height > 0)
        .cloned()
        .collect();
    if !request.copy_only {
        real_videos.sort_by_key(|stream| {
            (
                Reverse(stream.width.saturating_mul(stream.height)),
                Reverse(stream.default),
                stream.index,
            )
        });
    }
    let mut selected_pictures: Vec<_> = selected_videos
        .iter()
        .filter(|stream| stream.attached_pic)
        .cloned()
        .collect();
    let out_videos: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == "video" && !stream.attached_pic)
        .cloned()
        .collect();
    let mut out_pictures: Vec<_> = output_refs
        .iter()
        .filter(|stream| stream.codec_type == "video" && stream.attached_pic)
        .cloned()
        .collect();
    if request.expect_poster {
        selected_pictures.retain(|stream| !is_replaceable_cover(stream));
        out_pictures.retain(|stream| !is_replaceable_cover(stream));
    }
    if out_videos.is_empty() {
        return Err("Output contains no usable video stream.".into());
    }
    if out_videos.len() != real_videos.len() {
        return Err(format!(
            "Selected real video stream count changed: expected {} output {}",
            real_videos.len(),
            out_videos.len()
        ));
    }
    if out_pictures.len() != selected_pictures.len() {
        return Err(format!(
            "Selected attached-picture count changed: expected {} output {}",
            selected_pictures.len(),
            out_pictures.len()
        ));
    }
    if !request.copy_only && !out_videos[0].default {
        return Err(
            "Selected highest-resolution video is not marked as the output default video track."
                .into(),
        );
    }
    for (source, output) in real_videos.iter().zip(&out_videos) {
        if encoded.contains(&source.index) {
            let target = request
                .video_target_formats
                .get(&source.index.to_string())
                .ok_or_else(|| {
                    format!(
                        "No source profile was retained for selected video stream #{}.",
                        source.index
                    )
                })?;
            if !matches!(
                output
                    .codec_name
                    .as_deref()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("hevc" | "h265")
            ) {
                return Err(format!(
                    "Selected video stream #{} output codec is {:?}; expected HEVC.",
                    source.index, output.codec_name
                ));
            }
            if output.pix_fmt.as_deref().unwrap_or("") != target {
                return Err(format!(
                    "Video stream #{} pixel format {:?} != planned {target:?}.",
                    source.index, output.pix_fmt
                ));
            }
        } else if output.codec_name != source.codec_name {
            return Err(format!(
                "Copied video stream #{} codec changed: {:?} -> {:?}.",
                source.index, source.codec_name, output.codec_name
            ));
        }
        if output.width != source.width || output.height != source.height {
            return Err(format!(
                "Video stream #{} resolution changed from {}x{} to {}x{}.",
                source.index, source.width, source.height, output.width, output.height
            ));
        }
    }

    if !request.test_mode && !request.copy_only {
        // Re-encoded video must keep one frame per source frame (the encode uses
        // passthrough frame timing and one-frame-per-frame deinterlacing). The
        // duration check below cannot see a truncated video track when copied
        // audio keeps the container's duration intact.
        for (source, output) in real_videos.iter().zip(&out_videos) {
            if !encoded.contains(&source.index) {
                continue;
            }
            let source_packets = media_scan::count_stream_packets(
                &request.ffprobe,
                &request.source_path,
                source.index,
            )?;
            let output_packets = media_scan::count_stream_packets(
                &request.ffprobe,
                &request.output_path,
                output.index,
            )?;
            check_video_packet_counts(source.index, source_packets, output_packets)?;
        }
    }

    validate_hdr_video_profiles(request, &real_videos, &out_videos, output_probe, encoded)?;

    let output_duration = output_probe
        .get("format")
        .and_then(|format| float_value(format.get("duration")))
        .unwrap_or(0.0);
    if !request.test_mode {
        let tolerance = 2.0_f64.max(request.source_duration * 0.005);
        let drift = (output_duration - request.source_duration).abs();
        if drift > tolerance {
            return Err(format!(
                "Output duration differs by {drift:.3}s (tolerance {tolerance:.3}s)."
            ));
        }
    }

    if !request.test_mode {
        for kind in ["audio", "subtitle", "attachment", "data"] {
            let mut source_kind = selected
                .iter()
                .filter(|stream| stream.codec_type == kind)
                .cloned()
                .collect::<Vec<_>>();
            let mut output_kind = output_refs
                .iter()
                .filter(|stream| stream.codec_type == kind)
                .cloned()
                .collect::<Vec<_>>();
            if kind == "attachment" && request.expect_poster {
                source_kind.retain(|stream| !is_replaceable_cover(stream));
                output_kind.retain(|stream| !is_replaceable_cover(stream));
            }
            compare_copied_type(request, kind, &source_kind, &output_kind)?;
        }
        let copied_videos: Vec<_> = real_videos
            .iter()
            .filter(|stream| !encoded.contains(&stream.index))
            .cloned()
            .collect();
        let copied_output_videos: Vec<_> = real_videos
            .iter()
            .zip(&out_videos)
            .filter(|(source, _)| !encoded.contains(&source.index))
            .map(|(_, output)| output.clone())
            .collect();
        compare_copied_type(request, "video", &copied_videos, &copied_output_videos)?;
        compare_copied_type(request, "video", &selected_pictures, &out_pictures)?;
        compare_codec_parameters(
            "audio",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
        )?;
        compare_codec_parameters(
            "subtitle",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
        )?;
        validate_timing(
            "audio",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
            0.010,
        )?;
        validate_timing(
            "subtitle",
            selected,
            output_refs,
            &request.source_probe,
            output_probe,
            0.020,
        )?;
        validate_chapters(request, output_probe)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
