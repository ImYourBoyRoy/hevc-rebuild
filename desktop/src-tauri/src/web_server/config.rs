//! `serve` command line: option parsing, the safety rules for where the server may listen, and the
//! blocking entry point that runs it.

use super::assets::{self, AssetSource};
use super::auth::Token;
use super::events::EventBus;
use super::{router, AppContext};
use crate::RunState;
use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) const DEFAULT_PORT: u16 = 8437;
const TOKEN_ENV: &str = "HEVC_REBUILD_SERVE_TOKEN";
const WEB_ROOT_ENV: &str = "HEVC_REBUILD_WEB_ROOT";

const HELP: &str = "HEVC Rebuild Studio headless server\n\n\
Usage:\n  hevc-rebuild-studio serve [options]\n\n\
Options:\n\
  --host ADDRESS        Address to listen on (default 127.0.0.1)\n\
  --port N              Port to listen on (default 8437)\n\
  --allow-remote        Allow a non-loopback --host (needs --allowed-host)\n\
  --allowed-host NAME   Host name browsers will use to reach the server; repeatable\n\
  --read-only           Observation only: refuse every command that changes state\n\
  --token-file PATH     Read the access token from a file instead of generating one\n\
                        (or set HEVC_REBUILD_SERVE_TOKEN)\n\
  --web-root DIR        Serve the GUI from DIR (for example desktop/build) instead of the\n\
                        copy embedded in this executable (or set HEVC_REBUILD_WEB_ROOT)\n\
  -h, --help            Show this help\n\n\
The server needs a token for every request. Without --token-file it generates a new one and\n\
prints a login link. Use a reverse proxy for TLS; see docs/maintainer/SERVE_MODE.md.\n";

#[derive(Debug)]
pub(crate) struct ServeConfig {
    pub(crate) address: SocketAddr,
    pub(crate) read_only: bool,
    pub(crate) token: Option<Token>,
    pub(crate) allowed_hosts: Vec<String>,
    pub(crate) web_root: Option<PathBuf>,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Token(..)")
    }
}

#[derive(Debug)]
pub(crate) enum Parsed {
    Help,
    Run(ServeConfig),
}

fn value_of(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value."))
}

/// Parse the options after `serve`. `env_token` / `env_web_root` are the environment fallbacks, passed in
/// so tests never touch the process environment.
pub(crate) fn parse_args(
    args: &[String],
    env_token: Option<String>,
    env_web_root: Option<String>,
) -> Result<Parsed, String> {
    let mut host: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut port = DEFAULT_PORT;
    let mut allow_remote = false;
    let mut read_only = false;
    let mut allowed_hosts = Vec::new();
    let mut token_file: Option<PathBuf> = None;
    let mut web_root = env_web_root
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--allow-remote" => allow_remote = true,
            "--read-only" => read_only = true,
            "--host" => {
                let text = value_of(args, index, "--host")?;
                host = if text == "localhost" {
                    IpAddr::V4(Ipv4Addr::LOCALHOST)
                } else {
                    text.parse()
                        .map_err(|_| format!("--host must be an IP address, not `{text}`."))?
                };
                index += 1;
            }
            "--port" => {
                let text = value_of(args, index, "--port")?;
                port = text.parse().map_err(|_| {
                    format!("--port must be a number from 1 to 65535, not `{text}`.")
                })?;
                if port == 0 {
                    return Err("--port must be a number from 1 to 65535.".into());
                }
                index += 1;
            }
            "--allowed-host" => {
                allowed_hosts.push(value_of(args, index, "--allowed-host")?);
                index += 1;
            }
            "--token-file" => {
                token_file = Some(PathBuf::from(value_of(args, index, "--token-file")?));
                index += 1;
            }
            "--web-root" => {
                web_root = Some(PathBuf::from(value_of(args, index, "--web-root")?));
                index += 1;
            }
            other => {
                return Err(format!(
                    "Unknown serve option `{other}`. Run `hevc-rebuild-studio serve --help`."
                ))
            }
        }
        index += 1;
    }
    if !host.is_loopback() {
        if !allow_remote {
            return Err(format!("Refusing to listen on {host}: pass --allow-remote to expose the server beyond this computer."));
        }
        if allowed_hosts.is_empty() {
            return Err("--allow-remote also needs at least one --allowed-host NAME (the name or address browsers will use).".into());
        }
    }
    let token = match (
        token_file,
        env_token.filter(|value| !value.trim().is_empty()),
    ) {
        (Some(path), _) => {
            let text = std::fs::read_to_string(&path).map_err(|error| {
                format!("Could not read the token file {}: {error}", path.display())
            })?;
            Some(Token::from_secret(&text)?)
        }
        (None, Some(secret)) => Some(Token::from_secret(&secret)?),
        (None, None) => None,
    };
    Ok(Parsed::Run(ServeConfig {
        address: SocketAddr::new(host, port),
        read_only,
        token,
        allowed_hosts,
        web_root,
    }))
}

fn asset_source(web_root: Option<PathBuf>) -> Result<AssetSource, String> {
    let source = match web_root {
        Some(root) => AssetSource::Directory(root),
        None => AssetSource::Memory(assets::embedded_assets()),
    };
    if source.has_index() {
        return Ok(source);
    }
    Err("No GUI files were found. Build the frontend (`pnpm --dir desktop build`) and pass --web-root desktop/build, or use an executable built with the embedded frontend.".into())
}

async fn run(
    config: ServeConfig,
    generated: bool,
    token: Token,
    source: AssetSource,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(config.address)
        .await
        .map_err(|error| format!("Could not listen on {}: {error}", config.address))?;
    let bound = listener.local_addr().map_err(|error| error.to_string())?;
    let bus = EventBus::default();
    let run_state = RunState::default();
    let events = super::bus_events(&bus);
    let privilege_route: super::PrivilegeProbe = Arc::new(crate::provision::probe_headless_route);
    let mcp = super::mcp_for(&run_state, &events, &privilege_route);
    mcp.start_event_bridge();
    let ctx = AppContext {
        mcp,
        run_state,
        events,
        bus,
        privilege_route,
        read_only: config.read_only,
        token: Arc::new(token.clone()),
        allowed_hosts: Arc::new(config.allowed_hosts),
        assets: Arc::new(source),
    };
    println!("HEVC Rebuild Studio server listening on http://{bound}");
    if generated {
        println!(
            "Open this login link in a browser (it sets a session cookie, then hides the token):"
        );
        println!("  http://{bound}/?token={}", token.expose());
        println!(
            "Agents authenticate with the header:  Authorization: Bearer {}",
            token.expose()
        );
    } else {
        println!("Authenticate with the configured token: open http://{bound}/?token=<token>, or send `Authorization: Bearer <token>`.");
    }
    if config.read_only {
        println!("Read-only mode: commands that change state are refused.");
    }
    println!("Press Ctrl+C to stop.");
    axum::serve(listener, router(ctx))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|error| format!("The server stopped: {error}"))
}

/// Entry point for `hevc-rebuild-studio serve ...`; returns the process exit code.
pub fn serve_mode(args: Vec<OsString>) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let config = match parse_args(
        &args,
        std::env::var(TOKEN_ENV).ok(),
        std::env::var(WEB_ROOT_ENV).ok(),
    ) {
        Ok(Parsed::Help) => {
            print!("{HELP}");
            return 0;
        }
        Ok(Parsed::Run(config)) => config,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let source = match asset_source(config.web_root.clone()) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };
    let (generated, token) = match config.token.clone() {
        Some(token) => (false, token),
        None => match Token::generate() {
            Ok(token) => (true, token),
            Err(message) => {
                eprintln!("{message}");
                return 1;
            }
        },
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start the async runtime: {error}");
            return 1;
        }
    };
    match runtime.block_on(run(config, generated, token, source)) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Parsed, String> {
        parse_args(
            &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
            None,
            None,
        )
    }

    fn config(args: &[&str]) -> ServeConfig {
        match parse(args).expect("valid options") {
            Parsed::Run(config) => config,
            Parsed::Help => panic!("unexpected help"),
        }
    }

    #[test]
    fn defaults_listen_on_loopback_with_a_generated_token_and_full_access() {
        let config = config(&[]);
        assert_eq!(
            config.address,
            "127.0.0.1:8437".parse::<SocketAddr>().unwrap()
        );
        assert!(
            !config.read_only
                && config.token.is_none()
                && config.allowed_hosts.is_empty()
                && config.web_root.is_none()
        );
    }

    #[test]
    fn help_and_unknown_options_are_reported() {
        assert!(matches!(parse(&["--help"]), Ok(Parsed::Help)));
        assert!(parse(&["--no-auth"])
            .unwrap_err()
            .contains("Unknown serve option `--no-auth`"));
        assert!(parse(&["--port"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--port", "http"]).is_err() && parse(&["--port", "0"]).is_err());
    }

    #[test]
    fn listening_beyond_loopback_needs_explicit_opt_in_and_an_allowed_host() {
        assert!(parse(&["--host", "0.0.0.0"])
            .unwrap_err()
            .contains("--allow-remote"));
        assert!(parse(&["--host", "0.0.0.0", "--allow-remote"])
            .unwrap_err()
            .contains("--allowed-host"));
        let config = config(&[
            "--host",
            "0.0.0.0",
            "--allow-remote",
            "--allowed-host",
            "studio.example.com",
            "--port",
            "9000",
        ]);
        assert_eq!(config.address.port(), 9000);
        assert_eq!(config.allowed_hosts, vec!["studio.example.com".to_string()]);
        assert!(parse(&["--host", "not-an-ip"]).is_err());
        assert_eq!(
            self::config(&["--host", "localhost"]).address.ip(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert!(self::config(&["--host", "::1"]).address.ip().is_loopback());
    }

    #[test]
    fn token_comes_from_a_file_or_the_environment_and_must_be_strong_enough() {
        let directory = crate::test_dir("serve-token");
        let file = directory.join("token");
        std::fs::write(&file, "file-token-0123456789abcdef\n").unwrap();
        let from_file = config(&["--token-file", file.to_str().unwrap()]);
        assert_eq!(
            from_file.token.unwrap().expose(),
            "file-token-0123456789abcdef"
        );
        std::fs::write(&file, "short").unwrap();
        assert!(parse(&["--token-file", file.to_str().unwrap()]).is_err());
        assert!(parse(&["--token-file", directory.join("missing").to_str().unwrap()]).is_err());
        let from_env = parse_args(&[], Some("env-token-0123456789abcdef".into()), None).unwrap();
        match from_env {
            Parsed::Run(config) => {
                assert_eq!(config.token.unwrap().expose(), "env-token-0123456789abcdef")
            }
            Parsed::Help => panic!(),
        }
        assert!(parse_args(&[], Some("short".into()), None).is_err());
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn read_only_and_web_root_options_are_recorded() {
        let config = config(&["--read-only", "--web-root", "desktop/build"]);
        assert!(config.read_only);
        assert_eq!(config.web_root, Some(PathBuf::from("desktop/build")));
        match parse_args(&[], None, Some("/srv/gui".into())).unwrap() {
            Parsed::Run(config) => assert_eq!(config.web_root, Some(PathBuf::from("/srv/gui"))),
            Parsed::Help => panic!(),
        }
    }

    #[test]
    fn a_missing_gui_directory_is_a_clear_error() {
        let directory = crate::test_dir("serve-no-gui");
        let error = asset_source(Some(directory.clone()))
            .err()
            .expect("no index.html");
        assert!(error.contains("--web-root"));
        std::fs::write(directory.join("index.html"), "<html></html>").unwrap();
        assert!(asset_source(Some(directory.clone())).is_ok());
        std::fs::remove_dir_all(directory).ok();
    }
}
