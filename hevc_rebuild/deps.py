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
from pathlib import Path

from .model import Toolchain
from .util import RebuildError, portable_data_dir, prompt_enter, run, which

APT_PACKAGES = {
    "python3": ["python3"],
    "ffmpeg": ["ffmpeg", "ffprobe"],
    "mediainfo": ["mediainfo"],
    "mkvtoolnix": ["mkvpropedit", "mkvmerge", "mkvextract"],
    "fontconfig": ["fc-list"],
    "ca-certificates": [],
}
GITHUB_RELEASE_API = "https://api.github.com/repos/quietvoid/hdr10plus_tool/releases/latest"
_PORTABLE_DATA = portable_data_dir()
APP_DATA = (_PORTABLE_DATA / "share" / "hevc-rebuild") if _PORTABLE_DATA else Path.home() / ".local" / "share" / "hevc-rebuild"
MANAGED_BIN = APP_DATA / "bin"


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
    """Accept supported Ubuntu and Ubuntu-based Linux Mint releases.

    Runtime capability tests remain authoritative. Distro/version checks only prevent
    accidental use on unrelated package ecosystems where our automated APT repair
    logic could be unsafe.
    """
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


def ensure_apt_dependencies(*, assume_yes: bool = False, skip_update_check: bool = False) -> None:
    if not skip_update_check:
        refresh_apt_catalog(assume_yes=assume_yes)
    ensure_media_repository(assume_yes=assume_yes)

    repair_attempts = 0
    while True:
        missing: list[str] = []
        updates: list[tuple[str, str, str]] = []
        broken_commands: list[str] = []
        for package, commands in APT_PACKAGES.items():
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
        broken_packages = sorted({p for p, cmds in APT_PACKAGES.items() if any(c in broken_commands for c in cmds)})
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
        return json.load(response)


def parse_tool_version(text: str) -> str | None:
    m = re.search(r"(?<!\d)(\d+\.\d+(?:\.\d+)?)(?!\d)", text)
    return m.group(1) if m else None


def hdr10plus_version(path: str) -> str | None:
    cp = run([path, "--version"], check=False)
    return parse_tool_version((cp.stdout or "") + "\n" + (cp.stderr or ""))


def arch_asset_token() -> str:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-musl"
    if machine in {"aarch64", "arm64"}:
        return "aarch64-unknown-linux-musl"
    raise RebuildError(f"No managed hdr10plus_tool binary is configured for architecture {machine!r}.")


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
    asset = next(
        (a for a in release.get("assets", []) if token in a.get("name", "") and a.get("name", "").endswith(".tar.gz")),
        None,
    )
    if not asset:
        raise RebuildError(f"Latest hdr10plus_tool release has no {token} tarball.")
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
            target = MANAGED_BIN / "hdr10plus_tool"
            staged = MANAGED_BIN / ".hdr10plus_tool.new"
            shutil.copy2(temp_binary, staged)
            staged.chmod(0o755)
            os.replace(staged, target)
    version = hdr10plus_version(str(MANAGED_BIN / "hdr10plus_tool"))
    if not version:
        raise RebuildError("Installed hdr10plus_tool did not pass its version smoke test.")
    return str(MANAGED_BIN / "hdr10plus_tool")


def ensure_hdr10plus(*, assume_yes: bool = False, skip_update_check: bool = False) -> str:
    managed = MANAGED_BIN / "hdr10plus_tool"
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


def resolve_toolchain(
    *,
    assume_yes: bool = False,
    skip_update_check: bool = False,
    allow_unsupported_os: bool = False,
    install_missing: bool = True,
    ffmpeg_path: str | None = None,
) -> Toolchain:
    validate_platform(allow_unsupported=allow_unsupported_os)
    if install_missing:
        ensure_apt_dependencies(assume_yes=assume_yes, skip_update_check=skip_update_check)
        hdr = ensure_hdr10plus(assume_yes=assume_yes, skip_update_check=skip_update_check)
    else:
        managed_hdr = MANAGED_BIN / "hdr10plus_tool"
        hdr = str(managed_hdr) if managed_hdr.is_file() and os.access(managed_hdr, os.X_OK) else which("hdr10plus_tool")
        if not hdr:
            raise RebuildError("hdr10plus_tool is missing. Use Environment → Install HDR10+ helper, or install it and retry.")
    paths = {name: which(name) for name in ("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract")}
    if ffmpeg_path:
        selected = Path(ffmpeg_path).expanduser()
        try:
            selected = selected.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"Selected FFmpeg binary is unavailable: {selected}: {exc}") from exc
        if not selected.is_file() or not os.access(selected, os.X_OK):
            raise RebuildError(f"Selected FFmpeg path is not an executable file: {selected}")
        paths["ffmpeg"] = str(selected)
        companion_probe = selected.with_name("ffprobe")
        if companion_probe.is_file() and os.access(companion_probe, os.X_OK):
            paths["ffprobe"] = str(companion_probe)
    missing = [name for name, value in paths.items() if not value]
    if missing:
        raise RebuildError("Required commands still missing after dependency repair: " + ", ".join(missing))
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
    for command in ([paths["mediainfo"] or "mediainfo", "--Version"], [paths["mkvpropedit"] or "mkvpropedit", "--version"], [paths["mkvmerge"] or "mkvmerge", "--version"], [paths["mkvextract"] or "mkvextract", "--version"], [hdr, "--version"]):
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
        x265_dolbyvision=x265_supports_dolbyvision(x265_help),
    )
