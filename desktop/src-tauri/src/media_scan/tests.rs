//! Tests for discovery, profiling, packet counting and the engine profile requests.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "hevc-rebuild-scan-{label}-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create isolated scan fixture");
    path
}

#[test]
fn profile_cache_keys_invalidate_for_source_settings_tool_and_version_changes() {
    let root = temp_dir("cache-key");
    let source = root.join("source.mkv");
    let tool = root.join("tool");
    fs::write(&source, b"synthetic").unwrap();
    fs::write(&tool, b"tool").unwrap();
    let host = json!({"checks": [{"key": "ffprobe", "version": "1"}, {"key": "mediainfo", "version": "1"}]});
    let key = |settings: &Value, host: &Value| {
        profile_cache_key(
            &source,
            tool.to_str().unwrap(),
            Some(tool.to_str().unwrap()),
            settings,
            host,
        )
        .unwrap()
    };
    let baseline = key(&json!({"video": {"promote": true}}), &host);
    assert_eq!(baseline, key(&json!({"video": {"promote": true}}), &host));
    assert_ne!(baseline, key(&json!({"video": {"promote": false}}), &host));
    assert_ne!(
        baseline,
        key(
            &json!({"video": {"promote": true}}),
            &json!({"checks": [{"key": "ffprobe", "version": "2"}]})
        )
    );
    fs::write(&source, b"modified synthetic").unwrap();
    assert_ne!(baseline, key(&json!({"video": {"promote": true}}), &host));
    let after_source = key(&json!({"video": {"promote": true}}), &host);
    fs::write(&tool, b"updated tool").unwrap();
    assert_ne!(
        after_source,
        key(&json!({"video": {"promote": true}}), &host)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn packet_counter_skips_discard_preroll_but_counts_everything_else() {
    let listing = "K__\n___\n_D_\nKD_\n\n__C\n  ___  \n";
    // K__, ___, __C and ___ produce frames; _D_ and KD_ are preroll.
    assert_eq!(count_decodable_packets(listing.as_bytes()), 4);
    assert_eq!(count_decodable_packets("".as_bytes()), 0);
    assert_eq!(count_decodable_packets("_D_\nKD_\n".as_bytes()), 0);
}

#[test]
fn repeated_scan_profile_cache_is_validated() {
    let root = temp_dir("cache-benchmark");
    if !crate::media_tools_ready("scan cache integration", &["ffmpeg"]) {
        let _ = fs::remove_dir_all(root);
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("HEVC_REBUILD_SCAN_TIMING", "1");
    let clip = root.join("synthetic-0.mkv");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x180:r=24",
            "-t",
            "1",
            "-c:v",
            "libx264",
            "-threads",
            "1"
        ])
        .arg(&clip)
        .status()
        .unwrap()
        .success());
    for index in 1..20 {
        fs::copy(&clip, root.join(format!("synthetic-{index}.mkv"))).unwrap();
    }
    let options = ScanOptions {
        root: root.to_string_lossy().into(),
        recursive: false,
        minimum_size_mib: 0,
        filename_include_patterns: vec![],
        filename_exclude_patterns: vec![],
        tool_paths: ToolPaths::default(),
        stop_file: None,
    };
    let cold = scan_media(options.clone(), |_, _, _| {}).unwrap();
    let warm = scan_media(options.clone(), |_, _, _| {}).unwrap();
    for (cold_entry, warm_entry) in cold["entries"]
        .as_array()
        .unwrap()
        .iter()
        .zip(warm["entries"].as_array().unwrap())
    {
        assert_eq!(cold_entry["profile"], warm_entry["profile"]);
        assert_eq!(cold_entry["status"], warm_entry["status"]);
    }
    assert_eq!(warm["scan_timings"]["profile_cache_hits"], 20);
    fs::write(&clip, b"changed, invalid media").unwrap();
    let changed = scan_media(options, |_, _, _| {}).unwrap();
    assert_eq!(changed["scan_timings"]["profile_cache_hits"], 19);
    assert_eq!(changed["entries"][0]["status"], "Analysis failed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filename_filters_match_case_insensitive_wildcards_and_exclusions_win() {
    let include = vec!["*Movie*Group.mkv".into(), "Special?.mp4".into()];
    let exclude = vec!["*sample*".into()];
    assert!(filename_matches_filters(
        "GreatMovieReleaseGroup.MKV",
        &include,
        &exclude
    ));
    assert!(filename_matches_filters("Special1.mp4", &include, &exclude));
    assert!(!filename_matches_filters(
        "GreatMovieSampleGroup.mkv",
        &include,
        &exclude
    ));
    assert!(!filename_matches_filters("Other.mkv", &include, &exclude));
    assert!(filename_matches_filters("Other.mkv", &[], &[]));
}

#[test]
fn filename_filter_validation_rejects_unsafe_or_ambiguous_rules() {
    for invalid in ["", "***", "?*", "../*.mkv", "bad\\name", " padded"] {
        assert!(
            validate_filename_patterns(&[invalid.into()], "include").is_err(),
            "{invalid}"
        );
    }
    assert!(validate_filename_patterns(&["Film*".into(), "film*".into()], "exclude").is_err());
    assert!(validate_filename_patterns(&["x".repeat(129)], "include").is_err());
}

fn profile_fixture() -> Value {
    json!({
        "streams": [
            {
                "index": 0, "codec_type": "video", "codec_name": "hevc", "profile": "Main 10",
                "width": 3840, "height": 2160, "pix_fmt": "yuv420p10le", "bits_per_raw_sample": "10",
                "avg_frame_rate": "24000/1001", "field_order": "progressive",
                "color_primaries": "bt2020", "color_transfer": "smpte2084", "color_space": "bt2020nc",
                "color_range": "tv", "chroma_location": "left",
                "tags": {"language": "und", "title": "4K"},
                "disposition": {"default": 1, "attached_pic": 0},
                "side_data_list": [
                    {"side_data_type": "DOVI configuration record", "dv_profile": 8, "dv_level": 6, "rpu_present_flag": 1, "el_present_flag": 0, "bl_present_flag": 1, "dv_bl_signal_compatibility_id": 1},
                    {"side_data_type": "HDR Dynamic Metadata SMPTE2094-40 (HDR10+)"},
                    {"side_data_type": "Mastering display metadata", "red_x": 0.68, "red_y": 0.32, "green_x": 0.265, "green_y": 0.69, "blue_x": 0.15, "blue_y": 0.06, "white_point_x": 0.3127, "white_point_y": 0.329, "min_luminance": 0.005, "max_luminance": 1000},
                    {"side_data_type": "Content light level metadata", "max_content": 1000, "max_average": 400}
                ]
            },
            {
                "index": 1, "codec_type": "video", "codec_name": "h264", "profile": "High",
                "width": 1920, "height": 1080, "pix_fmt": "yuv420p", "bits_per_raw_sample": "8",
                "avg_frame_rate": "24000/1001", "field_order": "progressive",
                "tags": {"language": "eng", "title": "1080p"},
                "disposition": {"default": 0, "attached_pic": 0}
            },
            {
                "index": 2, "codec_type": "audio", "codec_name": "ac3", "channels": 6,
                "channel_layout": "5.1(side)", "sample_rate": "48000", "bit_rate": "640000",
                "tags": {"language": "eng", "title": "Surround"},
                "disposition": {"default": 1, "forced": 0}
            },
            {
                "index": 3, "codec_type": "subtitle", "codec_name": "subrip",
                "tags": {"language": "eng", "title": "English"}, "disposition": {"default": 0, "forced": 1}
            },
            {
                "index": 4, "codec_type": "attachment", "codec_name": "ttf",
                "tags": {"filename": "font.ttf", "mimetype": "application/x-truetype-font"},
                "disposition": {"default": 0}
            },
            {
                "index": 5, "codec_type": "video", "codec_name": "mjpeg", "width": 600, "height": 600,
                "pix_fmt": "yuvj420p", "disposition": {"attached_pic": 1}
            }
        ],
        "format": {"format_name": "matroska,webm", "duration": "3600.5", "size": "123456789", "tags": {"title": "Example"}},
        "chapters": [{"id": 0}, {"id": 1}]
    })
}

fn settings_fixture() -> Value {
    json!({
        "video": {"preset": "slow", "promote_8bit_to_10bit": false},
        "resources": {
            "threading": "auto", "reserved_cpu_threads": 1, "max_worker_threads": 0,
            "frame_threads": 0, "min_free_memory_gib": 2.0, "memory_reserve_percent": 8.0
        }
    })
}

#[test]
fn discovery_matches_extensions_and_non_recursive_season_layout() {
    let root = temp_dir("discovery");
    let season = root.join("Show").join("Season 01");
    fs::create_dir_all(&season).unwrap();
    fs::write(root.join("Film.MKV"), b"mkv").unwrap();
    fs::write(root.join("Test_ignored.mkv"), b"test").unwrap();
    fs::write(root.join("backup.hevc-rebuild.tmp.mkv"), b"temp").unwrap();
    fs::write(season.join("Show.S01E01.mkv"), b"episode").unwrap();
    fs::write(root.join("Show").join("outside.mp4"), b"not in season").unwrap();
    let found = discover_folder(&root, false, 0, &[], None).files;
    let names = found
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Film.MKV", "Show.S01E01.mkv"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recursive_discovery_applies_removable_folder_exclusions_and_is_sorted() {
    let root = temp_dir("recursive");
    fs::create_dir_all(root.join(".hidden")).unwrap();
    fs::create_dir_all(root.join("old")).unwrap();
    fs::create_dir_all(root.join("Season 01")).unwrap();
    fs::write(root.join("zeta.mkv"), b"z").unwrap();
    fs::write(root.join("Alpha.mp4"), b"a").unwrap();
    fs::write(root.join(".hidden").join("private.mkv"), b"x").unwrap();
    fs::write(root.join("old").join("backup.mkv"), b"x").unwrap();
    fs::write(root.join("Season 01").join("episode.mkv"), b"e").unwrap();
    let found = discover_folder(&root, true, 0, &["old".into()], None).files;
    let relative = found
        .iter()
        .map(|path| {
            // Compare with forward slashes so the expectation holds on Windows too.
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        relative,
        vec![
            ".hidden/private.mkv",
            "Alpha.mp4",
            "Season 01/episode.mkv",
            "zeta.mkv"
        ]
    );
    let without_user_exclusions = discover_folder(&root, true, 0, &[], None).files;
    assert!(without_user_exclusions
        .iter()
        .any(|path| path.ends_with(".hidden/private.mkv")));
    let hidden_excluded = discover_folder(&root, true, 0, &[".*".into()], None).files;
    assert!(!hidden_excluded
        .iter()
        .any(|path| path.ends_with(".hidden/private.mkv")));
    assert!(!filename_matches_filters(
        "trailer.mkv",
        &[],
        &["trailer".into()]
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_prefers_highest_resolution_and_surfaces_all_probe_stream_facts() {
    let probe = profile_fixture();
    let mediainfo = json!({
        "media": {"track": [
            {"@type": "General", "Format": "Matroska", "CompleteName": "/private/source.mkv", "FileSize": "123456789", "Duration": "3600500", "Title": "Example"},
            {"@type": "Video", "StreamOrder": "0", "Format": "HEVC", "Format_Profile": "Main 10", "CodecID": "V_MPEGH/ISO/HEVC", "Width": "3 840 pixels", "Height": "2 160 pixels", "BitDepth": "10 bits", "ChromaSubsampling": "4:2:0", "FrameRate": "23.976 FPS", "HDR_Format": "Dolby Vision, Version 1.0, Profile 8.1, dvhe.08.06", "MaxCLL": "1000 nits", "MaxFALL": "400 nits", "Language": "und", "Title": "4K"},
            {"@type": "Video", "StreamOrder": "1", "Format": "AVC", "Width": "1920 pixels", "Height": "1080 pixels", "BitDepth": "8 bits", "Language": "eng", "Title": "1080p"},
            {"@type": "Audio", "StreamOrder": "2", "Format": "AC-3", "Channels": "6", "ChannelLayout": "L R C LFE Ls Rs", "SamplingRate": "48.0 kHz", "Language": "eng"},
            {"@type": "Text", "StreamOrder": "3", "Format": "UTF-8", "Language": "eng"}
        ]}
    });
    let path = Path::new("fixture.mkv");
    let profile = build_profile(
        &probe,
        &mediainfo,
        path,
        &settings_fixture(),
        "/missing/ffprobe",
    )
    .unwrap();
    assert_eq!(profile.payload["video"]["width"], 3840);
    assert_eq!(profile.payload["video"]["secondary_count"], 1);
    assert_eq!(
        profile.payload["video"]["target_pixel_format"],
        "yuv420p10le"
    );
    assert_eq!(profile.payload["audio_count"], 1);
    assert_eq!(profile.payload["subtitle_count"], 1);
    assert_eq!(profile.payload["attachment_count"], 1);
    assert_eq!(profile.payload["chapters_count"], 2);
    assert_eq!(profile.payload["hdr"]["dolby_vision_profile"], 8);
    assert_eq!(profile.payload["hdr"]["dolby_vision_level"], 6);
    assert_eq!(profile.payload["hdr"]["dolby_vision_rpu"], true);
    assert_eq!(profile.payload["hdr"]["hdr10plus_hint"], true);
    assert_eq!(profile.payload["max_cll"], 1000);
    assert_eq!(profile.payload["max_fall"], 400);
    assert!(profile.payload["mediainfo_general"]
        .get("CompleteName")
        .is_none());
    assert_eq!(
        profile.payload["tracks"][0]["ffprobe"]["tags"]["title"],
        "4K"
    );
    assert_eq!(profile.payload["tracks"][2]["channel_layout"], "5.1(side)");
    assert_eq!(profile.payload["tracks"][4]["filename"], "font.ttf");
    assert_eq!(profile.payload["tracks"][5]["attached_picture"], true);
}

#[test]
fn profile_can_target_the_second_video_stream_for_output_validation() {
    let probe = profile_fixture();
    let profile = build_profile_for_video(
        &probe,
        &json!({}),
        Path::new("fixture.mkv"),
        &settings_fixture(),
        "/missing/ffprobe",
        Some(1),
    )
    .unwrap();
    assert_eq!(profile.payload["video"]["width"], 1920);
    assert_eq!(profile.payload["color_primaries"], Value::Null);
    assert_eq!(profile.payload["hdr"]["hdr10"], false);
    assert_eq!(profile.payload["hdr"]["dolby_vision"], false);
}

#[test]
fn mediainfo_video_match_handles_spaced_dimensions_and_codec_ties() {
    let tracks = vec![
        json!({"@type":"Video", "Width":"1920 pixels", "Height":"1080 pixels", "Format":"AVC"}),
        json!({"@type":"Video", "Width":"3 840 pixels", "Height":"2 160 pixels", "Format":"AVC"}),
        json!({"@type":"Video", "Width":"3840 pixels", "Height":"2160 pixels", "Format":"HEVC"}),
    ];
    let primary = json!({"index":0, "codec_name":"hevc"});
    let selected = match_mediainfo_video(&tracks, &primary, 0, 3840, 2160).unwrap();
    assert_eq!(selected["Format"], "HEVC");
    assert_eq!(media_dimensions(selected), (3840, 2160));
}

#[test]
fn profile_uses_mediainfo_hdr_fallbacks_and_ignores_unknown_color_labels() {
    let mut probe = profile_fixture();
    let primary = probe["streams"][0].as_object_mut().unwrap();
    primary.remove("color_primaries");
    primary.remove("color_transfer");
    primary.remove("color_space");
    primary.remove("chroma_location");
    primary.insert("side_data_list".into(), json!([]));
    let mediainfo = json!({
        "media": {"track": [{
            "@type":"Video", "StreamOrder":"0",
            "colour_primaries":"unspecified",
            "transfer_characteristics":"unknown",
            "matrix_coefficients":"unspecified",
            "MasteringDisplay_ColorPrimaries":"BT.2020",
            "MasteringDisplay_Luminance":"min: 0.005 cd/m2, max: 1000 cd/m2",
            "ChromaSubsampling_Position":"2 (top-left)",
            "MaxCLL":"1000.8 nits", "MaxFALL":"400.5 nits"
        }]}
    });
    let profile = build_profile(
        &probe,
        &mediainfo,
        Path::new("fixture.mkv"),
        &settings_fixture(),
        "/missing/ffprobe",
    )
    .unwrap();
    assert_eq!(
        profile.payload["mastering_display_source"],
        "MediaInfo-label-fallback"
    );
    assert_eq!(
        profile.payload["mastering_display"],
        "G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,50)"
    );
    assert_eq!(profile.payload["chroma_location"], 2);
    assert_eq!(profile.payload["max_cll"], 1001);
    assert_eq!(profile.payload["max_fall"], 400);
    assert!(profile.payload["color_primaries"].is_null());
    assert!(profile.payload["color_transfer"].is_null());
    assert!(profile.payload["color_matrix"].is_null());
}

#[test]
fn frame_profile_helpers_include_all_sampled_frames() {
    let frame = json!({"frames": [
        {"interlaced_frame":0, "side_data_list":[]},
        {"interlaced_frame":1, "side_data_list":[{"side_data_type":"HDR Dynamic Metadata SMPTE2094-40"}]}
    ]});
    assert_eq!(frame_side_data(Some(&frame)).len(), 1);
    assert!(frames_are_interlaced(Some(&frame)));
}

#[test]
fn youtube_estimates_preserve_the_source_size_budget_and_never_downscale() {
    let profile = json!({
        "video": {"width": 1280, "height": 720},
        "hdr": {"hdr10": false, "hlg": false, "dolby_vision": false},
        "audio_count": 1,
        "primary_stream_index": 0,
        "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "24/1", "side_data_list": []}}]
    });
    assert_eq!(
        crate::youtube::expected_dimensions(&profile, "1440p").unwrap(),
        (2560, 1440)
    );
    let estimate =
        crate::youtube::estimate(&profile, 80_000_000, 3600.0, "1440p", "balanced", 2.0).unwrap();
    assert_eq!(estimate["video_bitrate_kbps"], 250);
    assert!(estimate["estimated_output_bytes"].as_u64().unwrap() < 1_000_000_000);
    let larger = json!({
        "video": {"width": 3840, "height": 2160},
        "hdr": {"hdr10": true, "hlg": false, "dolby_vision": false},
        "source_bit_depth": 10,
        "color_primaries": "bt2020",
        "color_transfer": "smpte2084",
        "color_matrix": "bt2020nc",
        "primary_stream_index": 0,
        "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "60/1"}}]
    });
    assert_eq!(
        crate::youtube::expected_dimensions(&larger, "1440p").unwrap(),
        (3840, 2160)
    );
    assert_eq!(
        crate::youtube::estimate(&larger, 400_000_000, 600.0, "4k", "recommended", 2.0).unwrap()
            ["recommended_video_bitrate_kbps"],
        75_500
    );
}

#[test]
fn memory_forecast_uses_ram_and_pressure_but_never_counts_swap_as_headroom() {
    let settings = settings_fixture();
    let host = json!({"hardware": {
        "logical_cpus": 16, "memory_total_gib": 40.0, "memory_available_gib": 12.0,
        "swap_free_gib": 8.0, "swap_used_gib": 8.0, "swap_used_percent": 50.0
    }});
    let profile = json!({
        "video": {"width": 3840, "height": 2160, "source_bit_depth": 10, "source_chroma": "4:2:0", "target_pixel_format": "yuv420p10le"},
        "primary_stream_index": 0, "hdr": {},
        "tracks": [{"index": 0, "ffprobe": {"avg_frame_rate": "24/1"}}]
    });
    let forecast = memory_forecast(MemoryForecastInput {
        profile: &profile,
        settings: &settings,
        host: &host,
        full_memory_psi: Some(0.0),
        target_dimensions: None,
        preset_override: None,
        frame_threads_override: None,
        concurrent_video_streams: 1,
    });
    assert_eq!(forecast["level"], "constrained");
    assert_eq!(forecast["swap_free_gib"], 8.0);
    assert_eq!(forecast["preset_factor"], 1.0);
    assert!(forecast["planning_budget_gib"].as_f64().unwrap() < 12.0);
    assert!(forecast["planned_workers"].as_i64().unwrap() < 15);
    let pressured = memory_forecast(MemoryForecastInput {
        profile: &profile,
        settings: &settings,
        host: &host,
        full_memory_psi: Some(1.2),
        target_dimensions: None,
        preset_override: None,
        frame_threads_override: None,
        concurrent_video_streams: 1,
    });
    assert!(
        pressured["planning_budget_gib"].as_f64().unwrap()
            < forecast["planning_budget_gib"].as_f64().unwrap()
    );
}

#[test]
fn memory_forecast_blocks_below_the_dynamic_reserve_even_with_free_swap() {
    let settings = settings_fixture();
    let host = json!({"hardware": {
        "logical_cpus": 16, "memory_total_gib": 40.0, "memory_available_gib": 2.0,
        "swap_free_gib": 16.0, "swap_used_gib": 0.0, "swap_used_percent": 0.0
    }});
    let profile = json!({"video":{"width":3840,"height":2160,"source_bit_depth":10,"source_chroma":"4:2:0","target_pixel_format":"yuv420p10le"}});
    let forecast = memory_forecast(MemoryForecastInput {
        profile: &profile,
        settings: &settings,
        host: &host,
        full_memory_psi: Some(0.0),
        target_dimensions: None,
        preset_override: None,
        frame_threads_override: None,
        concurrent_video_streams: 1,
    });
    assert_eq!(forecast["level"], "blocked");
    assert!(forecast["planned_workers"].is_null());
}

#[test]
fn scan_stops_after_the_current_file_when_requested() {
    let root = temp_dir("scan-stop");
    fs::write(root.join("a.mkv"), b"not-media").unwrap();
    fs::write(root.join("b.mkv"), b"not-media").unwrap();
    let token = crate::stop_control::StopToken::new().unwrap();
    let stop_path = token.path().to_path_buf();
    let payload = scan_media(
        ScanOptions {
            root: root.to_string_lossy().into(),
            recursive: false,
            minimum_size_mib: 0,
            filename_include_patterns: vec![],
            filename_exclude_patterns: vec![],
            tool_paths: ToolPaths::default(),
            stop_file: Some(stop_path.clone()),
        },
        |current, _, _| {
            if current == 1 {
                crate::stop_control::request(&stop_path).unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(payload["interrupted"], true);
    assert_eq!(payload["entries"].as_array().unwrap().len(), 1);
    assert!(payload["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning
            .as_str()
            .is_some_and(|text| text.contains("Scan stopped"))));
    fs::remove_dir_all(root).unwrap();
}
