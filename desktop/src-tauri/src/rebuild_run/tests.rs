//! Tests for rebuild option parsing, halt policy, stream selection and the end-to-end short test and full rebuild.

use super::*;

fn base_args(extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        std::env::temp_dir().to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        "5".into(),
        "--no-dependency-repair".into(),
        "--skip-update-check".into(),
        "--crf".into(),
        "22".into(),
        "--preset".into(),
        "slow".into(),
        "--tune".into(),
        "auto".into(),
        "--detect-animation".into(),
        "--preserve-hdr10plus".into(),
        "--dolby-vision-policy".into(),
        "safe".into(),
        "--secondary-video".into(),
        "encode".into(),
        "--data-streams".into(),
        "preserve".into(),
        "--verify-copy-hashes".into(),
        "--resource-threading".into(),
        "auto".into(),
        "--reserved-cpu-threads".into(),
        "1".into(),
        "--max-worker-threads".into(),
        "0".into(),
        "--frame-threads".into(),
        "0".into(),
        "--min-free-memory-gib".into(),
        "3".into(),
        "--memory-reserve-percent".into(),
        "8".into(),
        "--min-free-disk-gib".into(),
        "0".into(),
        "--preflight-output-ratio".into(),
        "1.25".into(),
        "--disk-projection-margin-percent".into(),
        "15".into(),
        "--calibration-mode".into(),
        "auto".into(),
        "--calibration-cache-hours".into(),
        "168".into(),
        "--calibration-large-file-gib".into(),
        "20".into(),
        "--calibration-sample-seconds".into(),
        "4".into(),
        "--calibration-max-candidates".into(),
        "3".into(),
        "--calibration-tolerance-percent".into(),
        "5".into(),
        "--comment".into(),
        "".into(),
        "--embed-artwork".into(),
        "--replace-metadata-text-blacklist".into(),
    ];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    args
}

#[test]
fn backup_location_preflight_rejects_file_and_symlink_old_entries() {
    let root = std::env::temp_dir().join(format!(
        "hevc-rebuild-backup-preflight-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let source = root.join("movie.mp4");
    fs::write(&source, b"original").unwrap();

    // No backup folder yet: the commit will create it.
    preflight_backup_location(&source, false).unwrap();

    // A plain file named old blocks the backup.
    fs::write(
        root.join(crate::backup_folder::BACKUP_FOLDER),
        b"not a folder",
    )
    .unwrap();
    assert!(preflight_backup_location(&source, false)
        .unwrap_err()
        .contains("must be a real folder"));
    fs::remove_file(root.join(crate::backup_folder::BACKUP_FOLDER)).unwrap();

    // A symlinked backup folder is refused even when it points at a real folder.
    #[cfg(unix)]
    {
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join(crate::backup_folder::BACKUP_FOLDER))
            .unwrap();
        assert!(preflight_backup_location(&source, false).is_err());
        fs::remove_file(root.join(crate::backup_folder::BACKUP_FOLDER)).unwrap();
    }

    // A real backup folder is accepted, including with a name collision when
    // reprocessing allocates a numbered backup.
    fs::create_dir(root.join(crate::backup_folder::BACKUP_FOLDER)).unwrap();
    fs::write(
        root.join(crate::backup_folder::BACKUP_FOLDER)
            .join("movie.mp4"),
        b"earlier backup",
    )
    .unwrap();
    preflight_backup_location(&source, true).unwrap();
    fs::remove_dir_all(&root).unwrap();
}

// ---- End-to-end tests on a synthetic clip (real ffmpeg/MKVToolNix) ----

const MEDIA_TOOLS: [&str; 6] = [
    "ffmpeg",
    "ffprobe",
    "mkvmerge",
    "mkvpropedit",
    "mkvextract",
    "mediainfo",
];

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hevc-rebuild-run-{label}-{}", unique_token()));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// A 3 second 24 fps H.264 + AAC Matroska clip: not HEVC, so a rebuild must
/// really re-encode it.
fn make_h264_clip(root: &Path, file_name: &str) -> Option<PathBuf> {
    let source = root.join(file_name);
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=24:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(&source)
        .status();
    made.is_ok_and(|status| status.success()).then_some(source)
}

fn rebuild_args(root: &Path, extra: &[&str]) -> Vec<String> {
    let mut args = base_args(&[
        "--min-size-mib",
        "0",
        "--preset",
        "ultrafast",
        "--crf",
        "30",
        "--calibration-mode",
        "off",
        "--min-free-memory-gib",
        "0",
        // The end-to-end tests below encode at a deliberately poor CRF to stay
        // fast; the quality gate has its own tests and is off here unless a
        // test turns it on (a later flag overrides this one).
        "--quality-gate",
        "off",
    ]);
    args[0] = root.to_string_lossy().into_owned();
    args.extend(extra.iter().map(|value| (*value).to_string()));
    args
}

fn first_video_codec(path: &Path) -> String {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn visible_entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn full_rebuild_replaces_in_place_and_preserves_the_original() {
    if !crate::media_tools_ready("full rebuild end to end", &MEDIA_TOOLS) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("full");
    let source = make_h264_clip(&root, "Clip (2024).mkv").expect("build the H.264 fixture");
    let original = fs::read(&source).unwrap();
    assert_eq!(first_video_codec(&source), "h264");
    let original_packets = crate::media_scan::count_stream_packets("ffprobe", &source, 0)
        .expect("count the source video packets");

    let options = RebuildRunOptions::from_engine_args(&rebuild_args(&root, &[])).unwrap();
    let result = run_rebuild(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1, "{:?}", result.files);

    // The validated HEVC output now sits at the original path...
    assert_eq!(first_video_codec(&source), "hevc");
    assert_ne!(fs::read(&source).unwrap(), original);
    let rebuilt_packets = crate::media_scan::count_stream_packets("ffprobe", &source, 0)
        .expect("count the output video packets");
    assert_eq!(rebuilt_packets, original_packets, "no frames may be lost");
    // ...and the untouched original was preserved byte for byte in the originals folder.
    assert_eq!(
        fs::read(
            root.join(crate::backup_folder::BACKUP_FOLDER)
                .join("Clip (2024).mkv")
        )
        .unwrap(),
        original
    );
    // No temporary, quarantined (.error), or log leftovers beside the media.
    let leftovers: Vec<String> = visible_entries(&root)
        .into_iter()
        .filter(|name| name != "Clip (2024).mkv" && name != crate::backup_folder::BACKUP_FOLDER)
        .collect();
    assert!(leftovers.is_empty(), "unexpected leftovers: {leftovers:?}");
    let report = fs::read_to_string(&result.report_path).unwrap();
    assert!(report.contains("OK"), "{report}");

    // A second run sees the backup and leaves the finished file alone.
    let finished = fs::read(&source).unwrap();
    let again = RebuildRunOptions::from_engine_args(&rebuild_args(&root, &[])).unwrap();
    let second = run_rebuild(&again, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!((second.failed, second.succeeded, second.skipped), (0, 0, 1));
    assert_eq!(fs::read(&source).unwrap(), finished);

    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// Runs one full rebuild of a fresh H.264 clip with the given quality-gate flags
/// and hands back what the tests inspect.
struct GateRun {
    root: PathBuf,
    state_dir: PathBuf,
    source: PathBuf,
    original: Vec<u8>,
    result: RebuildJobResult,
}

fn rebuild_with_gate(label: &str, extra: &[&str]) -> Option<GateRun> {
    if !crate::media_tools_ready("quality gate end to end", &MEDIA_TOOLS) {
        return None;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch(&format!("{label}-state"));
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch(label);
    let source = make_h264_clip(&root, "Clip (2024).mkv").expect("build the H.264 fixture");
    let original = fs::read(&source).unwrap();
    let options = RebuildRunOptions::from_engine_args(&rebuild_args(&root, extra)).unwrap();
    let result = run_rebuild(&options, |_| {}, |_| {}, |_| {}).unwrap();
    Some(GateRun {
        root,
        state_dir,
        source,
        original,
        result,
    })
}

impl GateRun {
    fn clean_up(self) {
        for dir in [self.root, self.state_dir] {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn quality_gate_refuses_a_picture_below_the_floor_and_leaves_the_original_alone() {
    // A floor of 1.0 means "identical", which no lossy encode reaches: the
    // stand-in for an output that is visibly worse than the source.
    let Some(run) = rebuild_with_gate(
        "gate-enforce",
        &["--quality-gate", "enforce", "--quality-min-ssim", "1.0"],
    ) else {
        return;
    };
    assert_eq!(run.result.failed, 1, "{:?}", run.result.files);
    assert_eq!(run.result.succeeded, 0);
    let message = &run.result.files[0].message;
    assert!(
        message.contains("Picture quality check failed"),
        "{message}"
    );
    assert!(message.contains("original is untouched"), "{message}");
    assert_eq!(
        fs::read(&run.source).unwrap(),
        run.original,
        "source untouched"
    );
    assert!(
        !run.root.join(crate::backup_folder::BACKUP_FOLDER).exists(),
        "nothing was installed, so no backup was made"
    );
    run.clean_up();
}

#[test]
fn quality_gate_in_report_mode_installs_anyway_and_says_so() {
    let Some(run) = rebuild_with_gate(
        "gate-report",
        &["--quality-gate", "report", "--quality-min-ssim", "1.0"],
    ) else {
        return;
    };
    assert_eq!(
        (run.result.failed, run.result.succeeded),
        (0, 1),
        "{:?}",
        run.result.files
    );
    let message = &run.result.files[0].message;
    assert!(message.contains("report only"), "{message}");
    assert!(message.contains("SSIM"), "{message}");
    assert_eq!(first_video_codec(&run.source), "hevc");
    assert!(run
        .root
        .join(crate::backup_folder::BACKUP_FOLDER)
        .join("Clip (2024).mkv")
        .is_file());
    run.clean_up();
}

#[test]
fn quality_gate_passes_a_faithful_encode_and_reports_the_score() {
    // Near-lossless encode, so the picture matches and the floor is cleared.
    let Some(run) = rebuild_with_gate(
        "gate-pass",
        &[
            "--crf",
            "10",
            "--quality-gate",
            "enforce",
            "--quality-min-ssim",
            "0.95",
        ],
    ) else {
        return;
    };
    assert_eq!(
        (run.result.failed, run.result.succeeded),
        (0, 1),
        "{:?}",
        run.result.files
    );
    let message = &run.result.files[0].message;
    assert!(message.contains("Picture quality: SSIM"), "{message}");
    assert_eq!(first_video_codec(&run.source), "hevc");
    run.clean_up();
}

#[test]
fn quality_gate_off_measures_nothing() {
    let Some(run) = rebuild_with_gate("gate-off", &["--quality-gate", "off"]) else {
        return;
    };
    assert_eq!(
        (run.result.failed, run.result.succeeded),
        (0, 1),
        "{:?}",
        run.result.files
    );
    assert!(!run.result.files[0].message.contains("SSIM"));
    run.clean_up();
}

#[test]
fn quality_gate_options_parse_validate_and_default_to_enforce() {
    let defaults = RebuildRunOptions::from_engine_args(&base_args(&[])).unwrap();
    assert_eq!(
        defaults.quality_gate,
        crate::quality_gate::GateMode::Enforce
    );
    assert_eq!(
        defaults.quality_min_ssim,
        crate::quality_gate::DEFAULT_MIN_SSIM
    );

    let chosen = RebuildRunOptions::from_engine_args(&base_args(&[
        "--quality-gate",
        "report",
        "--quality-min-ssim",
        "0.9",
    ]))
    .unwrap();
    assert_eq!(chosen.quality_gate, crate::quality_gate::GateMode::Report);
    assert_eq!(chosen.quality_min_ssim, 0.9);

    for bad in [
        &["--quality-gate", "strict"][..],
        &["--quality-min-ssim", "0.2"][..],
        &["--quality-min-ssim", "high"][..],
    ] {
        assert!(
            RebuildRunOptions::from_engine_args(&base_args(bad)).is_err(),
            "{bad:?}"
        );
    }
}

/// A source Matroska cover must survive a rebuild as a `cover.jpg`
/// attachment, and must not appear as a second video track. FFmpeg exposes the
/// cover as an attached-picture video stream, so stream-copying it would turn
/// it into a real video track (found on real media).
#[test]
fn full_rebuild_keeps_an_embedded_cover_as_an_attachment_not_a_video_track() {
    if !crate::media_tools_ready("cover carry-over end to end", &MEDIA_TOOLS) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("cover-state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("cover");
    let plain = make_h264_clip(&root, "plain.mkv").expect("build the H.264 fixture");
    let cover_image = root.join("art.jpg");
    let source = root.join("Clip (2024).mkv");
    let ffmpeg = |args: &[&std::ffi::OsStr]| {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    use std::ffi::OsStr;
    assert!(ffmpeg(&[
        OsStr::new("-f"),
        OsStr::new("lavfi"),
        OsStr::new("-i"),
        OsStr::new("testsrc=size=240x360:rate=1"),
        OsStr::new("-frames:v"),
        OsStr::new("1"),
        cover_image.as_os_str(),
    ]));
    assert!(ffmpeg(&[
        OsStr::new("-i"),
        plain.as_os_str(),
        OsStr::new("-map"),
        OsStr::new("0"),
        OsStr::new("-c"),
        OsStr::new("copy"),
        OsStr::new("-attach"),
        cover_image.as_os_str(),
        OsStr::new("-metadata:s:t:0"),
        OsStr::new("mimetype=image/jpeg"),
        OsStr::new("-metadata:s:t:0"),
        OsStr::new("filename=cover.jpg"),
        source.as_os_str(),
    ]));
    fs::remove_file(&plain).unwrap();
    fs::remove_file(&cover_image).unwrap();
    let listing = |path: &Path| {
        let output = std::process::Command::new("mkvmerge")
            .arg("-i")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(
        listing(&source).contains("cover.jpg"),
        "fixture needs a cover"
    );

    let options = RebuildRunOptions::from_engine_args(&rebuild_args(&root, &[])).unwrap();
    let result = run_rebuild(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1, "{:?}", result.files);

    let rebuilt = listing(&source);
    assert!(
        rebuilt.contains("cover.jpg"),
        "the cover must be re-attached:\n{rebuilt}"
    );
    assert_eq!(
        rebuilt.matches("video (").count(),
        1,
        "the cover must not become a second video track:\n{rebuilt}"
    );
    assert_eq!(first_video_codec(&source), "hevc");
    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// An MP4 `covr` cover is invisible to `mkvextract`; it must still come
/// through the rebuild as a `cover.jpg` attachment on the new `.mkv`.
#[test]
fn full_rebuild_carries_an_mp4_cover_into_the_new_mkv() {
    if !crate::media_tools_ready("mp4 cover carry-over end to end", &MEDIA_TOOLS) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("mp4cover-state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("mp4cover");
    let plain = make_h264_clip(&root, "plain.mkv").expect("build the H.264 fixture");
    let cover_image = root.join("art.jpg");
    let source = root.join("Clip (2024).mp4");
    use std::ffi::OsStr;
    let ffmpeg = |args: &[&OsStr]| {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    assert!(ffmpeg(&[
        OsStr::new("-f"),
        OsStr::new("lavfi"),
        OsStr::new("-i"),
        OsStr::new("testsrc=size=240x360:rate=1"),
        OsStr::new("-frames:v"),
        OsStr::new("1"),
        cover_image.as_os_str(),
    ]));
    assert!(ffmpeg(&[
        OsStr::new("-i"),
        plain.as_os_str(),
        OsStr::new("-i"),
        cover_image.as_os_str(),
        OsStr::new("-map"),
        OsStr::new("0:v:0"),
        OsStr::new("-map"),
        OsStr::new("0:a:0"),
        OsStr::new("-map"),
        OsStr::new("1:v:0"),
        OsStr::new("-c"),
        OsStr::new("copy"),
        OsStr::new("-disposition:v:1"),
        OsStr::new("attached_pic"),
        OsStr::new("-metadata:s:a:0"),
        OsStr::new("language=eng"),
        source.as_os_str(),
    ]));
    fs::remove_file(&plain).unwrap();
    fs::remove_file(&cover_image).unwrap();

    let options = RebuildRunOptions::from_engine_args(&rebuild_args(&root, &[])).unwrap();
    let result = run_rebuild(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1, "{:?}", result.files);

    let rebuilt = root.join("Clip (2024).mkv");
    assert!(rebuilt.is_file(), "{:?}", visible_entries(&root));
    let listing = String::from_utf8_lossy(
        &std::process::Command::new("mkvmerge")
            .arg("-i")
            .arg(&rebuilt)
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert!(listing.contains("cover.jpg"), "cover missing:\n{listing}");
    assert_eq!(listing.matches("video (").count(), 1, "{listing}");
    assert_eq!(first_video_codec(&rebuilt), "hevc");
    assert!(root
        .join(crate::backup_folder::BACKUP_FOLDER)
        .join("Clip (2024).mp4")
        .is_file());
    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn short_test_writes_a_separate_clip_and_never_touches_the_source() {
    if !crate::media_tools_ready("short test end to end", &MEDIA_TOOLS) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("test-state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("short");
    let source = make_h264_clip(&root, "Clip (2024).mkv").expect("build the H.264 fixture");
    let original = fs::read(&source).unwrap();

    let options = RebuildRunOptions::from_engine_args(&rebuild_args(&root, &["--test"])).unwrap();
    let result = run_rebuild(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1, "{:?}", result.files);

    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "source must be untouched"
    );
    assert!(
        !root.join(crate::backup_folder::BACKUP_FOLDER).exists(),
        "a test run makes no backup"
    );
    let test_clip = root.join("Test_Clip (2024).mkv");
    assert!(test_clip.is_file(), "{:?}", visible_entries(&root));
    assert_eq!(first_video_codec(&test_clip), "hevc");

    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn status_and_phase_text_is_stable_because_reports_and_logs_embed_it() {
    for (status, text) in [
        (RunStatus::Ok, "OK"),
        (RunStatus::TestOk, "TEST_OK"),
        (RunStatus::Info, "INFO"),
        (RunStatus::Skipped, "SKIPPED"),
        (RunStatus::Failed, "FAILED"),
        (RunStatus::Interrupted, "INTERRUPTED"),
    ] {
        assert_eq!(status.as_str(), text);
        assert_eq!(RunStatus::parse(text), Some(status));
    }
    assert_eq!(RunStatus::parse("ok"), None);
    assert_eq!(RunStatus::parse(""), None);
    let success: Vec<_> = [
        RunStatus::Ok,
        RunStatus::TestOk,
        RunStatus::Info,
        RunStatus::Skipped,
        RunStatus::Failed,
        RunStatus::Interrupted,
    ]
    .into_iter()
    .filter(|status| status.is_success())
    .collect();
    assert_eq!(
        success,
        [
            RunStatus::Ok,
            RunStatus::TestOk,
            RunStatus::Info,
            RunStatus::Skipped
        ]
    );

    let labels = [
        (Phase::MediaAnalysis, "media analysis", 0.0, false),
        (Phase::StoragePreflight, "storage preflight", 0.0, false),
        (
            Phase::Hdr10PlusExtraction,
            "HDR10+ metadata extraction",
            0.0,
            false,
        ),
        (
            Phase::HardwareCalibration,
            "hardware calibration",
            0.0,
            false,
        ),
        (Phase::VideoEncode, "video encode", 0.0, false),
        (Phase::Hdr10PlusInject, "HDR10+ metadata inject", 0.0, false),
        (Phase::MetadataEmbedding, "metadata embedding", 0.90, true),
        (Phase::OutputValidation, "output validation", 0.97, true),
        (
            Phase::ValidatedOutputInstall,
            "validated output install",
            0.99,
            true,
        ),
        (Phase::TransactionCommit, "transaction commit", 0.99, true),
    ];
    for (phase, label, fraction, finished) in labels {
        assert_eq!(phase.label(), label);
        assert_eq!(phase.failure_fraction(), fraction, "{label}");
        // Only phases after the encode completes may quarantine an output.
        assert_eq!(phase.holds_finished_output(), finished, "{label}");
    }
}

fn failed_state(phase: Phase, temp: Option<&Path>, final_output: Option<&Path>) -> FileState {
    let mut state = FileState {
        phase,
        temp_output: temp.map(Path::to_path_buf),
        final_output: final_output.map(Path::to_path_buf),
        row: Map::new(),
        interrupted: false,
    };
    state.set_status(RunStatus::Failed);
    state
        .row
        .insert("message".into(), json!("injected failure"));
    state
}

fn quiet_options(root: &Path) -> RebuildRunOptions {
    RebuildRunOptions::from_engine_args(&rebuild_args(root, &[])).unwrap()
}

#[test]
fn early_failure_discards_the_partial_temp_file_and_keeps_the_source() {
    let dir = scratch("early-fail");
    let source = dir.join("Film.mkv");
    let temp = dir.join(".Film.hevc-rebuild.tmp.mkv");
    fs::write(&source, b"source").unwrap();
    fs::write(&temp, b"partial encode").unwrap();
    let mut state = failed_state(Phase::VideoEncode, Some(&temp), Some(&source));
    release_temporary_output(&mut state, &source, None, &quiet_options(&dir));
    assert!(!temp.exists(), "a partial encode must not linger");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert!(visible_entries(&dir)
        .iter()
        .all(|name| !name.contains("error")));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn late_failure_keeps_the_finished_output_as_error_artifacts_and_never_overwrites() {
    let dir = scratch("late-fail");
    let source = dir.join("Film.mp4");
    let final_output = dir.join("Film.mkv");
    let temp = dir.join(".Film.hevc-rebuild.tmp.mkv");
    fs::write(&source, b"source").unwrap();
    fs::write(&temp, b"finished but unvalidated").unwrap();
    let mut state = failed_state(Phase::OutputValidation, Some(&temp), Some(&final_output));
    release_temporary_output(
        &mut state,
        &source,
        Some("duration mismatch"),
        &quiet_options(&dir),
    );
    let retained = dir.join("Film.error.mkv");
    assert!(!temp.exists());
    assert_eq!(fs::read(&retained).unwrap(), b"finished but unvalidated");
    let log = fs::read_to_string(dir.join("Film.error.log")).unwrap();
    assert!(log.contains("Phase: output validation"), "{log}");
    assert!(log.contains("Status: FAILED"), "{log}");
    assert!(log.contains("duration mismatch"), "{log}");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert!(state.row["message"]
        .as_str()
        .unwrap()
        .contains("remains unvalidated"));

    // A second late failure gets its own numbered artifacts.
    let again = dir.join(".Film.hevc-rebuild.again.tmp.mkv");
    fs::write(&again, b"second").unwrap();
    let mut second = failed_state(Phase::OutputValidation, Some(&again), Some(&final_output));
    release_temporary_output(&mut second, &source, None, &quiet_options(&dir));
    assert_eq!(fs::read(dir.join("Film.error.1.mkv")).unwrap(), b"second");
    assert_eq!(fs::read(&retained).unwrap(), b"finished but unvalidated");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_successful_file_leaves_its_installed_output_alone() {
    let dir = scratch("success-release");
    let source = dir.join("Film.mkv");
    fs::write(&source, b"installed output").unwrap();
    let mut state = failed_state(Phase::TransactionCommit, None, Some(&source));
    state.set_status(RunStatus::Ok);
    release_temporary_output(&mut state, &source, None, &quiet_options(&dir));
    assert_eq!(fs::read(&source).unwrap(), b"installed output");
    assert_eq!(visible_entries(&dir), ["Film.mkv"]);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn parses_short_test_mode() {
    let options = RebuildRunOptions::from_engine_args(&base_args(&["--test"])).unwrap();
    assert!(options.test_mode);
    assert!(!options.info_only);
}

#[test]
fn the_short_test_encodes_at_the_runs_crf_not_a_separate_fixed_one() {
    // It was a fixed 30, so a "preview" looked far worse than the rebuild it previews.
    let follows = RebuildRunOptions::from_engine_args(&base_args(&["--test"])).unwrap();
    assert_eq!(follows.crf, 22, "base_args sets --crf 22");
    assert_eq!(follows.test_crf, 22, "the short test follows the run's CRF");
    let bare = RebuildRunOptions::from_engine_args(&[
        std::env::temp_dir().to_string_lossy().into_owned(),
        "--test".into(),
    ])
    .unwrap();
    assert_eq!(
        bare.test_crf,
        crate::settings::DEFAULT_CRF,
        "with no CRF given it is the documented default"
    );
    assert_eq!(bare.test_crf, 21);
}

#[test]
fn an_explicit_test_crf_still_wins_wherever_it_is_given() {
    let before =
        RebuildRunOptions::from_engine_args(&base_args(&["--test-crf", "26", "--test"])).unwrap();
    assert_eq!((before.crf, before.test_crf), (22, 26));
    let after =
        RebuildRunOptions::from_engine_args(&base_args(&["--test", "--test-crf", "26"])).unwrap();
    assert_eq!((after.crf, after.test_crf), (22, 26));
}

#[test]
fn rejects_youtube_flags() {
    assert!(RebuildRunOptions::from_engine_args(&base_args(&["--youtube-upload"])).is_err());
}

#[test]
fn rejects_unknown_flags() {
    assert!(RebuildRunOptions::from_engine_args(&base_args(&["--surprise"])).is_err());
}

#[test]
fn halt_policy_stops_on_low_space_immediately_and_on_a_failure_streak() {
    let mut policy = HaltPolicy::new(3);
    assert_eq!(policy.observe(RunStatus::Failed, "bad stream map"), None);
    assert_eq!(policy.observe(RunStatus::Failed, "bad stream map"), None);
    // A success breaks the streak, so isolated bad files never end a library run.
    assert_eq!(policy.observe(RunStatus::Ok, ""), None);
    assert_eq!(policy.observe(RunStatus::Failed, "x"), None);
    // Skipped files are neutral: they neither extend nor reset the streak.
    assert_eq!(policy.observe(RunStatus::Skipped, ""), None);
    assert_eq!(policy.observe(RunStatus::Failed, "x"), None);
    assert_eq!(
        policy.observe(RunStatus::Failed, "x"),
        Some(BatchHalt::ConsecutiveFailures(3))
    );

    // Low space ends the run on the very first occurrence, even with the streak limit off.
    let mut unlimited = HaltPolicy::new(0);
    for _ in 0..10 {
        assert_eq!(unlimited.observe(RunStatus::Failed, "x"), None);
    }
    assert_eq!(
        unlimited.observe(
            RunStatus::Failed,
            "Insufficient free space even to start safely: 1.0 GiB free"
        ),
        Some(BatchHalt::LowSpace)
    );
}

#[test]
fn low_space_detector_matches_the_messages_the_engine_actually_produces() {
    // Pinned to the real producers, so rewording either message cannot silently
    // turn a full disk into an ordinary per-file failure.
    let preflight = storage_preflight(
        50 * GIB,
        GIB / 4,
        3600.0,
        &RebuildRunOptions::from_engine_args(&base_args(&[])).unwrap(),
    )
    .unwrap_err();
    assert!(is_low_space_failure(&preflight), "{preflight}");
    let live = crate::encode_run::describe_disk_halt(
        Some("projected_output"),
        Some(2 * GIB),
        Some(9 * GIB),
    );
    assert!(is_low_space_failure(&live), "{live}");
    let reserve = crate::encode_run::describe_disk_halt(Some("disk_reserve"), Some(GIB), None);
    assert!(is_low_space_failure(&reserve), "{reserve}");
    assert!(!is_low_space_failure(
        "Selected real video stream count changed"
    ));
}

#[test]
fn consecutive_failure_limit_is_an_engine_option() {
    let options = RebuildRunOptions::from_engine_args(&base_args(&[])).unwrap();
    assert_eq!(options.max_consecutive_failures, 0);
    let options =
        RebuildRunOptions::from_engine_args(&base_args(&["--max-consecutive-failures", "3"]))
            .unwrap();
    assert_eq!(options.max_consecutive_failures, 3);
    assert!(RebuildRunOptions::from_engine_args(&base_args(&[
        "--max-consecutive-failures",
        "1000"
    ]))
    .is_err());
}

#[test]
fn a_failure_streak_halts_the_run_reports_each_file_live_and_leaves_the_rest_untouched() {
    if !crate::media_tools_ready("failure streak halt", &MEDIA_TOOLS) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("state-halt");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("halt");
    let names = ["A.mkv", "B.mkv", "C.mkv", "D.mkv"];
    for name in names {
        fs::write(root.join(name), b"this is not media").unwrap();
    }
    let options = RebuildRunOptions::from_engine_args(&rebuild_args(
        &root,
        &["--max-consecutive-failures", "2"],
    ))
    .unwrap();
    let mut seen = Vec::new();
    let result = run_rebuild_observed(
        &options,
        |_| {},
        |_| {},
        |_| {},
        |file| seen.push(file.path.clone()),
    )
    .unwrap();
    assert_eq!(result.halted, Some(BatchHalt::ConsecutiveFailures(2)));
    assert_eq!(
        result.failed, 2,
        "the run must stop after the second failure"
    );
    assert_eq!(seen.len(), 2, "each finished file is reported as it ends");
    assert!(seen[0].ends_with("A.mkv") && seen[1].ends_with("B.mkv"));
    for name in names {
        assert_eq!(
            fs::read(root.join(name)).unwrap(),
            b"this is not media",
            "{name} must be untouched"
        );
    }
    assert!(
        !root.join(crate::backup_folder::BACKUP_FOLDER).exists(),
        "no backup folder for failed files"
    );
    let lines = closing_lines(&result);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("consecutive failures")),
        "{lines:?}"
    );
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state_dir);
}
