//! Process entry points for the `cli`, `json`, internal, metadata, organizer and MCP command-line modes.

use super::*;

pub fn cli_mode(mode: &str, args: Vec<std::ffi::OsString>) -> i32 {
    let strings: Vec<String> = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if mode == "json" {
        return run_json_cli(&strings);
    }
    if strings
        .iter()
        .any(|argument| argument == "--youtube-upload")
    {
        return match youtube_run::YoutubeRunOptions::from_engine_args(&strings).and_then(
            |options| {
                youtube_run::run_youtube_export(
                    &options,
                    |_| {},
                    |line| eprintln!("{line}"),
                    |_| {},
                )
            },
        ) {
            Ok(job) => {
                for line in youtube_run::closing_lines(&job) {
                    println!("{line}");
                }
                i32::from(job.failed > 0)
            }
            Err(error) => {
                eprintln!("{error}");
                2
            }
        };
    }
    if strings.iter().any(|argument| argument == "--metadata-only") {
        return match metadata_run::MetadataRunOptions::from_engine_args(&strings).and_then(
            |options| {
                metadata_run::run_metadata_only(
                    &options,
                    |_| {},
                    |line| eprintln!("{line}"),
                    |_| {},
                )
            },
        ) {
            Ok(job) => {
                for line in metadata_run::closing_lines(&job) {
                    println!("{line}");
                }
                i32::from(job.failed > 0)
            }
            Err(error) => {
                eprintln!("{error}");
                2
            }
        };
    }
    match rebuild_run::RebuildRunOptions::from_engine_args(&strings).and_then(|options| {
        rebuild_run::run_rebuild(&options, |_| {}, |line| eprintln!("{line}"), |_| {})
    }) {
        Ok(job) => {
            for line in rebuild_run::closing_lines(&job) {
                println!("{line}");
            }
            i32::from(job.failed > 0)
        }
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}

pub fn internal_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let mut args = args.into_iter();
    let Some(action) = args.next() else {
        eprintln!("Missing internal helper action.");
        return 2;
    };
    match action.to_string_lossy().as_ref() {
        "packet-fingerprints" | "stream-copy-hash" => {
            packet_verify::cli_mode(std::iter::once(action).chain(args).collect())
        }
        "validate-streams" => validation::cli_mode(args.collect()),
        "runtime-guard" => runtime_guard::cli_mode(args.collect()),
        "encode-plan" => encode_plan::cli_mode(args.collect()),
        "encode-supervisor" => encode_supervisor::cli_mode(args.collect()),
        "hdr-metadata" => hdr_metadata::cli_mode(args.collect()),
        "commit-output" => transactions::cli_mode(args.collect()),
        "install-output-copy" => transactions::install_copy_cli_mode(args.collect()),
        "calibration" => calibration::cli_mode(args.collect()),
        "content-tune" => content::cli_mode(args.collect()),
        "metadata-text" => metadata_text::cli_mode(args.collect()),
        "metadata-tags" => matroska_tags::cli_mode(args.collect()),
        "metadata-scrub" => matroska_scrub::cli_mode(args.collect()),
        "metadata-apply" => matroska_apply::cli_mode(args.collect()),
        "metadata-poster" => matroska_poster::cli_mode(args.collect()),
        "metadata-verify" => matroska_verify::cli_mode(args.collect()),
        "report-append" => reports::cli_mode(args.collect()),
        "resource-plan" => resource_plan::runtime_cli_mode(args.collect()),
        "resource-retry" => resource_plan::retry_cli_mode(args.collect()),
        "youtube" => youtube::cli_mode(args.collect()),
        "discover" => media_scan::discover_cli_mode(),
        "engine-profile" => {
            if args.next().is_some() {
                eprintln!("The internal engine-profile command does not accept options.");
                2
            } else {
                media_scan::engine_profile_cli_mode()
            }
        }
        "engine-profile-tracks" => {
            if args.next().is_some() {
                eprintln!("The internal engine-profile-tracks command does not accept options.");
                2
            } else {
                media_scan::engine_track_profiles_cli_mode()
            }
        }
        "resolve-toolchain" => environment::resolve_toolchain_cli_mode(),
        _ => {
            eprintln!("Unknown internal helper action.");
            2
        }
    }
}

pub fn metadata_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    if args.first().is_some_and(|command| command == "assets") {
        return library_assets::cli_mode(&args);
    }
    let usage = "Usage: hevc-rebuild-studio metadata movie search TITLE | movie details WIKIDATA_ID | tv search TITLE | tv details TVMAZE_ID | artwork save REQUEST.json | assets inspect REQUEST.json";
    if args.len() < 3 {
        eprintln!("{usage}");
        return 2;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start metadata lookup runtime: {error}");
            return 2;
        }
    };
    let result = match (args[0].as_str(), args[1].as_str()) {
        ("movie", "search") if args.len() >= 3 => {
            let query = args[2..].join(" ");
            runtime
                .block_on(library_metadata::search_films(&query))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("movie", "details") if args.len() == 3 => runtime
            .block_on(library_metadata::load_film_details(&args[2]))
            .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string())),
        ("tv", "search") if args.len() >= 3 => {
            let query = args[2..].join(" ");
            runtime
                .block_on(library_metadata::search_tv_shows(&query))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("tv", "details") if args.len() == 3 => {
            let Ok(show_id) = args[2].parse::<u64>() else {
                eprintln!("TVmaze show ID must be a positive integer.");
                return 2;
            };
            runtime
                .block_on(library_metadata::load_tv_series_details(show_id))
                .and_then(|value| serde_json::to_value(value).map_err(|error| error.to_string()))
        }
        ("artwork", "save") if args.len() == 3 => {
            let request_text = match fs::read_to_string(&args[2]) {
                Ok(value) if value.len() <= 1024 * 1024 => value,
                Ok(_) => {
                    eprintln!("Artwork request JSON is limited to 1 MiB.");
                    return 2;
                }
                Err(error) => {
                    eprintln!("Could not read artwork request JSON: {error}");
                    return 2;
                }
            };
            match serde_json::from_str::<library_artwork::ArtworkSaveRequest>(&request_text) {
                Ok(request) => runtime
                    .block_on(library_artwork::save_artwork(
                        request.candidate,
                        request.root,
                        request.targets,
                    ))
                    .and_then(|value| {
                        serde_json::to_value(value).map_err(|error| error.to_string())
                    }),
                Err(error) => Err(format!("Artwork request JSON is invalid: {error}")),
            }
        }
        _ => {
            eprintln!("{usage}");
            return 2;
        }
    };
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                0
            }
            Err(error) => {
                eprintln!("Could not format metadata JSON: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

pub fn organizer_cli_mode(args: Vec<std::ffi::OsString>) -> i32 {
    let strings = args
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    library_organizer::organizer_cli_mode(&strings)
}

pub fn mcp_mode() -> i32 {
    mcp_server::run()
}
