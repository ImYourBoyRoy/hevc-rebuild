//! Tests for YouTube run options, output naming, source policy and the end-to-end exports.

use super::*;

fn sdr_profile(width: i64, height: i64) -> Value {
    json!({
        "video": {"codec": "h264", "width": width, "height": height},
        "hdr": {"hdr10": false, "hdr10plus_hint": false, "hlg": false, "dolby_vision": false},
        "audio_count": 1, "subtitle_count": 0, "duration_seconds": 60.0
    })
}

#[test]
fn colliding_stems_get_extension_suffixes_and_unique_ones_do_not() {
    let mkv = Path::new("Show/Movie.mkv");
    let mp4 = Path::new("Show/Movie.MP4");
    assert_eq!(stem_key(mkv), stem_key(mp4));
    assert_eq!(
        output_relative_path(mkv, true),
        PathBuf::from("Show/Movie [mkv].mp4")
    );
    assert_eq!(
        output_relative_path(mp4, true),
        PathBuf::from("Show/Movie [mp4].mp4")
    );
    assert_eq!(
        output_relative_path(Path::new("Movie.2020.mkv"), false),
        PathBuf::from("Movie.2020.mp4")
    );
}

#[test]
fn sdr_sources_are_always_exportable_without_consent() {
    let policy = source_policy(&sdr_profile(1920, 1080), "1440p", &[], false, false).unwrap();
    assert_eq!(policy.color_mode, "sdr");
    assert!(policy.note.is_none());
}

fn hdr_profile() -> Value {
    json!({
        "video": {"codec": "hevc", "width": 3840, "height": 2160, "source_bit_depth": 10},
        "hdr": {"hdr10": true, "hdr10plus_hint": true, "hlg": false, "dolby_vision": false},
        "audio_count": 1, "duration_seconds": 600.0, "source_bit_depth": 10,
        "color_primaries": "bt2020", "transfer": "smpte2084", "color_matrix": "bt2020nc"
    })
}

#[test]
fn dolby_vision_skips_even_with_consent() {
    let mut dovi = hdr_profile();
    dovi["hdr"]["dolby_vision"] = json!(true);
    let message = source_policy(&dovi, "4k", &["yuv420p10le".into()], true, false)
        .err()
        .unwrap();
    assert!(message.contains("Dolby Vision"));
}

#[test]
fn hdr10plus_loss_requires_per_file_consent_and_leaves_a_note() {
    let formats = vec!["yuv420p10le".to_string()];
    let declined = source_policy(&hdr_profile(), "4k", &formats, false, false)
        .err()
        .unwrap();
    assert!(declined.contains("HDR10+ dynamic metadata cannot be retained"));
    let accepted = source_policy(&hdr_profile(), "4k", &formats, true, false).unwrap();
    assert_eq!(accepted.color_mode, "hdr10");
    assert!(accepted
        .note
        .unwrap()
        .contains("explicit per-file approval"));
    let mut plain_hdr = hdr_profile();
    plain_hdr["hdr"]["hdr10plus_hint"] = json!(false);
    let policy = source_policy(&plain_hdr, "4k", &formats, false, false).unwrap();
    assert!(policy.note.is_none());
}

#[test]
fn hdr10plus_inject_skips_loss_consent() {
    let formats = vec!["yuv420p10le".to_string()];
    let policy = source_policy(&hdr_profile(), "4k", &formats, false, true).unwrap();
    assert_eq!(policy.color_mode, "hdr10");
    assert!(policy.note.unwrap().contains("hdr10plus_tool inject"));
}

#[test]
fn hdr_requires_ten_bit_x265_and_at_least_720p() {
    let message = source_policy(&hdr_profile(), "4k", &[], true, false)
        .err()
        .unwrap();
    assert!(message.contains("10-bit libx265"));
    let mut small = hdr_profile();
    small["video"]["width"] = json!(640);
    small["video"]["height"] = json!(360);
    let message = source_policy(&small, "source", &["yuv420p10le".into()], true, false)
        .err()
        .unwrap();
    assert!(message.contains("below 720p"));
    let mut untagged = hdr_profile();
    untagged["transfer"] = json!("unknown");
    untagged["hdr"]["hdr10"] = json!(true);
    let message = source_policy(&untagged, "4k", &["yuv420p10le".into()], true, false)
        .err()
        .unwrap();
    assert!(message.contains("PQ or HLG"));
}

#[test]
fn engine_arguments_parse_options_and_ignore_settings_noise() {
    let dir = std::env::temp_dir().join(format!(
        "hevc-youtube-args-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("clip.mkv");
    fs::write(&file, b"x").unwrap();
    let root = dir.to_string_lossy().into_owned();
    let arguments = |extra: &[&str]| -> Vec<String> {
        let mut args = vec![
            root.clone(),
            "--min-size-mib".into(),
            "0".into(),
            "--no-dependency-repair".into(),
            "--skip-update-check".into(),
            "--recursive".into(),
            "--youtube-upload".into(),
            "--upload-resolution".into(),
            "4k".into(),
            "--upload-quality-mode".into(),
            "recommended".into(),
            "--upload-source-multiplier".into(),
            "1.5".into(),
            "--crf".into(),
            "20".into(),
            "--no-embed-artwork".into(),
            "--min-free-disk-gib".into(),
            "2".into(),
            "--reserved-cpu-threads".into(),
            "2".into(),
            "--allow-youtube-hdr10plus-loss".into(),
            file.to_string_lossy().into_owned(),
        ];
        args.extend(extra.iter().map(|value| value.to_string()));
        args
    };
    let options = YoutubeRunOptions::from_engine_args(&arguments(&[])).unwrap();
    assert_eq!(options.resolution, "4k");
    assert_eq!(options.quality_mode, "recommended");
    assert_eq!(options.source_multiplier, 1.5);
    assert!(options.recursive);
    assert_eq!(options.hdr10plus_loss.len(), 1);
    assert_eq!(options.disk_reserve_bytes(), (2.0 * GIB) as u64);
    assert_eq!(
        options.settings.pointer("/resources/reserved_cpu_threads"),
        Some(&json!(2))
    );
    assert!(YoutubeRunOptions::from_engine_args(&arguments(&["--surprise"])).is_err());
    assert!(YoutubeRunOptions::from_engine_args(&arguments(&[
        "--file-stream-overrides-json",
        "{}"
    ]))
    .is_err());
    assert!(
        YoutubeRunOptions::from_engine_args(&arguments(&["--upload-source-multiplier", "9"]))
            .is_err()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn closing_lines_expose_log_report_and_output_folder() {
    let job = YoutubeJobResult {
        files: vec![YoutubeFileResult {
            path: "/m/a.mkv".into(),
            status: "OK".into(),
            message: String::new(),
            output: None,
            elapsed_seconds: 1.0,
        }],
        log_path: "/logs/a.log".into(),
        report_path: "/logs/a.csv".into(),
        destination_root: "/m/YouTube Uploads".into(),
        stop_requested: false,
        failed: 0,
        succeeded: 1,
        skipped: 0,
    };
    let lines = closing_lines(&job);
    assert_eq!(lines[0], "Log: /logs/a.log");
    assert_eq!(lines[1], "Run report: /logs/a.csv");
    assert!(lines[2].contains("1 validated copy"));
    assert_eq!(lines[3], "Output folder: /m/YouTube Uploads");
}

#[test]
fn temporary_names_are_hidden_unique_mp4_siblings() {
    let destination = Path::new("/out/Movie.mp4");
    let first = temporary_name(destination);
    let second = temporary_name(destination);
    assert_ne!(first, second);
    assert_eq!(first.parent(), destination.parent());
    let text = first.file_name().unwrap().to_string_lossy();
    assert!(text.starts_with(".Movie.upload-") && text.ends_with(".tmp.mp4"));
}

/// Fresh canonical media root and state directory; also points the engine's state there.
fn scratch_dirs() -> (PathBuf, PathBuf) {
    let token = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    );
    let root = std::env::temp_dir().join(format!("hevc-youtube-run-{token}"));
    let state_dir = std::env::temp_dir().join(format!("hevc-youtube-state-{token}"));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&state_dir).unwrap();
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    (root.canonicalize().unwrap(), state_dir)
}

fn tools_present() -> bool {
    crate::media_tools_ready(
        "native YouTube integration",
        &["ffmpeg", "ffprobe", "mediainfo"],
    )
}

/// 2560x1440 H.264 + AAC fixture; `kbps` forces a constant (padded) video bitrate.
fn make_1440p_clip(path: &Path, kbps: u32) -> bool {
    let rate = format!("{kbps}k");
    std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=2560x1440:rate=24:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-b:v",
            &rate,
            "-minrate",
            &rate,
            "-maxrate",
            &rate,
            "-bufsize",
            &rate,
            "-x264-params",
            "nal-hrd=cbr:force-cfr=1",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// End to end on a synthetic SDR clip: needs ffmpeg (libx264 + aac), ffprobe, and
/// MediaInfo; skipped with a note on machines without them.
#[test]
fn synthetic_sdr_clip_exports_a_validated_youtube_copy() {
    if !tools_present() {
        return;
    }
    let _guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (root, state_dir) = scratch_dirs();
    let source = root.join("Clip One (2024).mkv");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=640x360:rate=24:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(&source)
        .status();
    if !made.is_ok_and(|status| status.success()) {
        eprintln!("skipping native YouTube integration: could not build the fixture");
        let _ = fs::remove_dir_all(&root);
        return;
    }
    let before = fs::read(&source).unwrap();
    let args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        "0".into(),
        "--youtube-upload".into(),
        "--upload-resolution".into(),
        "source".into(),
    ];
    let options = YoutubeRunOptions::from_engine_args(&args).unwrap();
    let mut phases = Vec::new();
    let mut saw_progress_percent = false;
    let result = run_youtube_export(
        &options,
        |progress| {
            saw_progress_percent |= progress.file_percent.is_some();
            phases.push(progress.phase);
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1, "{:?}", result.files);
    let output = PathBuf::from(result.files[0].output.clone().unwrap());
    assert!(output.is_file(), "missing {}", output.display());
    assert_eq!(
        output.extension().and_then(|value| value.to_str()),
        Some("mp4")
    );
    assert_eq!(
        fs::read(&source).unwrap(),
        before,
        "source must stay untouched"
    );
    assert!(!phases.is_empty());
    assert!(saw_progress_percent, "encode progress was never reported");
    let leftovers: Vec<_> = fs::read_dir(output.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary files left behind: {leftovers:?}"
    );
    let report = fs::read_to_string(&result.report_path).unwrap();
    assert!(report.contains("Clip One (2024).mkv"), "{report}");
    assert!(report.contains("OK"), "{report}");
    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// Optimize mode on two 1440p clips: a healthy one must be stream-copied with
/// YouTube tags (video packets untouched), an oversized one re-encoded smaller.
#[test]
fn optimize_copies_healthy_clips_and_shrinks_oversized_ones() {
    if !tools_present() {
        return;
    }
    let _guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (root, state_dir) = scratch_dirs();
    let healthy = root.join("Healthy.mkv");
    let huge = root.join("Huge.mkv");
    if !make_1440p_clip(&healthy, 6_000) || !make_1440p_clip(&huge, 60_000) {
        eprintln!("skipping native YouTube optimize integration: could not build fixtures");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&state_dir);
        return;
    }
    let before = (fs::read(&healthy).unwrap(), fs::read(&huge).unwrap());
    let args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        "0".into(),
        "--youtube-upload".into(),
        "--youtube-mode".into(),
        "optimize".into(),
        "--youtube-preset".into(),
        "medium".into(),
        "--max-worker-threads".into(),
        "1".into(),
        "--min-free-memory-gib".into(),
        "0".into(),
        "--memory-reserve-percent".into(),
        "0".into(),
        "--disk-projection-margin-percent".into(),
        "0".into(),
    ];
    let options = YoutubeRunOptions::from_engine_args(&args).unwrap();
    let result = run_youtube_export(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 2, "{:?}", result.files);
    let find = |needle: &str| {
        result
            .files
            .iter()
            .find(|file| file.path.contains(needle))
            .unwrap_or_else(|| panic!("no result for {needle}: {:?}", result.files))
    };
    let copied = find("Healthy");
    assert!(
        copied.message.contains("copied untouched"),
        "{}",
        copied.message
    );
    assert!(
        copied.message.contains("packets verified identical"),
        "{}",
        copied.message
    );
    let shrunk = find("Huge");
    assert!(shrunk.message.contains("re-encoded"), "{}", shrunk.message);
    // The saved CSV must carry the Optimize decision for each file.
    let saved = crate::reports::read_report(
        &crate::state_log_directory().unwrap(),
        Path::new(&result.report_path),
    )
    .unwrap();
    let saved_rows = saved["rows"].as_array().unwrap();
    let saved_row = |needle: &str| {
        saved_rows
            .iter()
            .find(|row| {
                row["input"]
                    .as_str()
                    .is_some_and(|path| path.contains(needle))
            })
            .unwrap_or_else(|| panic!("no report row for {needle}: {saved_rows:?}"))
    };
    let healthy_row = saved_row("Healthy");
    assert_eq!(healthy_row["upload_mode"], "optimize");
    assert_eq!(healthy_row["optimize_action"], "copy");
    // Booleans keep the report schema's established "True"/"False" spelling.
    assert_eq!(healthy_row["video_payload_identical"], "True");
    assert!(
        !healthy_row["optimize_reason"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "copy rows must say why the stream was kept: {healthy_row:?}"
    );
    let huge_row = saved_row("Huge");
    assert_eq!(huge_row["optimize_action"], "encode");
    let ssim: f64 = huge_row["quality_ssim_min"]
        .as_str()
        .unwrap_or_else(|| panic!("encode rows must record the SSIM value: {huge_row:?}"))
        .parse()
        .unwrap();
    assert!((0.0..=1.0).contains(&ssim), "SSIM out of range: {ssim}");
    let huge_out = PathBuf::from(shrunk.output.clone().unwrap());
    assert!(huge_out.is_file());
    assert!(
        fs::metadata(&huge_out).unwrap().len() < before.1.len() as u64 / 2,
        "the oversized clip should shrink by more than half"
    );
    assert_eq!(
        fs::read(&healthy).unwrap(),
        before.0,
        "source must stay untouched"
    );
    assert_eq!(
        fs::read(&huge).unwrap(),
        before.1,
        "source must stay untouched"
    );
    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}
