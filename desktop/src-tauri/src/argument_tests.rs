//! Characterization tests for the engine arguments built from settings, selections and overrides.

use super::*;

#[test]
fn metadata_blacklist_is_forwarded_as_individual_engine_arguments() {
    let settings = json!({"metadata": {"text_blacklist": ["Movie*Group", "Explicit"]}});
    assert_eq!(
        metadata_text_blacklist_arguments(&settings).unwrap(),
        vec![
            "--embed-artwork",
            "--replace-metadata-text-blacklist",
            "--metadata-text-blacklist",
            "Movie*Group",
            "--metadata-text-blacklist",
            "Explicit"
        ]
    );
    assert!(metadata_text_blacklist_arguments(&json!({"metadata": {}}))
        .unwrap()
        .eq(&vec!["--embed-artwork"]));
    assert_eq!(
        metadata_text_blacklist_arguments(&json!({"metadata": {"text_blacklist": []}})).unwrap(),
        vec!["--embed-artwork", "--replace-metadata-text-blacklist"]
    );
}

#[test]
fn metadata_blacklist_rejects_empty_wildcard_and_duplicate_patterns() {
    assert!(
        metadata_text_blacklist_arguments(&json!({"metadata": {"text_blacklist": ["***"]}}))
            .is_err()
    );
    assert!(metadata_text_blacklist_arguments(
        &json!({"metadata": {"text_blacklist": ["Movie*Group", "movie*group"]}})
    )
    .is_err());
}

#[test]
fn metadata_blacklist_rejects_oversized_and_controlled_patterns() {
    assert!(metadata_text_blacklist_arguments(
        &json!({"metadata": {"text_blacklist": ["x".repeat(129)]}})
    )
    .is_err());
    assert!(metadata_text_blacklist_arguments(
        &json!({"metadata": {"text_blacklist": ["bad\npattern"]}})
    )
    .is_err());
}

#[test]
fn builtin_secondary_video_default_and_all_supported_policies_reach_engine() {
    let defaults = settings::default_settings();
    let args = settings_arguments(&defaults)
        .expect("the built-in settings must pass the Rust engine adapter");
    let work_root = args
        .windows(2)
        .find(|pair| pair[0] == "--work-root")
        .expect("the shared dynamic work-root policy is forwarded");
    assert_eq!(work_root[1], "auto");
    let option = args
        .windows(2)
        .find(|pair| pair[0] == "--secondary-video")
        .expect("secondary video policy is forwarded");
    assert_eq!(option[1], "encode");

    let mut legacy = settings::default_settings();
    legacy.as_object_mut().unwrap().remove("paths");
    let legacy_args = settings_arguments(&legacy)
        .expect("older saved settings without paths must use the dynamic default");
    assert_eq!(
        legacy_args
            .windows(2)
            .find(|pair| pair[0] == "--work-root")
            .unwrap()[1],
        "auto"
    );

    for policy in ["encode", "copy", "drop"] {
        let mut configured = settings::default_settings();
        configured["streams"]["secondary_video"] = json!(policy);
        let args = settings_arguments(&configured)
            .expect("all documented secondary video policies must be accepted");
        let option = args
            .windows(2)
            .find(|pair| pair[0] == "--secondary-video")
            .expect("secondary video policy is forwarded");
        assert_eq!(option[1], policy);
    }
}

#[test]
fn selected_media_files_are_canonical_and_confined_to_the_analyzed_root() {
    let root = test_dir("selected-media-root");
    let outside = test_dir("selected-media-outside");
    let source = root.join("film.mkv");
    let other = outside.join("outside.mkv");
    fs::write(&source, b"fixture").unwrap();
    fs::write(&other, b"fixture").unwrap();
    let mut args = Vec::new();
    append_selected_file_arguments(&mut args, &root, &[source.to_string_lossy().into_owned()])
        .unwrap();
    assert_eq!(args[0], "--selected-file");
    assert_eq!(
        args[1],
        source
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    );
    assert!(append_selected_file_arguments(
        &mut Vec::new(),
        &root,
        &[other.to_string_lossy().into_owned()]
    )
    .is_err());
    assert!(
        append_selected_file_arguments(&mut Vec::new(), &root, &["relative.mkv".into()]).is_err()
    );
    let duplicate = source.to_string_lossy().into_owned();
    assert!(append_selected_file_arguments(
        &mut Vec::new(),
        &root,
        &[duplicate.clone(), duplicate]
    )
    .is_err());
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn poster_override_arguments_validate_and_forward_scoped_images() {
    let root = test_dir("poster-overrides");
    let media = root.join("Film.mkv");
    let artwork_dir = root.join("Artwork");
    fs::create_dir_all(&artwork_dir).unwrap();
    fs::write(&media, b"media fixture").unwrap();
    let poster = artwork_dir.join("selected.png");
    fs::write(&poster, b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00").unwrap();

    let args = poster_override_arguments(
        &root,
        BTreeMap::from([(
            media.to_string_lossy().into_owned(),
            poster.to_string_lossy().into_owned(),
        )]),
        &root.join("private-cache/artwork"),
    )
    .expect("accept a valid image under the selected library root");
    assert_eq!(args[0], "--file-poster-overrides-json");
    let decoded: BTreeMap<String, String> = serde_json::from_str(&args[1]).unwrap();
    assert_eq!(
        decoded.get(&media.canonicalize().unwrap().to_string_lossy().into_owned()),
        Some(
            &poster
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );

    let external = test_dir("poster-outside").join("outside.jpg");
    fs::write(&external, b"\xff\xd8\xff".repeat(5)).unwrap();
    assert!(poster_override_arguments(
        &root,
        BTreeMap::from([(
            media.to_string_lossy().into_owned(),
            external.to_string_lossy().into_owned(),
        )]),
        &root.join("private-cache/artwork"),
    )
    .unwrap_err()
    .contains("analyzed input"));

    let cache = root.join("private-cache/artwork");
    fs::create_dir_all(&cache).unwrap();
    let cached_poster = cache.join("poster.webp");
    fs::write(&cached_poster, b"RIFF0000WEBPVP8 ").unwrap();
    let args = poster_override_arguments(
        &root,
        BTreeMap::from([(
            media.to_string_lossy().into_owned(),
            cached_poster.to_string_lossy().into_owned(),
        )]),
        &cache,
    )
    .expect("accept valid artwork from the approved private cache");
    assert_eq!(args[0], "--allowed-poster-cache-root");
    assert_eq!(args[2], "--file-poster-overrides-json");

    fs::remove_dir_all(root).unwrap();
    let external_root = external.parent().unwrap();
    fs::remove_dir_all(external_root).unwrap();
}

#[test]
fn metadata_override_arguments_validate_scope_and_forward_reviewed_fields() {
    let root = test_dir("metadata-overrides");
    let media = root.join("Film.mkv");
    fs::write(&media, b"media fixture").unwrap();
    let args = metadata_override_arguments(
        &root,
        BTreeMap::from([(
            media.to_string_lossy().into_owned(),
            json!({"title": "Reviewed title", "genres": ["Drama", "Animation"]}),
        )]),
    )
    .expect("accept reviewed fields for a file inside the selected library");
    assert_eq!(args[0], "--file-metadata-overrides-json");
    let decoded: BTreeMap<String, Value> = serde_json::from_str(&args[1]).unwrap();
    let canonical = media.canonicalize().unwrap().to_string_lossy().into_owned();
    assert_eq!(decoded[&canonical]["title"], "Reviewed title");
    assert_eq!(decoded[&canonical]["genres"][0], "Drama");

    let outside_root = test_dir("metadata-overrides-outside");
    let outside = outside_root.join("Outside.mkv");
    fs::write(&outside, b"fixture").unwrap();
    assert!(metadata_override_arguments(
        &root,
        BTreeMap::from([(
            outside.to_string_lossy().into_owned(),
            json!({"title": "Outside"}),
        )]),
    )
    .unwrap_err()
    .contains("inside the analyzed input"));
    assert!(metadata_override_arguments(
        &root,
        BTreeMap::from([(media.to_string_lossy().into_owned(), json!({}))]),
    )
    .is_err());

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside_root).unwrap();
}

#[test]
fn youtube_metadata_loss_arguments_are_canonical_and_scoped() {
    let root = test_dir("hdr10plus-root");
    let source = root.join("source.mkv");
    fs::write(&source, b"fixture").expect("create test source");
    let arguments =
        youtube_hdr10plus_loss_arguments(&root, vec![source.to_string_lossy().into_owned()])
            .expect("accept consent for a source in the selected root");
    let canonical = source
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        arguments,
        vec![String::from("--allow-youtube-hdr10plus-loss"), canonical]
    );
    fs::remove_dir_all(root).expect("remove test directory");
}

#[test]
fn youtube_metadata_loss_arguments_reject_duplicates_and_out_of_root_paths() {
    let root = test_dir("hdr10plus-scope");
    let outside = test_dir("hdr10plus-outside");
    let source = root.join("source.mkv");
    let other = outside.join("outside.mkv");
    fs::write(&source, b"fixture").expect("create in-root source");
    fs::write(&other, b"fixture").expect("create outside source");
    let raw_source = source.to_string_lossy().into_owned();
    assert!(youtube_hdr10plus_loss_arguments(&root, vec![raw_source.clone(), raw_source]).is_err());
    assert!(
        youtube_hdr10plus_loss_arguments(&root, vec![other.to_string_lossy().into_owned()])
            .is_err()
    );
    fs::remove_dir_all(root).expect("remove root fixture");
    fs::remove_dir_all(outside).expect("remove outside fixture");
}

#[test]
fn rebuild_output_arguments_validate_sources_and_forward_canonical_paths() {
    let root = crate::test_temp_base().join(format!("hevc-rebuild-output-test-{}", new_job_id()));
    let source = root.join("Season 1").join("Episode.mkv");
    let output = root.join("output");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(&source, b"source").unwrap();

    let (args, resource_path) = rebuild_output_arguments(
        &root,
        None,
        BTreeMap::from([(
            source.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
        )]),
    )
    .expect("build validated output arguments");
    assert_eq!(resource_path, Some(output.canonicalize().unwrap()));
    assert_eq!(args[0], "--file-output-dirs-json");
    let destinations: BTreeMap<String, String> = serde_json::from_str(&args[1]).unwrap();
    assert_eq!(
        destinations.get(
            &source
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ),
        Some(
            &output
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );
    assert!(rebuild_output_arguments(
        &root,
        None,
        BTreeMap::from([(
            "/outside/video.mkv".into(),
            output.to_string_lossy().into_owned()
        )]),
    )
    .is_err());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quality_gate_settings_reach_the_engine_and_are_validated() {
    let value_of = |args: &[String], flag: &str| {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
    };
    let defaults = settings_arguments(&settings::default_settings()).unwrap();
    assert_eq!(
        value_of(&defaults, "--quality-gate").as_deref(),
        Some("enforce")
    );
    assert_eq!(
        value_of(&defaults, "--quality-min-ssim").as_deref(),
        Some("0.95")
    );

    let mut configured = settings::default_settings();
    configured["quality"] = json!({"ssim_gate": "report", "min_ssim": 0.97});
    let args = settings_arguments(&configured).unwrap();
    assert_eq!(value_of(&args, "--quality-gate").as_deref(), Some("report"));
    assert_eq!(
        value_of(&args, "--quality-min-ssim").as_deref(),
        Some("0.97")
    );

    // Settings saved before the gate existed get the safe defaults.
    let mut legacy = settings::default_settings();
    legacy.as_object_mut().unwrap().remove("quality");
    let legacy_args = settings_arguments(&legacy).unwrap();
    assert_eq!(
        value_of(&legacy_args, "--quality-gate").as_deref(),
        Some("enforce")
    );

    for bad in [
        json!({"ssim_gate": "strict", "min_ssim": 0.95}),
        json!({"ssim_gate": "enforce", "min_ssim": 0.2}),
        json!({"ssim_gate": "enforce", "min_ssim": "high"}),
    ] {
        let mut broken = settings::default_settings();
        broken["quality"] = bad;
        assert!(settings_arguments(&broken).is_err());
    }
}
