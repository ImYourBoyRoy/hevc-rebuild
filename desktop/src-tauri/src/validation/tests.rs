//! Tests for stream, copied-payload, HDR, timing and chapter output validation.

use super::{
    approved_scrub_change, check_video_packet_counts, flag_value, float_value,
    is_replaceable_cover, packet_count_tolerance, source_value_is_present, stream_refs,
    validate_chapters, validate_hdr_video_profile, HdrTrackExpectation, Request,
};
use serde_json::{json, Value};

fn empty_request() -> Request {
    Request {
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        mediainfo: None,
        hdr10plus_tool: None,
        source_path: "input.mkv".into(),
        output_path: "output.mkv".into(),
        source_probe: json!({"streams": []}),
        selected_stream_indices: vec![],
        encode_video_indices: vec![],
        video_target_formats: Default::default(),
        video_hdr_expectations: Default::default(),
        verify_copy_hashes: false,
        test_mode: false,
        copy_only: false,
        expect_poster: false,
        source_duration: 0.0,
        source_chapters_count: 0,
        scrubbed_text_values: vec![],
        chapter_text_edits: vec![],
    }
}

#[test]
fn packet_count_gate_rejects_truncated_and_padded_encodes() {
    // Identical and near-identical counts pass.
    check_video_packet_counts(0, 172_800, 172_800).unwrap();
    check_video_packet_counts(0, 172_800, 172_800 - 17).unwrap();
    check_video_packet_counts(0, 100, 97).unwrap();
    // Tolerance scales with length but never below three packets.
    assert_eq!(packet_count_tolerance(100), 3);
    assert_eq!(packet_count_tolerance(172_800), 17);
    // A truncated encode (about 1 minute of a 2 hour title) is refused.
    let truncated = check_video_packet_counts(1, 172_800, 171_360).unwrap_err();
    assert!(truncated.contains("missing 1440 frame(s)"), "{truncated}");
    assert!(truncated.contains("Refusing to replace the original"));
    // Duplicated frames are refused too.
    let padded = check_video_packet_counts(1, 1_000, 1_050).unwrap_err();
    assert!(padded.contains("extra frame(s)"), "{padded}");
    // A stream with no packets (still image) has nothing to compare.
    check_video_packet_counts(2, 0, 1).unwrap();
}

/// Real files: a 3 s clip against a 1 s truncation of it must be refused,
/// while the clip compared with itself must pass.
#[test]
fn packet_count_gate_catches_a_really_truncated_video() {
    if !crate::media_tools_ready("packet count gate", &["ffmpeg", "ffprobe"]) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("hevc-rebuild-packet-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let full = dir.join("full.mkv");
    let truncated = dir.join("truncated.mkv");
    let encode = |duration: &str, output: &std::path::Path| {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc=size=160x120:rate=24:duration={duration}"))
            .args(["-c:v", "libx265", "-preset", "ultrafast"])
            .args(["-x265-params", "log-level=none", "-y"])
            .arg(output)
            .status()
            .unwrap()
            .success()
    };
    assert!(encode("3", &full) && encode("1", &truncated));
    let full_packets = crate::media_scan::count_stream_packets("ffprobe", &full, 0).unwrap();
    let short_packets = crate::media_scan::count_stream_packets("ffprobe", &truncated, 0).unwrap();
    assert_eq!((full_packets, short_packets), (72, 24));
    check_video_packet_counts(0, full_packets, full_packets).unwrap();
    let error = check_video_packet_counts(0, full_packets, short_packets).unwrap_err();
    assert!(error.contains("missing 48 frame(s)"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Regression for a real failure: a stream-copy cut of a B-frame H.264 file
/// starts with discard-flagged preroll packets that never become frames. The
/// source then has more packets than frames, and a raw packet comparison
/// refused a perfectly good encode. The counter must match the decoder.
#[test]
fn packet_counter_matches_decoded_frames_when_a_copy_cut_leaves_preroll() {
    if !crate::media_tools_ready("preroll packet count", &["ffmpeg", "ffprobe"]) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("hevc-rebuild-preroll-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let whole = dir.join("whole.mp4");
    let cut = dir.join("cut.mp4");
    let ffmpeg = |args: &[&str]| {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    assert!(ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=160x120:rate=24:duration=8",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-g",
        "48",
        "-bf",
        "3",
        "-pix_fmt",
        "yuv420p",
        whole.to_str().unwrap(),
    ]));
    // Seek into the middle of a GOP and stream-copy: leading packets are kept
    // as decoder preroll and flagged for discard.
    assert!(ffmpeg(&[
        "-ss",
        "2.5",
        "-i",
        whole.to_str().unwrap(),
        "-t",
        "3",
        "-c",
        "copy",
        cut.to_str().unwrap(),
    ]));
    let probe = |args: &[&str]| {
        let output = std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0"])
            .args(args)
            .arg(&cut)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    let both = probe(&[
        "-count_frames",
        "-count_packets",
        "-show_entries",
        "stream=nb_read_frames,nb_read_packets",
        "-of",
        "csv=p=0",
    ]);
    let (frames, raw_packets) = both.split_once(',').expect("frames,packets");
    let frames: u64 = frames.parse().unwrap();
    let raw_packets: u64 = raw_packets.parse().unwrap();
    assert!(
        raw_packets > frames,
        "the fixture must reproduce preroll (packets {raw_packets}, frames {frames})"
    );
    let counted = crate::media_scan::count_stream_packets("ffprobe", &cut, 0).unwrap();
    assert_eq!(
        counted, frames,
        "preroll must not count (raw packets {raw_packets})"
    );
    // Identical content therefore passes the gate even though raw counts differ.
    check_video_packet_counts(0, counted, frames).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn parses_stream_ordinals_and_attached_picture_flags() {
    let refs = stream_refs(&json!({"streams": [
        {"index": 0, "codec_type": "video", "width": 3840, "height": 2160, "disposition": {"default": 1}},
        {"index": 1, "codec_type": "video", "width": 1920, "height": 1080, "disposition": {"attached_pic": 1}},
        {"index": 2, "codec_type": "audio", "disposition": {"default": "1"}}
    ]})).unwrap();
    assert_eq!(refs[1].type_ordinal, 1);
    assert!(refs[1].attached_pic);
    assert_eq!(refs[2].type_ordinal, 0);
    assert!(refs[2].default);
}

#[test]
fn recognizes_only_cover_like_image_attachments_and_attached_pictures() {
    let refs = stream_refs(&json!({"streams": [
        {"index": 0, "codec_type": "video", "disposition": {"attached_pic": 1}, "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}},
        {"index": 1, "codec_type": "video", "disposition": {"attached_pic": 1}, "tags": {"filename": "backdrop.jpg", "mimetype": "image/jpeg"}},
        {"index": 2, "codec_type": "video", "width": 1920, "height": 1080, "tags": {"filename": "cover.jpg", "mimetype": "image/jpeg"}},
        {"index": 3, "codec_type": "attachment", "tags": {"filename": "poster.png", "mimetype": "image/png"}},
        {"index": 4, "codec_type": "attachment", "tags": {"filename": "font.ttf", "mimetype": "application/x-truetype-font"}}
    ]})).unwrap();
    assert!(is_replaceable_cover(&refs[0]));
    assert!(!is_replaceable_cover(&refs[1]));
    assert!(!is_replaceable_cover(&refs[2]));
    assert!(is_replaceable_cover(&refs[3]));
    assert!(!is_replaceable_cover(&refs[4]));
}

#[test]
fn parses_only_finite_numeric_timing_and_source_values() {
    assert_eq!(float_value(Some(&json!("2.5"))), Some(2.5));
    assert_eq!(float_value(Some(&json!("N/A"))), None);
    assert!(!source_value_is_present(Some(&json!(0))));
    assert!(!source_value_is_present(Some(&json!(0.0))));
    assert!(!source_value_is_present(Some(&json!("0"))));
    assert!(source_value_is_present(Some(&json!(2))));
    assert!(flag_value(Some(&json!(true))));
    assert!(!flag_value(Some(&json!(2))));
}

#[test]
fn only_exact_matching_text_scrub_changes_are_approved() {
    let edits = vec![json!({
        "scope": "track_tag", "field": "title", "before": "MovieReleaseGroup",
        "after": "", "chapter_index": null
    })];
    assert!(approved_scrub_change(
        &edits,
        &["track_tag", "track_header"],
        "MovieReleaseGroup",
        "",
        None
    ));
    assert!(!approved_scrub_change(
        &edits,
        &["track_tag"],
        "MovieReleaseGroup",
        "Other",
        None
    ));
}

#[test]
fn chapter_scrub_requires_matching_chapter_index() {
    let edits = vec![json!({
        "scope": "chapter", "field": "title", "before": "Scene Group",
        "after": "Scene", "chapter_index": 1
    })];
    let request = super::Request {
        ffmpeg: "ffmpeg".into(),
        ffprobe: "ffprobe".into(),
        mediainfo: None,
        hdr10plus_tool: None,
        source_path: "input.mkv".into(),
        output_path: "output.mkv".into(),
        source_probe: json!({"chapters": [
            {"start_time": "0", "end_time": "1", "tags": {"title": "Intro"}},
            {"start_time": "1", "end_time": "2", "tags": {"title": "Scene Group"}}
        ]}),
        selected_stream_indices: vec![],
        encode_video_indices: vec![],
        video_target_formats: Default::default(),
        video_hdr_expectations: Default::default(),
        verify_copy_hashes: false,
        test_mode: false,
        copy_only: false,
        source_duration: 2.0,
        source_chapters_count: 2,
        expect_poster: false,
        scrubbed_text_values: vec![],
        chapter_text_edits: edits,
    };
    let output = json!({"chapters": [
        {"start_time": "0", "end_time": "1", "tags": {"title": "Intro"}},
        {"start_time": "1", "end_time": "2", "tags": {"title": "Scene"}}
    ]});
    assert!(validate_chapters(&request, &output).is_ok());
}

#[test]
fn hdr_profile_validation_preserves_static_hdr_and_color_per_video_track() {
    let request = empty_request();
    let expected = HdrTrackExpectation {
        color_primaries: Some("bt2020".into()),
        transfer: Some("smpte2084".into()),
        color_matrix: Some("bt2020nc".into()),
        master_display: Some("G(1,2)".into()),
        max_cll: Some(1000),
        max_fall: Some(400),
        hdr10: true,
        ..Default::default()
    };
    let actual = json!({
        "color_primaries": "bt2020",
        "color_transfer": "smpte2084",
        "color_matrix": "bt2020nc",
        "mastering_display": "G(1,2)",
        "max_cll": 1000,
        "max_fall": 400,
        "hdr": {"hdr10": true, "hlg": false, "dolby_vision": false}
    });
    assert!(validate_hdr_video_profile(&request, 4, 0, &expected, &actual).is_ok());

    let mut changed: Value = actual.clone();
    changed["max_cll"] = json!(900);
    assert!(
        validate_hdr_video_profile(&request, 4, 0, &expected, &changed)
            .unwrap_err()
            .contains("MaxCLL changed")
    );
}

#[test]
fn dolby_vision_profile_validation_requires_matching_rpu_and_compatibility() {
    let request = empty_request();
    let expected = HdrTrackExpectation {
        hdr10: true,
        dolby_vision: true,
        dovi_profile: Some(8),
        dovi_rpu_present: true,
        dovi_compatibility_id: Some(1),
        preserve_dolbyvision: true,
        ..Default::default()
    };
    let actual = json!({
        "hdr": {
            "hdr10": true, "hlg": false, "dolby_vision": true,
            "dolby_vision_profile": 8, "dolby_vision_rpu": true,
            "dolby_vision_enhancement_layer": false,
            "dolby_vision_compatibility_id": 1
        }
    });
    assert!(validate_hdr_video_profile(&request, 2, 0, &expected, &actual).is_ok());

    let mut changed: Value = actual.clone();
    changed["hdr"]["dolby_vision_rpu"] = json!(false);
    assert!(
        validate_hdr_video_profile(&request, 2, 0, &expected, &changed)
            .unwrap_err()
            .contains("does not report an RPU")
    );
}
