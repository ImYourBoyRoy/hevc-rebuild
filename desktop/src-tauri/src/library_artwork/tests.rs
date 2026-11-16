//! Tests for artwork validation, saves, the private cache and its reviewed purge.

use super::*;

#[test]
fn artwork_cache_preview_protects_assignments_and_purge_removes_only_unused_managed_files() {
    let temp = crate::test_temp_base().join(format!(
        "hevc-artwork-cache-test-{}-{}",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&temp).unwrap();
    let root = temp.canonicalize().unwrap();
    let assigned = root.join(format!("local-{}.jpg", "a".repeat(32)));
    let unused = root.join(format!("provider-{}.webp", "b".repeat(32)));
    let attribution = PathBuf::from(format!("{}.attribution.json", unused.display()));
    let unmanaged = root.join("keep-this-note.txt");
    fs::write(&assigned, b"assigned").unwrap();
    fs::write(&unused, b"image").unwrap();
    fs::write(&attribution, b"credit").unwrap();
    fs::write(&unmanaged, b"leave untouched").unwrap();
    let references = HashSet::from([assigned.clone()]);

    let preview = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
    assert_eq!(preview.info.image_count, 2);
    assert_eq!(preview.info.protected_image_count, 1);
    assert_eq!(preview.info.purgeable_image_count, 1);
    assert_eq!(preview.info.purgeable_bytes, 11);
    assert_eq!(preview.info.unmanaged_entry_count, 1);

    let stale_preview_id = preview.info.preview_id.clone();
    fs::write(&unmanaged, b"changed after review").unwrap();
    let changed = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
    assert!(purge_artwork_cache_snapshot(changed, &stale_preview_id).is_err());
    assert!(assigned.exists());
    assert!(unused.exists());

    let current = build_artwork_cache_snapshot_at(Some(root.clone()), &references).unwrap();
    let current_preview_id = current.info.preview_id.clone();
    let result = purge_artwork_cache_snapshot(current, &current_preview_id).unwrap();
    assert_eq!(result.deleted_image_count, 1);
    assert_eq!(result.deleted_bytes, 11);
    assert_eq!(result.failed_file_count, 0);
    assert!(assigned.exists());
    assert!(!unused.exists());
    assert!(!attribution.exists());
    assert_eq!(fs::read(&unmanaged).unwrap(), b"changed after review");
    assert_eq!(result.remaining.protected_image_count, 1);
    assert_eq!(result.remaining.purgeable_image_count, 0);
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn artwork_cache_purge_requires_explicit_confirmation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let error = runtime
        .block_on(purge_unused_app_artwork_cache("", false))
        .unwrap_err();
    assert!(error.contains("explicit confirmation"));
}

fn candidate(provider: MetadataProvider, url: &str) -> ArtworkCandidate {
    let source_url = format!("https://{}/source", expected_source_host(&provider));
    ArtworkCandidate {
        id: "poster-1".into(),
        provider,
        title: "Selected poster".into(),
        image_url: url.into(),
        thumbnail_url: url.into(),
        source_url,
        creator: None,
        license: Some("CC BY-SA".into()),
        license_url: Some("https://example.org/license".into()),
        attribution: Some("Example attribution".into()),
    }
}

#[test]
fn only_provider_owned_https_image_hosts_are_accepted() {
    assert!(validate_candidate(&candidate(
        MetadataProvider::TvMaze,
        "https://static.tvmaze.com/images/poster.jpg"
    ))
    .is_ok());
    assert!(validate_candidate(&candidate(
        MetadataProvider::Tmdb,
        "https://image.tmdb.org/t/p/original/poster.jpg"
    ))
    .is_ok());
    assert!(validate_candidate(&candidate(
        MetadataProvider::WikimediaCommons,
        "https://upload.wikimedia.org/wikipedia/commons/poster.png"
    ))
    .is_ok());
    assert!(validate_candidate(&candidate(
        MetadataProvider::TvMaze,
        "https://evil.example/poster.jpg"
    ))
    .is_err());
    assert!(validate_candidate(&candidate(
        MetadataProvider::TvMaze,
        "http://static.tvmaze.com/poster.jpg"
    ))
    .is_err());
    assert!(validate_candidate(&candidate(
        MetadataProvider::Wikidata,
        "https://upload.wikimedia.org/poster.jpg"
    ))
    .is_err());
    assert!(validate_candidate(&candidate(
        MetadataProvider::TvMaze,
        "https://static.tvmaze.com/poster.svg"
    ))
    .is_err());
}

#[test]
fn downloaded_image_type_is_verified_from_bytes() {
    let mut jpeg = vec![0; 128];
    jpeg[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    assert_eq!(detect_image(&jpeg), Some(("jpg", "image/jpeg")));
    let mut png = vec![0; 128];
    png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    assert_eq!(detect_image(&png), Some(("png", "image/png")));
    let mut webp = vec![0; 128];
    webp[..4].copy_from_slice(b"RIFF");
    webp[8..12].copy_from_slice(b"WEBP");
    assert_eq!(detect_image(&webp), Some(("webp", "image/webp")));
    assert_eq!(detect_image(b"<html>not artwork</html>"), None);
}

#[test]
fn optional_credit_fields_are_bounded_and_safe_to_embed() {
    let mut value = candidate(
        MetadataProvider::TvMaze,
        "https://static.tvmaze.com/images/poster.jpg",
    );
    value.attribution = Some("Example\ncredit".into());
    normalize_credit_fields(&mut value).unwrap();
    assert_eq!(value.attribution.as_deref(), Some("Example credit"));
    assert!(validate_candidate(&value).is_ok());

    value.attribution = Some("Bad\0credit".into());
    assert!(normalize_credit_fields(&mut value).is_err());

    value.attribution = Some("A".repeat(4097));
    assert!(normalize_credit_fields(&mut value).is_err());

    value.attribution = Some("Example attribution".into());
    value.license_url = Some("javascript:alert(1)".into());
    assert!(validate_candidate(&value).is_err());

    value.license_url = Some("https://creativecommons.org/licenses/by/4.0/".into());
    assert!(validate_candidate(&value).is_ok());
}

#[test]
fn artwork_targets_stay_inside_the_selected_root_and_are_unique() {
    let temp = crate::test_temp_base().join(format!(
        "hevc-artwork-test-{}-{}",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let root = temp.join("library");
    let outside = temp.join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let inside = root.join("Season 01");
    fs::create_dir_all(&inside).unwrap();
    assert!(validate_targets(
        root.to_str().unwrap(),
        &[ArtworkTarget {
            directory: inside.to_string_lossy().into_owned(),
            basename: "poster".into()
        }]
    )
    .is_ok());
    assert!(validate_targets(
        root.to_str().unwrap(),
        &[ArtworkTarget {
            directory: outside.to_string_lossy().into_owned(),
            basename: "poster".into()
        }]
    )
    .is_err());
    assert!(validate_targets(
        root.to_str().unwrap(),
        &[ArtworkTarget {
            directory: root.to_string_lossy().into_owned(),
            basename: "../poster".into()
        }]
    )
    .is_err());
    let duplicate = ArtworkTarget {
        directory: inside.to_string_lossy().into_owned(),
        basename: "poster".into(),
    };
    assert!(validate_targets(root.to_str().unwrap(), &[duplicate.clone(), duplicate]).is_err());
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn artwork_allows_server_specific_names_in_one_folder_but_rejects_normalized_duplicates() {
    let temp = crate::test_temp_base().join(format!(
        "hevc-artwork-names-test-{}-{}",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let root = temp.join("library");
    fs::create_dir_all(&root).unwrap();

    assert!(validate_targets(
        root.to_str().unwrap(),
        &[
            ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "folder".into(),
            },
            ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "poster".into(),
            },
        ]
    )
    .is_ok());

    assert!(validate_targets(
        root.to_str().unwrap(),
        &[
            ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "Season01".into(),
            },
            ArtworkTarget {
                directory: root.to_string_lossy().into_owned(),
                basename: "season01".into(),
            },
        ]
    )
    .is_err());

    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn local_poster_copy_validates_image_and_refuses_to_overwrite() {
    let temp = crate::test_temp_base().join(format!(
        "hevc-local-poster-test-{}-{}",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let root = temp.join("library");
    fs::create_dir_all(&root).unwrap();
    let source = temp.join("custom-poster.jpg");
    let mut bytes = vec![0; 128];
    bytes[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    fs::write(&source, &bytes).unwrap();
    let request = LocalArtworkSaveRequest {
        root: root.to_string_lossy().into_owned(),
        source_path: source.to_string_lossy().into_owned(),
        targets: vec![ArtworkTarget {
            directory: root.to_string_lossy().into_owned(),
            basename: "film-poster".into(),
        }],
    };
    let result = save_local_artwork(request.clone()).unwrap();
    assert_eq!(
        result.files,
        vec![root.join("film-poster.jpg").to_string_lossy().into_owned()]
    );
    assert_eq!(fs::read(&result.files[0]).unwrap(), bytes);
    assert!(save_local_artwork(request).is_err());
    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn local_poster_rejects_an_extension_that_disagrees_with_image_bytes() {
    let temp = crate::test_temp_base().join(format!(
        "hevc-local-poster-extension-test-{}-{}",
        std::process::id(),
        FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let root = temp.join("library");
    fs::create_dir_all(&root).unwrap();
    let source = temp.join("custom-poster.png");
    let mut bytes = vec![0; 128];
    bytes[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    fs::write(&source, &bytes).unwrap();
    let request = LocalArtworkSaveRequest {
        root: root.to_string_lossy().into_owned(),
        source_path: source.to_string_lossy().into_owned(),
        targets: vec![ArtworkTarget {
            directory: root.to_string_lossy().into_owned(),
            basename: "film-poster".into(),
        }],
    };

    let error = save_local_artwork(request).unwrap_err();
    assert!(error.contains("contains JPG image data"));
    assert!(!root.join("film-poster.jpg").exists());
    assert!(!root.join("film-poster.png").exists());
    fs::remove_dir_all(temp).unwrap();
}
