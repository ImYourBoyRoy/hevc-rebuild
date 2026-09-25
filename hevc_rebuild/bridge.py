from __future__ import annotations

import argparse
import contextlib
import csv
import json
import shutil
import sys
from dataclasses import asdict
from pathlib import Path
from typing import Any

from .app import already_processed
from .config import load_config, user_config_path
from .deps import ensure_hdr10plus
from .environment import EnvironmentReport, inspect_environment, missing_apt_packages
from .probe import build_profile
from .encode import choose_resource_plan
from .resources import MemoryStats, PressureStats, automatic_memory_reserve_gib, memory_stats, pressure_stats
from .util import RebuildError, user_log_directory


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
        "ready": report.ready,
        "missing_apt_packages": list(missing_apt_packages(report, using_custom_ffmpeg=custom_ffmpeg)),
    }


def _profile_payload(profile) -> dict[str, Any]:
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
        "chapters_count": profile.chapters_count,
        "duration_seconds": profile.duration,
        "interlaced": profile.interlaced,
    }


def _memory_forecast(profile, settings: dict[str, Any], mem: MemoryStats, pressure: PressureStats) -> dict[str, Any]:
    resources = settings["resources"]
    try:
        plan = choose_resource_plan(
            profile,
            threading=resources["threading"],
            reserved_cpu_threads=resources["reserved_cpu_threads"],
            max_worker_threads=resources["max_worker_threads"],
            frame_threads=resources["frame_threads"],
            preset=settings["video"]["preset"],
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


def _scan(root: Path, *, recursive: bool, min_size_mib: int, ffmpeg_path: str | None) -> dict[str, Any]:
    root = root.expanduser().resolve()
    if not root.is_dir():
        raise ValueError(f"Media folder does not exist: {root}")
    from .discovery import discover

    files = discover(root, recursive=recursive, min_size=min_size_mib * 1024 * 1024)
    report = inspect_environment(ffmpeg_path)
    mem = memory_stats()
    pressure = pressure_stats()
    entries: list[dict[str, Any]] = []
    with contextlib.redirect_stdout(sys.stderr):
        config, config_sources = load_config(root, create_user=False)
    for index, path in enumerate(files, start=1):
        print(f"__HEVC_SCAN_PROGRESS__\t{index}\t{len(files)}\t{path.name}", flush=True)
        try:
            size_bytes = path.stat().st_size
        except OSError:
            size_bytes = 0
        entry: dict[str, Any] = {
            "path": str(path),
            "relative_path": str(path.relative_to(root)),
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
                entry["status"] = "Backed up" if entry["already_processed"] else "Ready"
            except Exception as exc:
                entry["status"] = "Analysis failed"
                entry["error"] = f"{type(exc).__name__}: {exc}"
        else:
            entry["error"] = "FFprobe and MediaInfo are needed to read stream profiles."
        entries.append(entry)
    return {
        "root": str(root),
        "recursive": recursive,
        "minimum_size_mib": min_size_mib,
        "candidate_count": len(files),
        "total_input_bytes": sum(row["size_bytes"] for row in entries),
        "disk_free_bytes": shutil.disk_usage(root).free,
        "entries": entries,
        "environment": _environment_payload(report, custom_ffmpeg=bool(ffmpeg_path)),
        "settings": config,
        "settings_sources": [str(path) for path in config_sources],
    }


def _defaults(root: Path) -> dict[str, Any]:
    root = root.expanduser()
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

    env_parser = subparsers.add_parser("environment")
    env_parser.add_argument("--ffmpeg-path")

    scan_parser = subparsers.add_parser("scan")
    scan_parser.add_argument("--root", required=True)
    scan_parser.add_argument("--recursive", action="store_true")
    scan_parser.add_argument("--min-size-mib", type=int, default=5)
    scan_parser.add_argument("--ffmpeg-path")

    defaults_parser = subparsers.add_parser("defaults")
    defaults_parser.add_argument("--root", required=True)

    reports_parser = subparsers.add_parser("reports")
    reports_parser.add_argument("--limit", type=int, default=12)

    subparsers.add_parser("install-hdr10plus")

    args = parser.parse_args(argv)
    try:
        if args.action == "environment":
            report = inspect_environment(args.ffmpeg_path)
            _emit(_environment_payload(report, custom_ffmpeg=bool(args.ffmpeg_path)))
        elif args.action == "scan":
            if args.min_size_mib < 0:
                raise ValueError("Minimum file size must be zero or greater.")
            payload = _scan(
                Path(args.root),
                recursive=args.recursive,
                min_size_mib=args.min_size_mib,
                ffmpeg_path=args.ffmpeg_path,
            )
            _emit(payload)
        elif args.action == "defaults":
            _emit(_defaults(Path(args.root)))
        elif args.action == "reports":
            _emit(_reports(max(1, min(args.limit, 50))))
        elif args.action == "install-hdr10plus":
            path = ensure_hdr10plus(assume_yes=True, skip_update_check=True)
            _emit({"installed": path})
        return 0
    except Exception as exc:
        print(f"{type(exc).__name__}: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
