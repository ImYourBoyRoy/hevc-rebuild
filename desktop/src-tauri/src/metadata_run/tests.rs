//! Tests for metadata-only run option parsing, output planning and the end-to-end remux.

use super::*;

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hevc-metadata-run-{label}-{}", unique_token()));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn options_for(root: &Path) -> MetadataRunOptions {
    MetadataRunOptions {
        root: root.to_path_buf(),
        minimum_size_mib: 0,
        embed_artwork: true,
        ..MetadataRunOptions::default()
    }
}

#[test]
fn engine_arguments_cover_the_gui_vocabulary() {
    let root = scratch("args");
    let args: Vec<String> = [
        root.to_str().unwrap(),
        "--min-size-mib",
        "0",
        "--no-dependency-repair",
        "--skip-update-check",
        "--recursive",
        "--include-filename-pattern",
        "*.mkv",
        "--exclude-filename-pattern",
        "*sample*",
        "--output-dir",
        root.to_str().unwrap(),
        "--crf",
        "20",
        "--preset",
        "slow",
        "--tune",
        "auto",
        "--detect-animation",
        "--no-promote-8bit-to-10bit",
        "--preserve-hdr10plus",
        "--verify-copy-hashes",
        "--comment",
        "Hello",
        "--no-embed-artwork",
        "--replace-metadata-text-blacklist",
        "--metadata-text-blacklist",
        "Movie*Group",
        "--metadata-text-blacklist",
        "movie*group",
        "--min-free-disk-gib",
        "2.5",
        "--preflight-output-ratio",
        "0.5",
        "--stop-on-error",
        "--metadata-only",
        "--ffmpeg-path",
        "/usr/bin/ffmpeg",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let options = MetadataRunOptions::from_engine_args(&args).unwrap();
    assert!(options.recursive && options.stop_on_error);
    assert_eq!(options.minimum_size_mib, 0);
    assert_eq!(options.comment, "Hello");
    assert!(!options.embed_artwork);
    assert_eq!(options.text_blacklist, vec!["Movie*Group".to_string()]);
    assert_eq!(options.min_free_disk_gib, 2.5);
    assert_eq!(options.include, vec!["*.mkv".to_string()]);
    assert_eq!(
        options.tool_paths.ffmpeg.as_deref(),
        Some("/usr/bin/ffmpeg")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn every_settings_argument_the_app_sends_is_understood() {
    let root = scratch("settings-args");
    let settings = effective_settings(&root).unwrap();
    let mut args = vec![root.to_string_lossy().into_owned()];
    args.extend(crate::settings_arguments::settings_arguments(&settings).unwrap());
    args.extend(crate::settings_arguments::metadata_text_blacklist_arguments(&settings).unwrap());
    args.extend([
        "--metadata-only".into(),
        "--stop-request-file".into(),
        "/tmp/stop".into(),
    ]);
    let options = MetadataRunOptions::from_engine_args(&args).unwrap();
    assert_eq!(options.root, root);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unknown_and_encode_only_flags_are_rejected() {
    let root = scratch("reject");
    let base = root.to_str().unwrap().to_string();
    for (extra, needle) in [
        (vec!["--mystery-flag"], "do not recognise"),
        (
            vec!["--file-stream-overrides-json", "{}"],
            "preserves every source stream",
        ),
        (vec!["--file-settings-overrides-json", "{}"], "do not apply"),
    ] {
        let mut args = vec![base.clone()];
        args.extend(extra.into_iter().map(str::to_owned));
        let error = MetadataRunOptions::from_engine_args(&args).err().unwrap();
        assert!(error.contains(needle), "{error}");
    }
    assert!(MetadataRunOptions::from_engine_args(&[]).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn output_paths_never_resolve_to_the_source() {
    let root = scratch("paths");
    let season = root.join("Show").join("Season 1");
    fs::create_dir_all(&season).unwrap();
    let source = season.join("Episode.One.mkv");
    fs::write(&source, b"x").unwrap();
    let mut options = options_for(&root);
    assert_eq!(
        metadata_output_path(&source, &options).unwrap(),
        season.join("Episode.One.metadata.mkv")
    );
    // A separate folder mirrors the relative layout for folder inputs.
    let out = scratch("paths-out");
    options.output_dir = Some(out.clone());
    assert_eq!(
        metadata_output_path(&source, &options).unwrap(),
        out.join("Show").join("Season 1").join("Episode.One.mkv")
    );
    // A per-file folder stores the file directly in that folder.
    let direct = scratch("paths-direct");
    options
        .file_output_dirs
        .insert(source.clone(), direct.clone());
    assert_eq!(
        metadata_output_path(&source, &options).unwrap(),
        direct.join("Episode.One.mkv")
    );
    // Pointing the destination back at the source folder avoids the source name.
    let mut same = options_for(&season);
    same.output_dir = Some(season.clone());
    assert_eq!(
        metadata_output_path(&source, &same).unwrap(),
        season.join("Episode.One.hevc-rebuilt.mkv")
    );
    for dir in [root, out, direct] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn destination_validation_blocks_existing_and_duplicate_outputs() {
    let root = scratch("collide");
    let first = root.join("a.mp4");
    let second = root.join("a.mkv");
    fs::write(&first, b"x").unwrap();
    fs::write(&second, b"y").unwrap();
    let options = options_for(&root);
    // a.mp4 -> a.mkv exists already (the second source), so it must be refused.
    let error = validate_output_destinations(std::slice::from_ref(&first), &options)
        .err()
        .unwrap();
    assert!(error.contains("will not be overwritten"), "{error}");
    // Two sources sharing one planned output in a separate folder collide.
    let out = scratch("collide-out");
    let mut separate = options_for(&root);
    separate.output_dir = Some(out.clone());
    let error = validate_output_destinations(&[first, second], &separate)
        .err()
        .unwrap();
    assert!(error.contains("same output"), "{error}");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(out);
}

#[test]
fn storage_preflight_matches_the_copy_budget() {
    let one_gib = 1024 * MIB;
    assert!(storage_preflight(10 * one_gib, one_gib, 0.0).is_ok());
    let error = storage_preflight(10 * one_gib, 400 * MIB, 0.0).unwrap_err();
    assert!(error.contains("Insufficient free space"), "{error}");
    assert!(storage_preflight(MIB, 100 * MIB, 1.0).is_err());
    assert!(storage_preflight(MIB, 2 * one_gib, 1.0).is_ok());
}

#[test]
fn hdr_expectation_reads_both_profile_shapes() {
    let profile = json!({
        "color_primaries": "bt2020",
        "color_transfer": "smpte2084",
        "color_matrix": "bt2020nc",
        "mastering_display": "G(1,2)",
        "max_cll": 1000,
        "max_fall": 400,
        "hdr": {"hdr10": true, "hlg": false, "dolby_vision": true,
                "dolby_vision_profile": 8, "dolby_vision_rpu": true,
                "dolby_vision_enhancement_layer": false,
                "dolby_vision_compatibility_id": 1}
    });
    let value = hdr_expectation(&profile, true);
    assert_eq!(value["transfer"], "smpte2084");
    assert_eq!(value["hdr10plus"], true);
    assert_eq!(value["dovi_profile"], 8);
    assert_eq!(value["dovi_compatibility_id"], 1);
    assert_eq!(value["max_cll"], 1000);
    assert_eq!(value["preserve_dolbyvision"], false);
}

#[test]
fn retained_output_gets_error_names_and_a_private_log() {
    let dir = scratch("retain");
    let source = dir.join("Film.mkv");
    let final_output = dir.join("Film.metadata.mkv");
    fs::write(&source, b"source").unwrap();
    let temp = dir.join(".Film.metadata.hevc-metadata.abc.tmp.mkv");
    fs::write(&temp, b"finished but unvalidated").unwrap();
    let mut state = FileState {
        phase: "lossless output validation",
        temp_output: Some(temp.clone()),
        final_output: Some(final_output.clone()),
        row: Map::new(),
        interrupted: false,
    };
    state.row.insert("status".into(), json!("FAILED"));
    state.row.insert("message".into(), json!("hash mismatch"));
    release_temporary_output(
        &mut state,
        &source,
        Some("hash mismatch"),
        &options_for(&dir),
    );
    let retained = dir.join("Film.metadata.error.mkv");
    assert!(retained.is_file());
    assert!(!temp.exists());
    assert!(!final_output.exists());
    let log = fs::read_to_string(dir.join("Film.metadata.error.log")).unwrap();
    assert!(log.contains("Phase: lossless output validation"));
    assert!(log.contains("hash mismatch"));
    assert!(state.row["message"]
        .as_str()
        .unwrap()
        .contains("remains unvalidated"));
    // A second failure never overwrites the first review artifacts.
    let again = dir.join(".again.tmp.mkv");
    fs::write(&again, b"second").unwrap();
    let second = keep_finished_output(&again, Some(&final_output)).unwrap();
    assert_eq!(second, dir.join("Film.metadata.error.1.mkv"));
    assert_eq!(fs::read(&retained).unwrap(), b"finished but unvalidated");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn early_failures_discard_the_temporary_file_only() {
    let dir = scratch("discard");
    let source = dir.join("Film.mkv");
    fs::write(&source, b"source").unwrap();
    let temp = dir.join(".Film.tmp.mkv");
    fs::write(&temp, b"partial").unwrap();
    let mut state = FileState {
        phase: "stream remux",
        temp_output: Some(temp.clone()),
        final_output: Some(dir.join("Film.metadata.mkv")),
        row: Map::new(),
        interrupted: false,
    };
    state.row.insert("status".into(), json!("FAILED"));
    release_temporary_output(&mut state, &source, None, &options_for(&dir));
    assert!(!temp.exists());
    assert!(source.is_file());
    let _ = fs::remove_dir_all(dir);
}

fn make_normalized_synthetic_clip(root: &Path, file_name: &str) -> Option<PathBuf> {
    let source = root.join(file_name);
    let raw = root.join("raw-fixture.mkv");
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=24:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-preset",
            "ultrafast",
            "-x265-params",
            "log-level=none",
            "-c:a",
            "aac",
            "-shortest",
            "-y",
        ])
        .arg(&raw)
        .status();
    let normalised = made.is_ok_and(|status| status.success())
        && Command::new("mkvmerge")
            .arg("-o")
            .arg(&source)
            .arg(&raw)
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|status| status.code().is_some_and(|code| code <= 1));
    let _ = fs::remove_file(&raw);
    if normalised {
        Some(source)
    } else {
        let _ = fs::remove_file(&source);
        None
    }
}

/// End to end on a synthetic clip: needs ffmpeg + MKVToolNix + MediaInfo and is
/// skipped (with a note) on machines without them.
#[test]
fn synthetic_clip_round_trips_through_native_metadata_only() {
    if !crate::media_tools_ready(
        "native metadata-only integration",
        &[
            "ffmpeg",
            "ffprobe",
            "mkvmerge",
            "mkvpropedit",
            "mkvextract",
            "mediainfo",
        ],
    ) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("clip");
    let Some(source) = make_normalized_synthetic_clip(&root, "Clip.One (2024 PG).mkv") else {
        eprintln!("skipping native metadata-only integration: could not build the fixture");
        let _ = fs::remove_dir_all(root);
        return;
    };
    let before = fs::read(&source).unwrap();
    let mut overrides = Map::new();
    overrides.insert(
        source.to_string_lossy().into_owned(),
        json!({"title": "Clip One", "year": "2024", "plot": "A synthetic test pattern."}),
    );
    let args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        "0".into(),
        "--comment".into(),
        "native test".into(),
        "--file-metadata-overrides-json".into(),
        Value::Object(overrides).to_string(),
    ];
    let options = MetadataRunOptions::from_engine_args(&args).unwrap();
    let mut phases = Vec::new();
    let result = run_metadata_only(
        &options,
        |progress| phases.push(progress.phase),
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1);
    let output = root.join("Clip.One (2024 PG).metadata.mkv");
    assert!(output.is_file(), "missing {}", output.display());
    assert_eq!(
        fs::read(&source).unwrap(),
        before,
        "the source must stay byte-identical"
    );
    assert!(phases
        .iter()
        .any(|phase| phase == "Validating copied tracks and metadata"));
    assert_eq!(
        phases.last().map(String::as_str),
        Some("Validated output ready")
    );
    let tags = Command::new("mkvextract")
        .arg(&output)
        .args(["tags", "-"])
        .output()
        .unwrap();
    let xml = String::from_utf8_lossy(&tags.stdout);
    assert!(xml.contains("native test"), "comment tag missing: {xml}");
    assert!(
        xml.contains("A synthetic test pattern."),
        "plot tag missing: {xml}"
    );
    let report = fs::read_to_string(&result.report_path).unwrap();
    assert!(report.contains("METADATA_OK"));
    // A rerun refuses to overwrite the validated output and leaves it intact.
    let again = MetadataRunOptions::from_engine_args(&args).unwrap();
    let error = run_metadata_only(&again, |_| {}, |_| {}, |_| {})
        .err()
        .unwrap();
    assert!(error.contains("will not be overwritten"), "{error}");
    assert!(output.is_file());
    for dir in [root, state_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// Default metadata-only behavior must validate a normalized native MKV
/// without requiring an environment-selected real-media fixture.
#[test]
fn synthetic_default_media_sample_validates_natively() {
    if !crate::media_tools_ready(
        "native metadata-only integration",
        &[
            "ffmpeg",
            "ffprobe",
            "mkvmerge",
            "mkvpropedit",
            "mkvextract",
            "mediainfo",
        ],
    ) {
        return;
    }
    let _state_guard = crate::TEST_STATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state_dir = scratch("default-state");
    std::env::set_var("HEVC_REBUILD_PORTABLE_DATA_DIR", &state_dir);
    let root = scratch("default-clip");
    let Some(source) = make_normalized_synthetic_clip(&root, "Default Clip.mkv") else {
        eprintln!("skipping native metadata-only integration: could not build the fixture");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&state_dir);
        return;
    };
    let before = fs::read(&source).unwrap();
    let args = vec![
        root.to_string_lossy().into_owned(),
        "--min-size-mib".into(),
        "0".into(),
        "--no-embed-artwork".into(),
    ];
    let options = MetadataRunOptions::from_engine_args(&args).unwrap();
    let result = run_metadata_only(&options, |_| {}, |_| {}, |_| {}).unwrap();
    assert_eq!(result.failed, 0, "{:?}", result.files);
    assert_eq!(result.succeeded, 1);
    assert!(root.join("Default Clip.metadata.mkv").is_file());
    assert_eq!(fs::read(&source).unwrap(), before);
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(state_dir);
}
