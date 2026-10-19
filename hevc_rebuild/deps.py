from __future__ import annotations

import json
import logging
import os
import platform
import re
import shutil
import stat
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path
from typing import Mapping

from .model import Toolchain
from .util import RebuildError, native_json_action, portable_data_dir, prompt_enter, run, which

APT_PACKAGES = {
    "python3": ["python3"],
    "ffmpeg": ["ffmpeg", "ffprobe"],
    "mediainfo": ["mediainfo"],
    "mkvtoolnix": ["mkvpropedit", "mkvmerge", "mkvextract"],
    "fontconfig": ["fc-list"],
    "ca-certificates": [],
}
WINDOWS_WINGET_PACKAGES = {
    "ffmpeg": "Gyan.FFmpeg",
    "mediainfo": "MediaArea.MediaInfo",
    "mkvtoolnix": "MoritzBunkus.MKVToolNix",
}
WINDOWS_COMMANDS = {
    "ffmpeg": ("ffmpeg", "ffprobe"),
    "mediainfo": ("mediainfo",),
    "mkvtoolnix": ("mkvpropedit", "mkvmerge", "mkvextract"),
}
TOOL_OVERRIDE_KEYS = (
    "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract",
)
GITHUB_RELEASE_API = "https://api.github.com/repos/quietvoid/hdr10plus_tool/releases/latest"
_PORTABLE_DATA = portable_data_dir()
if _PORTABLE_DATA:
    APP_DATA = _PORTABLE_DATA / "share" / "hevc-rebuild"
elif os.name == "nt":
    APP_DATA = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "hevc-rebuild"
else:
    APP_DATA = Path.home() / ".local" / "share" / "hevc-rebuild"
MANAGED_BIN = APP_DATA / "bin"


def is_windows() -> bool:
    return os.name == "nt" or platform.system().lower() == "windows"


def os_release() -> dict[str, str]:
    data: dict[str, str] = {}
    try:
        for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
            if "=" not in line:
                continue
            k, v = line.split("=", 1)
            data[k] = v.strip().strip('"')
    except OSError as exc:
        raise RebuildError(f"Cannot read /etc/os-release: {exc}") from exc
    return data


def version_tuple(text: str) -> tuple[int, ...]:
    nums = re.findall(r"\d+", text)
    return tuple(int(n) for n in nums[:3])


def validate_platform(allow_unsupported: bool = False) -> None:
    """Accept supported Ubuntu/Mint or Windows hosts with the right resolver.

    Runtime capability tests remain authoritative. Distro/version checks only prevent
    accidental use on unrelated package ecosystems where automated package repair
    could be unsafe.
    """
    if is_windows():
        if which("winget"):
            logging.info("Supported Windows platform detected with winget.")
            return
        msg = "Windows builds require the Windows Package Manager (winget) for dependency repair."
        if allow_unsupported:
            logging.warning("%s Continuing because --allow-unsupported-os was supplied.", msg)
            return
        raise RebuildError(msg)

    info = os_release()
    ident = info.get("ID", "").lower()
    version = info.get("VERSION_ID", "0")
    id_like = {x.lower() for x in info.get("ID_LIKE", "").split()}

    ubuntu_ok = ident == "ubuntu" and version_tuple(version) >= version_tuple("24.04")
    mint_ok = (
        ident == "linuxmint"
        and "ubuntu" in id_like
        and version_tuple(version) >= version_tuple("22")
    )
    apt_ok = bool(which("apt-get") and which("apt-cache") and which("dpkg") and which("dpkg-query"))

    if (ubuntu_ok or mint_ok) and apt_ok:
        logging.info(
            "Supported platform detected: ID=%s VERSION_ID=%s ID_LIKE=%s",
            ident, version, " ".join(sorted(id_like)) or "-",
        )
        return

    msg = (
        "This build supports Ubuntu 24.04+ and Ubuntu-based Linux Mint 22+ "
        f"with APT/dpkg. Detected ID={ident!r} VERSION_ID={version!r} "
        f"ID_LIKE={info.get('ID_LIKE', '')!r}."
    )
    if allow_unsupported:
        logging.warning("%s Continuing because --allow-unsupported-os was supplied.", msg)
    else:
        raise RebuildError(msg)


def _refresh_windows_path() -> None:
    """Refresh this Python process after winget changes PATH."""
    if not is_windows():
        return
    machine = os.environ.get("PATH", "")
    try:
        import winreg

        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment") as key:
            machine = winreg.QueryValueEx(key, "Path")[0] or machine
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Environment") as key:
            user = winreg.QueryValueEx(key, "Path")[0] or ""
    except (ImportError, OSError):
        user = ""
    links = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "Microsoft" / "WinGet" / "Links"
    entries = [item for item in (machine, user, str(links)) if item]
    for candidate in (
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "MediaInfo",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "MKVToolNix",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "FFmpeg" / "bin",
    ):
        if candidate.is_dir():
            entries.append(str(candidate))
    os.environ["PATH"] = os.pathsep.join(dict.fromkeys(entries))


def _windows_command_paths() -> dict[str, str | None]:
    _refresh_windows_path()
    return {
        command: which(command)
        for command in (
            "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract",
        )
    }


def _winget_package_present(package_id: str) -> bool:
    result = run(
        ["winget", "list", "--id", package_id, "--exact", "--accept-source-agreements"],
        check=False,
    )
    return result.returncode == 0


def ensure_windows_dependencies(*, assume_yes: bool = False, required_packages: tuple[str, ...] | None = None) -> None:
    """Install only HEVC-owned media packages through explicit winget IDs."""
    if not is_windows():
        raise RebuildError("Windows dependency repair was requested on a non-Windows host.")
    validate_platform()
    package_names = required_packages or tuple(WINDOWS_WINGET_PACKAGES)
    unknown = set(package_names) - set(WINDOWS_WINGET_PACKAGES)
    if unknown:
        raise RebuildError("Unknown Windows dependency package(s): " + ", ".join(sorted(unknown)))
    for package_name in package_names:
        commands = WINDOWS_COMMANDS[package_name]
        missing = [command for command in commands if not which(command)]
        if not missing:
            continue
        package_id = WINDOWS_WINGET_PACKAGES[package_name]
        if not _winget_package_present(package_id):
            if not assume_yes:
                prompt_enter(
                    f"Press ENTER to install {package_id} through winget for missing commands: "
                    + ", ".join(missing)
                )
            run(
                [
                    "winget", "install", "--id", package_id, "--exact", "--silent",
                    "--accept-source-agreements", "--accept-package-agreements",
                    "--disable-interactivity",
                ],
                capture=False,
            )
        _refresh_windows_path()
        paths = _windows_command_paths()
        still_missing = [command for command in commands if not paths.get(command)]
        if still_missing:
            raise RebuildError(
                f"winget package {package_id} completed but commands remain unavailable: "
                + ", ".join(still_missing)
            )


def apt_lists_fresh(max_age_hours: float = 12.0) -> bool:
    lists = Path("/var/lib/apt/lists")
    try:
        mtimes = [p.stat().st_mtime for p in lists.iterdir() if p.is_file()]
    except OSError:
        return False
    if not mtimes:
        return False
    return (time.time() - max(mtimes)) < max_age_hours * 3600


def sudo(args: list[str]) -> list[str]:
    if os.geteuid() == 0:
        return args
    return ["sudo", *args]


def refresh_apt_catalog(assume_yes: bool = False, force: bool = False) -> None:
    if not force and apt_lists_fresh():
        return
    if not assume_yes:
        action = prompt_enter("APT package metadata is stale. Refresh it now so dependency/update checks are accurate?")
        if action != "continue":
            return
    run(sudo(["apt-get", "update"]), capture=False)


def dpkg_installed_version(package: str) -> str | None:
    cp = run(["dpkg-query", "-W", "-f=${Status}\t${Version}", package], check=False)
    if cp.returncode != 0:
        return None
    text = cp.stdout.strip()
    if not text.startswith("install ok installed\t"):
        return None
    return text.split("\t", 1)[1]


def apt_candidate(package: str) -> str | None:
    cp = run(["apt-cache", "policy", package], check=False)
    if cp.returncode != 0:
        return None
    m = re.search(r"^\s*Candidate:\s*(\S+)", cp.stdout, re.MULTILINE)
    if not m or m.group(1) == "(none)":
        return None
    return m.group(1)


def apt_update_status() -> dict:
    """Inspect only the local APT cache; this function never refreshes or installs packages."""
    if is_windows():
        raise RebuildError("APT package updates are Linux-only; use the Windows HEVC dependency resolver.")
    validate_platform()
    packages: list[dict[str, str]] = []
    for package in sorted(APT_PACKAGES):
        installed = dpkg_installed_version(package)
        candidate = apt_candidate(package)
        if installed and candidate and version_lt(installed, candidate):
            packages.append({"name": package, "installed_version": installed, "candidate_version": candidate})
    lists = Path("/var/lib/apt/lists")
    try:
        timestamps = [item.stat().st_mtime for item in lists.iterdir() if item.is_file()]
        updated_at = max(timestamps) if timestamps else None
    except OSError:
        updated_at = None
    return {
        "packages": packages,
        "catalog_updated_at": updated_at,
        "catalog_stale": not apt_lists_fresh(),
        "source": "local APT package index",
        "checked_at": time.time(),
    }


def validate_apt_update_confirmation(status: Mapping[str, object], confirmed: list[str]) -> list[str]:
    """Require the caller to confirm the exact, fresh allowlisted update set."""
    if status.get("catalog_stale") is True:
        raise RebuildError("The local APT package catalog is stale. Refresh it explicitly, then check updates again.")
    rows = status.get("packages")
    if not isinstance(rows, list):
        raise RebuildError("The APT update report is malformed; check for updates again.")
    available: list[str] = []
    for row in rows:
        if not isinstance(row, Mapping) or not isinstance(row.get("name"), str):
            raise RebuildError("The APT update report contains an invalid package entry.")
        available.append(row["name"])
    available = sorted(set(available))
    if any(package not in APT_PACKAGES for package in available):
        raise RebuildError("The APT update report contains a package outside the fixed allowlist.")
    if len(set(confirmed)) != len(confirmed):
        raise RebuildError("Each package can appear only once in the confirmation list.")
    if not available:
        raise RebuildError("The checked Ubuntu package catalog reports no available updates.")
    if sorted(confirmed) != available:
        raise RebuildError(
            "Package update confirmation must exactly match the current checked list: "
            + ", ".join(available)
            + "."
        )
    return available


def validate_apt_upgrade_plan(packages: list[str], stdout: str, returncode: int = 0) -> None:
    """Reject simulated APT transactions that alter anything beyond the confirmed set."""
    if not packages or len(set(packages)) != len(packages) or any(package not in APT_PACKAGES for package in packages):
        raise RebuildError("APT upgrades accept only a unique, non-empty list from the fixed Ubuntu package allowlist.")
    if returncode != 0:
        raise RebuildError("APT could not simulate this update plan; no package changes were made.")
    planned: set[str] = set()
    removals: list[str] = []
    for line in stdout.splitlines():
        if line.startswith("Inst "):
            package = line[5:].split(maxsplit=1)[0]
            planned.add(package.split(":", 1)[0])
        elif line.startswith("Rem "):
            removals.append(line[4:].split(maxsplit=1)[0])
    requested = set(packages)
    if planned != requested or removals:
        raise RebuildError(
            "APT's simulated plan includes changes beyond the confirmed package list "
            f"(upgrade plan: [{', '.join(sorted(planned))}]; removals: [{', '.join(removals)}]). "
            "No package changes were made."
        )


def upgrade_apt_packages(confirmed: list[str]) -> dict[str, object]:
    """Upgrade only the exact current package set after a safe local simulation."""
    packages = validate_apt_update_confirmation(apt_update_status(), confirmed)
    simulation = run(
        ["apt-get", "-s", "install", "--only-upgrade", "--no-install-recommends", "--no-remove", "--", *packages],
        check=False,
    )
    validate_apt_upgrade_plan(packages, simulation.stdout, simulation.returncode)
    # Capture APT output so the JSON bridge keeps stdout as one parseable document.
    run(
        sudo([
            "apt-get", "install", "--only-upgrade", "--no-install-recommends",
            "--no-remove", "--yes", "--", *packages,
        ]),
    )
    return {"upgraded": packages, "note": "The confirmed allowlisted package updates completed."}


def version_lt(installed: str, candidate: str) -> bool:
    cp = run(["dpkg", "--compare-versions", installed, "lt", candidate], check=False)
    return cp.returncode == 0


def ensure_media_repository(assume_yes: bool = False) -> None:
    if apt_candidate("ffmpeg"):
        return

    info = os_release()
    ident = info.get("ID", "").lower()
    if ident == "ubuntu":
        if not assume_yes:
            prompt_enter("Ubuntu's Universe repository appears unavailable. Press ENTER to enable Universe before continuing.")
        if not which("add-apt-repository"):
            run(sudo(["apt-get", "install", "-y", "software-properties-common"]), capture=False)
        run(sudo(["add-apt-repository", "-y", "universe"]), capture=False)
        run(sudo(["apt-get", "update"]), capture=False)
        if apt_candidate("ffmpeg"):
            return

    # Linux Mint normally exposes Ubuntu's media packages through its configured
    # repositories already. Never inject an Ubuntu repository into Mint automatically.
    raise RebuildError(
        "FFmpeg is not available from the currently configured APT repositories. "
        "On Linux Mint, open Update Manager -> Software Sources and restore the default "
        "official mirrors, then run the installer again."
    )


def ensure_apt_dependencies(
    *,
    assume_yes: bool = False,
    skip_update_check: bool = False,
    required_packages: tuple[str, ...] | None = None,
) -> None:
    if is_windows():
        raise RebuildError("APT dependency repair is Linux-only; use ensure_windows_dependencies().")
    package_names = required_packages or tuple(APT_PACKAGES)
    unknown_packages = set(package_names) - set(APT_PACKAGES)
    if unknown_packages:
        raise RebuildError("Unknown required APT package(s): " + ", ".join(sorted(unknown_packages)))
    package_commands = {name: APT_PACKAGES[name] for name in package_names}
    if not skip_update_check:
        refresh_apt_catalog(assume_yes=assume_yes)
    if "ffmpeg" in package_commands:
        ensure_media_repository(assume_yes=assume_yes)

    repair_attempts = 0
    while True:
        missing: list[str] = []
        updates: list[tuple[str, str, str]] = []
        broken_commands: list[str] = []
        for package, commands in package_commands.items():
            installed = dpkg_installed_version(package)
            if not installed:
                missing.append(package)
            else:
                if not skip_update_check:
                    candidate = apt_candidate(package)
                    if candidate and version_lt(installed, candidate):
                        updates.append((package, installed, candidate))
            for command in commands:
                if not which(command):
                    broken_commands.append(command)

        if not missing and not updates and not broken_commands:
            return

        lines: list[str] = []
        if missing:
            lines.append("Missing required packages: " + ", ".join(missing))
        if broken_commands:
            lines.append("Required commands not currently resolvable: " + ", ".join(sorted(set(broken_commands))))
        if updates:
            lines.append("Updates are available for required software:")
            lines.extend(f"  {p}: {old} -> {new}" for p, old, new in updates)
        message = "\n".join(lines)
        print("\n" + message + "\n")
        if not assume_yes:
            prompt_enter("Press ENTER to install/update the required software before continuing.")
        packages = sorted(set(missing + [p for p, _, _ in updates]))
        broken_packages = sorted({p for p, cmds in package_commands.items() if any(c in broken_commands for c in cmds)})
        repair_attempts += 1
        if repair_attempts > 3:
            raise RebuildError(
                "Required software is still missing/broken after three APT repair attempts. "
                "Refusing an infinite install loop. Check the preceding APT output."
            )
        if broken_packages and not missing and not updates:
            run(sudo(["apt-get", "install", "-y", "--reinstall", *broken_packages]), capture=False)
        else:
            packages = sorted(set(packages + broken_packages))
            run(sudo(["apt-get", "install", "-y", *packages]), capture=False)
        # Loop until commands and versions are actually usable.


def github_latest_hdr10plus(timeout: float = 6.0) -> dict:
    req = urllib.request.Request(
        GITHUB_RELEASE_API,
        headers={"Accept": "application/vnd.github+json", "User-Agent": "hevc-rebuild-ubuntu/1.0"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        from .util import require_json_object
        value = require_json_object(json.load(response), "GitHub HDR10+ release", required_keys=("assets", "tag_name"))
        if not isinstance(value["assets"], list) or any(not isinstance(asset, dict) for asset in value["assets"]):
            raise RebuildError("GitHub HDR10+ release returned invalid assets.")
        return value


def parse_tool_version(text: str) -> str | None:
    m = re.search(r"(?<!\d)(\d+\.\d+(?:\.\d+)?)(?!\d)", text)
    return m.group(1) if m else None


def hdr10plus_version(path: str) -> str | None:
    cp = run([path, "--version"], check=False)
    return parse_tool_version((cp.stdout or "") + "\n" + (cp.stderr or ""))


def arch_asset_token() -> str:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        if is_windows():
            return "x86_64-pc-windows-msvc"
        return "x86_64-unknown-linux-musl"
    if machine in {"aarch64", "arm64"}:
        if is_windows():
            return "aarch64-pc-windows-msvc"
        return "aarch64-unknown-linux-musl"
    raise RebuildError(f"No managed hdr10plus_tool binary is configured for architecture {machine!r}.")


def managed_hdr_path() -> Path:
    return MANAGED_BIN / ("hdr10plus_tool.exe" if is_windows() else "hdr10plus_tool")


def companion_ffprobe(ffmpeg_path: str | None) -> str | None:
    if not ffmpeg_path:
        return None
    ffmpeg = Path(ffmpeg_path)
    names = ("ffprobe.exe", "ffprobe") if is_windows() else ("ffprobe",)
    for name in names:
        candidate = ffmpeg.with_name(name)
        if candidate.is_file() and (is_windows() or os.access(candidate, os.X_OK)):
            return str(candidate.resolve())
    return None


def _asset_checksum(release: dict, asset: dict, timeout: float) -> str | None:
    digest = asset.get("digest")
    if isinstance(digest, str) and digest.lower().startswith("sha256:"):
        return digest.split(":", 1)[1].lower()
    name = asset.get("name", "")
    for suffix in (".sha256", ".sha256sum", ".sha256.txt"):
        side = next((a for a in release.get("assets", []) if a.get("name") == name + suffix), None)
        if not side:
            continue
        req = urllib.request.Request(side["browser_download_url"], headers={"User-Agent": "hevc-rebuild-ubuntu/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as response:
            text = response.read().decode("utf-8", errors="replace")
        m = re.search(r"\b([0-9a-fA-F]{64})\b", text)
        if m:
            return m.group(1).lower()
    return None


def install_hdr10plus(release: dict, *, assume_yes: bool = False, timeout: float = 20.0) -> str:
    token = arch_asset_token()
    suffix = ".zip" if is_windows() else ".tar.gz"
    asset = next(
        (a for a in release.get("assets", []) if token in a.get("name", "") and a.get("name", "").endswith(suffix)),
        None,
    )
    if not asset:
        raise RebuildError(f"Latest hdr10plus_tool release has no {token} {suffix} asset.")
    checksum = _asset_checksum(release, asset, timeout)
    if not checksum:
        raise RebuildError(
            "The upstream hdr10plus_tool asset did not expose a SHA-256 digest/sidecar. "
            "Refusing an unverifiable automatic binary install."
        )
    if not assume_yes:
        prompt_enter(f"Press ENTER to install hdr10plus_tool {release.get('tag_name', '?')} from the official upstream release.")

    MANAGED_BIN.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="hevc-rebuild-hdr10plus-") as tmp:
        tmp_path = Path(tmp)
        archive = tmp_path / asset["name"]
        req = urllib.request.Request(asset["browser_download_url"], headers={"User-Agent": "hevc-rebuild-ubuntu/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as response, archive.open("wb") as out:
            shutil.copyfileobj(response, out)
        import hashlib
        h = hashlib.sha256()
        with archive.open("rb") as f:
            for chunk in iter(lambda: f.read(1024 * 1024), b""):
                h.update(chunk)
        if h.hexdigest().lower() != checksum:
            raise RebuildError("hdr10plus_tool SHA-256 verification failed; refusing installation.")
        if is_windows():
            with zipfile.ZipFile(archive) as zf:
                candidates = [
                    info for info in zf.infolist()
                    if Path(info.filename).name.lower() == "hdr10plus_tool.exe" and not info.is_dir()
                ]
                if len(candidates) != 1:
                    raise RebuildError("Unexpected Windows hdr10plus_tool release archive layout.")
                temp_binary = tmp_path / "hdr10plus_tool.exe"
                with zf.open(candidates[0]) as extracted, temp_binary.open("wb") as out:
                    shutil.copyfileobj(extracted, out)
            target = managed_hdr_path()
            staged = MANAGED_BIN / ".hdr10plus_tool.new.exe"
            shutil.copy2(temp_binary, staged)
            os.replace(staged, target)
        else:
            with tarfile.open(archive, "r:gz") as tf:
                # Avoid arbitrary-path extraction.
                candidates = [m for m in tf.getmembers() if Path(m.name).name == "hdr10plus_tool" and m.isfile()]
                if len(candidates) != 1:
                    raise RebuildError("Unexpected hdr10plus_tool release archive layout.")
                member = candidates[0]
                extracted = tf.extractfile(member)
                if extracted is None:
                    raise RebuildError("Unable to read hdr10plus_tool from release archive.")
                temp_binary = tmp_path / "hdr10plus_tool"
                with temp_binary.open("wb") as out:
                    shutil.copyfileobj(extracted, out)
                temp_binary.chmod(temp_binary.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
                target = managed_hdr_path()
                staged = MANAGED_BIN / ".hdr10plus_tool.new"
                shutil.copy2(temp_binary, staged)
                staged.chmod(0o755)
                os.replace(staged, target)
    version = hdr10plus_version(str(managed_hdr_path()))
    if not version:
        raise RebuildError("Installed hdr10plus_tool did not pass its version smoke test.")
    return str(managed_hdr_path())


def ensure_hdr10plus(*, assume_yes: bool = False, skip_update_check: bool = False) -> str:
    managed = managed_hdr_path()
    current = str(managed) if managed.exists() else which("hdr10plus_tool")
    current_version = hdr10plus_version(current) if current else None
    release = None
    if not skip_update_check or not current:
        try:
            release = github_latest_hdr10plus()
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            if current and current_version:
                logging.warning("Could not check hdr10plus_tool updates (%s); using installed %s.", exc, current_version)
                return current
            raise RebuildError(f"hdr10plus_tool is missing and its upstream release cannot be reached: {exc}") from exc

    if current and current_version and release:
        latest = str(release.get("tag_name", "")).lstrip("v")
        if latest and version_tuple(current_version) < version_tuple(latest):
            print(f"\nhdr10plus_tool update available: {current_version} -> {latest}\n")
            if assume_yes or prompt_enter("Press ENTER to update hdr10plus_tool before continuing.") == "continue":
                return install_hdr10plus(release, assume_yes=True)
        return current
    if current and current_version:
        return current
    if release is None:
        release = github_latest_hdr10plus()
    return install_hdr10plus(release, assume_yes=assume_yes)


def get_x265_help(ffmpeg: str) -> str:
    cp = run([ffmpeg, "-hide_banner", "-h", "encoder=libx265"], check=False)
    text = (cp.stdout or "") + "\n" + (cp.stderr or "")
    if "libx265" not in text:
        raise RebuildError("Installed FFmpeg does not expose the libx265 encoder.")
    return text


def get_x265_pixel_formats(ffmpeg: str, help_text: str | None = None) -> set[str]:
    text = help_text if help_text is not None else get_x265_help(ffmpeg)
    m = re.search(r"Supported pixel formats:\s*(.+)", text)
    if not m:
        raise RebuildError("Unable to determine libx265 supported pixel formats from FFmpeg.")
    return set(m.group(1).split())


def x265_supports_dolbyvision(help_text: str) -> bool:
    # FFmpeg 7+ with x265 build >=167 exposes the native RPU coding switch.
    return bool(re.search(r"(?:^|\s)-dolbyvision\s+<", help_text, re.MULTILINE))


def _native_toolchain(
    purpose: str,
    *,
    allow_unsupported_os: bool,
    ffmpeg_path: str | None,
    tool_paths: Mapping[str, str | None] | None,
) -> Toolchain | None:
    """Use Rust's path, version, encoder, and smoke checks for app-launched jobs."""
    overrides = dict(tool_paths or {})
    if ffmpeg_path and not overrides.get("ffmpeg"):
        overrides["ffmpeg"] = ffmpeg_path
    value = native_json_action(
        "resolve-toolchain",
        {
            "purpose": purpose,
            "allow_unsupported_os": allow_unsupported_os,
            "tool_paths": overrides,
        },
        timeout=180.0,
    )
    if value is None:
        return None

    required = ("ffmpeg", "ffprobe") if purpose == "check" else ("ffmpeg", "ffprobe", "mediainfo")
    if purpose == "rebuild":
        required += ("mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
    elif purpose == "metadata":
        required += ("mkvpropedit", "mkvmerge", "mkvextract")
    paths: dict[str, str] = {}
    for name in required:
        path = value.get(name)
        if not isinstance(path, str) or not path:
            raise RebuildError(f"The Rust toolchain resolver returned no {name} path.")
        paths[name] = path
    formats = value.get("x265_pixel_formats", [])
    if not isinstance(formats, list) or any(not isinstance(item, str) for item in formats):
        raise RebuildError("The Rust toolchain resolver returned invalid x265 pixel-format capabilities.")
    dolbyvision = value.get("x265_dolbyvision", False)
    if not isinstance(dolbyvision, bool):
        raise RebuildError("The Rust toolchain resolver returned invalid Dolby Vision encoder capability data.")
    return Toolchain(
        ffmpeg=paths["ffmpeg"],
        ffprobe=paths["ffprobe"],
        mediainfo=paths.get("mediainfo", overrides.get("mediainfo") or "mediainfo"),
        mkvpropedit=paths.get("mkvpropedit", overrides.get("mkvpropedit") or "mkvpropedit"),
        mkvmerge=paths.get("mkvmerge", overrides.get("mkvmerge") or "mkvmerge"),
        mkvextract=paths.get("mkvextract", overrides.get("mkvextract") or "mkvextract"),
        hdr10plus_tool=paths.get("hdr10plus_tool", "hdr10plus_tool"),
        x265_pixel_formats=set(formats),
        x265_dolbyvision=dolbyvision,
    )


def resolve_toolchain(
    *,
    assume_yes: bool = False,
    skip_update_check: bool = False,
    allow_unsupported_os: bool = False,
    install_missing: bool = True,
    ffmpeg_path: str | None = None,
    tool_paths: Mapping[str, str | None] | None = None,
    purpose: str = "rebuild",
) -> Toolchain:
    if purpose not in {"check", "rebuild", "metadata"}:
        raise RebuildError("Toolchain purpose must be 'check', 'rebuild', or 'metadata'.")
    if purpose == "check":
        # A read-only preflight must never trigger package repair or require
        # encoder/remux helpers that it does not use.
        install_missing = False
    if not install_missing:
        native = _native_toolchain(
            purpose,
            allow_unsupported_os=allow_unsupported_os,
            ffmpeg_path=ffmpeg_path,
            tool_paths=tool_paths,
        )
        if native is not None:
            return native
    validate_platform(allow_unsupported=allow_unsupported_os)
    if install_missing and is_windows():
        required_packages = ("ffmpeg", "mediainfo", "mkvtoolnix")
        ensure_windows_dependencies(
            assume_yes=assume_yes,
            required_packages=required_packages,
        )
        if purpose == "rebuild":
            hdr = ensure_hdr10plus(assume_yes=assume_yes, skip_update_check=skip_update_check)
        else:
            managed_hdr = managed_hdr_path()
            hdr = str(managed_hdr) if managed_hdr.is_file() else (which("hdr10plus_tool") or "hdr10plus_tool")
    elif install_missing:
        required_packages = ("python3", "ffmpeg", "mediainfo", "mkvtoolnix") if purpose == "metadata" else None
        ensure_apt_dependencies(
            assume_yes=assume_yes,
            skip_update_check=skip_update_check,
            required_packages=required_packages,
        )
        if purpose == "rebuild":
            hdr = ensure_hdr10plus(assume_yes=assume_yes, skip_update_check=skip_update_check)
        else:
            managed_hdr = MANAGED_BIN / "hdr10plus_tool"
            hdr = str(managed_hdr) if managed_hdr.is_file() and os.access(managed_hdr, os.X_OK) else (which("hdr10plus_tool") or "hdr10plus_tool")
    elif install_missing is False:
        # The non-installing Windows path still uses the managed .exe name when
        # a caller has already installed the helper.
        if purpose == "rebuild" and is_windows():
            managed = managed_hdr_path()
            hdr = str(managed) if managed.is_file() else (which("hdr10plus_tool") or which("hdr10plus_tool.exe"))
        else:
            managed_hdr = MANAGED_BIN / "hdr10plus_tool"
            hdr = str(managed_hdr) if managed_hdr.is_file() and os.access(managed_hdr, os.X_OK) else which("hdr10plus_tool")
        if not hdr and purpose == "rebuild":
            raise RebuildError("hdr10plus_tool is missing. Use Environment → Install HDR10+ helper, or install it and retry.")
        if not hdr:
            hdr = "hdr10plus_tool"
    overrides = dict(tool_paths or {})
    if ffmpeg_path and not overrides.get("ffmpeg"):
        overrides["ffmpeg"] = ffmpeg_path
    _refresh_windows_path()
    paths = {name: which(name) for name in TOOL_OVERRIDE_KEYS}
    for name, value in overrides.items():
        if name not in TOOL_OVERRIDE_KEYS or not value:
            continue
        selected = Path(value).expanduser()
        try:
            selected = selected.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"Selected {name} executable is unavailable: {selected}: {exc}") from exc
        if not selected.is_file() or not os.access(selected, os.X_OK):
            raise RebuildError(f"Selected {name} path is not an executable file: {selected}")
        paths[name] = str(selected)
    if overrides.get("ffmpeg") and not overrides.get("ffprobe"):
        sibling_probe = companion_ffprobe(paths["ffmpeg"])
        if sibling_probe:
            paths["ffprobe"] = sibling_probe
    required_commands = ("ffmpeg", "ffprobe") if purpose == "check" else tuple(paths) if purpose == "rebuild" else (
        "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract",
    )
    missing = [name for name in required_commands if not paths.get(name)]
    if missing:
        raise RebuildError("Required commands still missing after dependency repair: " + ", ".join(missing))
    pixel_formats: set[str] = set()
    x265_dolbyvision = False
    if purpose == "rebuild":
        x265_help = get_x265_help(paths["ffmpeg"] or "ffmpeg")
        pixel_formats = get_x265_pixel_formats(paths["ffmpeg"] or "ffmpeg", x265_help)
        if "yuv420p" not in pixel_formats:
            raise RebuildError("FFmpeg/libx265 does not advertise normal 8-bit 4:2:0 HEVC encoding support.")
        # Functional smoke tests catch broken shared libraries/encoders even when packages and executables exist.
        for pix_fmt in ("yuv420p", "yuv420p10le", "yuv420p12le"):
            if pix_fmt not in pixel_formats:
                continue
            smoke = run([
                paths["ffmpeg"] or "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                "-i", "color=size=64x64:rate=1:duration=0.1", "-c:v", "libx265", "-pix_fmt", pix_fmt,
                "-frames:v", "1", "-f", "null", "-"
            ], check=False)
            if smoke.returncode != 0:
                raise RebuildError(f"FFmpeg/libx265 smoke encode failed for {pix_fmt}: " + (smoke.stderr or "").strip())
        x265_dolbyvision = x265_supports_dolbyvision(x265_help)
    commands = [
        ([paths["ffmpeg"] or "ffmpeg", "-version"]),
        ([paths["ffprobe"] or "ffprobe", "-version"]),
    ]
    if purpose != "check":
        commands.extend([
            ([paths["mediainfo"] or "mediainfo", "--Version"]),
            ([paths["mkvpropedit"] or "mkvpropedit", "--version"]),
            ([paths["mkvmerge"] or "mkvmerge", "--version"]),
            ([paths["mkvextract"] or "mkvextract", "--version"]),
        ])
    if purpose == "rebuild":
        commands.append([hdr, "--version"])
    for command in commands:
        cp = run(command, check=False)
        if cp.returncode != 0:
            raise RebuildError(f"Dependency capability check failed: {' '.join(command)}")
    return Toolchain(
        ffmpeg=paths["ffmpeg"] or "ffmpeg",
        ffprobe=paths["ffprobe"] or "ffprobe",
        mediainfo=paths["mediainfo"] or "mediainfo",
        mkvpropedit=paths["mkvpropedit"] or "mkvpropedit",
        mkvmerge=paths["mkvmerge"] or "mkvmerge",
        mkvextract=paths["mkvextract"] or "mkvextract",
        hdr10plus_tool=hdr,
        x265_pixel_formats=pixel_formats,
        x265_dolbyvision=x265_dolbyvision,
    )


def resolve_youtube_toolchain(
    *,
    allow_unsupported_os: bool = False,
    ffmpeg_path: str | None = None,
    tool_paths: Mapping[str, str | None] | None = None,
) -> Toolchain:
    """Resolve baseline YouTube export tools and probe optional 10-bit HEVC support.

    SDR needs libx264/AAC. Source-compatible HDR can use libx265 Main 10 when the
    selected FFmpeg exposes 10-bit 4:2:0; files requiring HDR are skipped otherwise.
    This function never installs packages and does not require MKVToolNix, Fontconfig,
    or the HDR10+ helper.
    """
    native = _native_toolchain(
        "youtube",
        allow_unsupported_os=allow_unsupported_os,
        ffmpeg_path=ffmpeg_path,
        tool_paths=tool_paths,
    )
    if native is not None:
        return native
    validate_platform(allow_unsupported=allow_unsupported_os)
    overrides = dict(tool_paths or {})
    if ffmpeg_path and not overrides.get("ffmpeg"):
        overrides["ffmpeg"] = ffmpeg_path
    paths = {name: which(name) for name in ("ffmpeg", "ffprobe", "mediainfo")}
    for name in paths:
        value = overrides.get(name)
        if not value:
            continue
        selected = Path(value).expanduser()
        try:
            selected = selected.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"Selected {name} executable is unavailable: {selected}: {exc}") from exc
        if not selected.is_file() or not os.access(selected, os.X_OK):
            raise RebuildError(f"Selected {name} path is not an executable file: {selected}")
        paths[name] = str(selected)
    if overrides.get("ffmpeg") and not overrides.get("ffprobe"):
        sibling_probe = companion_ffprobe(paths["ffmpeg"])
        if sibling_probe:
            paths["ffprobe"] = sibling_probe

    missing = [name for name, value in paths.items() if not value]
    if missing:
        raise RebuildError("YouTube export needs working FFmpeg, FFprobe, and MediaInfo; missing: " + ", ".join(missing))

    ffmpeg = paths["ffmpeg"] or "ffmpeg"
    for encoder in ("libx264", "aac"):
        cp = run([ffmpeg, "-hide_banner", "-h", f"encoder={encoder}"], check=False)
        text = (cp.stdout or "") + "\n" + (cp.stderr or "")
        if cp.returncode != 0 or encoder not in text:
            raise RebuildError(f"The selected FFmpeg does not expose the required {encoder} encoder.")

    smoke = run([
        ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin",
        "-f", "lavfi", "-i", "color=size=64x64:rate=1:duration=0.1",
        "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-t", "0.1",
        "-c:v", "libx264", "-c:a", "aac", "-f", "null", "-",
    ], check=False)
    if smoke.returncode != 0:
        detail = (smoke.stderr or "").strip()
        raise RebuildError("FFmpeg H.264/AAC capability check failed" + (f": {detail}" if detail else "."))

    x265_pixel_formats: set[str] = set()
    try:
        x265_help = get_x265_help(ffmpeg)
        x265_pixel_formats = get_x265_pixel_formats(ffmpeg, x265_help)
    except RebuildError:
        # Keep SDR export available on builds without x265. HDR files receive a
        # per-file SKIPPED result if 10-bit Main 10 support is unavailable.
        pass

    return Toolchain(
        ffmpeg=ffmpeg,
        ffprobe=paths["ffprobe"] or "ffprobe",
        mediainfo=paths["mediainfo"] or "mediainfo",
        mkvpropedit="mkvpropedit",
        mkvmerge="mkvmerge",
        mkvextract="mkvextract",
        hdr10plus_tool="hdr10plus_tool",
        x265_pixel_formats=x265_pixel_formats,
    )
