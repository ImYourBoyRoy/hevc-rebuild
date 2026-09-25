// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hevc_rebuild_studio_lib::initialize_portable_storage();
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref() {
        Some(arg) if arg == "--mcp" || arg == "mcp" => {
            std::process::exit(hevc_rebuild_studio_lib::mcp_mode())
        }
        Some(arg) if arg == "cli" => {
            std::process::exit(hevc_rebuild_studio_lib::cli_mode("cli", args.collect()))
        }
        Some(arg) if arg == "json" => {
            std::process::exit(hevc_rebuild_studio_lib::cli_mode("json", args.collect()))
        }
        Some(arg) if arg == "--help" || arg == "-h" => {
            println!("HEVC Rebuild Studio\n\nUsage:\n  hevc-rebuild-studio                         Launch the desktop app\n  hevc-rebuild-studio cli [engine options]    Run the original media engine CLI\n  hevc-rebuild-studio json [bridge action]    Return machine-readable JSON\n  hevc-rebuild-studio --mcp                   Start the local MCP stdio server\n");
        }
        _ => hevc_rebuild_studio_lib::run(),
    }
}
