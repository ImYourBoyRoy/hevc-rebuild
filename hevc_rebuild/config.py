from __future__ import annotations

import copy
import json
import logging
import math
import os
import tomllib
from pathlib import Path
from typing import Any

from .util import RebuildError, app_preferences_path, portable_data_dir


DEFAULTS: dict[str, Any] = {
    "video": {
        "crf": 21,
        "preset": "slow",
        "promote_8bit_to_10bit": False,
    },
    "content": {
        "tune": "auto",
        "detect_animation": True,
    },
    "hdr": {
        "preserve_hdr10plus": True,
        "dolby_vision": "safe",
    },
    "streams": {
        "secondary_video": "encode",
        "data_streams": "preserve",
    },
    "metadata": {
        "comment": "Brought To you By V1x0r",
        "text_blacklist": ["Movie*Group"],
        "embed_artwork": True,
    },
    "safety": {
        "verify_copy_hashes": True,
    },
    "calibration": {
        "mode": "auto",
        "cache_hours": 24.0,
        "large_file_gib": 20.0,
        "sample_seconds": 3.0,
        "max_candidates": 2,
        "throughput_tolerance_percent": 3.0,
    },
    "resources": {
        "threading": "auto",
        "reserved_cpu_threads": 1,
        "max_worker_threads": 0,
        "frame_threads": 0,
        "min_free_memory_gib": 3.0,
        "memory_reserve_percent": 8.0,
        "min_free_disk_gib": 0.0,
        "preflight_output_ratio": 1.30,
        "disk_projection_margin_percent": 15.0,
    },
}

PRESETS = {"ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"}
TUNES = {"auto", "animation", "grain", "none"}
DV_POLICIES = {"safe", "skip", "hdr10-fallback"}
SECONDARY_VIDEO = {"encode", "copy", "drop"}
DATA_STREAMS = {"preserve", "drop"}
RESOURCE_THREADING = {"auto", "unlimited"}
CALIBRATION_MODES = {"auto", "ask", "always", "off"}


def user_config_path() -> Path:
    portable = portable_data_dir()
    if portable is not None:
        return portable / "engine-config.toml"
    base = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return base / "hevc-rebuild" / "config.toml"


def default_config_text() -> str:
    return '''# HEVC Rebuild Settings
#
# These defaults are recommended. Most people do not need to change anything.
# Edit this file, save it, then run HEVC Rebuild normally.

[video]
# Picture quality. Lower = better quality / larger files.
# 21 = high quality and recommended.
crf = 21

# Compression effort:
# medium   = faster, somewhat larger files
# slow     = recommended
# slower   = smaller files, MUCH slower
# veryslow = usually tiny extra savings for a huge time cost
preset = "slow"

# Keep 8-bit sources as 8-bit by default.
# true may reduce new banding in some encodes, but it does NOT add missing colors.
promote_8bit_to_10bit = false

[content]
# auto      = recommended; uses animation tuning only when reliable metadata says Animation
# animation = force x265 animation tuning
# grain     = preserve real film grain; larger files
# none      = normal x265 tuning
tune = "auto"

# When tune="auto", use Kodi/embedded genre metadata to identify animation.
detect_animation = true

[hdr]
# Preserve HDR10+ dynamic scene/frame metadata and verify it after encoding.
preserve_hdr10plus = true

# Dolby Vision handling:
# safe          = preserve supported single-layer DV when FFmpeg can do so safely; otherwise skip
# skip          = skip every Dolby Vision file
# hdr10-fallback = explicit option; Profile 7/8 only, with FFprobe-verified base layer + independent HDR10 identification
dolby_vision = "safe"

[streams]
# Selected real video tracks are encoded to HEVC. Use "copy" for legacy pass-through or "drop" for only the primary.
secondary_video = "encode"

# Preserve non-audio/non-subtitle data streams when Matroska supports them.
data_streams = "preserve"

# Audio and subtitles are ALWAYS stream-copied exactly. There is no setting to transcode them.

[resources]
# v1.2.4 adaptive resource policy
# CPU use:
# auto      = reserve configured CPU threads; memory limits still apply
# unlimited = use all eligible CPU threads before profile-aware memory limiting
# If RAM pressure appears, auto mode safely retries with fewer threads.
threading = "auto"
# Leave this many logical CPU threads free for Linux/desktop responsiveness.
reserved_cpu_threads = 1

# 0 = automatic. Leave these at 0 unless you want a manual cap.
max_worker_threads = 0
frame_threads = 0

# RAM safety: keep at least 3 GiB AND 8% of usable RAM available.
# The larger value wins. If crossed, the temporary encode retries with fewer threads.
min_free_memory_gib = 3.0
memory_reserve_percent = 8.0

# Optional extra free-space floor. Zero uses only the per-file working-space estimate.
# Before starting, budget up to 130% of source size for the temporary encode.
# During encoding, projected output size is always watched.
min_free_disk_gib = 0.0
preflight_output_ratio = 1.30
disk_projection_margin_percent = 15.0

[metadata]
# Matroska COMMENT tag added/updated by the utility.
comment = "Brought To you By V1x0r"

# Remove matching substrings from imported title clues, rename proposals, and
# human-readable media metadata on rebuilt output.
# Each entry is a separate case-insensitive pattern; '*' matches any sequence.
# Example: text_blacklist = ["Movie*Group"]
text_blacklist = ["Movie*Group"]

# Attach selected local poster art to the output Matroska file.
embed_artwork = true

[safety]
# Verify copied audio/subtitle/secondary-video packet payloads and codec data with SHA-256.
verify_copy_hashes = true

[calibration]
# Test a few x265 thread layouts on demanding media; the live memory guard remains authoritative.
# auto = only 4K/large files and reuse a matching result for 24 hours
# ask = prompt in an interactive terminal when no fresh cache entry exists; non-interactive runs skip
# always = benchmark every processed file (can add repeated overhead); off = disable benchmarks
mode = "auto"
cache_hours = 24.0
large_file_gib = 20.0
sample_seconds = 3.0
max_candidates = 2
throughput_tolerance_percent = 3.0
'''


def _insert_resource_options(text: str) -> tuple[str, bool]:
    """Add new calibration/resource defaults without overwriting user values."""
    changed = False
    if "[resources]" not in text:
        addition = """
[resources]
# CPU use: auto reserves configured CPU threads; unlimited skips that CPU reserve.
# Both policies retain the profile-aware memory cap and live memory guard.
threading = "auto"
reserved_cpu_threads = 1
max_worker_threads = 0
frame_threads = 0

# Keep at least 3 GiB AND 8% of usable RAM available; the larger value wins.
min_free_memory_gib = 3.0
        memory_reserve_percent = 8.0

# Optional extra free-space floor; 0 uses the media-derived working-space estimate.
min_free_disk_gib = 0.0
preflight_output_ratio = 1.30
disk_projection_margin_percent = 15.0
"""
        text = text.rstrip() + "\n\n" + addition.lstrip()
        changed = True
    else:
        required = [
            ("reserved_cpu_threads", "reserved_cpu_threads = 1"),
            ("memory_reserve_percent", "memory_reserve_percent = 8.0"),
            ("min_free_disk_gib", "min_free_disk_gib = 0.0"),
            ("preflight_output_ratio", "preflight_output_ratio = 1.30"),
            ("disk_projection_margin_percent", "disk_projection_margin_percent = 15.0"),
        ]
        missing = [line for key, line in required if key not in text]
        if missing:
            lines = text.splitlines()
            idx = next(i for i, line in enumerate(lines) if line.strip() == "[resources]")
            end = len(lines)
            for i in range(idx + 1, len(lines)):
                if lines[i].strip().startswith("[") and lines[i].strip().endswith("]"):
                    end = i
                    break
            block = [
                "",
                "# Adaptive resource/storage safety defaults",
                "# RAM uses the larger of the GiB floor and percentage reserve.",
                "# Disk is checked before and during every encode.",
                *missing,
            ]
            lines[end:end] = block
            text = "\n".join(lines) + ("\n" if text.endswith("\n") else "")
            changed = True

    if "[calibration]" not in text:
        addition = '''
[calibration]
# auto = quick calibration for 4K/large media, cached for 24 hours
# ask = prompt in an interactive terminal when uncached; always = fresh benchmark for every processed file
# off = disabled
mode = "auto"
cache_hours = 24.0
large_file_gib = 20.0
sample_seconds = 3.0
max_candidates = 2
throughput_tolerance_percent = 3.0
'''
        text = text.rstrip() + "\n\n" + addition.lstrip()
        changed = True
    return text, changed


def ensure_user_config() -> Path:
    path = user_config_path()
    if path.exists():
        try:
            text = path.read_text(encoding="utf-8")
            updated, changed = _insert_resource_options(text)
            if changed:
                path.write_text(updated, encoding="utf-8")
                print(f"Updated settings file with calibration/resource safety options: {path}")
        except OSError as exc:
            raise RebuildError(f"Cannot update settings file {path}: {exc}") from exc
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(default_config_text(), encoding="utf-8")
    print(f"Created settings file: {path}")
    print("The defaults are recommended. You can edit this file later if you want to change quality/speed settings.\n")
    return path


def _merge(base: dict[str, Any], overlay: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(base)
    for section, values in overlay.items():
        if section not in result:
            logging.warning("Ignoring unknown config section [%s].", section)
            continue
        if not isinstance(values, dict):
            raise RebuildError(f"Config section [{section}] must be a table.")
        for key, value in values.items():
            if key not in result[section]:
                logging.warning("Ignoring unknown config option %s.%s.", section, key)
                continue
            result[section][key] = value
    return result


def _read(path: Path) -> dict[str, Any]:
    try:
        with path.open("rb") as f:
            data = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as exc:
        raise RebuildError(f"Cannot read settings file {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise RebuildError(f"Settings file {path} is not a TOML table.")
    return data


def _expect_bool(cfg: dict[str, Any], section: str, key: str) -> bool:
    value = cfg[section][key]
    if not isinstance(value, bool):
        raise RebuildError(f"Config {section}.{key} must be true or false.")
    return value


def validate_metadata_text_blacklist(value: Any) -> list[str]:
    if not isinstance(value, list):
        raise RebuildError("Config metadata.text_blacklist must be a list of text patterns.")
    if len(value) > 100:
        raise RebuildError("Config metadata.text_blacklist can contain at most 100 patterns.")
    patterns: list[str] = []
    seen: set[str] = set()
    for index, pattern in enumerate(value, start=1):
        if not isinstance(pattern, str):
            raise RebuildError(f"Config metadata.text_blacklist entry {index} must be text.")
        if pattern != pattern.strip() or not pattern:
            raise RebuildError(f"Config metadata.text_blacklist entry {index} must not be empty or padded with spaces.")
        if len(pattern.encode("utf-8")) > 128:
            raise RebuildError(f"Config metadata.text_blacklist entry {index} is limited to 128 UTF-8 bytes.")
        if all(char == "*" for char in pattern):
            raise RebuildError(f"Config metadata.text_blacklist entry {index} must include text besides '*'.")
        if any(ord(char) < 32 or ord(char) == 127 for char in pattern):
            raise RebuildError(f"Config metadata.text_blacklist entry {index} cannot contain control characters.")
        normalized = pattern.casefold()
        if normalized in seen:
            raise RebuildError("Config metadata.text_blacklist cannot contain duplicate patterns.")
        seen.add(normalized)
        patterns.append(pattern)
    return patterns


def validate_config(cfg: dict[str, Any]) -> None:
    for section, values in cfg.items():
        if isinstance(values, dict):
            for key, value in values.items():
                if isinstance(value, float) and not math.isfinite(value):
                    raise RebuildError(f"Config {section}.{key} must be finite.")
    crf = cfg["video"]["crf"]
    if not isinstance(crf, int) or isinstance(crf, bool) or not 0 <= crf <= 51:
        raise RebuildError("Config video.crf must be an integer from 0 to 51.")
    if cfg["video"]["preset"] not in PRESETS:
        raise RebuildError("Config video.preset must be one of: " + ", ".join(sorted(PRESETS)))
    _expect_bool(cfg, "video", "promote_8bit_to_10bit")
    if cfg["content"]["tune"] not in TUNES:
        raise RebuildError("Config content.tune must be auto, animation, grain, or none.")
    _expect_bool(cfg, "content", "detect_animation")
    _expect_bool(cfg, "hdr", "preserve_hdr10plus")
    if cfg["hdr"]["dolby_vision"] not in DV_POLICIES:
        raise RebuildError("Config hdr.dolby_vision must be safe, skip, or hdr10-fallback.")
    if cfg["streams"]["secondary_video"] not in SECONDARY_VIDEO:
        raise RebuildError("Config streams.secondary_video must be encode, copy, or drop.")
    if cfg["streams"]["data_streams"] not in DATA_STREAMS:
        raise RebuildError("Config streams.data_streams must be preserve or drop.")
    if not isinstance(cfg["metadata"]["comment"], str):
        raise RebuildError("Config metadata.comment must be text.")
    if not isinstance(cfg["metadata"].get("embed_artwork", True), bool):
        raise RebuildError("Config metadata.embed_artwork must be true or false.")
    validate_metadata_text_blacklist(cfg["metadata"].get("text_blacklist", []))
    _expect_bool(cfg, "safety", "verify_copy_hashes")
    if not isinstance(cfg["calibration"]["mode"], str) or cfg["calibration"]["mode"] not in CALIBRATION_MODES:
        raise RebuildError("Config calibration.mode must be auto, ask, always, or off.")
    value = cfg["calibration"]["cache_hours"]
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not 0 <= value <= 720:
        raise RebuildError("Config calibration.cache_hours must be from 0 to 720.")
    value = cfg["calibration"]["large_file_gib"]
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not 0 <= value <= 1000:
        raise RebuildError("Config calibration.large_file_gib must be from 0 to 1000.")
    value = cfg["calibration"]["sample_seconds"]
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not 1 <= value <= 15:
        raise RebuildError("Config calibration.sample_seconds must be from 1 to 15.")
    value = cfg["calibration"]["max_candidates"]
    if not isinstance(value, int) or isinstance(value, bool) or not 1 <= value <= 4:
        raise RebuildError("Config calibration.max_candidates must be an integer from 1 to 4.")
    value = cfg["calibration"]["throughput_tolerance_percent"]
    if not isinstance(value, (int, float)) or isinstance(value, bool) or not 0 <= value <= 20:
        raise RebuildError("Config calibration.throughput_tolerance_percent must be from 0 to 20.")
    if cfg["resources"]["threading"] not in RESOURCE_THREADING:
        raise RebuildError("Config resources.threading must be auto or unlimited.")
    value = cfg["resources"]["reserved_cpu_threads"]
    if not isinstance(value, int) or isinstance(value, bool) or value < 0 or value > 64:
        raise RebuildError("Config resources.reserved_cpu_threads must be an integer from 0 to 64.")
    value = cfg["resources"]["max_worker_threads"]
    if not isinstance(value, int) or isinstance(value, bool) or value < 0 or value > 64:
        raise RebuildError("Config resources.max_worker_threads must be an integer from 0 to 64.")
    value = cfg["resources"]["frame_threads"]
    if not isinstance(value, int) or isinstance(value, bool) or value < 0 or value > 16:
        raise RebuildError("Config resources.frame_threads must be an integer from 0 to 16.")
    reserve = cfg["resources"]["min_free_memory_gib"]
    if not isinstance(reserve, (int, float)) or isinstance(reserve, bool) or reserve < 0:
        raise RebuildError("Config resources.min_free_memory_gib must be 0 or greater.")
    percent = cfg["resources"]["memory_reserve_percent"]
    if not isinstance(percent, (int, float)) or isinstance(percent, bool) or not 0 <= percent <= 50:
        raise RebuildError("Config resources.memory_reserve_percent must be from 0 to 50.")
    disk = cfg["resources"]["min_free_disk_gib"]
    if not isinstance(disk, (int, float)) or isinstance(disk, bool) or disk < 0:
        raise RebuildError("Config resources.min_free_disk_gib must be 0 or greater.")
    ratio = cfg["resources"]["preflight_output_ratio"]
    if not isinstance(ratio, (int, float)) or isinstance(ratio, bool) or ratio < 0.25 or ratio > 4.0:
        raise RebuildError("Config resources.preflight_output_ratio must be from 0.25 to 4.0.")
    margin = cfg["resources"]["disk_projection_margin_percent"]
    if not isinstance(margin, (int, float)) or isinstance(margin, bool) or margin < 0 or margin > 100:
        raise RebuildError("Config resources.disk_projection_margin_percent must be from 0 to 100.")


def load_config(root: Path, *, create_user: bool = True) -> tuple[dict[str, Any], list[Path]]:
    user_path = ensure_user_config() if create_user else user_config_path()
    cfg = copy.deepcopy(DEFAULTS)
    loaded: list[Path] = []
    if user_path.exists():
        cfg = _merge(cfg, _read(user_path))
        loaded.append(user_path)
    preferences_path = app_preferences_path()
    if preferences_path.exists():
        try:
            preferences = json.loads(preferences_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise RebuildError(f"Could not read application preferences at {preferences_path}: {exc}") from exc
        profile = preferences.get("settings") if isinstance(preferences, dict) else None
        if profile is not None:
            if not isinstance(profile, dict):
                raise RebuildError(f"Saved application settings in {preferences_path} must be an object or null.")
            cfg = _merge(cfg, profile)
            loaded.append(preferences_path)
    local = root / ".hevc-rebuild.toml"
    if local.exists():
        cfg = _merge(cfg, _read(local))
        loaded.append(local)
    validate_config(cfg)
    return cfg, loaded


def apply_to_args(args: Any, cfg: dict[str, Any]) -> None:
    """Apply settings to parser/compatibility namespaces; optional fields may be absent."""
    # CLI values win. argparse uses None for settings that are configurable here.
    args.crf = cfg["video"]["crf"] if args.crf is None else args.crf
    args.preset = cfg["video"]["preset"] if args.preset is None else args.preset
    args.promote_8bit_to_10bit = (
        cfg["video"]["promote_8bit_to_10bit"]
        if args.promote_8bit_to_10bit is None else args.promote_8bit_to_10bit
    )
    args.tune = cfg["content"]["tune"] if args.tune is None else args.tune
    args.detect_animation = cfg["content"]["detect_animation"] if args.detect_animation is None else args.detect_animation
    args.preserve_hdr10plus = cfg["hdr"]["preserve_hdr10plus"] if args.preserve_hdr10plus is None else args.preserve_hdr10plus
    args.dolby_vision_policy = cfg["hdr"]["dolby_vision"] if args.dolby_vision_policy is None else args.dolby_vision_policy
    args.secondary_video = cfg["streams"]["secondary_video"] if args.secondary_video is None else args.secondary_video
    args.data_streams = cfg["streams"]["data_streams"] if args.data_streams is None else args.data_streams
    args.comment = cfg["metadata"]["comment"] if args.comment is None else args.comment
    current_artwork = getattr(args, "embed_artwork", None)
    args.embed_artwork = cfg["metadata"].get("embed_artwork", True) if current_artwork is None else current_artwork
    cli_patterns = getattr(args, "metadata_text_blacklist", None)
    if getattr(args, "replace_metadata_text_blacklist", False):
        patterns = list(cli_patterns or [])
    else:
        patterns = list(cfg["metadata"].get("text_blacklist", []))
        known = {pattern.casefold() for pattern in patterns}
        for pattern in cli_patterns or []:
            if pattern.casefold() not in known:
                patterns.append(pattern)
                known.add(pattern.casefold())
    args.metadata_text_blacklist = validate_metadata_text_blacklist(patterns)
    args.verify_copy_hashes = cfg["safety"]["verify_copy_hashes"] if args.verify_copy_hashes is None else args.verify_copy_hashes
    current = getattr(args, "resource_threading", None)
    args.resource_threading = cfg["resources"]["threading"] if current is None else current
    current = getattr(args, "reserved_cpu_threads", None)
    args.reserved_cpu_threads = cfg["resources"]["reserved_cpu_threads"] if current is None else current
    current = getattr(args, "max_worker_threads", None)
    args.max_worker_threads = cfg["resources"]["max_worker_threads"] if current is None else current
    current = getattr(args, "frame_threads", None)
    args.frame_threads = cfg["resources"]["frame_threads"] if current is None else current
    current = getattr(args, "min_free_memory_gib", None)
    args.min_free_memory_gib = cfg["resources"]["min_free_memory_gib"] if current is None else current
    current = getattr(args, "memory_reserve_percent", None)
    args.memory_reserve_percent = cfg["resources"]["memory_reserve_percent"] if current is None else current
    current = getattr(args, "min_free_disk_gib", None)
    args.min_free_disk_gib = cfg["resources"]["min_free_disk_gib"] if current is None else current
    current = getattr(args, "preflight_output_ratio", None)
    args.preflight_output_ratio = cfg["resources"]["preflight_output_ratio"] if current is None else current
    current = getattr(args, "disk_projection_margin_percent", None)
    args.disk_projection_margin_percent = cfg["resources"]["disk_projection_margin_percent"] if current is None else current
    current = getattr(args, "calibration_mode", None)
    args.calibration_mode = cfg["calibration"]["mode"] if current is None else current
    current = getattr(args, "calibration_cache_hours", None)
    args.calibration_cache_hours = cfg["calibration"]["cache_hours"] if current is None else current
    current = getattr(args, "calibration_large_file_gib", None)
    args.calibration_large_file_gib = cfg["calibration"]["large_file_gib"] if current is None else current
    current = getattr(args, "calibration_sample_seconds", None)
    args.calibration_sample_seconds = cfg["calibration"]["sample_seconds"] if current is None else current
    current = getattr(args, "calibration_max_candidates", None)
    args.calibration_max_candidates = cfg["calibration"]["max_candidates"] if current is None else current
    current = getattr(args, "calibration_tolerance_percent", None)
    args.calibration_tolerance_percent = cfg["calibration"]["throughput_tolerance_percent"] if current is None else current
