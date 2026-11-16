//! Tests for templates, rename planning, collision-safe apply and rollback, and folder scans.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
    let path = crate::test_temp_base().join(format!(
        "hevc-rename-test-{}-{nonce}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn installed_rename_sync_failure_keeps_paths_and_reports_unconfirmed_durability() {
    let root = scratch();
    let source = root.join("source.mkv");
    fs::write(&source, b"original bytes").unwrap();
    let plan = build_plan_with_patterns(
        &root.to_string_lossy(),
        vec![input(
            &source,
            RenameTemplateKind::Movie,
            "{title}{ext}",
            &[("title", "Renamed")],
        )],
        &[],
    )
    .unwrap();
    let error =
        apply_plan_with_sync(plan, |_| Err(io::Error::other("injected sync failure"))).unwrap_err();
    assert!(error.contains("durability could not be confirmed"));
    assert!(!source.exists());
    assert_eq!(
        fs::read(root.join("Renamed.mkv")).unwrap(),
        b"original bytes"
    );
    fs::remove_dir_all(root).unwrap();
}

fn input(
    source: &Path,
    kind: RenameTemplateKind,
    template: &str,
    values: &[(&str, &str)],
) -> RenameTemplateInput {
    RenameTemplateInput {
        source_path: source.to_string_lossy().into_owned(),
        kind,
        template: template.into(),
        values: values
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
    }
}

#[test]
fn renders_film_and_episode_defaults_and_optional_parts() {
    let film = render_template(
        "{title} [[({year} {certification})]]{ext}",
        &RenameTemplateKind::Movie,
        &BTreeMap::from([
            ("title".into(), "Gnomeo & Juliet".into()),
            ("year".into(), "2011".into()),
            ("certification".into(), "G".into()),
            ("ext".into(), ".mkv".into()),
        ]),
    )
    .unwrap();
    assert_eq!(film, "Gnomeo & Juliet (2011 G).mkv");
    let no_rating = render_template(
        "{title} [[({year} {certification})]]{ext}",
        &RenameTemplateKind::Movie,
        &BTreeMap::from([
            ("title".into(), "Example".into()),
            ("year".into(), "2011".into()),
            ("ext".into(), ".mkv".into()),
        ]),
    )
    .unwrap();
    assert_eq!(no_rating, "Example (2011).mkv");
    let episode = render_template(
        "S{season:02}E{episode:02}.{episode_title}{ext}",
        &RenameTemplateKind::Episode,
        &BTreeMap::from([
            ("season".into(), "1".into()),
            ("episode".into(), "1".into()),
            ("episode_title".into(), "The Tale of the Frog Prince".into()),
            ("ext".into(), ".mkv".into()),
        ]),
    )
    .unwrap();
    assert_eq!(episode, "S01E01.The Tale of the Frog Prince.mkv");
}

#[test]
fn invalid_shortcode_and_reserved_filename_are_rejected() {
    assert!(
        render_template("{unknown}", &RenameTemplateKind::Movie, &BTreeMap::new())
            .unwrap_err()
            .contains("Unknown")
    );
    assert!(portable_component_error("CON.mkv", false).is_some());
    assert!(portable_component_error("bad/name.mkv", false).is_some());
}

#[test]
fn duplicate_normalized_targets_block_the_whole_plan() {
    let root = scratch();
    let first = root.join("first.mkv");
    let second = root.join("second.mkv");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let plan = build_plan_with_patterns(
        &root.to_string_lossy(),
        vec![
            input(
                &first,
                RenameTemplateKind::Movie,
                "Caf\u{00e9}.mkv",
                &[("title", "unused")],
            ),
            input(
                &second,
                RenameTemplateKind::Movie,
                "Cafe\u{0301}.mkv",
                &[("title", "unused")],
            ),
        ],
        &[],
    )
    .unwrap();
    assert!(!plan.preview.can_apply);
    assert!(plan
        .preview
        .conflicts
        .iter()
        .any(|conflict| conflict.reason.contains("same destination")));
    assert!(first.exists() && second.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn existing_destination_is_blocked_without_overwrite() {
    let root = scratch();
    let source = root.join("source.mkv");
    let destination = root.join("destination.mkv");
    fs::write(&source, b"source bytes").unwrap();
    fs::write(&destination, b"destination bytes").unwrap();
    let plan = build_plan_with_patterns(
        &root.to_string_lossy(),
        vec![input(
            &source,
            RenameTemplateKind::Movie,
            "destination{ext}",
            &[],
        )],
        &[],
    )
    .unwrap();
    assert!(!plan.preview.can_apply);
    assert!(apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        vec![input(
            &source,
            RenameTemplateKind::Movie,
            "destination{ext}",
            &[]
        ),],
        Vec::new(),
        plan.preview.preview_id,
        true
    )
    .is_err());
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    assert_eq!(fs::read(&destination).unwrap(), b"destination bytes");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cycles_and_nested_tv_folder_renames_keep_file_contents() {
    let root = scratch();
    let show = root.join("Faerie Tale Theatre");
    let season = show.join("Season 1");
    fs::create_dir_all(&season).unwrap();
    let episode = season.join("Faerie.Tale.Theatre.S01E01.avi");
    fs::write(&episode, b"video bytes").unwrap();
    let plan = build_plan_with_patterns(
        &root.to_string_lossy(),
        vec![
            input(
                &show,
                RenameTemplateKind::Series,
                "{series_title} [[({year_range})]]",
                &[
                    ("series_title", "Faerie Tale Theatre"),
                    ("year_range", "1982-1987"),
                ],
            ),
            input(
                &season,
                RenameTemplateKind::Season,
                "Season {season:02}",
                &[("season", "1")],
            ),
            input(
                &episode,
                RenameTemplateKind::Episode,
                "S{season:02}E{episode:02}.{episode_title}{ext}",
                &[
                    ("season", "1"),
                    ("episode", "1"),
                    ("episode_title", "The Tale of the Frog Prince"),
                ],
            ),
        ],
        &[],
    )
    .unwrap();
    let expected = root
        .join("Faerie Tale Theatre (1982-1987)/Season 01/S01E01.The Tale of the Frog Prince.avi");
    assert!(plan.preview.can_apply);
    assert!(plan
        .preview
        .items
        .iter()
        .any(|item| item.target_path == expected.to_string_lossy()));
    let applied = apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        vec![
            input(
                &show,
                RenameTemplateKind::Series,
                "{series_title} [[({year_range})]]",
                &[
                    ("series_title", "Faerie Tale Theatre"),
                    ("year_range", "1982-1987"),
                ],
            ),
            input(
                &season,
                RenameTemplateKind::Season,
                "Season {season:02}",
                &[("season", "1")],
            ),
            input(
                &episode,
                RenameTemplateKind::Episode,
                "S{season:02}E{episode:02}.{episode_title}{ext}",
                &[
                    ("season", "1"),
                    ("episode", "1"),
                    ("episode_title", "The Tale of the Frog Prince"),
                ],
            ),
        ],
        Vec::new(),
        plan.preview.preview_id,
        true,
    )
    .unwrap();
    assert_eq!(applied.renamed_count, 3);
    assert_eq!(fs::read(expected).unwrap(), b"video bytes");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn preview_marks_unchanged_filenames_as_changed_when_parent_moves() {
    let root = scratch();
    let show = root.join("Old Show");
    let season = show.join("Season 1");
    fs::create_dir_all(&season).unwrap();
    let episode = season.join("Episode.mkv");
    fs::write(&episode, b"video bytes").unwrap();
    let inputs = || {
        vec![
            input(
                &show,
                RenameTemplateKind::Series,
                "{series_title}",
                &[("series_title", "New Show")],
            ),
            input(
                &season,
                RenameTemplateKind::Season,
                "Season {season}",
                &[("season", "1")],
            ),
            input(
                &episode,
                RenameTemplateKind::Episode,
                "{episode_title}{ext}",
                &[("episode_title", "Episode")],
            ),
        ]
    };
    let preview = preview_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        inputs(),
        Vec::new(),
    )
    .unwrap();
    let episode_preview = preview
        .items
        .iter()
        .find(|item| item.source_path == episode.to_string_lossy())
        .unwrap();
    assert_eq!(
        Path::new(&episode_preview.target_path).file_name().unwrap(),
        "Episode.mkv"
    );
    assert!(!episode_preview.unchanged);

    let applied = apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        inputs(),
        Vec::new(),
        preview.preview_id,
        true,
    )
    .unwrap();
    assert_eq!(applied.renamed_count, 3);
    assert_eq!(
        fs::read(root.join("New Show/Season 1/Episode.mkv")).unwrap(),
        b"video bytes"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_renames_include_matching_metadata_artwork_subtitle_and_audio_sidecars() {
    let root = scratch();
    let source = root.join("Movie.mkv");
    let nfo = root.join("Movie.nfo");
    let poster = root.join("Movie-poster.jpg");
    let credit = root.join("Movie-poster.jpg.attribution.json");
    let subtitle = root.join("movie.en.srt");
    let audio = root.join("Movie.commentary.ac3");
    let local_cover = root.join("Movie.jpg");
    let local_cover_credit = root.join("Movie.jpg.attribution.json");
    let local_fanart = root.join("Movie-cover.png");
    let local_fanart_credit = root.join("Movie-cover.png.attribution.json");
    let unrelated = root.join("MovieExtra.srt");
    fs::write(&source, b"video").unwrap();
    fs::write(&nfo, b"movie metadata").unwrap();
    fs::write(&poster, b"poster image").unwrap();
    fs::write(&credit, b"poster credit").unwrap();
    fs::write(&subtitle, b"subtitle").unwrap();
    fs::write(&audio, b"audio").unwrap();
    fs::write(&local_cover, b"local cover").unwrap();
    fs::write(&local_cover_credit, b"local cover credit").unwrap();
    fs::write(&local_fanart, b"local fanart").unwrap();
    fs::write(&local_fanart_credit, b"local fanart credit").unwrap();
    fs::write(&unrelated, b"unrelated subtitle").unwrap();
    let inputs = || {
        vec![input(
            &source,
            RenameTemplateKind::Movie,
            "Renamed Movie{ext}",
            &[],
        )]
    };

    let preview = preview_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        inputs(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(preview.items.len(), 10);
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie.nfo")));
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie-poster.jpg")));
    assert!(preview.items.iter().any(|item| item
        .target_path
        .ends_with("Renamed Movie-poster.jpg.attribution.json")));
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie.en.srt")));
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie.commentary.ac3")));
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie.jpg")));
    assert!(preview.items.iter().any(|item| item
        .target_path
        .ends_with("Renamed Movie.jpg.attribution.json")));
    assert!(preview
        .items
        .iter()
        .any(|item| item.target_path.ends_with("Renamed Movie-cover.png")));
    assert!(preview.items.iter().any(|item| item
        .target_path
        .ends_with("Renamed Movie-cover.png.attribution.json")));
    assert!(preview.can_apply);

    let applied = apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        inputs(),
        Vec::new(),
        preview.preview_id,
        true,
    )
    .unwrap();
    assert_eq!(applied.renamed_count, 10);
    assert_eq!(fs::read(root.join("Renamed Movie.mkv")).unwrap(), b"video");
    assert_eq!(
        fs::read(root.join("Renamed Movie.nfo")).unwrap(),
        b"movie metadata"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie-poster.jpg")).unwrap(),
        b"poster image"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie-poster.jpg.attribution.json")).unwrap(),
        b"poster credit"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie.en.srt")).unwrap(),
        b"subtitle"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie.commentary.ac3")).unwrap(),
        b"audio"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie.jpg")).unwrap(),
        b"local cover"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie.jpg.attribution.json")).unwrap(),
        b"local cover credit"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie-cover.png")).unwrap(),
        b"local fanart"
    );
    assert_eq!(
        fs::read(root.join("Renamed Movie-cover.png.attribution.json")).unwrap(),
        b"local fanart credit"
    );
    assert_eq!(fs::read(unrelated).unwrap(), b"unrelated subtitle");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parses_standard_and_multi_episode_ids() {
    assert_eq!(
        parse_episode_id("Faerie.Tale.S01E02.The.Show"),
        (Some(1), Some(2), false)
    );
    assert_eq!(
        parse_episode_id("Show.S1E2E3.Title"),
        (Some(1), Some(2), true)
    );
    assert_eq!(parse_episode_id("Movie.2020.1080p"), (None, None, false));
}

#[test]
fn episode_gap_summary_is_bounded_and_ignores_multi_episode_seasons() {
    let file = |name: &str, season, episode, multiple_episodes| OrganizerMediaFile {
        path: name.into(),
        name: name.into(),
        parent_path: String::new(),
        extension: "mkv".into(),
        season,
        episode,
        multiple_episodes,
    };
    let files = vec![
        file("Show.S01E01.mkv", Some(1), Some(1), false),
        file("Show.S01E03.mkv", Some(1), Some(3), false),
        file("Show.S01E10.mkv", Some(1), Some(10), false),
        file("Show.S02E01.mkv", Some(2), Some(1), false),
        file("Show.S02E03E04.mkv", Some(2), Some(3), true),
    ];
    let gaps = episode_number_gaps(&files);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0].season, 1);
    assert_eq!(gaps[0].missing_count, 7);
    assert_eq!(gaps[0].examples, vec![2, 4, 5, 6, 7, 8, 9]);

    let long_gap = vec![
        file("Show.S03E01.mkv", Some(3), Some(1), false),
        file("Show.S03E1000.mkv", Some(3), Some(1000), false),
    ];
    let long_gaps = episode_number_gaps(&long_gap);
    assert_eq!(long_gaps[0].missing_count, 998);
    assert_eq!(long_gaps[0].examples.len(), 8);
}

#[test]
fn folder_scan_includes_filename_gap_advisories() {
    let root = scratch();
    fs::write(root.join("Show.S01E01.mkv"), b"fixture").unwrap();
    fs::write(root.join("Show.S01E03.mkv"), b"fixture").unwrap();

    let scan = scan_organizer_folder(root.to_string_lossy().into_owned()).unwrap();
    assert_eq!(scan.files.len(), 2);
    assert_eq!(scan.episode_gaps.len(), 1);
    assert_eq!(scan.episode_gaps[0].season, 1);
    assert_eq!(scan.episode_gaps[0].missing_count, 1);
    assert_eq!(scan.episode_gaps[0].examples, vec![2]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_preview_id_and_missing_confirmation_never_rename() {
    let root = scratch();
    let source = root.join("source.mkv");
    fs::write(&source, b"source bytes").unwrap();
    let make_plan = || {
        vec![input(
            &source,
            RenameTemplateKind::Movie,
            "renamed{ext}",
            &[("title", "renamed")],
        )]
    };
    let preview = preview_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        make_plan(),
        Vec::new(),
    )
    .unwrap();
    assert!(apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        make_plan(),
        Vec::new(),
        preview.preview_id.clone(),
        false
    )
    .is_err());
    assert!(apply_rename_batch_with_patterns(
        root.to_string_lossy().into_owned(),
        make_plan(),
        Vec::new(),
        "stale-preview".into(),
        true
    )
    .is_err());
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    assert!(!root.join("renamed.mkv").exists());
    fs::remove_dir_all(root).unwrap();
}
