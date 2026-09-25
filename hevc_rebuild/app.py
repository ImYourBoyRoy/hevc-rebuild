from __future__ import annotations

import argparse
import logging
import os
import shutil
import signal
import sys
import tempfile
import time
import uuid
from urllib.parse import unquote, urlparse
from pathlib import Path

from . import __version__
from .calibration import calibration_key, cached_plan, is_demanding, run_calibration, store_selection
from .config import apply_to_args, load_config, user_config_path
from .content import resolve_tune
from .deps import resolve_toolchain
from .discovery import discover
from .encode import DiskPressureError, MemoryPressureError, choose_resource_plan, encode_args, run_encode
from .hdr import (
    dolbyvision_hdr10_fallback_reasons, dolbyvision_safe_preservation_reasons,
    extract_hdr10plus, verify_hdr10plus,
)
from .metadata import apply_metadata, choose_poster
from .model import MediaProfile, Toolchain
from .probe import build_profile, ffprobe_json
from .util import (
    RebuildError, UserAbort, append_csv, configure_logging, fsync_dir, fsync_file,
    human_bytes, unique_backup, user_log_directory,
)
from .validate import validate_output
from .ui import print_batch_card, print_batch_summary, print_encode_card, print_phase, print_success, print_warning
from .resources import automatic_memory_reserve_gib, disk_stats, effective_cpu_threads, memory_stats, pressure_stats, snapshot


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="hevc-rebuild",
        description="Transactional HEVC/x265 media-library rebuild for Ubuntu and Linux Mint. Audio and subtitles are stream-copied and verified, never transcoded.",
    )
    p.add_argument("root", nargs="?", default=".", help="Movie/TV root directory (default: current directory)")
    p.add_argument("--crf", type=int, default=None, help="Override configured x265 CRF")
    p.add_argument("--preset", default=None, choices=["ultrafast","superfast","veryfast","faster","fast","medium","slow","slower","veryslow"], help="Override configured x265 preset")
    p.add_argument("--promote-8bit-to-10bit", action=argparse.BooleanOptionalAction, default=None, help="Optionally encode 8-bit sources as 10-bit HEVC; default comes from config")
    p.add_argument("--tune", choices=["auto","animation","grain","none"], default=None, help="Override content tune from config")
    p.add_argument("--detect-animation", action=argparse.BooleanOptionalAction, default=None, help="Auto-detect Animation from reliable metadata when tune=auto")
    p.add_argument("--preserve-hdr10plus", action=argparse.BooleanOptionalAction, default=None, help="Preserve and verify HDR10+ dynamic metadata")
    p.add_argument("--recursive", action="store_true", help="Process all supported media recursively instead of legacy movie/Season discovery")
    p.add_argument("--min-size-mib", type=int, default=5, help="Ignore files smaller than this")
    p.add_argument("--secondary-video", choices=["copy","drop"], default=None, help="Highest resolution is always primary; override secondary-video policy")
    p.add_argument("--data-streams", choices=["preserve","drop"], default=None)
    p.add_argument("--verify-copy-hashes", action=argparse.BooleanOptionalAction, default=None, help="Packet-by-packet SHA-256 + codec-extradata verify copied audio/subtitle/secondary video")
    p.add_argument("--resource-threading", choices=["auto","unlimited"], default=None, help="x265 CPU policy; both modes keep the profile-aware memory cap and live memory guard")
    p.add_argument("--reserved-cpu-threads", type=int, default=None, help="Logical CPU threads left free in auto mode; default 1")
    p.add_argument("--max-worker-threads", type=int, default=None, help="Advanced x265 worker-thread cap; 0/omitted = automatic")
    p.add_argument("--frame-threads", type=int, default=None, help="Advanced x265 concurrent frame cap; 0/omitted = automatic")
    p.add_argument("--min-free-memory-gib", type=float, default=None, help="Minimum RAM reserve floor in GiB")
    p.add_argument("--memory-reserve-percent", type=float, default=None, help="Also reserve this percent of usable RAM; larger of percent/floor wins")
    p.add_argument("--min-free-disk-gib", type=float, default=None, help="Disk-space reserve that must remain free on the output filesystem")
    p.add_argument("--preflight-output-ratio", type=float, default=None, help="Conservative temporary-output budget as a ratio of source size")
    p.add_argument("--disk-projection-margin-percent", type=float, default=None, help="Extra margin applied to live projected output-size checks")
    p.add_argument("--calibration-mode", choices=["auto", "ask", "always", "off"], default=None, help="Quickly compare x265 thread layouts for demanding files; live resource guards remain active")
    p.add_argument("--calibration-cache-hours", type=float, default=None, help="Reuse matching hardware calibration for this many hours")
    p.add_argument("--calibration-large-file-gib", type=float, default=None, help="Files at least this size qualify for automatic calibration")
    p.add_argument("--calibration-sample-seconds", type=float, default=None, help="Representative video seconds encoded per candidate")
    p.add_argument("--calibration-max-candidates", type=int, default=None, help="Maximum thread layouts compared during calibration")
    p.add_argument("--calibration-tolerance-percent", type=float, default=None, help="Within this throughput margin, prefer the lower-memory layout")
    p.add_argument("--info-only", action="store_true", help="Analyze and print planned behavior; do not encode")
    p.add_argument("--test", action="store_true", help="Create a short Test_ encode without moving the source")
    p.add_argument("--test-seconds", type=int, default=5)
    p.add_argument("--test-crf", type=int, default=30)
    p.add_argument("--reprocess", action="store_true", help="Process even if an old/ backup already exists")
    p.add_argument("--keep-work", action="store_true")
    p.add_argument("--force-deinterlace", action="store_true")
    p.add_argument("--disable-deinterlace", action="store_true")
    p.add_argument("--disable-hdr10-opt", action="store_true")
    p.add_argument("--preserve-source-level", action="store_true", help="Force x265 level from source when >=5; disabled by default because auto-level is safer")
    p.add_argument("--dolby-vision-policy", choices=["safe","skip","hdr10-fallback"], default=None, help="safe preserves verified single-layer Profile 8 when supported; hdr10-fallback explicitly strips DV only from verified Profile 7/8 HDR10-base sources")
    p.add_argument("--comment", default=None, help="Override configured Matroska COMMENT tag")
    p.add_argument("--show-config", action="store_true", help="Print the active config path and settings, then exit")
    p.add_argument("--setup-only", action="store_true", help="Install/update/check dependencies then exit")
    p.add_argument("--assume-yes", action="store_true", help="Noninteractive dependency installation/update approval")
    p.add_argument("--skip-update-check", action="store_true", help="Do not refresh/check dependency updates")
    p.add_argument("--ffmpeg-path", default=None, help="Use this FFmpeg executable and its adjacent FFprobe when available")
    p.add_argument("--no-dependency-repair", action="store_true", help="Check existing tools only; never install packages or download helpers")
    p.add_argument("--stop-request-file", default=None, help=argparse.SUPPRESS)
    p.add_argument("--allow-unsupported-os", action="store_true", help="Expert override for unsupported/non-Ubuntu-family systems")
    p.add_argument("--stop-on-error", action="store_true")
    p.add_argument("--verbose", action="store_true")
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    return p



def _clean_user_path(value: str) -> str:
    value = value.strip().strip('\"').strip("'")
    if value.startswith("file://"):
        parsed = urlparse(value)
        value = unquote(parsed.path)
    return value


def interactive_wizard() -> list[str]:
    """Beginner-friendly launcher used when the command is run with no arguments."""
    print("\nHEVC Rebuild - Guided Mode")
    print("==========================")
    print("This tool rebuilds video as HEVC/x265 while copying audio and subtitles exactly.")
    print("Your original files are kept until the new files pass verification.\n")

    while True:
        raw = input("Media folder to process (you can drag the folder into this window): ")
        root = _clean_user_path(raw)
        if not root:
            root = str(Path.cwd())
        path = Path(root).expanduser()
        if path.exists() and path.is_dir():
            root = str(path.resolve())
            break
        print(f"I can't find that folder: {path}")
        print("Try again, or press Ctrl+C to quit.\n")

    print("\nWhat would you like to do?")
    print("  1) Check only - safest first step; changes nothing")
    print("  2) Make short test videos - originals stay untouched")
    print("  3) Full rebuild - process the real files")
    while True:
        choice = input("Choose 1, 2, or 3 [1]: ").strip() or "1"
        if choice in {"1", "2", "3"}:
            break
        print("Please enter 1, 2, or 3.")

    print("\nDoes that folder contain movies/shows inside other folders?")
    print("Press ENTER for Yes (recommended for a normal media library), or type N for No.")
    recursive = (input("Search inside subfolders? [Y/n]: ").strip().lower() not in {"n", "no"})

    print("\nHow should unsupported Dolby Vision Profile 7/8 files be handled?")
    print("  1) Safe mode - preserve compatible Dolby Vision; otherwise skip the file")
    print("  2) HDR10 fallback - when a DV base layer is verified and the picture identifies as HDR10, discard Dolby Vision")
    print("     The untouched original is still kept in old/ after a successful full rebuild.")
    while True:
        dv_choice = input("Choose 1 or 2 [1]: ").strip() or "1"
        if dv_choice in {"1", "2"}:
            break
        print("Please enter 1 or 2.")

    argv = [root]
    if recursive:
        argv.append("--recursive")
    argv += ["--dolby-vision-policy", "hdr10-fallback" if dv_choice == "2" else "safe"]
    if choice == "1":
        argv.append("--info-only")
    elif choice == "2":
        argv.append("--test")
    return argv

def print_profile(profile: MediaProfile) -> None:
    print(f"\n{profile.path}")
    print(f"  Primary video: {profile.primary_video.label}  DEFAULT after rebuild")
    if profile.secondary_videos:
        print("  Other video:   " + "; ".join(v.label for v in profile.secondary_videos))
    print(f"  Audio/subs:    {len(profile.audio)} audio / {len(profile.subtitles)} subtitles (COPY ONLY)")
    print(f"  Pixel format:  {profile.primary_video.pix_fmt} -> {profile.target_pix_fmt}")
    print(f"  Color:         {profile.color_primaries or '?'} / {profile.transfer or '?'} / {profile.color_matrix or '?'}")
    print(f"  HDR:           HDR10={profile.hdr10} HDR10+={profile.hdr10plus} HLG={profile.hlg} DolbyVision={profile.dolby_vision}")
    if profile.dolby_vision:
        print(f"  Dolby Vision:  profile={profile.dovi_profile or '?'} RPU={profile.dovi_rpu_present} EL={profile.dovi_el_present} BL={profile.dovi_bl_present} compat={profile.dovi_compatibility_id if profile.dovi_compatibility_id is not None else '?'}")
        print(f"  DV evidence:    {profile.dovi_source or 'explicit Dolby Vision signaling (details incomplete)'}")
    print(f"  Interlaced:    {profile.interlaced}")


def output_path_for(source: Path) -> Path:
    return source.with_suffix(".mkv")


def backup_path_for(source: Path, reprocess: bool) -> Path:
    old_dir = source.parent / "old"
    candidate = old_dir / source.name
    return unique_backup(candidate) if reprocess else candidate


def already_processed(source: Path) -> bool:
    return (source.parent / "old" / source.name).exists()


def storage_preflight(
    source: Path,
    *,
    duration: float,
    test_mode: bool,
    test_seconds: int,
    min_free_disk_gib: float,
    output_ratio: float,
) -> dict[str, int | float]:
    stats = disk_stats(source.parent)
    if stats is None:
        raise RebuildError(f"Cannot determine free space for {source.parent}")
    source_size = source.stat().st_size
    reserve = int(max(0.0, min_free_disk_gib) * 1024**3)
    if test_mode:
        # Budget from the source's byte rate, then multiply generously because
        # CRF test output can have a very different instantaneous bitrate.
        fraction = min(1.0, max(0.0, test_seconds / duration)) if duration > 0 else 0.05
        estimated_temp = max(512 * 1024**2, int(source_size * max(0.01, fraction) * 2.5))
        minimum_temp = 512 * 1024**2
    else:
        # The conservative budget is advisory because CRF output size is unknowable
        # before encoding. The hard start threshold is smaller; the live disk guard
        # continuously stops the temporary output before the reserve is consumed.
        estimated_temp = max(2 * 1024**3, int(source_size * output_ratio))
        minimum_temp = max(1 * 1024**3, min(8 * 1024**3, int(source_size * 0.05)))
    minimum_required = reserve + minimum_temp
    recommended_required = reserve + estimated_temp
    if stats.free_bytes < minimum_required:
        raise RebuildError(
            "Insufficient free space even to start safely: "
            f"{human_bytes(stats.free_bytes)} free, need at least {human_bytes(minimum_required)} "
            f"({human_bytes(reserve)} reserve + {human_bytes(minimum_temp)} working headroom)."
        )
    return {
        "free_bytes": stats.free_bytes,
        "total_bytes": stats.total_bytes,
        "estimated_temp_bytes": estimated_temp,
        "reserve_bytes": reserve,
        "required_bytes": minimum_required,
        "recommended_bytes": recommended_required,
        "conservative_ok": int(stats.free_bytes >= recommended_required),
    }


def _resource_log_line(path: Path) -> str:
    snap = snapshot(path)
    parts: list[str] = []
    if snap.load1 is not None:
        parts.append(f"load1={snap.load1:.2f}")
    if snap.memory.available_gib is not None:
        parts.append(f"RAM_available={snap.memory.available_gib:.2f}GiB")
    if snap.memory.swap_free_gib is not None:
        parts.append(f"swap_free={snap.memory.swap_free_gib:.2f}GiB")
    if snap.memory.swap_used_gib is not None:
        parts.append(f"swap_used={snap.memory.swap_used_gib:.2f}GiB")
    if snap.disk is not None:
        parts.append(f"disk_free={human_bytes(snap.disk.free_bytes)}")
    p = snap.pressure
    if p.cpu_some_avg10 is not None:
        parts.append(f"PSI_cpu_some10={p.cpu_some_avg10:.2f}%")
    if p.memory_full_avg10 is not None:
        parts.append(f"PSI_mem_full10={p.memory_full_avg10:.2f}%")
    if p.io_full_avg10 is not None:
        parts.append(f"PSI_io_full10={p.io_full_avg10:.2f}%")
    if snap.temperature_c is not None:
        parts.append(f"temperature={snap.temperature_c:.1f}C")
    return ", ".join(parts) if parts else "resource metrics unavailable"


def _log_failure_summary(source: Path, phase: str, exc: Exception) -> None:
    logging.error(
        "FAILURE SUMMARY\n"
        "  File: %s\n"
        "  Phase: %s\n"
        "  Exception: %s: %s\n"
        "  Resources: %s\n"
        "  Original: not committed/replaced by this failed operation",
        source, phase, type(exc).__name__, exc, _resource_log_line(source.parent),
    )


def commit_transaction(source: Path, temp_output: Path, final_output: Path, backup: Path) -> None:
    backup.parent.mkdir(parents=True, exist_ok=True)
    fsync_file(temp_output)
    fsync_dir(temp_output.parent)
    if final_output != source and final_output.exists():
        raise RebuildError(f"Final output already exists and will not be overwritten: {final_output}")
    if backup.exists():
        raise RebuildError(f"Backup path already exists and will not be overwritten: {backup}")

    source_moved = False
    final_installed = False
    try:
        os.replace(source, backup)
        source_moved = True
        fsync_dir(backup.parent)
        os.replace(temp_output, final_output)
        final_installed = True
        fsync_dir(final_output.parent)
    except Exception:
        # Before the replacement file is installed, restore the original immediately.
        # If final installation already happened and only durability syncing failed, keep
        # both the new final file and the original backup rather than destroying either.
        if source_moved and not final_installed and not source.exists() and backup.exists():
            try:
                os.replace(backup, source)
                fsync_dir(source.parent)
            except Exception:
                logging.exception("CRITICAL: automatic rollback failed; original remains at %s", backup)
        raise


def _record_memory_forecast(row: dict, plan) -> None:
    forecast = plan.memory_forecast
    if forecast is None:
        return
    row.update({
        "memory_forecast": forecast.level,
        "memory_available_gib": forecast.available_gib,
        "memory_total_gib": forecast.total_gib,
        "memory_swap_used_percent": forecast.swap_used_percent,
        "memory_budget_gib": forecast.planning_budget_gib,
        "memory_worker_cap": forecast.memory_worker_cap,
        "planned_workers": forecast.planned_workers,
        "planned_frame_threads": forecast.planned_frame_threads,
        "memory_forecast_message": " ".join(forecast.messages),
    })


def _merge_resource_summary(target: dict[str, float | None], sample: dict[str, float | None]) -> None:
    for key, value in sample.items():
        if value is None:
            continue
        current = target.get(key)
        if current is None:
            target[key] = value
        elif key == "min_memory_available_gib":
            target[key] = min(current, value)
        else:
            target[key] = max(current, value)


def process_one(tools: Toolchain, source: Path, args, csv_path: Path, number: int, total: int) -> dict:
    started = time.monotonic()
    phase = "media analysis"
    row = {"input": str(source), "status": "FAILED", "message": ""}
    temp_output: Path | None = None
    work_dir = Path(tempfile.mkdtemp(prefix="hevc-rebuild-"))
    try:
        profile = build_profile(tools, source, promote_8bit_to_10bit=args.promote_8bit_to_10bit)
        # Actual HDR10+ detection is tied to the selected video stream, not merely the first video in the container.
        actual_hdr10plus = verify_hdr10plus(tools, profile)
        if args.preserve_hdr10plus and profile.hdr10plus_hint and not actual_hdr10plus:
            codec = (profile.primary_video.codec_name or "unknown").lower()
            if codec not in {"hevc", "h265"}:
                raise RebuildError(
                    f"HDR10+ is indicated on the selected {codec} primary stream, but this preservation path "
                    "requires an HEVC source for frame-aligned metadata extraction. Refusing metadata loss."
                )
            raise RebuildError(
                "MediaInfo/ffprobe indicates HDR10+ but hdr10plus_tool cannot verify it on the selected primary stream; "
                "refusing possible metadata loss."
            )
        profile.hdr10plus = actual_hdr10plus
        content_tune = resolve_tune(profile.path, profile.mediainfo, args.tune, args.detect_animation)
        info_plan = None
        info_plan_error = None
        if args.info_only:
            print_profile(profile)
            print(f"  x265 tune:     {content_tune or 'normal'}")
            try:
                info_plan = choose_resource_plan(
                    profile,
                    threading=args.resource_threading,
                    reserved_cpu_threads=args.reserved_cpu_threads,
                    max_worker_threads=args.max_worker_threads,
                    frame_threads=args.frame_threads,
                    preset=args.preset,
                    min_free_memory_gib=args.min_free_memory_gib,
                    memory_reserve_percent=args.memory_reserve_percent,
                )
                forecast = info_plan.memory_forecast
                print(
                    f"  Memory plan:   {forecast.level.upper()} · "
                    f"{forecast.planned_workers or 'x265-auto'} worker(s), "
                    f"{forecast.planned_frame_threads or 'x265-auto'} frame thread(s) · "
                    f"{forecast.available_gib:.1f} GiB available / "
                    f"{forecast.total_gib:.1f} GiB effective total"
                    if forecast.available_gib is not None and forecast.total_gib is not None
                    else f"  Memory plan:   {forecast.level.upper()} · current memory capacity unavailable"
                )
                for message in forecast.messages:
                    print(f"                 {message}")
            except RebuildError as exc:
                info_plan_error = str(exc)
                print(f"  Memory plan:   BLOCKED · {info_plan_error}")
        row.update({
            "primary_video": profile.primary_video.codec_name,
            "resolution": f"{profile.primary_video.width}x{profile.primary_video.height}",
            "audio_count": len(profile.audio), "subtitle_count": len(profile.subtitles),
            "hdr10": profile.hdr10, "hdr10plus": profile.hdr10plus, "dolby_vision": profile.dolby_vision,
            "input_bytes": source.stat().st_size,
        })
        if info_plan is not None:
            _record_memory_forecast(row, info_plan)
        elif info_plan_error:
            row["memory_forecast"] = "blocked"
            row["memory_forecast_message"] = info_plan_error

        preserve_dolbyvision = False
        strip_dolbyvision = False
        if profile.dolby_vision:
            row["dovi_profile"] = profile.dovi_profile
            row["dovi_source"] = profile.dovi_source or "explicit signaling; source incomplete"
            if args.dolby_vision_policy == "skip":
                row["status"] = "SKIPPED"
                row["dolby_vision_action"] = "skip"
                row["message"] = "Dolby Vision detected; skipped by configured policy."
                logging.warning("%s", row["message"])
                return row
            if args.dolby_vision_policy == "hdr10-fallback":
                fallback_reasons = dolbyvision_hdr10_fallback_reasons(profile)
                if fallback_reasons:
                    raise RebuildError(
                        "Dolby Vision HDR10 fallback requested, but safety checks failed: "
                        + "; ".join(fallback_reasons)
                    )
                strip_dolbyvision = True
                row["dolby_vision_action"] = "hdr10-fallback"
                logging.warning(
                    "Dolby Vision is intentionally being discarded. FFprobe verified a Dolby Vision base layer and "
                    "the selected picture independently identifies as HDR10; the rebuilt output must validate as HDR10 "
                    "and must not retain Dolby Vision signaling."
                )
            else:
                reasons = dolbyvision_safe_preservation_reasons(
                    profile, tools, force_deinterlace=args.force_deinterlace
                )
                if reasons:
                    row["status"] = "SKIPPED"
                    row["dolby_vision_action"] = "skip-unsafe-preservation"
                    row["message"] = "Dolby Vision safe-preservation requirements not met: " + "; ".join(reasons)
                    fallback_reasons = dolbyvision_hdr10_fallback_reasons(profile)
                    if not fallback_reasons:
                        row["message"] += (
                            ". Verified HDR10 fallback is available; rerun with "
                            "--dolby-vision-policy hdr10-fallback, or choose HDR10 fallback in Guided Mode."
                        )
                    logging.warning("%s", row["message"])
                    return row
                preserve_dolbyvision = True
                row["dolby_vision_action"] = "preserve"
                logging.info("Dolby Vision Profile 8 RPU will be preserved through FFmpeg/libx265 native per-frame metadata coding.")

        if args.info_only:
            row["status"] = "INFO"
            row["message"] = "Analysis only; no changes made."
            if info_plan_error:
                row["message"] += " Memory forecast blocks encoding until available RAM recovers or the configured reserve is changed."
            return row
        phase = "storage preflight"
        storage_plan = storage_preflight(
            source, duration=profile.duration, test_mode=args.test, test_seconds=args.test_seconds,
            min_free_disk_gib=args.min_free_disk_gib, output_ratio=args.preflight_output_ratio,
        )
        logging.info(
            "Storage preflight: free=%s minimum=%s conservative_budget=%s temporary_budget=%s reserve=%s",
            human_bytes(storage_plan["free_bytes"]), human_bytes(storage_plan["required_bytes"]),
            human_bytes(storage_plan["recommended_bytes"]),
            human_bytes(storage_plan["estimated_temp_bytes"]), human_bytes(storage_plan["reserve_bytes"]),
        )
        if not storage_plan["conservative_ok"]:
            print_warning(
                f"Free space is below the conservative {human_bytes(storage_plan['recommended_bytes'])} budget, "
                "but above the safe start floor. Live disk projection will stop before the reserve is consumed."
            )
        hdr_json = None
        if profile.hdr10plus and args.preserve_hdr10plus:
            phase = "HDR10+ metadata extraction"
            hdr_json = work_dir / "hdr10plus.json"
            extract_hdr10plus(tools, profile, hdr_json)
        elif profile.hdr10plus:
            logging.warning("HDR10+ source detected, but preservation is disabled in config/CLI; dynamic metadata will not be carried into the re-encode.")

        if args.test:
            final_output = source.with_name("Test_" + source.stem + ".mkv")
            if final_output.exists():
                final_output = source.with_name(f"Test_{source.stem}_{int(time.time())}.mkv")
            temp_output = source.with_name(f".{source.stem}.hevc-rebuild.{uuid.uuid4().hex}.tmp.mkv")
        else:
            final_output = output_path_for(source)
            if final_output != source and final_output.exists():
                raise RebuildError(f"Output already exists: {final_output}")
            temp_output = source.with_name(f".{source.stem}.hevc-rebuild.{uuid.uuid4().hex}.tmp.mkv")

        encode_crf = args.test_crf if args.test else args.crf
        expected_duration = min(profile.duration, args.test_seconds) if args.test else profile.duration
        plan = choose_resource_plan(
            profile,
            threading=args.resource_threading,
            reserved_cpu_threads=args.reserved_cpu_threads,
            max_worker_threads=args.max_worker_threads,
            frame_threads=args.frame_threads,
            preset=args.preset,
            min_free_memory_gib=args.min_free_memory_gib,
            memory_reserve_percent=args.memory_reserve_percent,
        )
        demanding = is_demanding(
            profile,
            source.stat().st_size,
            large_file_gib=args.calibration_large_file_gib,
        )
        calibration_due = args.calibration_mode == "always" or (
            args.calibration_mode in {"auto", "ask"} and demanding
        )
        if calibration_due and args.resource_threading == "auto":
            current_memory = memory_stats()
            selection = None
            calibration_skip_reason = ""
            calibration_not_approved = False
            if current_memory.available_gib is None:
                calibration_skip_reason = "Available-memory telemetry is unavailable."
                print_warning(f"Hardware calibration skipped because {calibration_skip_reason.lower()} The normal plan and live guards remain in use.")
            elif current_memory.available_gib < plan.memory_guard_gib:
                calibration_skip_reason = (
                    f"Available RAM is below the configured {plan.memory_guard_gib:.1f} GiB reserve."
                )
                print_warning(f"Hardware calibration skipped. {calibration_skip_reason} The normal plan and live guards remain in use.")
            else:
                key = calibration_key(
                    tools,
                    profile,
                    plan,
                    preset=args.preset,
                    tune=content_tune,
                    crf=encode_crf,
                    total_memory_gib=current_memory.total_gib,
                    sample_seconds=args.calibration_sample_seconds,
                    max_candidates=args.calibration_max_candidates,
                    throughput_tolerance_percent=args.calibration_tolerance_percent,
                    hdr10plus_enabled=bool(hdr_json),
                    preserve_source_level=args.preserve_source_level,
                    disable_hdr10_opt=args.disable_hdr10_opt,
                    force_deinterlace=args.force_deinterlace,
                    disable_deinterlace=args.disable_deinterlace,
                    preserve_dolbyvision=preserve_dolbyvision,
                    strip_dolbyvision=strip_dolbyvision,
                )
                if args.calibration_mode != "always":
                    selection = cached_plan(key, plan, cache_hours=args.calibration_cache_hours)
                if selection is not None:
                    print_phase(
                        f"Using cached hardware calibration ({selection.age_hours:.1f}h old): "
                        f"{selection.plan.worker_threads} workers / {selection.plan.frame_threads} frame thread(s)."
                    )
                else:
                    approved = True
                    if args.calibration_mode == "ask":
                        if sys.stdin.isatty():
                            answer = input(
                                "\nQuick x265 hardware calibration is due for this demanding file. "
                                f"It compares up to {args.calibration_max_candidates} layouts using "
                                f"{args.calibration_sample_seconds:g}s of source video each. Run it? [Y/n]: "
                            ).strip().lower()
                            approved = answer not in {"n", "no"}
                        else:
                            approved = False
                        calibration_not_approved = not approved
                    if approved:
                        phase = "hardware calibration"
                        print_phase(
                            f"Calibrating x265 thread layout: up to {args.calibration_max_candidates} candidates × "
                            f"{args.calibration_sample_seconds:g}s of representative video."
                        )
                        try:
                            selection = run_calibration(
                                tools,
                                profile,
                                plan,
                                preset=args.preset,
                                tune=content_tune,
                                crf=encode_crf,
                                sample_seconds=args.calibration_sample_seconds,
                                max_candidates=args.calibration_max_candidates,
                                throughput_tolerance_percent=args.calibration_tolerance_percent,
                                hdr10plus_json=hdr_json,
                                preserve_dolbyvision=preserve_dolbyvision,
                                strip_dolbyvision=strip_dolbyvision,
                                preserve_source_level=args.preserve_source_level,
                                disable_hdr10_opt=args.disable_hdr10_opt,
                                force_deinterlace=args.force_deinterlace,
                                disable_deinterlace=args.disable_deinterlace,
                            )
                        except Exception as exc:
                            logging.warning("Hardware calibration failed; continuing with the normal resource plan: %s", exc)
                            selection = None
                        if selection is not None and selection.source == "benchmark":
                            try:
                                store_selection(key, selection)
                            except OSError as exc:
                                logging.warning("Could not cache hardware calibration: %s", exc)
                        if selection is not None:
                            for result in selection.results:
                                state = "healthy" if result.healthy else "rejected"
                                peak = f" · {result.peak_rss_gib:.2f} GiB peak RSS" if result.peak_rss_gib is not None else ""
                                speed = f" · {result.speed_x:.2f}x" if result.healthy else ""
                                note = f" · {result.reason}" if result.reason else ""
                                print_phase(
                                    f"Calibration {result.worker_threads}w/{result.frame_threads}f: {state}{speed}{peak}{note}"
                                )
                            if selection.source == "benchmark":
                                logging.info(
                                    "Hardware calibration selected workers=%s frame_threads=%s",
                                    selection.plan.worker_threads,
                                    selection.plan.frame_threads,
                                )
                            elif selection.reason:
                                print_warning(selection.reason)
                    else:
                        selection = None
                        print_phase("Hardware calibration skipped; using the normal resource plan.")
                if selection is not None:
                    plan = selection.plan
                    row["calibration_source"] = selection.source
                    row["calibration_workers"] = plan.worker_threads
                    row["calibration_frame_threads"] = plan.frame_threads
                    row["calibration_candidate_count"] = len(selection.results)
                    row["calibration_note"] = selection.reason or (
                        f"Selected by benchmark; {len(selection.results)} candidate(s) measured."
                        if selection.source == "benchmark"
                        else "Reused a cached measurement."
                    )
                elif calibration_skip_reason:
                    row["calibration_source"] = "skipped"
                    row["calibration_note"] = calibration_skip_reason
                elif calibration_not_approved:
                    row["calibration_source"] = "skipped"
                    row["calibration_note"] = (
                        "No interactive terminal was available."
                        if not sys.stdin.isatty()
                        else "Calibration was not approved."
                    )
                else:
                    row["calibration_source"] = "fallback"
                    row["calibration_note"] = "Normal resource plan retained after calibration could not run."
        _record_memory_forecast(row, plan)
        if plan.memory_forecast:
            logging.info(
                "Memory forecast: level=%s available=%sGiB total=%sGiB budget=%sGiB workers=%s frames=%s model=%s",
                plan.memory_forecast.level,
                f"{plan.memory_forecast.available_gib:.1f}" if plan.memory_forecast.available_gib is not None else "unknown",
                f"{plan.memory_forecast.total_gib:.1f}" if plan.memory_forecast.total_gib is not None else "unknown",
                f"{plan.memory_forecast.planning_budget_gib:.1f}" if plan.memory_forecast.planning_budget_gib is not None else "unknown",
                plan.worker_threads,
                plan.frame_threads,
                plan.memory_forecast.model,
            )
        print_encode_card(
            profile, number=number, total=total, crf=encode_crf, preset=args.preset, tune=content_tune,
            worker_threads=plan.worker_threads, frame_threads=plan.frame_threads, cpu_threads=plan.cpu_threads,
            mem_available_gib=plan.mem_available_gib,
            mem_total_gib=plan.memory_forecast.total_gib if plan.memory_forecast else None,
            memory_guard_gib=plan.memory_guard_gib,
            memory_forecast_level=plan.memory_forecast.level if plan.memory_forecast else "unknown",
            memory_forecast_budget_gib=plan.memory_forecast.planning_budget_gib if plan.memory_forecast else None,
            disk_free_bytes=int(storage_plan["free_bytes"]), disk_required_bytes=int(storage_plan["recommended_bytes"]),
            disk_guard_gib=args.min_free_disk_gib, test_mode=args.test,
        )

        # Start with the profile-aware memory plan. If the live guard detects
        # pressure, only the TEMPORARY encode is stopped and retried with lower
        # parallelism. CRF/preset/tune/HDR never change.
        phase = "video encode"
        max_attempts = 3
        attempt = 1
        run_resource_summary: dict[str, float | None] = {}
        while True:
            ffargs, _ = encode_args(
                tools, profile, temp_output,
                crf=encode_crf, preset=args.preset, tune=content_tune,
                hdr10plus_json=hdr_json, preserve_dolbyvision=preserve_dolbyvision, strip_dolbyvision=strip_dolbyvision,
                test_seconds=(args.test_seconds if args.test else None),
                preserve_source_level=args.preserve_source_level, disable_hdr10_opt=args.disable_hdr10_opt,
                secondary_video_policy=args.secondary_video, data_policy=args.data_streams,
                force_deinterlace=args.force_deinterlace, disable_deinterlace=args.disable_deinterlace,
                resource_threading=args.resource_threading, reserved_cpu_threads=args.reserved_cpu_threads,
                max_worker_threads=args.max_worker_threads,
                frame_threads=args.frame_threads, min_free_memory_gib=args.min_free_memory_gib,
                memory_reserve_percent=args.memory_reserve_percent, resource_plan=plan,
            )
            logging.info(
                "[%d/%d] Encoding %s (attempt %d; workers=%s frame_threads=%s)",
                number, total, source.name, attempt, plan.worker_threads, plan.frame_threads,
            )
            try:
                encode_summary = run_encode(
                    ffargs, expected_duration=expected_duration, input_size=source.stat().st_size,
                    label=f"[{number}/{total}] {source.name}", memory_guard_gib=plan.memory_guard_gib,
                    output_path=temp_output, disk_guard_gib=args.min_free_disk_gib,
                    disk_projection_margin_percent=args.disk_projection_margin_percent,
                    batch_number=number, batch_total=total,
                )
                _merge_resource_summary(run_resource_summary, encode_summary)
                row.update(run_resource_summary)
                break
            except MemoryPressureError as exc:
                _merge_resource_summary(run_resource_summary, exc.resource_summary)
                row.update(run_resource_summary)
                logging.warning("Adaptive memory protection: %s", exc)
                if exc.evidence:
                    logging.warning("%s", exc.evidence)
                next_plan = plan.reduced()
                if attempt >= max_attempts or (next_plan.worker_threads == plan.worker_threads and next_plan.frame_threads == plan.frame_threads):
                    raise RebuildError(
                        f"Memory pressure persisted after {attempt} safe encode attempt(s). "
                        "Close memory-heavy programs or manually lower max_worker_threads/frame_threads in config. "
                        "The source file was not changed."
                    ) from exc
                if temp_output.exists():
                    temp_output.unlink()
                print_phase(
                    f"Memory pressure detected; retrying with {next_plan.worker_threads} worker / "
                    f"{next_plan.frame_threads} frame thread(s). Quality settings are unchanged."
                )
                plan = next_plan
                attempt += 1

        phase = "metadata embedding"
        print_phase("Embedding and merging metadata")
        meta = apply_metadata(tools, temp_output, profile, work_dir, comment=args.comment)
        phase = "output validation"
        print_phase("Verifying video, copied streams, metadata, and HDR signaling")
        expect_poster = meta.poster_path is not None
        validation = validate_output(
            tools, profile, temp_output, secondary_video_policy=args.secondary_video,
            verify_copy_hashes=args.verify_copy_hashes, expect_poster=expect_poster, test_mode=args.test,
            expected_global_tags=meta.expected_global_tags, preserve_hdr10plus=args.preserve_hdr10plus,
            preserve_dolbyvision=preserve_dolbyvision, strip_dolbyvision=strip_dolbyvision,
        )
        if args.test:
            os.replace(temp_output, final_output)
            temp_output = None
            row["status"] = "TEST_OK"
            row["output"] = str(final_output)
            row["output_bytes"] = final_output.stat().st_size
            row["message"] = "Short test encode validated; source untouched."
            logging.info("Test output: %s", final_output)
            print_success(f"Validated test ready: {final_output.name}")
            return row

        phase = "transaction commit"
        backup = backup_path_for(source, args.reprocess)
        commit_transaction(source, temp_output, final_output, backup)
        temp_output = None
        row["status"] = "OK"
        row["output"] = str(final_output)
        row["output_bytes"] = final_output.stat().st_size
        row["message"] = f"Validated and committed; original moved to {backup}"
        logging.info("Committed %s; original preserved at %s", final_output, backup)
        print_success(f"Validated rebuild complete; original preserved in {backup.parent.name}/")
    except KeyboardInterrupt:
        row["status"] = "INTERRUPTED"
        row["message"] = "Interrupted; source was not committed/moved."
        raise
    except Exception as exc:
        row["message"] = str(exc)
        _log_failure_summary(source, phase, exc)
        logging.exception("Failed: %s", source)
        print_warning(f"File {number}/{total} failed safely during {phase}: {exc}")
        if args.stop_on_error:
            raise
    finally:
        if temp_output and temp_output.exists() and not args.keep_work:
            try:
                temp_output.unlink()
            except OSError:
                pass
        row["elapsed_seconds"] = round(time.monotonic() - started, 3)
        append_csv(csv_path, row)
        if args.keep_work:
            logging.info("Work directory retained: %s", work_dir)
        else:
            shutil.rmtree(work_dir, ignore_errors=True)
    return row

def main(argv: list[str] | None = None) -> int:
    raw_argv = list(sys.argv[1:] if argv is None else argv)
    if not raw_argv and sys.stdin.isatty():
        raw_argv = interactive_wizard()
    args = parser().parse_args(raw_argv)
    root = Path(args.root).expanduser().resolve()
    if not root.exists() or not root.is_dir():
        print(f"ERROR: Root directory does not exist: {root}", file=sys.stderr)
        return 2
    try:
        cfg, config_files = load_config(root)
        apply_to_args(args, cfg)
    except RebuildError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2
    if args.show_config:
        print(f"User config: {user_config_path()}")
        for path in config_files:
            print(f"Loaded:      {path}")
        print(f"CRF={args.crf} preset={args.preset} promote_8bit_to_10bit={args.promote_8bit_to_10bit} tune={args.tune} DolbyVision={args.dolby_vision_policy}")
        mem = memory_stats()
        effective_guard = automatic_memory_reserve_gib(mem.total_gib, floor_gib=args.min_free_memory_gib, percent=args.memory_reserve_percent)
        print(f"Resources: threading={args.resource_threading} reserved_cpu_threads={args.reserved_cpu_threads} max_worker_threads={args.max_worker_threads} frame_threads={args.frame_threads}")
        print(f"Memory: floor={args.min_free_memory_gib} GiB percent={args.memory_reserve_percent}% effective_now={effective_guard:.1f} GiB")
        print(f"Storage: reserve={args.min_free_disk_gib} GiB preflight_ratio={args.preflight_output_ratio} projection_margin={args.disk_projection_margin_percent}%")
        print(f"Calibration: mode={args.calibration_mode} cache={args.calibration_cache_hours}h large_file={args.calibration_large_file_gib}GiB sample={args.calibration_sample_seconds}s candidates={args.calibration_max_candidates} tolerance={args.calibration_tolerance_percent}%")
        return 0
    if args.force_deinterlace and args.disable_deinterlace:
        print("ERROR: --force-deinterlace and --disable-deinterlace are mutually exclusive.", file=sys.stderr)
        return 2
    if not (0 <= args.crf <= 51 and 0 <= args.test_crf <= 51):
        print("ERROR: CRF values must be between 0 and 51.", file=sys.stderr)
        return 2
    calibration_limits = (
        ("calibration cache hours", args.calibration_cache_hours, 0, 720),
        ("calibration large-file threshold", args.calibration_large_file_gib, 0, 1000),
        ("calibration sample seconds", args.calibration_sample_seconds, 1, 15),
        ("calibration throughput tolerance", args.calibration_tolerance_percent, 0, 20),
    )
    for label, value, minimum, maximum in calibration_limits:
        if not minimum <= value <= maximum:
            print(f"ERROR: {label} must be between {minimum} and {maximum}.", file=sys.stderr)
            return 2
    if not 1 <= args.calibration_max_candidates <= 4:
        print("ERROR: calibration candidate count must be from 1 to 4.", file=sys.stderr)
        return 2
    log_dir = user_log_directory()
    log_path, csv_path = configure_logging(log_dir, args.verbose)
    logging.info("HEVC Rebuild Ubuntu/Linux Mint %s", __version__)
    logging.info("Root: %s", root)
    try:
        tools = resolve_toolchain(
            assume_yes=args.assume_yes,
            skip_update_check=args.skip_update_check,
            allow_unsupported_os=args.allow_unsupported_os,
            install_missing=not args.no_dependency_repair,
            ffmpeg_path=args.ffmpeg_path,
        )
        logging.info("Toolchain: ffmpeg=%s mediainfo=%s mkvpropedit=%s hdr10plus=%s x265_dolbyvision=%s", tools.ffmpeg, tools.mediainfo, tools.mkvpropedit, tools.hdr10plus_tool, tools.x265_dolbyvision)
        if args.setup_only:
            print("All required software is installed, current (unless update checks were disabled/offline), and capability-tested.")
            return 0
        files = discover(root, recursive=args.recursive, min_size=args.min_size_mib * 1024 * 1024)
        if not files:
            logging.info("No candidate media files found.")
            print("No supported media files were found in the selected folder.")
            print(f"\nLog: {log_path}\nRun report: {csv_path}")
            return 0
        eligible: list[Path] = []
        pre_skipped = 0
        for f in files:
            if already_processed(f) and not args.reprocess:
                pre_skipped += 1
                logging.info("Skipping already-processed source (old/ copy exists): %s", f)
            else:
                eligible.append(f)
        if pre_skipped:
            print_phase(f"Skipped {pre_skipped} already-processed file(s) with an existing old/ backup")
        if not eligible:
            print("No eligible files remain after safety filtering.")
            print(f"\nLog: {log_path}\nRun report: {csv_path}")
            return 0

        run_started = time.monotonic()
        total_input = sum(f.stat().st_size for f in eligible)
        mem = memory_stats()
        disk = disk_stats(root)
        effective_guard = automatic_memory_reserve_gib(
            mem.total_gib, floor_gib=args.min_free_memory_gib, percent=args.memory_reserve_percent
        )
        mode = "CHECK ONLY" if args.info_only else ("SHORT TESTS" if args.test else "FULL REBUILD")
        print_batch_card(
            total_files=len(eligible), total_input_bytes=total_input,
            disk_free_bytes=(disk.free_bytes if disk else None), disk_total_bytes=(disk.total_bytes if disk else None),
            cpu_threads=effective_cpu_threads(), reserved_cpu_threads=args.reserved_cpu_threads,
            mem_total_gib=mem.total_gib, mem_available_gib=mem.available_gib,
            memory_guard_gib=effective_guard, disk_guard_gib=args.min_free_disk_gib, mode=mode,
        )
        logging.info(
            "Run resource baseline: %s; memory_guard=%.2fGiB disk_guard=%.2fGiB queue=%d total_input=%s",
            _resource_log_line(root), effective_guard, args.min_free_disk_gib, len(eligible), human_bytes(total_input),
        )

        results: list[dict] = []
        for i, source in enumerate(eligible, start=1):
            if args.stop_request_file and Path(args.stop_request_file).exists():
                logging.info("Stop requested by operator; remaining files will stay untouched.")
                print_warning(f"Stopped before file {i}/{len(eligible)}; remaining files were not changed.")
                break
            results.append(process_one(tools, source, args, csv_path, i, len(eligible)))

        counts = {"OK": 0, "TEST_OK": 0, "INFO": 0, "SKIPPED": pre_skipped, "FAILED": 0}
        for row in results:
            status = str(row.get("status", "FAILED"))
            counts[status] = counts.get(status, 0) + 1
        print_batch_summary(
            ok=counts.get("OK", 0), tests=counts.get("TEST_OK", 0), info=counts.get("INFO", 0),
            skipped=counts.get("SKIPPED", 0), failed=counts.get("FAILED", 0),
            elapsed=time.monotonic() - run_started,
        )
        print(f"\nLog: {log_path}\nRun report: {csv_path}")
        return 1 if counts.get("FAILED", 0) else 0
    except UserAbort as exc:
        logging.warning("%s", exc)
        return 130
    except KeyboardInterrupt:
        logging.warning("Interrupted by user. Any uncommitted source remains untouched.")
        return 130
    except RebuildError as exc:
        logging.error("%s", exc)
        return 1
    except Exception as exc:
        logging.exception("Unhandled failure")
        return 1
