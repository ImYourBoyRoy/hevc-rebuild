// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next();
    if mode.as_deref().is_some_and(|arg| arg == "internal") {
        std::process::exit(hevc_rebuild_studio_lib::internal_cli_mode(args.collect()));
    }
    hevc_rebuild_studio_lib::initialize_portable_storage();
    match mode.as_deref() {
        Some(arg) if arg == "--mcp" || arg == "mcp" => {
            std::process::exit(hevc_rebuild_studio_lib::mcp_mode())
        }
        Some(arg) if arg == "cli" => {
            std::process::exit(hevc_rebuild_studio_lib::cli_mode("cli", args.collect()))
        }
        Some(arg) if arg == "json" => {
            std::process::exit(hevc_rebuild_studio_lib::cli_mode("json", args.collect()))
        }
        Some(arg) if arg == "metadata" => {
            std::process::exit(hevc_rebuild_studio_lib::metadata_cli_mode(args.collect()))
        }
        Some(arg) if arg == "organize" => {
            std::process::exit(hevc_rebuild_studio_lib::organizer_cli_mode(args.collect()))
        }
        Some(arg) if arg == "--help" || arg == "-h" => {
            println!("HEVC Rebuild Studio\n\nUsage:\n  hevc-rebuild-studio                         Launch the desktop app\n  hevc-rebuild-studio cli [engine options]    Run the original media engine CLI\n  hevc-rebuild-studio json [bridge action]    Return machine-readable JSON\n  hevc-rebuild-studio json queue [options]    Read a page of the saved media queue\n  hevc-rebuild-studio json artwork-cache inspect\n                                             Review private poster-cache size and purgeable items\n  hevc-rebuild-studio json artwork-cache purge --preview-id ID --confirm\n                                             Remove reviewed unused app-managed posters\n  hevc-rebuild-studio json queue-select --request SELECTION.json\n                                             Save global selection from analyzed paths\n  hevc-rebuild-studio json queue-run --request RUNS.json\n                                             Run selected files from one or more roots\n  hevc-rebuild-studio json output-plan --request REQUEST.json\n                                             Preview multi-source output paths\n  hevc-rebuild-studio metadata ...            Search metadata or preview/apply NFO metadata\n  hevc-rebuild-studio organize ...            Scan, preview, or confirm a rename plan\n  hevc-rebuild-studio --mcp                   Start the local MCP stdio server\n");
        }
        _ => hevc_rebuild_studio_lib::run(),
    }
}
