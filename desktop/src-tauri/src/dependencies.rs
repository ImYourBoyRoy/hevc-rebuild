use flate2::read::GzDecoder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HDR10PLUS_RELEASE_API: &str =
    "https://api.github.com/repos/quietvoid/hdr10plus_tool/releases/latest";
const MAX_RELEASE_JSON_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CHECKSUM_BYTES: u64 = 64 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 8192;

const APT_LISTS: &str = "/var/lib/apt/lists";
const APT_CATALOG_MAX_AGE_SECONDS: f64 = 12.0 * 60.0 * 60.0;

fn command_output(program: &str, arguments: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("Could not run {program}: {error}"))
}

fn installed_version(package: &str) -> Option<String> {
    let output = command_output("dpkg-query", &["-W", "-f=${Status}\t${Version}", package]).ok()?;
    if !output.status.success() {
        return None;
    }
    parse_installed_version(&output.stdout)
}

fn candidate_version(package: &str) -> Option<String> {
    let output = command_output("apt-cache", &["policy", package]).ok()?;
    if !output.status.success() {
        return None;
    }
    parse_candidate_version(&output.stdout)
}

fn parse_installed_version(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    let (status, version) = text.trim().split_once('\t')?;
    (status == "install ok installed" && !version.is_empty()).then(|| version.to_string())
}

fn parse_candidate_version(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output).lines().find_map(|line| {
        let (name, value) = line.trim().split_once(':')?;
        (name == "Candidate" && value.trim() != "(none)").then(|| value.trim().to_string())
    })
}

fn catalog_is_stale(updated_at: Option<f64>, now: f64) -> bool {
    updated_at.is_none_or(|updated| now - updated >= APT_CATALOG_MAX_AGE_SECONDS)
}

fn package_version_is_older(installed: &str, candidate: &str) -> bool {
    Command::new("dpkg")
        .args(["--compare-versions", installed, "lt", candidate])
        .status()
        .is_ok_and(|status| status.success())
}

fn latest_catalog_timestamp() -> Option<f64> {
    let entries = fs::read_dir(APT_LISTS).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .filter_map(|metadata| metadata.modified().ok())
        .filter_map(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64())
        .max_by(f64::total_cmp)
}

fn current_timestamp() -> Result<f64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .map_err(|error| format!("System clock is before the Unix epoch: {error}"))
}

/// Inspect the locally cached APT catalog. This function never refreshes or installs packages.
pub(crate) fn apt_update_status() -> Result<Value, String> {
    let environment = super::environment::inspect_environment(super::ToolPaths::default(), None)?;
    if environment
        .pointer("/os/package_manager")
        .and_then(Value::as_str)
        != Some("apt")
    {
        return Err("APT package checks are available only on Linux hosts using APT; use the host platform setup lane instead.".into());
    }
    if environment
        .pointer("/os/supported")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(
            "APT package checks are limited to supported Ubuntu and Linux Mint releases.".into(),
        );
    }
    for program in ["apt-get", "apt-cache", "dpkg", "dpkg-query"] {
        if !super::executable_in_path(program) {
            return Err(format!("APT package checks require {program} on PATH."));
        }
    }

    let now = current_timestamp()?;
    let catalog_updated_at = latest_catalog_timestamp();
    let catalog_stale = catalog_is_stale(catalog_updated_at, now);
    let mut packages = Vec::new();
    let mut allowlist = super::APT_ALLOWLIST.to_vec();
    allowlist.sort_unstable();
    for package in allowlist {
        let Some(installed) = installed_version(package) else {
            continue;
        };
        let Some(candidate) = candidate_version(package) else {
            continue;
        };
        if package_version_is_older(&installed, &candidate) {
            packages.push(json!({
                "name": package,
                "installed_version": installed,
                "candidate_version": candidate,
            }));
        }
    }

    Ok(json!({
        "packages": packages,
        "catalog_updated_at": catalog_updated_at,
        "catalog_stale": catalog_stale,
        "source": "local APT package index",
        "checked_at": now,
    }))
}

/// Download and install the official HDR10+ helper after the user explicitly requests it.
/// The asset must be HTTPS-hosted by GitHub, publish a SHA-256 digest or sidecar, and
/// contain exactly one regular file named hdr10plus_tool.
pub(crate) fn install_latest_hdr10plus_tool() -> Result<String, String> {
    let environment = super::environment::inspect_environment(super::ToolPaths::default(), None)?;
    if environment
        .pointer("/os/supported")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(
            "Managed hdr10plus_tool installation is limited to supported Ubuntu 24.04+, Linux Mint 22+, Windows, and macOS hosts."
                .into(),
        );
    }

    let (token, suffix) = hdr10plus_asset_selector()?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("hevc-rebuild-studio/1.3.0")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not configure the HDR10+ helper download: {error}"))?;

    let release_bytes = fetch_limited(
        &client,
        HDR10PLUS_RELEASE_API,
        &["api.github.com"],
        MAX_RELEASE_JSON_BYTES,
    )?;
    let release: Value = serde_json::from_slice(&release_bytes).map_err(|error| {
        format!("The official HDR10+ release response was invalid JSON: {error}")
    })?;
    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("The official HDR10+ release response contained no asset list.")?;
    let asset = assets
        .iter()
        .find(|asset| {
            asset
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.contains(token) && name.ends_with(suffix))
        })
        .ok_or_else(|| {
            format!("The latest hdr10plus_tool release has no {token} {suffix} asset.")
        })?;
    let asset_name = asset
        .get("name")
        .and_then(Value::as_str)
        .ok_or("The selected HDR10+ release asset has no name.")?;
    if Path::new(asset_name)
        .file_name()
        .and_then(|name| name.to_str())
        != Some(asset_name)
    {
        return Err("The HDR10+ release asset name is not a plain filename.".into());
    }
    let download_url = asset
        .get("browser_download_url")
        .and_then(Value::as_str)
        .ok_or("The selected HDR10+ release asset has no download URL.")?;
    let expected_digest = checksum_from_release(&client, assets, asset_name, asset)?;

    let temporary = TemporaryDirectory::create("hevc-rebuild-hdr10plus")?;
    let archive_path = temporary.0.join(asset_name);
    let actual_digest = download_to_file(&client, download_url, &archive_path)?;
    if actual_digest != expected_digest {
        return Err("hdr10plus_tool SHA-256 verification failed; refusing installation.".into());
    }
    let extracted_path = temporary.0.join("verified-hdr10plus-tool");
    extract_single_tool(&archive_path, &extracted_path)?;
    set_executable_permissions(&extracted_path)?;
    let version = tool_version(&extracted_path)?;

    let mut installed = Vec::new();
    let mut failures = Vec::new();
    for directory in super::environment::managed_tool_directories() {
        match place_verified_tool(&extracted_path, &version, &directory) {
            Ok(path) => installed.push(path),
            Err(error) => failures.push(format!("{}: {error}", directory.display())),
        }
    }
    let Some(path) = installed.into_iter().next() else {
        return Err(format!(
            "Could not install the verified HDR10+ helper. {}",
            failures.join(" ")
        ));
    };
    Ok(path.to_string_lossy().into_owned())
}

fn hdr10plus_asset_selector() -> Result<(&'static str, &'static str), String> {
    #[cfg(target_os = "windows")]
    {
        let token = match std::env::consts::ARCH {
            "x86_64" => "x86_64-pc-windows-msvc",
            "aarch64" => "aarch64-pc-windows-msvc",
            architecture => {
                return Err(format!(
                    "No managed hdr10plus_tool binary is configured for architecture {architecture:?}."
                ));
            }
        };
        return Ok((token, ".zip"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(("universal-macOS", ".zip"));
    }
    #[cfg(target_os = "linux")]
    {
        let token = match std::env::consts::ARCH {
            "x86_64" => "x86_64-unknown-linux-musl",
            "aarch64" => "aarch64-unknown-linux-musl",
            architecture => {
                return Err(format!(
                    "No managed hdr10plus_tool binary is configured for architecture {architecture:?}."
                ));
            }
        };
        return Ok((token, ".tar.gz"));
    }
    Err("No managed hdr10plus_tool asset is configured for this operating system.".into())
}

fn place_verified_tool(
    source: &Path,
    expected_version: &str,
    directory: &Path,
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Could not create the managed binary folder: {error}"))?;
    let target = directory.join(managed_tool_name());
    let (staged_path, mut staged_file) = create_unique_file(directory, ".hdr10plus_tool.new")?;
    let stage_result: Result<(), String> = (|| {
        let mut input = File::open(source)
            .map_err(|error| format!("Could not open the verified HDR10+ helper: {error}"))?;
        io::copy(&mut input, &mut staged_file)
            .map_err(|error| format!("Could not stage the HDR10+ helper: {error}"))?;
        staged_file
            .sync_all()
            .map_err(|error| format!("Could not sync the staged HDR10+ helper: {error}"))?;
        // Linux refuses to execute a file that still has a writable descriptor
        // open (ETXTBSY, "Text file busy"). Close the staged copy first.
        drop(staged_file);
        set_executable_permissions(&staged_path)?;
        let staged_version = tool_version(&staged_path)?;
        if staged_version != expected_version {
            return Err(
                "The staged hdr10plus_tool version did not match the verified download.".into(),
            );
        }
        fs::rename(&staged_path, &target)
            .map_err(|error| format!("Could not install the verified HDR10+ helper: {error}"))?;
        Ok(())
    })();
    if stage_result.is_err() {
        let _ = fs::remove_file(&staged_path);
    }
    stage_result?;
    Ok(target)
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn create(prefix: &str) -> Result<Self, String> {
        let root = std::env::temp_dir();
        for attempt in 0..32_u32 {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("System clock is before the Unix epoch: {error}"))?
                .as_nanos();
            let path = root.join(format!("{prefix}-{}-{stamp}-{attempt}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "Could not create a temporary helper folder: {error}"
                    ));
                }
            }
        }
        Err("Could not allocate a unique temporary helper folder.".into())
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn trusted_url(raw: &str, allowed_initial_hosts: &[&str]) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw)
        .map_err(|error| format!("The HDR10+ release returned an invalid URL: {error}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|port| port != 443)
        || !url
            .host_str()
            .is_some_and(|host| allowed_initial_hosts.contains(&host))
    {
        return Err("The HDR10+ helper URL was not an approved HTTPS GitHub URL.".into());
    }
    Ok(url)
}

fn trusted_redirect(url: &reqwest::Url, allowed_hosts: &[&str]) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
        && url
            .host_str()
            .is_some_and(|host| allowed_hosts.contains(&host))
}

fn send_github_request(
    client: &reqwest::blocking::Client,
    raw_url: &str,
    allowed_initial_hosts: &[&str],
) -> Result<reqwest::blocking::Response, String> {
    let url = trusted_url(raw_url, allowed_initial_hosts)?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .map_err(|error| format!("Could not download the HDR10+ release: {error}"))?;
    let redirects = [
        "api.github.com",
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
    ];
    if !trusted_redirect(response.url(), &redirects) {
        return Err("The HDR10+ download redirected outside the approved GitHub hosts.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned HTTP {} while fetching the HDR10+ helper.",
            response.status()
        ));
    }
    Ok(response)
}

fn fetch_limited(
    client: &reqwest::blocking::Client,
    raw_url: &str,
    initial_hosts: &[&str],
    maximum: u64,
) -> Result<Vec<u8>, String> {
    let response = send_github_request(client, raw_url, initial_hosts)?;
    if response
        .content_length()
        .is_some_and(|length| length > maximum)
    {
        return Err("The HDR10+ release response exceeded its size limit.".into());
    }
    let mut bytes = Vec::new();
    response
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read the HDR10+ release response: {error}"))?;
    if bytes.len() as u64 > maximum {
        return Err("The HDR10+ release response exceeded its size limit.".into());
    }
    Ok(bytes)
}

fn checksum_from_release(
    client: &reqwest::blocking::Client,
    assets: &[Value],
    asset_name: &str,
    asset: &Value,
) -> Result<String, String> {
    if let Some(digest) = asset.get("digest").and_then(Value::as_str) {
        if let Some(hex) = digest.strip_prefix("sha256:") {
            return normalize_sha256(hex)
                .ok_or("The HDR10+ release published an invalid SHA-256 digest.".into());
        }
    }
    for suffix in [".sha256", ".sha256sum", ".sha256.txt"] {
        let sidecar_name = format!("{asset_name}{suffix}");
        let Some(sidecar) = assets.iter().find(|candidate| {
            candidate.get("name").and_then(Value::as_str) == Some(sidecar_name.as_str())
        }) else {
            continue;
        };
        let Some(url) = sidecar.get("browser_download_url").and_then(Value::as_str) else {
            continue;
        };
        let bytes = fetch_limited(client, url, &["github.com"], MAX_CHECKSUM_BYTES)?;
        let text = String::from_utf8_lossy(&bytes);
        if let Some(digest) = find_sha256(&text) {
            return Ok(digest);
        }
    }
    Err(
        "The upstream hdr10plus_tool asset did not expose a SHA-256 digest or checksum sidecar; refusing an unverifiable binary install."
            .into(),
    )
}

fn normalize_sha256(value: &str) -> Option<String> {
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn find_sha256(text: &str) -> Option<String> {
    text.split(|character: char| !character.is_ascii_hexdigit())
        .find_map(normalize_sha256)
}

fn download_to_file(
    client: &reqwest::blocking::Client,
    raw_url: &str,
    path: &Path,
) -> Result<String, String> {
    let mut response = send_github_request(client, raw_url, &["github.com"])?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("The HDR10+ helper archive exceeded the 128 MiB download limit.".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create the temporary HDR10+ archive: {error}"))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| format!("Could not read the HDR10+ helper archive: {error}"))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("The HDR10+ helper archive size overflowed.")?;
        if total > MAX_ARCHIVE_BYTES {
            return Err("The HDR10+ helper archive exceeded the 128 MiB download limit.".into());
        }
        digest.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("Could not save the HDR10+ helper archive: {error}"))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("Could not sync the HDR10+ helper archive: {error}"))?;
    Ok(encode_hex(&digest.finalize()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn extract_single_tool(archive_path: &Path, destination: &Path) -> Result<(), String> {
    if archive_path.extension().and_then(|value| value.to_str()) == Some("zip") {
        return extract_single_tool_zip(archive_path, destination);
    }
    let compressed = File::open(archive_path)
        .map_err(|error| format!("Could not open the verified HDR10+ archive: {error}"))?;
    let decoder = GzDecoder::new(compressed);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("The verified HDR10+ tarball is invalid: {error}"))?;
    let mut count = 0_usize;
    let mut expanded = 0_u64;
    let mut extracted = false;
    for result in entries {
        count += 1;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err("The HDR10+ archive contained too many entries.".into());
        }
        let entry =
            result.map_err(|error| format!("The verified HDR10+ tarball is invalid: {error}"))?;
        let size = entry
            .header()
            .size()
            .map_err(|error| format!("The HDR10+ archive entry size is invalid: {error}"))?;
        expanded = expanded
            .checked_add(size)
            .ok_or("The HDR10+ archive expanded size overflowed.")?;
        if expanded > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err("The HDR10+ archive exceeded the 512 MiB expanded-size limit.".into());
        }
        let is_binary = {
            let path = entry.path().map_err(|error| {
                format!("The HDR10+ archive contained an invalid path: {error}")
            })?;
            entry.header().entry_type().is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name == managed_tool_name())
        };
        if is_binary {
            if extracted {
                return Err(
                    "The HDR10+ archive contained more than one hdr10plus_tool file.".into(),
                );
            }
            if size == 0 || size > MAX_BINARY_BYTES {
                return Err("The HDR10+ executable had an invalid or excessive size.".into());
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .map_err(|error| {
                    format!("Could not create the extracted HDR10+ helper: {error}")
                })?;
            let copied = io::copy(&mut entry.take(MAX_BINARY_BYTES + 1), &mut output)
                .map_err(|error| format!("Could not extract the HDR10+ helper: {error}"))?;
            if copied != size {
                return Err(
                    "The extracted HDR10+ helper size did not match its tar header.".into(),
                );
            }
            output
                .sync_all()
                .map_err(|error| format!("Could not sync the extracted HDR10+ helper: {error}"))?;
            extracted = true;
        }
    }
    if !extracted {
        return Err(
            "The HDR10+ archive did not contain a regular hdr10plus_tool executable.".into(),
        );
    }
    Ok(())
}

fn extract_single_tool_zip(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive_path)
        .map_err(|error| format!("Could not open the verified HDR10+ ZIP archive: {error}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("The verified HDR10+ ZIP archive is invalid: {error}"))?;
    let mut expanded = 0_u64;
    let mut extracted = false;
    for index in 0..archive.len() {
        if index >= MAX_ARCHIVE_ENTRIES {
            return Err("The HDR10+ archive contained too many entries.".into());
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("The verified HDR10+ ZIP archive is invalid: {error}"))?;
        let size = entry.size();
        expanded = expanded
            .checked_add(size)
            .ok_or("The HDR10+ archive expanded size overflowed.")?;
        if expanded > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err("The HDR10+ archive exceeded the 512 MiB expanded-size limit.".into());
        }
        let is_binary = entry.is_file()
            && Path::new(entry.name())
                .file_name()
                .is_some_and(|name| name == managed_tool_name());
        if !is_binary {
            continue;
        }
        if extracted {
            return Err(format!(
                "The HDR10+ archive contained more than one {} file.",
                managed_tool_name()
            ));
        }
        if size == 0 || size > MAX_BINARY_BYTES {
            return Err("The HDR10+ executable had an invalid or excessive size.".into());
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| format!("Could not create the extracted HDR10+ helper: {error}"))?;
        let mut limited = (&mut entry).take(MAX_BINARY_BYTES + 1);
        let copied = io::copy(&mut limited, &mut output)
            .map_err(|error| format!("Could not extract the HDR10+ helper: {error}"))?;
        if copied != size {
            return Err("The extracted HDR10+ helper size did not match its ZIP entry.".into());
        }
        output
            .sync_all()
            .map_err(|error| format!("Could not sync the extracted HDR10+ helper: {error}"))?;
        extracted = true;
    }
    if !extracted {
        return Err(format!(
            "The archive did not contain a regular {} executable.",
            managed_tool_name()
        ));
    }
    Ok(())
}

fn managed_tool_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        return "hdr10plus_tool.exe";
    }
    "hdr10plus_tool"
}

fn create_unique_file(directory: &Path, prefix: &str) -> Result<(PathBuf, File), String> {
    for attempt in 0..32_u32 {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("System clock is before the Unix epoch: {error}"))?
            .as_nanos();
        let path = directory.join(format!("{prefix}.{}.{stamp}.{attempt}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not create a staged HDR10+ helper: {error}")),
        }
    }
    Err("Could not allocate a unique staged HDR10+ helper path.".into())
}

fn set_executable_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Could not make the HDR10+ helper executable: {error}"))?;
    }
    Ok(())
}

fn tool_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("Could not run the HDR10+ helper version check: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "The HDR10+ helper failed its --version check with {}.",
            output.status
        ));
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_tool_version(&text).ok_or("The HDR10+ helper returned no recognizable version.".into())
}

fn parse_tool_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for (start, byte) in bytes.iter().enumerate() {
        if !byte.is_ascii_digit() || (start > 0 && bytes[start - 1].is_ascii_digit()) {
            continue;
        }
        let end = bytes[start..]
            .iter()
            .position(|value| !value.is_ascii_digit() && *value != b'.')
            .map(|offset| start + offset)
            .unwrap_or(bytes.len());
        let candidate = &text[start..end];
        let parts: Vec<_> = candidate.split('.').collect();
        if (2..=3).contains(&parts.len())
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|value| value.is_ascii_digit()))
        {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(test)]
mod hdr10plus_installer_tests {
    use super::{
        extract_single_tool, find_sha256, normalize_sha256, parse_tool_version,
        set_executable_permissions, tool_version, trusted_url, TemporaryDirectory,
    };
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::path::Path;
    use std::process::Command;

    fn append_tool(builder: &mut tar::Builder<GzEncoder<File>>, name: &str, bytes: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes)
            .expect("append archive entry");
    }

    fn create_archive(path: &Path, names: &[&str]) {
        let encoder = GzEncoder::new(
            File::create(path).expect("create archive"),
            Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);
        for name in names {
            append_tool(&mut builder, name, b"verified fixture binary");
        }
        builder.finish().expect("finish tar archive");
        builder
            .into_inner()
            .expect("finish gzip stream")
            .finish()
            .expect("write gzip footer");
    }

    #[test]
    fn parses_semver_from_helper_output_and_rejects_unversioned_text() {
        assert_eq!(
            parse_tool_version("hdr10plus_tool 1.2.3 (release)"),
            Some("1.2.3".into())
        );
        assert_eq!(parse_tool_version("no version available"), None);
        assert_eq!(parse_tool_version("build 20260927"), None);
    }

    #[test]
    fn checksum_parser_accepts_only_sha256_hex() {
        let digest = "aB".repeat(32);
        assert_eq!(normalize_sha256(&digest), Some("ab".repeat(32)));
        assert_eq!(normalize_sha256("xyz"), None);
        assert_eq!(
            find_sha256(&format!("{}  hdr10plus_tool.tar.gz", "0f".repeat(32))),
            Some("0f".repeat(32))
        );
    }

    #[test]
    fn download_urls_require_https_and_an_approved_host() {
        assert!(trusted_url("https://github.com/quietvoid/tool", &["github.com"]).is_ok());
        assert!(trusted_url("http://github.com/quietvoid/tool", &["github.com"]).is_err());
        assert!(trusted_url("https://example.com/tool", &["github.com"]).is_err());
        assert!(trusted_url("https://user@github.com/tool", &["github.com"]).is_err());
    }

    #[test]
    fn archive_reader_extracts_only_the_named_regular_file() {
        let temporary = TemporaryDirectory::create("hevc-rebuild-hdr-test").unwrap();
        let archive_path = temporary.0.join("helper.tar.gz");
        let output_path = temporary.0.join("extracted-tool");
        create_archive(&archive_path, &["release/bin/hdr10plus_tool"]);
        extract_single_tool(&archive_path, &output_path).unwrap();
        assert_eq!(fs::read(output_path).unwrap(), b"verified fixture binary");
    }

    #[test]
    fn archive_reader_rejects_duplicate_helper_names() {
        let temporary = TemporaryDirectory::create("hevc-rebuild-hdr-test").unwrap();
        let archive_path = temporary.0.join("helper.tar.gz");
        let output_path = temporary.0.join("extracted-tool");
        create_archive(
            &archive_path,
            &["release/bin/hdr10plus_tool", "other/hdr10plus_tool"],
        );
        assert!(extract_single_tool(&archive_path, &output_path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn version_check_runs_only_after_the_staged_file_is_closed() {
        let temporary = TemporaryDirectory::create("hevc-rebuild-hdr-busy").unwrap();
        let path = temporary.0.join("hdr10plus_tool");
        fs::write(&path, "#!/bin/sh\nprintf 'hdr10plus_tool 1.2.3\\n'\n").unwrap();
        set_executable_permissions(&path).unwrap();
        let staged = OpenOptions::new().write(true).open(&path).unwrap();
        let busy = Command::new(&path).arg("--version").output().unwrap_err();
        assert_eq!(busy.kind(), io::ErrorKind::ExecutableFileBusy);
        drop(staged);
        assert_eq!(tool_version(&path).unwrap(), "1.2.3");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_candidate_parser_handles_none_and_candidate_lines() {
        let candidate = b"  Candidate: 7:8.1.2-3ubuntu1\n  Version table:\n";
        let parsed = parse_candidate_version(candidate);
        assert_eq!(parsed.as_deref(), Some("7:8.1.2-3ubuntu1"));

        let none = b"  Candidate: (none)\n";
        let parsed = parse_candidate_version(none);
        assert_eq!(parsed, None);
    }

    #[test]
    fn installed_package_parser_requires_the_installed_status() {
        assert_eq!(
            parse_installed_version(b"install ok installed\t1.2.3-1ubuntu1\n").as_deref(),
            Some("1.2.3-1ubuntu1")
        );
        assert_eq!(
            parse_installed_version(b"deinstall ok config-files\t1.2.3-1ubuntu1\n"),
            None
        );
    }

    #[test]
    fn freshness_threshold_is_twelve_hours() {
        assert!(!catalog_is_stale(Some(1_000.0), 43_199.0));
        assert!(catalog_is_stale(Some(1_000.0), 44_200.0));
        assert!(catalog_is_stale(None, 1_000.0));
    }
}
