from __future__ import annotations

import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

from .deps import (
    APT_PACKAGES,
    MANAGED_BIN,
    TOOL_OVERRIDE_KEYS,
    companion_ffprobe,
    get_x265_pixel_formats,
    is_windows,
    managed_hdr_path,
    os_release,
    version_tuple,
    x265_supports_dolbyvision,
)
from .model import Toolchain
from .resources import MemoryStats, effective_cpu_threads, memory_stats


@dataclass(frozen=True)
class CommandStatus:
    key: str
    label: str
    path: str | None
    version: str | None
    available: bool
    required: bool = True
    detail: str = ""


@dataclass(frozen=True)
class EnvironmentReport:
    os_name: str
    os_version: str
    supported_platform: bool
    platform_note: str
    cpu_model: str
    logical_cpus: int
    memory: MemoryStats
    gpu_devices: tuple[str, ...]
    checks: tuple[CommandStatus, ...]
    x265_pixel_formats: frozenset[str]
    dolby_vision_x265: bool
    analysis_toolchain: Toolchain | None
    youtube_toolchain: Toolchain | None
    toolchain: Toolchain | None

    @property
    def ready(self) -> bool:
        return self.supported_platform and self.toolchain is not None

    @property
    def youtube_ready(self) -> bool:
        return self.supported_platform and self.youtube_toolchain is not None

    @property
    def missing(self) -> tuple[CommandStatus, ...]:
        return tuple(check for check in self.checks if check.required and not check.available)


_CHECKS = (
    ("python3", "Python 3 runtime", "--version", ""),
    ("ffmpeg", "FFmpeg", "-hide_banner", "-version"),
    ("ffprobe", "FFprobe", "-version", ""),
    ("mediainfo", "MediaInfo", "--Version", ""),
    ("mkvpropedit", "MKVToolNix editor", "--version", ""),
    ("mkvmerge", "MKVToolNix merger", "--version", ""),
    ("mkvextract", "MKVToolNix extractor", "--version", ""),
    ("fc-list", "Fontconfig", "--version", ""),
    ("hdr10plus_tool", "HDR10+ helper", "--version", ""),
    ("apt-get", "APT package manager", "--version", ""),
    ("apt-cache", "APT package catalog", "--version", ""),
    ("dpkg", "Debian package database", "--version", ""),
    ("dpkg-query", "Package query tool", "--version", ""),
    ("pkexec", "Administrator authentication", "--version", ""),
)


def _executable(path: str | None) -> str | None:
    if not path:
        return None
    candidate = Path(path).expanduser()
    try:
        resolved = candidate.resolve(strict=True)
    except OSError:
        return None
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        return None
    return str(resolved)


def _candidate_paths(ffmpeg_path: str | None, tool_paths: dict[str, str | None] | None = None) -> dict[str, str | None]:
    paths = {name: shutil.which(name) for name in (
        "python3", "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "fc-list",
        "apt-get", "apt-cache", "dpkg", "dpkg-query", "pkexec",
    )}
    if not paths["python3"]:
        paths["python3"] = shutil.which("python")
    overrides = dict(tool_paths or {})
    if ffmpeg_path and not overrides.get("ffmpeg"):
        overrides["ffmpeg"] = ffmpeg_path
    for name in TOOL_OVERRIDE_KEYS:
        selected = overrides.get(name)
        if selected:
            paths[name] = _executable(selected) or selected
    if overrides.get("ffmpeg") and not overrides.get("ffprobe"):
        ffmpeg = _executable(overrides["ffmpeg"])
        sibling_probe = companion_ffprobe(ffmpeg)
        if sibling_probe:
            paths["ffprobe"] = sibling_probe
    managed_hdr = _executable(str(managed_hdr_path()))
    paths["hdr10plus_tool"] = managed_hdr or shutil.which("hdr10plus_tool")
    return paths


def _check_command(key: str, label: str, path: str | None, args: tuple[str, ...], *, required: bool = True) -> CommandStatus:
    if not path:
        return CommandStatus(key, label, None, None, False, required, "Not found on PATH")
    try:
        result = subprocess.run(
            [path, *args], capture_output=True, text=True, timeout=8, check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return CommandStatus(key, label, path, None, False, required, str(exc))
    output = (result.stdout or "") + "\n" + (result.stderr or "")
    version = next((line.strip() for line in output.splitlines() if line.strip()), None)
    if result.returncode != 0:
        return CommandStatus(key, label, path, version, False, required, f"Version check exited {result.returncode}")
    return CommandStatus(key, label, path, version, True, required)


def _os_details() -> tuple[str, str, bool, str]:
    if is_windows():
        supported = bool(shutil.which("winget"))
        note = "Supported Windows build guest" if supported else "Windows Package Manager (winget) is unavailable"
        return "Windows", platform.version() or platform.release(), supported, note
    try:
        info = os_release()
    except Exception as exc:  # platform probes should still show the rest of the report
        return platform.system() or "Unknown OS", platform.release(), False, f"Could not read distro information: {exc}"
    ident = info.get("ID", "").lower()
    version = info.get("VERSION_ID", "unknown")
    id_like = {item.lower() for item in info.get("ID_LIKE", "").split()}
    ubuntu_ok = ident == "ubuntu" and version_tuple(version) >= (24, 4)
    mint_ok = ident == "linuxmint" and "ubuntu" in id_like and version_tuple(version) >= (22,)
    if ubuntu_ok:
        return "Ubuntu", version, True, "Supported release"
    if mint_ok:
        return "Linux Mint", version, True, "Supported Ubuntu-based release"
    return info.get("PRETTY_NAME", ident or "Linux"), version, False, "The current rebuild engine targets Ubuntu 24.04+ and Linux Mint 22+"


def _cpu_model() -> str:
    try:
        text = Path("/proc/cpuinfo").read_text(encoding="utf-8", errors="replace")
        match = re.search(r"^(?:model name|Hardware)\s*:\s*(.+)$", text, re.MULTILINE)
        if match:
            return match.group(1).strip()
    except OSError:
        pass
    return platform.processor() or platform.machine() or "Unknown CPU"


def _gpu_devices() -> tuple[str, ...]:
    vendor_names = {
        "0x10de": "NVIDIA",
        "0x1002": "AMD",
        "0x8086": "Intel",
        "0x106b": "Apple",
        "0x5143": "Qualcomm",
    }
    found: list[str] = []
    for device in sorted(Path("/sys/class/drm").glob("card[0-9]*/device")):
        try:
            vendor = (device / "vendor").read_text(encoding="utf-8").strip().lower()
            device_id = (device / "device").read_text(encoding="utf-8").strip().lower()
        except OSError:
            continue
        name = vendor_names.get(vendor, f"GPU vendor {vendor or 'unknown'}")
        label = f"{name} ({device_id})" if device_id else name
        if label not in found:
            found.append(label)
    return tuple(found)


def inspect_environment(
    ffmpeg_path: str | None = None,
    tool_paths: dict[str, str | None] | None = None,
) -> EnvironmentReport:
    """Read-only machine/tool check. It never installs packages or downloads binaries."""
    os_name, os_version, supported, platform_note = _os_details()
    paths = _candidate_paths(ffmpeg_path, tool_paths)
    checks: list[CommandStatus] = []
    arg_map = {
        "python3": ("--version",),
        "ffmpeg": ("-hide_banner", "-version"),
        "ffprobe": ("-version",),
        "mediainfo": ("--Version",),
        "mkvpropedit": ("--version",),
        "mkvmerge": ("--version",),
        "mkvextract": ("--version",),
        "fc-list": ("--version",),
        "hdr10plus_tool": ("--version",),
        "apt-get": ("--version",),
        "apt-cache": ("--version",),
        "dpkg": ("--version",),
        "dpkg-query": ("--version",),
        "pkexec": ("--version",),
    }
    labels = {key: label for key, label, _, _ in _CHECKS}
    for key, _, _, _ in _CHECKS:
        linux_only = {"fc-list", "apt-get", "apt-cache", "dpkg", "dpkg-query", "pkexec"}
        required = key not in linux_only if is_windows() else key != "pkexec"
        status = _check_command(key, labels[key], paths.get(key), arg_map[key], required=required)
        if key == "ffmpeg" and ffmpeg_path and not paths.get(key):
            status = CommandStatus(key, labels[key], ffmpeg_path, None, False, True, "Selected file is missing or not executable")
        checks.append(status)

    pixel_formats: set[str] = set()
    dv_supported = False
    ffmpeg_status = next(check for check in checks if check.key == "ffmpeg")
    if ffmpeg_status.available and ffmpeg_status.path:
        try:
            result = subprocess.run(
                [ffmpeg_status.path, "-hide_banner", "-h", "encoder=libx265"],
                capture_output=True, text=True, timeout=10, check=False,
            )
            help_text = (result.stdout or "") + "\n" + (result.stderr or "")
            if result.returncode == 0 and "libx265" in help_text:
                pixel_formats = get_x265_pixel_formats(ffmpeg_status.path, help_text)
                dv_supported = x265_supports_dolbyvision(help_text)
                available = "yuv420p" in pixel_formats
                detail = "CPU libx265 ready" if available else "libx265 does not expose 8-bit 4:2:0"
            else:
                available = False
                detail = "FFmpeg does not expose libx265"
        except (OSError, subprocess.TimeoutExpired, Exception) as exc:
            available = False
            detail = f"Could not inspect libx265: {exc}"
        checks.append(CommandStatus("libx265", "libx265 encoder", ffmpeg_status.path, None, available, True, detail))

        for encoder, label in (("libx264", "libx264 encoder"), ("aac", "AAC encoder")):
            try:
                result = subprocess.run(
                    [ffmpeg_status.path, "-hide_banner", "-h", f"encoder={encoder}"],
                    capture_output=True, text=True, timeout=10, check=False,
                )
                encoder_text = (result.stdout or "") + "\n" + (result.stderr or "")
                available = result.returncode == 0 and encoder in encoder_text
                detail = f"{encoder} available" if available else f"FFmpeg does not expose {encoder}"
            except (OSError, subprocess.TimeoutExpired) as exc:
                available = False
                detail = f"Could not inspect {encoder}: {exc}"
            checks.append(CommandStatus(encoder, label, ffmpeg_status.path, None, available, False, detail))
    else:
        checks.append(CommandStatus("libx265", "libx265 encoder", None, None, False, True, "Requires a working FFmpeg binary"))
        checks.extend((
            CommandStatus("libx264", "libx264 encoder", None, None, False, False, "Requires a working FFmpeg binary"),
            CommandStatus("aac", "AAC encoder", None, None, False, False, "Requires a working FFmpeg binary"),
        ))

    status_by_key = {check.key: check for check in checks}

    def make_toolchain() -> Toolchain:
        return Toolchain(
            ffmpeg=paths["ffmpeg"] or "ffmpeg",
            ffprobe=paths["ffprobe"] or "ffprobe",
            mediainfo=paths["mediainfo"] or "mediainfo",
            mkvpropedit=paths["mkvpropedit"] or "mkvpropedit",
            mkvmerge=paths["mkvmerge"] or "mkvmerge",
            mkvextract=paths["mkvextract"] or "mkvextract",
            hdr10plus_tool=paths["hdr10plus_tool"] or "hdr10plus_tool",
            x265_pixel_formats=pixel_formats,
            x265_dolbyvision=dv_supported,
        )

    analysis_ready = status_by_key["ffprobe"].available and status_by_key["mediainfo"].available
    analysis_toolchain = make_toolchain() if analysis_ready else None
    youtube_ready = all(status_by_key[key].available for key in ("python3", "ffmpeg", "ffprobe", "mediainfo", "libx264", "aac"))
    youtube_toolchain = make_toolchain() if youtube_ready else None
    required_ready = all(check.available for check in checks if check.required)
    toolchain = make_toolchain() if required_ready else None
    return EnvironmentReport(
        os_name=os_name,
        os_version=os_version,
        supported_platform=supported,
        platform_note=platform_note,
        cpu_model=_cpu_model(),
        logical_cpus=effective_cpu_threads(),
        memory=memory_stats(),
        gpu_devices=_gpu_devices(),
        checks=tuple(checks),
        x265_pixel_formats=frozenset(pixel_formats),
        dolby_vision_x265=dv_supported,
        analysis_toolchain=analysis_toolchain,
        youtube_toolchain=youtube_toolchain,
        toolchain=toolchain,
    )


def missing_apt_packages(report: EnvironmentReport, *, using_custom_ffmpeg: bool = False) -> tuple[str, ...]:
    """Return only known APT package names needed to address missing commands."""
    if is_windows():
        return ()
    missing_commands = {check.key for check in report.checks if check.required and not check.available}
    missing: list[str] = []
    for package, commands in APT_PACKAGES.items():
        if package == "ffmpeg" and using_custom_ffmpeg and "ffmpeg" not in missing_commands:
            continue
        if package == "ffmpeg" and using_custom_ffmpeg:
            # A selected FFmpeg still needs a matching FFprobe companion. If absent,
            # install the distro pair so analysis remains available.
            if "ffprobe" not in missing_commands:
                continue
        package_commands = set(commands)
        if package == "ca-certificates":
            try:
                status = subprocess.run(
                    ["dpkg-query", "-W", "-f=${Status}", package],
                    capture_output=True, text=True, timeout=5, check=False,
                )
                installed = status.returncode == 0 and status.stdout.strip() == "install ok installed"
            except (OSError, subprocess.TimeoutExpired):
                installed = False
            if not installed:
                missing.append(package)
        elif package_commands & missing_commands:
            missing.append(package)
    return tuple(sorted(set(missing)))
