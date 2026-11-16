//! HDR10+ dynamic-metadata extraction from the source and injection into the new output.

use super::*;

/// Stage 7: extract HDR10+ metadata and decide encode-time vs post-encode use.
pub(super) fn extract_hdr10plus(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    work_dir: &Path,
    plan: &FilePlan,
) -> Result<Hdr10PlusPlan, String> {
    let source = frame.source;
    let tools = context.tools;
    let options = context.options;
    let any_hdr10plus = plan.any_hdr10plus();
    let effective = &plan.effective;
    let verified_hdr10plus = &plan.verified_hdr10plus;
    let video_profiles = &plan.video_profiles;
    let mut hdr10plus_jsons: BTreeMap<i64, PathBuf> = BTreeMap::new();
    // Prefer encode-time dhdr10-info when libx265 has HDR10_PLUS; otherwise keep
    // static HDR10 in the encode and inject SEI afterward with hdr10plus_tool.
    let mut hdr10plus_post_inject = false;
    if any_hdr10plus && effective.preserve_hdr10plus {
        state.phase = Phase::Hdr10PlusExtraction;
        emit(
            context,
            frame,
            "Extracting HDR10+ metadata",
            0.06,
            None,
            None,
            None,
            None,
        );
        let helper = tools
            .hdr10plus
            .as_deref()
            .ok_or("HDR10+ preservation requires hdr10plus_tool.")?;
        let extract_limit = options.test_mode.then_some(options.test_seconds as f64);
        for index in verified_hdr10plus.iter() {
            let track = &video_profiles[index];
            let codec = track
                .pointer("/video/codec")
                .and_then(Value::as_str)
                .unwrap_or("hevc");
            let metadata_json = work_dir.join(format!("hdr10plus-stream-{index}.json"));
            crate::hdr_metadata::extract_hdr10plus(
                &tools.ffmpeg,
                helper,
                source,
                *index,
                codec,
                &metadata_json,
                extract_limit,
            )?;
            hdr10plus_jsons.insert(*index, metadata_json);
        }
        match crate::hdr_metadata::hdr10plus_preservation_mode(tools.x265_hdr10plus) {
            "encode-time" => {
                (context.line)(
                    "HDR10+ will be preserved during encode (libx265 dhdr10-info).".into(),
                );
            }
            _ => {
                hdr10plus_post_inject = true;
                (context.line)(
                    "HDR10+ will be preserved after encode with hdr10plus_tool inject (host libx265 lacks HDR10_PLUS)."
                        .into(),
                );
            }
        }
    } else if any_hdr10plus {
        (context.line)(
            "HDR10+ source detected, but preservation is disabled; selected video tracks will not carry dynamic metadata into the re-encode.".into(),
        );
    }
    Ok(Hdr10PlusPlan {
        jsons: hdr10plus_jsons,
        post_inject: hdr10plus_post_inject,
    })
}

/// Stage 9: when the host libx265 lacks HDR10+ support, inject it afterward.
pub(super) fn inject_hdr10plus(
    context: &mut Context<'_>,
    frame: &FileFrame<'_>,
    state: &mut FileState,
    work_dir: &Path,
    plan: &FilePlan,
    targets: &OutputTargets,
    hdr10plus: &Hdr10PlusPlan,
) -> Result<(), String> {
    let tools = context.tools;
    let encode_video_indices = &plan.encode_video_indices;
    let temp_output = &targets.temp_output;
    let hdr10plus_post_inject = hdr10plus.post_inject;
    let hdr10plus_jsons = &hdr10plus.jsons;
    if hdr10plus_post_inject {
        state.phase = Phase::Hdr10PlusInject;
        emit(
            context,
            frame,
            "Injecting HDR10+ metadata",
            0.88,
            None,
            file_size(temp_output),
            None,
            None,
        );
        let helper = tools
            .hdr10plus
            .as_deref()
            .ok_or("HDR10+ post-inject requires hdr10plus_tool.")?;
        // Encoded Matroska video ordinals follow encode_video_indices order.
        // Inject every track that has JSON; remux keeps non-injected videos too.
        let video_jsons: Vec<(usize, Option<&Path>)> = encode_video_indices
            .iter()
            .enumerate()
            .map(|(ordinal, stream_index)| {
                (
                    ordinal,
                    hdr10plus_jsons.get(stream_index).map(PathBuf::as_path),
                )
            })
            .collect();
        crate::hdr_metadata::apply_hdr10plus_injections_to_matroska(
            &tools.ffmpeg,
            &tools.mkvmerge,
            helper,
            temp_output,
            &video_jsons,
            work_dir,
        )?;
        for stream_index in hdr10plus_jsons.keys() {
            if encode_video_indices.contains(stream_index) {
                (context.line)(format!(
                    "Injected HDR10+ dynamic metadata into encoded video stream #{stream_index}."
                ));
            }
        }
    }

    Ok(())
}
