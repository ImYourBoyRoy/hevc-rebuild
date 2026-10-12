from __future__ import annotations

import argparse
import contextlib
import csv
import json
import re
import shutil
import sys
from dataclasses import asdict
from pathlib import Path
from typing import Any

from .app import already_processed
from .config import load_config, user_config_path
from .deps import TOOL_OVERRIDE_KEYS, apt_update_status, ensure_hdr10plus, refresh_apt_catalog, upgrade_apt_packages
from .environment import EnvironmentReport, inspect_environment, missing_apt_packages
from .probe import build_profile
from .encode import choose_resource_plan
from .resources import MemoryStats, PressureStats, automatic_memory_reserve_gib, memory_stats, pressure_stats
from .util import RebuildError, user_log_directory
from .youtube import _expected_dimensions, youtube_output_estimate


LOG_DIR = user_log_directory()


def _emit(value: Any) -> None:
    print(json.dumps(value, ensure_ascii=False, default=str))


def _environment_payload(report: EnvironmentReport, *, custom_ffmpeg: bool = False) -> dict[str, Any]:
    return {
        "os": {
            "name": report.os_name,
            "version": report.os_version,
            "supported": report.supported_platform,
            "note": report.platform_note,
        },
        "hardware": {
            "cpu_model": report.cpu_model,
            "logical_cpus": report.logical_cpus,
            "memory_total_gib": report.memory.total_gib,
            "memory_available_gib": report.memory.available_gib,
            "swap_total_gib": report.memory.swap_total_gib,
            "swap_free_gib": report.memory.swap_free_gib,
            "swap_used_gib": report.memory.swap_used_gib,
            "swap_used_percent": report.memory.swap_used_percent,
            "gpu_devices": list(report.gpu_devices),
            "encoder": "CPU libx265",
        },
        "checks": [asdict(check) for check in report.checks],
        "x265_pixel_formats": sorted(report.x265_pixel_formats),
        "dolby_vision_x265": report.dolby_vision_x265,
        "can_analyze": report.analysis_toolchain is not None,
        "youtube_ready": report.youtube_ready,
        "ready": report.ready,
        "missing_apt_packages": list(missing_apt_packages(report, using_custom_ffmpeg=custom_ffmpeg)),
    }


def _profile_payload(profile) -> dict[str, Any]:
    probe_streams = profile.probe.get("streams", []) or []
    mediainfo_tracks = ((profile.mediainfo.get("media") or {}).get("track") or [])
    media_tracks_by_order: dict[int, dict[str, Any]] = {}
    media_tracks_by_type: dict[str, list[dict[str, Any]]] = {}
    for track in mediainfo_tracks:
        track_kind = str(track.get("@type") or "").lower()
        media_tracks_by_type.setdefault(track_kind, []).append(track)
        raw_order = str(track.get("StreamOrder") or "")
        match = re.match(r"\s*(\d+)", raw_order)
        if match:
            media_tracks_by_order.setdefault(int(match.group(1)), track)
    selected_media_fields = (
        "Format", "Format_Profile", "CodecID", "CodecID_Description", "Width", "Height",
        "BitDepth", "ChromaSubsampling", "FrameRate", "FrameRate_Mode", "ScanType",
        "colour_primaries", "transfer_characteristics", "matrix_coefficients", "colour_range",
        "HDR_Format", "HDR_Format_Profile", "HDR_Format_Compatibility",
        "MasteringDisplay_ColorPrimaries", "MasteringDisplay_Luminance", "MaxCLL", "MaxFALL",
        "Language", "Title", "Channels", "ChannelLayout", "SamplingRate", "BitRate",
        "Compression_Mode", "Default", "Forced", "StreamSize", "Duration",
        "Format_Settings", "Encoded_Library", "WritingLibrary", "InternetMediaType",
    )
    tracks: list[dict[str, Any]] = []
    ordinals: dict[str, int] = {}
    for stream in probe_streams:
        kind = str(stream.get("codec_type") or "unknown")
        ordinal = ordinals.get(kind, 0)
        ordinals[kind] = ordinal + 1
        index = int(stream.get("index", -1))
        typed_mediainfo = media_tracks_by_type.get(kind, [])
        mi = media_tracks_by_order.get(index) or (typed_mediainfo[ordinal] if ordinal < len(typed_mediainfo) else {})
        tags = stream.get("tags") or {}
        dispositions = stream.get("disposition") or {}
        track: dict[str, Any] = {
            "index": index,
            "type": kind,
            "ordinal": ordinal,
            "codec": stream.get("codec_name"),
            "profile": stream.get("profile"),
            "width": stream.get("width"),
            "height": stream.get("height"),
            "pixel_format": stream.get("pix_fmt"),
            "bit_depth": stream.get("bits_per_raw_sample") or stream.get("bits_per_sample"),
            "color_primaries": stream.get("color_primaries"),
            "color_transfer": stream.get("color_transfer"),
            "color_matrix": stream.get("color_space"),
            "frame_rate": stream.get("avg_frame_rate"),
            "channels": stream.get("channels"),
            "channel_layout": stream.get("channel_layout"),
            "sample_rate": stream.get("sample_rate"),
            "bit_rate": stream.get("bit_rate"),
            "language": tags.get("language"),
            "title": tags.get("title"),
            "default": bool(dispositions.get("default", 0)),
            "forced": bool(dispositions.get("forced", 0)),
            "attached_picture": bool(dispositions.get("attached_pic", 0)),
            "mediainfo": {key: mi[key] for key in selected_media_fields if key in mi},
        }
        if kind == "attachment":
            track["filename"] = tags.get("filename")
            track["mime_type"] = tags.get("mimetype")
        tracks.append(track)

    return {
        "video": {
            "codec": profile.primary_video.codec_name,
            "width": profile.primary_video.width,
            "height": profile.primary_video.height,
            "source_bit_depth": profile.source_bit_depth,
            "source_chroma": profile.source_chroma,
            "target_pixel_format": profile.target_pix_fmt,
            "secondary_count": len(profile.secondary_videos),
        },
        "hdr": {
            "hdr10": profile.hdr10,
            "hdr10plus": profile.hdr10plus,
            "hlg": profile.hlg,
            "dolby_vision": profile.dolby_vision,
            "dolby_vision_profile": profile.dovi_profile,
        },
        "audio_count": len(profile.audio),
        "subtitle_count": len(profile.subtitles),
        "attachment_count": len(profile.attachments),
        "data_stream_count": len(profile.data),
        "tracks": tracks,
        "mediainfo_general": {
            key: value for key, value in next(
                (track for track in mediainfo_tracks if str(track.get("@type", "")).lower() == "general"), {}
            ).items() if key not in {"@type", "CompleteName", "FolderName", "FileName", "FileExtension", "FileSize", "Duration"}
        },
        "chapters_count": profile.chapters_count,
        "duration_seconds": profile.duration,
        "interlaced": profile.interlaced,
    }


def _memory_forecast(
    profile,
    settings: dict[str, Any],
    mem: MemoryStats,
    pressure: PressureStats,
    *,
    target_dimensions: tuple[int, int] | None = None,
    preset: str | None = None,
    frame_threads: int | None = None,
) -> dict[str, Any]:
    resources = settings["resources"]
    try:
        plan = choose_resource_plan(
            profile,
            target_dimensions=target_dimensions,
            threading=resources["threading"],
            reserved_cpu_threads=resources["reserved_cpu_threads"],
            max_worker_threads=resources["max_worker_threads"],
            frame_threads=resources["frame_threads"] if frame_threads is None else frame_threads,
            preset=preset or settings["video"]["preset"],
            min_free_memory_gib=resources["min_free_memory_gib"],
            memory_reserve_percent=resources["memory_reserve_percent"],
            mem_stats=mem,
            pressure=pressure,
        )
        return {
            **asdict(plan.memory_forecast),
            "plan_reason": plan.reason,
        }
    except RebuildError as exc:
        return {
            "model": "profile-scaled-concurrency-v1",
            "level": "blocked",
            "available_gib": mem.available_gib,
            "total_gib": mem.total_gib,
            "swap_free_gib": mem.swap_free_gib,
            "swap_used_gib": mem.swap_used_gib,
            "swap_used_percent": mem.swap_used_percent,
            "reserve_gib": automatic_memory_reserve_gib(
                mem.total_gib,
                floor_gib=resources["min_free_memory_gib"],
                percent=resources["memory_reserve_percent"],
            ),
            "planning_budget_gib": None,
            "planned_workers": None,
            "planned_frame_threads": None,
            "messages": [str(exc)],
        }


def _scan(root: Path, *, recursive: bool, min_size_mib: int, ffmpeg_path: str | None, tool_paths: dict[str, str | None] | None = None) -> dict[str, Any]:
    root = root.expanduser().resolve()
    if not (root.is_dir() or root.is_file()):
        raise ValueError(f"Media input does not exist: {root}")
    from .discovery import discover

    input_is_file = root.is_file()
    source_base = root.parent if input_is_file else root
    files = discover(root, recursive=recursive, min_size=min_size_mib * 1024 * 1024)
    report = inspect_environment(ffmpeg_path, tool_paths)
    mem = memory_stats()
    pressure = pressure_stats()
    entries: list[dict[str, Any]] = []
    with contextlib.redirect_stdout(sys.stderr):
        config, config_sources = load_config(source_base, create_user=False)
    for index, path in enumerate(files, start=1):
        print(f"__HEVC_SCAN_PROGRESS__\t{index}\t{len(files)}\t{path.name}", flush=True)
        try:
            size_bytes = path.stat().st_size
        except OSError:
            size_bytes = 0
        entry: dict[str, Any] = {
            "path": str(path),
            "relative_path": path.name if input_is_file else str(path.relative_to(source_base)),
            "name": path.name,
            "size_bytes": size_bytes,
            "status": "Tools required",
            "already_processed": already_processed(path),
            "profile": None,
            "error": None,
        }
        if report.analysis_toolchain:
            try:
                profile = build_profile(
                    report.analysis_toolchain,
                    path,
                    promote_8bit_to_10bit=config["video"]["promote_8bit_to_10bit"],
                )
                entry["profile"] = _profile_payload(profile)
                entry["resource_forecast"] = _memory_forecast(profile, config, mem, pressure)
                entry["youtube_estimates"] = {
                    resolution: {
                        quality_mode: youtube_output_estimate(profile, resolution, quality_mode)
                        for quality_mode in ("balanced", "recommended")
                    }
                    for resolution in ("source", "1440p", "4k")
                }
                entry["youtube_resource_forecasts"] = {
                    resolution: _memory_forecast(
                        profile,
                        config,
                        mem,
                        pressure,
                        target_dimensions=_expected_dimensions(profile, resolution),
                        preset="slow",
                        frame_threads=0,
                    )
                    for resolution in ("source", "1440p", "4k")
                }
                entry["status"] = "Backed up" if entry["already_processed"] else "Ready"
            except Exception as exc:
                entry["status"] = "Analysis failed"
                entry["error"] = f"{type(exc).__name__}: {exc}"
        else:
            entry["error"] = "FFprobe and MediaInfo are needed to read stream profiles."
        entries.append(entry)
    return {
        "root": str(root),
        "input_kind": "file" if input_is_file else "folder",
        "recursive": recursive,
        "minimum_size_mib": min_size_mib,
        "candidate_count": len(files),
        "total_input_bytes": sum(row["size_bytes"] for row in entries),
        "disk_free_bytes": shutil.disk_usage(source_base).free,
        "entries": entries,
        "environment": _environment_payload(report, custom_ffmpeg=bool(ffmpeg_path or (tool_paths or {}).get("ffmpeg"))),
        "settings": config,
        "settings_sources": [str(path) for path in config_sources],
    }


def _defaults(root: Path) -> dict[str, Any]:
    root = root.expanduser()
    if root.is_file():
        root = root.parent
    if not root.is_dir():
        root = Path.home()
    with contextlib.redirect_stdout(sys.stderr):
        config, sources = load_config(root, create_user=False)
    return {"settings": config, "sources": [str(path) for path in sources], "user_config": str(user_config_path())}


def _reports(limit: int = 12) -> dict[str, Any]:
    reports: list[dict[str, Any]] = []
    try:
        paths = sorted(LOG_DIR.glob("hevc-rebuild-*.csv"), key=lambda path: path.stat().st_mtime, reverse=True)[:limit]
    except OSError:
        paths = []
    for path in paths:
        try:
            with path.open(newline="", encoding="utf-8") as stream:
                rows = list(csv.DictReader(stream))
            log_path = path.with_suffix(".log")
            counts: dict[str, int] = {}
            for row in rows:
                status = row.get("status", "UNKNOWN")
                counts[status] = counts.get(status, 0) + 1
            reports.append({
                "csv_path": str(path),
                "log_path": str(log_path) if log_path.exists() else None,
                "created": path.stat().st_mtime,
                "counts": counts,
                "rows": rows,
            })
        except (OSError, csv.Error):
            continue
    return {"reports": reports}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="hevc-rebuild-bridge")
    subparsers = parser.add_subparsers(dest="action", required=True)

    def add_tool_paths(command_parser: argparse.ArgumentParser) -> None:
        for name in TOOL_OVERRIDE_KEYS:
            command_parser.add_argument(f"--{name.replace('_', '-')}-path")

    def selected_tool_paths(parsed) -> dict[str, str | None]:
        return {name: getattr(parsed, f"{name}_path", None) for name in TOOL_OVERRIDE_KEYS}

    env_parser = subparsers.add_parser("environment")
    add_tool_paths(env_parser)

    scan_parser = subparsers.add_parser("scan")
    scan_parser.add_argument("--root", required=True)
    scan_parser.add_argument("--recursive", action="store_true")
    scan_parser.add_argument("--min-size-mib", type=int, default=5)
    add_tool_paths(scan_parser)

    defaults_parser = subparsers.add_parser("defaults")
    defaults_parser.add_argument("--root", required=True)

    reports_parser = subparsers.add_parser("reports")
    reports_parser.add_argument("--limit", type=int, default=12)

    subparsers.add_parser("apt-updates", help="Compare installed packages to the currently cached Ubuntu APT catalog (read-only)")
    apt_refresh_parser = subparsers.add_parser("apt-refresh", help="Explicitly refresh the Ubuntu APT package index")
    apt_refresh_parser.add_argument("--confirm", action="store_true", help="Confirm the package-index refresh")
    apt_upgrade_parser = subparsers.add_parser(
        "apt-upgrade",
        help="Upgrade exactly the packages reported by apt-updates after a safe simulation",
    )
    apt_upgrade_parser.add_argument("--confirm", action="store_true", help="Confirm this exact package update")
    apt_upgrade_parser.add_argument(
        "--package", action="append", required=True, metavar="NAME",
        help="Package name from the current apt-updates result; repeat for each listed package",
    )
    subparsers.add_parser("install-hdr10plus")

    args = parser.parse_args(argv)
    try:
        if args.action == "environment":
            tool_paths = selected_tool_paths(args)
            report = inspect_environment(tool_paths=tool_paths)
            _emit(_environment_payload(report, custom_ffmpeg=bool(tool_paths.get("ffmpeg"))))
        elif args.action == "scan":
            if args.min_size_mib < 0:
                raise ValueError("Minimum file size must be zero or greater.")
            payload = _scan(
                Path(args.root),
                recursive=args.recursive,
                min_size_mib=args.min_size_mib,
                ffmpeg_path=args.ffmpeg_path,
                tool_paths=selected_tool_paths(args),
            )
            _emit(payload)
        elif args.action == "defaults":
            _emit(_defaults(Path(args.root)))
        elif args.action == "reports":
            _emit(_reports(max(1, min(args.limit, 50))))
        elif args.action == "apt-updates":
            _emit(apt_update_status())
        elif args.action == "apt-refresh":
            if not args.confirm:
                raise ValueError("APT catalog refresh requires --confirm.")
            refresh_apt_catalog(assume_yes=True, force=True)
            _emit({"refreshed": True, "note": "Package indexes refreshed; no packages were installed or upgraded."})
        elif args.action == "apt-upgrade":
            if not args.confirm:
                raise ValueError("APT package upgrades require --confirm.")
            _emit(upgrade_apt_packages(args.package))
        elif args.action == "install-hdr10plus":
            path = ensure_hdr10plus(assume_yes=True, skip_update_check=True)
            _emit({"installed": path})
        return 0
    except Exception as exc:
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
