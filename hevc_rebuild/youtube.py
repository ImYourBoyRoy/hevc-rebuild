from __future__ import annotations

import logging
import os
import time
import uuid
from pathlib import Path
from typing import Any, Sequence

from .discovery import candidate_file, discover, filter_by_filename_patterns, filter_by_selected_files
from .encode import MemoryPressureError, choose_resource_plan, run_encode
from .model import MediaProfile, Toolchain
from .probe import build_profile, ffprobe_json
from .resources import disk_stats, effective_cpu_threads, memory_stats
from .util import RebuildError, StopRequest, append_csv, human_bytes, native_json_action, safe_float
from .ui import print_batch_card, print_batch_summary, print_phase, print_warning


RESOLUTION_HEIGHTS = {
    "source": None,
    "1440p": 1440,
    "4k": 2160,
}
UPLOAD_QUALITY_MODES = {"balanced", "recommended"}
YOUTUBE_STEREO_AUDIO_KBPS = 384
DEFAULT_SOURCE_MULTIPLIER = 2.0
MIN_SOURCE_MULTIPLIER = 0.5
MAX_SOURCE_MULTIPLIER = 4.0


def _native_profile_payload(profile: MediaProfile) -> dict[str, object]:
    raw = next(
        (
            stream
            for stream in profile.probe.get("streams", []) or []
            if int(stream.get("index", -1)) == profile.primary_video.index
        ),
        {},
    )
    return {
        "video": {
            "width": profile.primary_video.width,
            "height": profile.primary_video.height,
            "source_bit_depth": profile.source_bit_depth,
        },
        "primary_stream_index": profile.primary_video.index,
        "primary_probe": raw,
        "tracks": [{"index": profile.primary_video.index, "ffprobe": raw}],
        "audio_count": len(profile.audio),
        "duration_seconds": profile.duration,
        "source_bit_depth": profile.source_bit_depth,
        "hdr": {
            "hdr10": profile.hdr10,
            "hdr10plus_hint": profile.hdr10plus_hint,
            "hlg": profile.hlg,
            "dolby_vision": profile.dolby_vision,
        },
        "interlaced": profile.interlaced,
        "color_primaries": profile.color_primaries,
        "transfer": profile.transfer,
        "color_matrix": profile.color_matrix,
        "color_range_x265": profile.color_range_x265,
        "color_range_ffmpeg": profile.color_range_ffmpeg,
        "master_display": profile.master_display,
        "max_cll": profile.max_cll,
        "max_fall": profile.max_fall,
    }


def _native_youtube_policy(
    profile: MediaProfile,
    resolution: str,
    quality_mode: str,
    source_multiplier: float,
    source_bytes: int | None = None,
) -> dict[str, object] | None:
    if source_bytes is None:
        try:
            source_bytes = profile.path.stat().st_size
        except OSError:
            source_bytes = 0
    result = native_json_action(
        "youtube",
        {
            "operation": "policy",
            "profile": _native_profile_payload(profile),
            "source_bytes": source_bytes,
            "duration_seconds": profile.duration,
            "resolution": resolution,
            "quality_mode": quality_mode,
            "source_multiplier": source_multiplier,
        },
    )
    if result is None:
        return None
    integer_fields = (
        "estimated_output_bytes",
        "video_bitrate_kbps",
        "recommended_video_bitrate_kbps",
        "output_width",
        "output_height",
        "source_width",
        "source_height",
        "source_bytes",
    )
    if not isinstance(result.get("color_mode"), str) or any(
        type(result.get(field)) is not int for field in integer_fields
    ):
        raise RebuildError("The Rust YouTube helper returned an invalid output policy.")
    return result


def _fps(profile: MediaProfile) -> float:
    raw_stream = next(
        (stream for stream in profile.probe.get("streams", []) or [] if int(stream.get("index", -1)) == profile.primary_video.index),
        {},
    )
    for key in ("avg_frame_rate", "r_frame_rate"):
        value = raw_stream.get(key)
        parsed = safe_float(value)
        if parsed is not None and parsed > 0:
            return parsed
    return 30.0


def _display_dimensions(profile: MediaProfile) -> tuple[int, int]:
    if result := native_json_action(
        "youtube",
        {"operation": "dimensions", "profile": _native_profile_payload(profile), "resolution": "source"},
    ):
        width, height = result.get("output_width"), result.get("output_height")
        if type(width) is int and type(height) is int:
            return width, height
        raise RebuildError("The Rust YouTube helper returned invalid source dimensions.")
    width, height = profile.primary_video.width, profile.primary_video.height
    raw_stream = next(
        (stream for stream in profile.probe.get("streams", []) or [] if int(stream.get("index", -1)) == profile.primary_video.index),
        {},
    )
    rotation: float | None = None
    for side_data in raw_stream.get("side_data_list", []) or []:
        if "display matrix" not in str(side_data.get("side_data_type", "")).lower():
            continue
        try:
            rotation = float(side_data.get("rotation"))
            break
        except (TypeError, ValueError):
            continue
    if rotation is None:
        try:
            rotation = float((raw_stream.get("tags") or {}).get("rotate"))
        except (TypeError, ValueError):
            rotation = 0.0
    normalized_rotation = abs(rotation) % 360
    if min(abs(normalized_rotation - 90), abs(normalized_rotation - 270)) <= 1:
        return height, width
    return width, height


def _expected_dimensions(source: MediaProfile, resolution: str) -> tuple[int, int]:
    if result := native_json_action(
        "youtube",
        {"operation": "dimensions", "profile": _native_profile_payload(source), "resolution": resolution},
    ):
        width, height = result.get("output_width"), result.get("output_height")
        if type(width) is int and type(height) is int:
            return width, height
        raise RebuildError("The Rust YouTube helper returned invalid output dimensions.")
    width, height = _display_dimensions(source)
    target_height = RESOLUTION_HEIGHTS[resolution]
    if not target_height:
        return width, height
    # YouTube's named tiers follow the short frame side: height for landscape,
    # width for portrait. Scale that side to the selected tier while retaining
    # the exact source aspect ratio; ultrawide output can exceed 16:9 width.
    tier_side = min(width, height)
    # Treat a selected size as an upscale target, never as a downscale limit.
    scale = max(1.0, target_height / max(tier_side, 1))
    return max(2, round(width * scale / 2) * 2), max(2, round(height * scale / 2) * 2)


def _recommended_video_rate_kbps(profile: MediaProfile, resolution: str) -> int:
    width, height = _expected_dimensions(profile, resolution)
    tier_side = min(width, height)
    hdr = _upload_color_mode(profile) in {"hdr10", "hlg"}
    high_frame_rate = _fps(profile) > 30.0
    if tier_side >= 2160:
        return 75500 if hdr and high_frame_rate else 50000 if hdr else 60500 if high_frame_rate else 45000
    if tier_side >= 1440:
        return 30000 if hdr and high_frame_rate else 20000 if hdr else 24000 if high_frame_rate else 16000
    if tier_side >= 1080:
        return 15000 if hdr and high_frame_rate else 10000 if hdr else 12000 if high_frame_rate else 8000
    if tier_side >= 720:
        return 9500 if hdr and high_frame_rate else 6500 if hdr else 7500 if high_frame_rate else 5000
    return 0 if hdr else (3750 if high_frame_rate else 2500)


def _video_rate_kbps(
    profile: MediaProfile,
    resolution: str,
    quality_mode: str = "balanced",
    source_multiplier: float = DEFAULT_SOURCE_MULTIPLIER,
) -> int:
    if safe_float(profile.duration) is None:
        raise RebuildError("YouTube source duration must be finite; forecast is unavailable.")
    if policy := _native_youtube_policy(profile, resolution, quality_mode, source_multiplier):
        return int(policy["video_bitrate_kbps"])
    recommended = _recommended_video_rate_kbps(profile, resolution)
    if quality_mode == "recommended":
        return recommended
    if quality_mode != "balanced":
        raise RebuildError("YouTube size profile must be balanced or recommended.")
    if not MIN_SOURCE_MULTIPLIER <= source_multiplier <= MAX_SOURCE_MULTIPLIER:
        raise RebuildError(
            f"YouTube source-size multiplier must be between {MIN_SOURCE_MULTIPLIER:g} and {MAX_SOURCE_MULTIPLIER:g}."
        )
    if profile.duration <= 0 or recommended <= 0:
        return recommended
    try:
        source_bytes = profile.path.stat().st_size
    except OSError:
        return recommended
    source_average_kbps = source_bytes * 8 / profile.duration / 1000
    output_audio_kbps = len(profile.audio) * YOUTUBE_STEREO_AUDIO_KBPS
    # Keep the output near the selected multiple of the source's total average bitrate,
    # then enforce YouTube's published rate as a ceiling. This avoids sending
    # low-bitrate sources through a large fixed-rate encode simply to upscale.
    source_aware_video_budget = round(source_average_kbps * source_multiplier - output_audio_kbps)
    return min(recommended, max(250, source_aware_video_budget))


def youtube_output_estimate(
    profile: MediaProfile,
    resolution: str,
    quality_mode: str = "balanced",
    source_multiplier: float = DEFAULT_SOURCE_MULTIPLIER,
) -> dict[str, int]:
    """Return a preflight estimate; actual size varies with content and rate control."""
    if safe_float(profile.duration) is None:
        raise RebuildError("YouTube source duration must be finite; forecast is unavailable.")
    if policy := _native_youtube_policy(profile, resolution, quality_mode, source_multiplier):
        return {
            key: int(policy[key])
            for key in (
                "estimated_output_bytes",
                "video_bitrate_kbps",
                "recommended_video_bitrate_kbps",
                "output_width",
                "output_height",
                "source_bytes",
            )
        }
    if resolution not in RESOLUTION_HEIGHTS:
        raise RebuildError("Upload resolution must be source, 1440p, or 4k.")
    if quality_mode not in UPLOAD_QUALITY_MODES:
        raise RebuildError("YouTube size profile must be balanced or recommended.")
    video_rate = _video_rate_kbps(profile, resolution, quality_mode, source_multiplier)
    recommended_rate = _recommended_video_rate_kbps(profile, resolution)
    audio_rate = len(profile.audio) * YOUTUBE_STEREO_AUDIO_KBPS
    dimensions = _expected_dimensions(profile, resolution)
    try:
        source_bytes = profile.path.stat().st_size
    except OSError:
        source_bytes = 0
    # Include a 12% planning margin for container/audio variability and to
    # keep the free-space preflight conservative. This is not a hard file cap.
    output_bytes = round(profile.duration * (video_rate + audio_rate) * 1000 / 8 * 1.12)
    return {
        "estimated_output_bytes": max(0, output_bytes),
        "video_bitrate_kbps": video_rate,
        "recommended_video_bitrate_kbps": recommended_rate,
        "output_width": dimensions[0],
        "output_height": dimensions[1],
        "source_bytes": source_bytes,
    }


def _upload_color_mode(profile: MediaProfile) -> str:
    """Choose SDR or a YouTube-compatible HDR path from the selected source video."""
    if result := native_json_action(
        "youtube", {"operation": "color-mode", "profile": _native_profile_payload(profile)}
    ):
        mode = result.get("color_mode")
        if isinstance(mode, str):
            return mode
        raise RebuildError("The Rust YouTube helper returned an invalid color mode.")
    transfer = (profile.transfer or "").lower()
    has_hdr_signal = bool(
        profile.hdr10 or profile.hdr10plus_hint or profile.hlg or profile.dolby_vision
        or transfer in {"smpte2084", "arib-std-b67", "smpte-st-2084", "hlg"}
        or (profile.source_bit_depth >= 10 and transfer.startswith("bt2020"))
    )
    if not has_hdr_signal:
        return "sdr"
    if profile.dolby_vision:
        return "unsupported-dolby-vision"
    if profile.source_bit_depth < 10:
        return "unsupported-bit-depth"
    if profile.color_primaries != "bt2020" or profile.color_matrix != "bt2020nc":
        return "unsupported-color-tags"
    if transfer == "smpte2084":
        return "hdr10"
    if transfer == "arib-std-b67":
        return "hlg"
    return "unsupported-transfer"


def _hdr10plus_loss_decision(profile: MediaProfile, *, accepted: bool) -> tuple[bool, str | None]:
    """Require explicit per-source consent when a YouTube transcode drops HDR10+ metadata."""
    if not profile.hdr10plus_hint:
        return True, None
    if not accepted:
        return False, (
            "HDR10+ dynamic metadata cannot be retained by this YouTube export path. "
            "No copy was written; explicitly accept this loss for the individual file to export it."
        )
    return True, (
        "HDR10+ dynamic metadata was not retained after explicit per-file approval; "
        "static HDR10/PQ signaling is retained."
    )


def _upload_args(
    tools: Toolchain,
    profile: MediaProfile,
    output: Path,
    *,
    resolution: str,
    bitrate_kbps: int,
    worker_threads: int | None,
    frame_threads: int | None,
    color_mode: str,
) -> list[str]:
    if result := native_json_action(
        "youtube",
        {
            "operation": "args",
            "profile": _native_profile_payload(profile),
            "tools": {
                "ffmpeg": tools.ffmpeg,
                "x265_pixel_formats": sorted(tools.x265_pixel_formats),
            },
            "source_path": str(profile.path),
            "output": str(output),
            "resolution": resolution,
            "video_bitrate_kbps": bitrate_kbps,
            "worker_threads": worker_threads,
            "frame_threads": frame_threads,
        },
    ):
        args = result.get("args")
        if not isinstance(args, list) or not all(isinstance(item, str) for item in args):
            raise RebuildError("The Rust YouTube helper returned invalid FFmpeg arguments.")
        if result.get("color_mode") != color_mode:
            raise RebuildError("The Rust YouTube helper detected a different source color mode.")
        return args
    args = [
        tools.ffmpeg, "-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-nostats",
        "-stats_period", "0.5", "-progress", "pipe:1", "-i", str(profile.path),
        "-map", f"0:{profile.primary_video.index}", "-map", "0:a?",
        "-map_metadata", "0", "-map_chapters", "0", "-sn", "-dn",
    ]
    target_height = RESOLUTION_HEIGHTS[resolution]
    filters: list[str] = []
    if profile.interlaced:
        filters.append("bwdif=mode=send_frame:parity=auto:deint=interlaced")
    if target_height:
        width, height = _expected_dimensions(profile, resolution)
        source_width, source_height = _display_dimensions(profile)
        if (width, height) != (source_width, source_height):
            filters.append(f"scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos")
    if filters:
        args += ["-filter:v:0", ",".join(filters)]
    if color_mode in {"hdr10", "hlg"}:
        args += [
            "-c:v:0", "libx265", "-profile:v:0", "main10", "-pix_fmt:v:0", "yuv420p10le",
            "-preset:v:0", "slow", "-tag:v:0", "hvc1",
        ]
        x265_params = [
            f"colorprim={profile.color_primaries}",
            f"transfer={profile.transfer}",
            f"colormatrix={profile.color_matrix}",
        ]
        if profile.color_range_x265:
            x265_params.append(f"range={profile.color_range_x265}")
        if color_mode == "hdr10":
            x265_params.append("hdr10=1")
            if profile.master_display:
                x265_params.append(f"master-display={profile.master_display}")
            if profile.max_cll is not None:
                x265_params.append(f"max-cll={profile.max_cll},{profile.max_fall or 0}")
        else:
            x265_params.extend(["atc-sei=18", "pic-struct=0"])
        if worker_threads:
            x265_params.append(f"pools={worker_threads}")
        if frame_threads is not None:
            x265_params.append(f"frame-threads={frame_threads}")
        args += ["-x265-params:v:0", ":".join(x265_params)]
    else:
        args += ["-c:v:0", "libx264", "-profile:v:0", "high", "-pix_fmt:v:0", "yuv420p", "-preset:v:0", "slow"]
    args += [
        "-b:v:0", f"{bitrate_kbps}k",
        "-maxrate:v:0", f"{bitrate_kbps}k", "-bufsize:v:0", f"{bitrate_kbps * 2}k",
        "-fps_mode:v:0", "passthrough", "-c:a", "aac", "-profile:a", "aac_low",
        "-b:a", "384k", "-ar", "48000", "-threads:v:0", str(max(1, worker_threads or 1)),
    ]
    for option, value in (
        ("-color_primaries:v:0", profile.color_primaries),
        ("-color_trc:v:0", profile.transfer),
        ("-colorspace:v:0", profile.color_matrix),
        ("-color_range:v:0", profile.color_range_ffmpeg),
    ):
        if value:
            args += [option, value]
    args += ["-movflags", "+faststart", "-f", "mp4", str(output)]
    return args


def _validate_upload(tools: Toolchain, source: MediaProfile, output: Path, resolution: str) -> tuple[int, int]:
    if result := native_json_action(
        "youtube",
        {
            "operation": "validate",
            "profile": _native_profile_payload(source),
            "tools": {"ffprobe": tools.ffprobe},
            "output": str(output),
            "resolution": resolution,
        },
        timeout=150.0,
    ):
        width, height = result.get("width"), result.get("height")
        if type(width) is int and type(height) is int:
            return width, height
        raise RebuildError("The Rust YouTube validator returned invalid output dimensions.")
    if not output.is_file() or output.stat().st_size < 1024:
        raise RebuildError(f"The MP4 output is missing or too small: {output}")
    probe = ffprobe_json(tools, output)
    streams = probe.get("streams", []) or []
    videos = [stream for stream in streams if stream.get("codec_type") == "video" and not int((stream.get("disposition") or {}).get("attached_pic", 0) or 0)]
    if not videos:
        raise RebuildError("The upload copy contains no usable video stream.")
    video = videos[0]
    color_mode = _upload_color_mode(source)
    expected_codec = "hevc" if color_mode in {"hdr10", "hlg"} else "h264"
    if video.get("codec_name") != expected_codec:
        raise RebuildError(f"Upload copy uses {video.get('codec_name')!r}; expected {expected_codec.upper()}.")
    width, height = int(video.get("width") or 0), int(video.get("height") or 0)
    if width <= 0 or height <= 0:
        raise RebuildError("Upload copy has invalid video dimensions.")
    expected_width, expected_height = _expected_dimensions(source, resolution)
    if abs(width - expected_width) > 2 or abs(height - expected_height) > 2:
        raise RebuildError(
            f"Upload copy dimensions {width}x{height} do not match the selected {resolution} profile; "
            f"expected about {expected_width}x{expected_height} with source aspect ratio preserved."
        )
    if color_mode in {"hdr10", "hlg"}:
        if "10" not in str(video.get("pix_fmt") or ""):
            raise RebuildError("HDR upload copy is not encoded at 10-bit or greater.")
        expected_hdr_tags = {
            "color_primaries": "bt2020",
            "color_transfer": "smpte2084" if color_mode == "hdr10" else "arib-std-b67",
            "color_space": "bt2020nc",
        }
        for key, expected in expected_hdr_tags.items():
            actual = str(video.get(key) or "").lower()
            if actual != expected:
                raise RebuildError(f"HDR upload copy has {key}={actual or 'unspecified'}; expected {expected}.")
    audio_in = [stream for stream in source.probe.get("streams", []) or [] if stream.get("codec_type") == "audio"]
    audio_out = [stream for stream in streams if stream.get("codec_type") == "audio"]
    if audio_in and (not audio_out or any(stream.get("codec_name") != "aac" for stream in audio_out)):
        raise RebuildError("The source has audio, but the upload copy did not validate as AAC audio.")
    actual_duration = safe_float((probe.get("format") or {}).get("duration"))
    if actual_duration is None or actual_duration <= 0:
        raise RebuildError("Upload copy has an invalid or unavailable duration.")
    if source.duration > 0 and abs(actual_duration - source.duration) > max(2.0, source.duration * 0.005):
        raise RebuildError("Upload copy duration differs from the source by more than the validation tolerance.")
    formats = str((probe.get("format") or {}).get("format_name") or "")
    if not any(name in formats.split(",") for name in ("mp4", "mov")):
        raise RebuildError(f"Upload copy is not identified as an MP4-family container ({formats or 'unknown'}).")
    return width, height


def _install_youtube_output(temporary: Path, destination: Path) -> None:
    """Install a validated copy without replacing a file created during encode."""
    result = native_json_action(
        "youtube",
        {"operation": "install", "temporary": str(temporary), "output": str(destination)},
    )
    if result is not None:
        if result.get("installed") is True:
            return
        raise RebuildError("The Rust YouTube installer returned an invalid response.")
    try:
        os.link(temporary, destination)
    except FileExistsError as exc:
        raise RebuildError(f"Output appeared during encoding and will not be overwritten: {destination}") from exc
    except OSError as exc:
        raise RebuildError(f"Could not install the validated copy without overwriting an existing file: {exc}") from exc
    temporary.unlink()


def youtube_upload_batch(
    tools: Toolchain,
    root: Path,
    *,
    resolution: str,
    quality_mode: str,
    output_dir: Path | None,
    recursive: bool,
    min_size_bytes: int,
    csv_path: Path,
    args: Any,
    include_name_patterns: Sequence[str] = (),
    exclude_name_patterns: Sequence[str] = (),
    selected_paths: Sequence[str] = (),
) -> int:
    """Create separate SDR or source-compatible HDR YouTube copies."""
    stop_request = getattr(args, "stop_request", None) or StopRequest(getattr(args, "stop_request_file", None))
    root = root.expanduser().resolve()
    if resolution not in RESOLUTION_HEIGHTS:
        raise RebuildError("Upload resolution must be source, 1440p, or 4k.")
    if quality_mode not in UPLOAD_QUALITY_MODES:
        raise RebuildError("YouTube size profile must be balanced or recommended.")
    if not MIN_SOURCE_MULTIPLIER <= args.upload_source_multiplier <= MAX_SOURCE_MULTIPLIER:
        raise RebuildError(
            f"YouTube source-size multiplier must be between {MIN_SOURCE_MULTIPLIER:g} and {MAX_SOURCE_MULTIPLIER:g}."
        )
    if not root.exists() or not (root.is_file() or root.is_dir()):
        raise RebuildError(f"Media input does not exist or is not a file/folder: {root}")
    if root.is_file():
        if not candidate_file(root, min_size_bytes):
            raise RebuildError(f"This file is not a supported media input or is below the size limit: {root}")
        files = filter_by_selected_files(filter_by_filename_patterns(
            [root], include=include_name_patterns, exclude=exclude_name_patterns
        ), selected_paths)
        source_base = root.parent
    else:
        files = discover(
            root,
            recursive=recursive,
            min_size=min_size_bytes,
            include_name_patterns=include_name_patterns,
            exclude_name_patterns=exclude_name_patterns,
            selected_paths=selected_paths,
        )
        source_base = root
    allowed_hdr10plus_loss = {
        str(Path(path).expanduser().resolve())
        for path in getattr(args, "allow_youtube_hdr10plus_loss_paths", set())
    }
    discovered_paths = {str(path.resolve()) for path in files}
    unknown_loss_paths = sorted(allowed_hdr10plus_loss - discovered_paths)
    if unknown_loss_paths:
        raise RebuildError(
            "YouTube HDR10+ loss consent does not belong to the selected media input: "
            + ", ".join(unknown_loss_paths[:5])
        )
    if not files:
        print("No supported media files were found in the selected input.")
        return 0

    destination_root = (output_dir or (source_base / "YouTube Uploads")).expanduser().resolve()
    if destination_root == root:
        raise RebuildError("The upload output folder cannot be the selected source path; choose a separate YouTube Uploads folder.")
    if root.is_dir() and destination_root.is_relative_to(root):
        # An in-tree export directory is supported, but only the dedicated default
        # folder is excluded by discovery; arbitrary nested destinations can recurse.
        if root.is_dir() and destination_root.name.lower() != "youtube uploads":
            raise RebuildError("When exporting inside the source folder, use a folder named 'YouTube Uploads' so later scans can safely exclude it.")
    destination_root.mkdir(parents=True, exist_ok=True)
    eligible = files
    mem = memory_stats()
    disk = disk_stats(destination_root)
    total_input = sum(path.stat().st_size for path in eligible)
    print_batch_card(
        total_files=len(eligible), total_input_bytes=total_input,
        disk_free_bytes=disk.free_bytes if disk else None,
        disk_total_bytes=disk.total_bytes if disk else None,
        cpu_threads=effective_cpu_threads(),
        reserved_cpu_threads=args.reserved_cpu_threads,
        mem_total_gib=mem.total_gib, mem_available_gib=mem.available_gib,
        memory_guard_gib=args.min_free_memory_gib, disk_guard_gib=args.min_free_disk_gib,
        mode=f"YOUTUBE EXPORT · {resolution.upper()}",
    )
    print_phase(f"Output copies will be written to {destination_root}; source files stay in place")
    results: list[dict[str, Any]] = []
    stem_counts: dict[str, int] = {}
    for path in eligible:
        relative = Path(path.name if root.is_file() else str(path.relative_to(source_base)))
        stem_counts[relative.with_suffix("").as_posix().casefold()] = stem_counts.get(relative.with_suffix("").as_posix().casefold(), 0) + 1
    run_started = time.monotonic()
    for number, source_path in enumerate(eligible, 1):
        file_started = time.monotonic()
        row: dict[str, Any] = {
            "input": str(source_path),
            "status": "FAILED",
            "message": "",
            "input_bytes": source_path.stat().st_size,
            "upload_resolution": resolution,
            "upload_quality_mode": quality_mode,
            "upload_source_multiplier": args.upload_source_multiplier,
        }
        temporary: Path | None = None
        try:
            relative = source_path.name if root.is_file() else str(source_path.relative_to(source_base))
            relative_source = Path(relative)
            if stem_counts[relative_source.with_suffix("").as_posix().casefold()] > 1:
                suffix = relative_source.suffix.removeprefix(".").lower() or "media"
                relative_path = relative_source.with_name(f"{relative_source.stem} [{suffix}].mp4")
            else:
                relative_path = relative_source.with_suffix(".mp4")
            destination = destination_root / relative_path
            if destination.resolve() == source_path.resolve():
                raise RebuildError("The output path matches the source file; choose a separate export folder.")
            row["output"] = str(destination)
            if destination.exists():
                row["status"] = "SKIPPED"
                row["message"] = "An output with this name already exists; it was left unchanged."
                print_warning(f"Skipping {source_path.name}; output already exists at {destination}")
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            profile = build_profile(tools, source_path, promote_8bit_to_10bit=False)
            row["primary_video"] = profile.primary_video.codec_name
            row["resolution"] = f"{profile.primary_video.width}x{profile.primary_video.height}"
            row["audio_count"] = len(profile.audio)
            row["subtitle_count"] = len(profile.subtitles)
            if profile.subtitles:
                row["subtitle_note"] = "Subtitle streams are not included in the YouTube MP4; upload supported caption files separately."
            native_policy = _native_youtube_policy(
                profile,
                resolution,
                quality_mode,
                args.upload_source_multiplier,
                row["input_bytes"],
            )
            color_mode = (
                str(native_policy["color_mode"])
                if native_policy is not None
                else _upload_color_mode(profile)
            )
            unsupported = {
                "unsupported-dolby-vision": "Dolby Vision requires a dedicated RPU-aware workflow; this export does not preserve Dolby Vision metadata.",
                "unsupported-bit-depth": "HDR upload requires a 10-bit or 12-bit source video stream.",
                "unsupported-color-tags": "HDR upload requires Rec. 2020 primaries and the Rec. 2020 non-constant-luminance matrix.",
                "unsupported-transfer": "HDR upload requires PQ or HLG transfer metadata; this source's HDR transfer tag is missing or unsupported.",
            }.get(color_mode)
            if unsupported:
                row["status"] = "SKIPPED"
                row["message"] = unsupported
                print_warning(f"{source_path.name}: {row['message']}")
                continue
            if color_mode in {"hdr10", "hlg"}:
                out_width, out_height = (
                    (int(native_policy["output_width"]), int(native_policy["output_height"]))
                    if native_policy is not None
                    else _expected_dimensions(profile, resolution)
                )
                if min(out_width, out_height) < 720:
                    row["status"] = "SKIPPED"
                    row["message"] = "YouTube's HDR upload guidance lists 720p and higher; this selected output would be below 720p."
                    print_warning(f"{source_path.name}: {row['message']}")
                    continue
                if "yuv420p10le" not in tools.x265_pixel_formats:
                    row["status"] = "SKIPPED"
                    row["message"] = "This FFmpeg does not expose 10-bit libx265 (yuv420p10le), which the HDR export requires."
                    print_warning(f"{source_path.name}: {row['message']}")
                    continue
                row["upload_color_mode"] = color_mode
                if profile.hdr10plus_hint:
                    can_export, loss_note = _hdr10plus_loss_decision(
                        profile,
                        accepted=str(source_path.resolve()) in allowed_hdr10plus_loss,
                    )
                    if not can_export:
                        row["status"] = "SKIPPED"
                        row["message"] = loss_note or "HDR10+ dynamic metadata loss was not approved; no output was written."
                        print_warning(f"{source_path.name}: {row['message']}")
                        continue
                    row["hdr10plus_note"] = loss_note
            else:
                row["upload_color_mode"] = "sdr"
            if native_policy is not None:
                expected_width, expected_height = (
                    int(native_policy["output_width"]),
                    int(native_policy["output_height"]),
                )
                source_width, source_height = (
                    int(native_policy["source_width"]),
                    int(native_policy["source_height"]),
                )
                estimate = {
                    key: int(native_policy[key])
                    for key in (
                        "estimated_output_bytes",
                        "video_bitrate_kbps",
                        "recommended_video_bitrate_kbps",
                        "output_width",
                        "output_height",
                        "source_bytes",
                    )
                }
            else:
                expected_width, expected_height = _expected_dimensions(profile, resolution)
                source_width, source_height = _display_dimensions(profile)
                estimate = youtube_output_estimate(
                    profile, resolution, quality_mode, args.upload_source_multiplier
                )
            if (expected_width, expected_height) != (source_width, source_height):
                print_phase(f"{source_path.name}: resizing to {expected_width}x{expected_height}; aspect ratio is preserved")
            bitrate = estimate["video_bitrate_kbps"]
            row["upload_resolution"] = resolution
            row["upload_quality_mode"] = quality_mode
            row["upload_source_multiplier"] = args.upload_source_multiplier
            row["upload_video_bitrate_kbps"] = bitrate
            row["youtube_recommended_video_bitrate_kbps"] = estimate["recommended_video_bitrate_kbps"]
            row["estimated_output_bytes"] = estimate["estimated_output_bytes"]
            print_phase(
                f"{source_path.name}: estimated upload copy {human_bytes(estimate['estimated_output_bytes'])} "
                f"at {bitrate} kb/s video; YouTube guidance is {estimate['recommended_video_bitrate_kbps']} kb/s."
            )
            output_disk = disk_stats(destination.parent)
            estimated_bytes = estimate["estimated_output_bytes"]
            required_bytes = estimated_bytes + int(args.min_free_disk_gib * 1024**3)
            if output_disk and output_disk.free_bytes < required_bytes:
                raise RebuildError(
                    f"Not enough free space for the conservative upload estimate ({human_bytes(estimated_bytes)} output plus {args.min_free_disk_gib:.1f} GiB reserve)."
                )

            plan = choose_resource_plan(
                profile, threading=args.resource_threading,
                target_dimensions=(expected_width, expected_height),
                reserved_cpu_threads=args.reserved_cpu_threads,
                max_worker_threads=args.max_worker_threads, frame_threads=0,
                preset="slow", min_free_memory_gib=args.min_free_memory_gib,
                memory_reserve_percent=args.memory_reserve_percent,
            )
            temporary = destination.with_name(f".{destination.stem}.upload-{uuid.uuid4().hex}.tmp.mp4")
            attempt = 1
            merged_resources: dict[str, Any] = {}
            while True:
                command = _upload_args(
                    tools, profile, temporary, resolution=resolution,
                    bitrate_kbps=bitrate, worker_threads=plan.worker_threads,
                    frame_threads=plan.frame_threads,
                    color_mode=color_mode,
                )
                try:
                    metrics = run_encode(
                        command, expected_duration=profile.duration, input_size=source_path.stat().st_size,
                        label=f"[{number}/{len(eligible)}] {source_path.name}",
                        memory_guard_gib=plan.memory_guard_gib, output_path=temporary,
                        disk_guard_gib=args.min_free_disk_gib,
                        disk_projection_margin_percent=args.disk_projection_margin_percent,
                        batch_number=number, batch_total=len(eligible),
                    )
                    merged_resources.update(metrics)
                    break
                except MemoryPressureError as exc:
                    merged_resources.update(exc.resource_summary)
                    next_plan = plan.reduced()
                    if attempt >= 4 or (next_plan.worker_threads == plan.worker_threads and next_plan.frame_threads == plan.frame_threads):
                        raise RebuildError("Memory pressure persisted after four safe attempts; the source remains untouched.") from exc
                    if temporary.exists():
                        temporary.unlink()
                    print_phase(f"Memory pressure detected; retrying {source_path.name} with {next_plan.worker_threads} encoder thread(s)")
                    plan = next_plan
                    attempt += 1
            row.update(merged_resources)
            codec_label = "HEVC Main 10/AAC" if color_mode in {"hdr10", "hlg"} else "H.264/AAC"
            print_phase(f"Validating {codec_label} streams, color tags, dimensions, and duration for {source_path.name}")
            output_width, output_height = _validate_upload(tools, profile, temporary, resolution)
            _install_youtube_output(temporary, destination)
            temporary = None
            row["status"] = "OK"
            row["output_bytes"] = destination.stat().st_size
            row["output_resolution"] = f"{output_width}x{output_height}"
            mode_label = "HDR10/PQ" if color_mode == "hdr10" else "HLG" if color_mode == "hlg" else "SDR"
            row["message"] = (
                f"Validated YouTube {mode_label} MP4 copy ({output_width}x{output_height}, "
                f"{bitrate} kb/s video target, {quality_mode} size profile, "
                f"{args.upload_source_multiplier:g}x source multiplier); source unchanged."
            )
            logging.info("Upload copy committed: %s", destination)
        except Exception as exc:
            row["status"] = "FAILED"
            row["message"] = f"{type(exc).__name__}: {exc}"
            logging.exception("YouTube upload preparation failed for %s", source_path)
            print_warning(f"{source_path.name}: {exc}")
        finally:
            if temporary and temporary.exists():
                temporary.unlink(missing_ok=True)
            row["elapsed_seconds"] = round(time.monotonic() - file_started, 3)
            append_csv(csv_path, row)
            results.append(row)
        if row["status"] == "FAILED" and args.stop_on_error:
            print_warning("Stop-on-error is enabled; remaining files were not started.")
            break
        if stop_request.requested():
            print_warning("Safe stop requested; remaining files were not started.")
            break

    counts: dict[str, int] = {}
    for row in results:
        status = str(row["status"])
        counts[status] = counts.get(status, 0) + 1
    print_batch_summary(
        ok=counts.get("OK", 0), tests=0, info=0,
        skipped=counts.get("SKIPPED", 0), failed=counts.get("FAILED", 0),
        elapsed=time.monotonic() - run_started,
    )
    print(f"\nOutput folder: {destination_root}")
    return 1 if counts.get("FAILED", 0) else 0
