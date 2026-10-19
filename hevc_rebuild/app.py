from __future__ import annotations

import argparse
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from urllib.parse import unquote, urlparse
from pathlib import Path

from . import __version__
from .calibration import calibration_key, cached_plan, is_demanding, run_calibration, store_selection
from .config import apply_to_args, load_config, user_config_path
from .content import resolve_run_tune
from .deps import TOOL_OVERRIDE_KEYS, resolve_toolchain, resolve_youtube_toolchain
from .discovery import discover
from .encode import MemoryPressureError, ResourcePlan, choose_resource_plan, encode_args, run_encode
from .hdr import (
    dolbyvision_hdr10_fallback_reasons, dolbyvision_safe_preservation_reasons,
    extract_hdr10plus, verify_hdr10plus,
)
from .metadata import apply_metadata
from .model import MediaProfile, Toolchain
from .probe import (
    build_native_profile_set,
    build_native_video_profiles,
    build_profile,
    choose_primary_video,
    stream_refs,
)
from .util import (
    ConsoleClosed, RebuildError, StopRequest, UserAbort, append_csv, configure_logging,
    fsync_dir, fsync_file, human_bytes, rename_no_replace, unique_backup, user_log_directory,
)
from .transactions import commit_transaction as commit_transaction, install_output_copy as install_output_copy
from .validate import validate_output
from .ui import emit_gui_progress, print_batch_card, print_batch_summary, print_encode_card, print_phase, print_success, print_warning
from .resources import automatic_memory_reserve_gib, disk_stats, effective_cpu_threads, memory_stats, snapshot


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="hevc-rebuild",
        description="Transactional HEVC/x265 media-library rebuild for Ubuntu and Linux Mint. Audio and subtitles are stream-copied and verified, never transcoded.",
    )
    p.add_argument("root", nargs="?", default=".", help="Movie/TV root directory (default: current directory)")
    p.add_argument("--youtube-upload", action="store_true", help="Create separate YouTube SDR or source-compatible HDR MP4 copies; originals remain in place")
    p.add_argument("--upload-resolution", choices=["source", "1440p", "4k"], default="1440p", help="Minimum YouTube output frame tier; smaller sources are upscaled, larger sources are never downscaled")
    p.add_argument("--upload-quality-mode", choices=["balanced", "recommended"], default="balanced", help="Balanced targets overall output bitrate near 2x the source average, subject to YouTube's video-rate recommendation; recommended targets YouTube's published rate and may create much larger files")
    p.add_argument("--upload-source-multiplier", type=float, default=2.0, help="Balanced YouTube profile target: approximate multiple of source average total bitrate (0.5–4.0); YouTube rate guidance remains the ceiling")
    p.add_argument("--upload-output-dir", default=None, help="Upload output folder (default: YouTube Uploads beside the selected file/folder)")
    p.add_argument("--allow-youtube-hdr10plus-loss", action="append", default=[], metavar="MEDIA_PATH", help="For this YouTube source only, accept dropping HDR10+ dynamic metadata; otherwise HDR10+ files are skipped")
    p.add_argument("--crf", type=int, default=None, help="Override configured x265 CRF")
    p.add_argument("--preset", default=None, choices=["ultrafast","superfast","veryfast","faster","fast","medium","slow","slower","veryslow"], help="Override configured x265 preset")
    p.add_argument("--promote-8bit-to-10bit", action=argparse.BooleanOptionalAction, default=None, help="Optionally encode 8-bit sources as 10-bit HEVC; default comes from config")
    p.add_argument("--tune", choices=["auto","animation","grain","none"], default=None, help="Override content tune from config")
    p.add_argument("--detect-animation", action=argparse.BooleanOptionalAction, default=None, help="Auto-detect Animation from reliable metadata when tune=auto")
    p.add_argument("--file-tune-overrides-json", default=None, help="JSON object mapping absolute media paths to per-file 'animation' or 'none' tune overrides")
    p.add_argument("--file-stream-overrides-json", default=None, help="JSON object mapping absolute media paths to selected FFprobe stream indices")
    p.add_argument("--file-poster-overrides-json", default=None, help="JSON object mapping absolute media paths to selected local JPEG/PNG/WebP artwork paths")
    p.add_argument("--allowed-poster-cache-root", default=None, help=argparse.SUPPRESS)
    p.add_argument("--file-metadata-overrides-json", default=None, help="JSON object mapping absolute media paths to reviewed metadata fields")
    p.add_argument("--ffprobe-path", default=None, help="Override the FFprobe executable")
    p.add_argument("--mediainfo-path", default=None, help="Override the MediaInfo executable")
    p.add_argument("--mkvpropedit-path", default=None, help="Override the MKVToolNix mkvpropedit executable")
    p.add_argument("--mkvmerge-path", default=None, help="Override the MKVToolNix mkvmerge executable")
    p.add_argument("--mkvextract-path", default=None, help="Override the MKVToolNix mkvextract executable")
    p.add_argument("--preserve-hdr10plus", action=argparse.BooleanOptionalAction, default=None, help="Preserve and verify HDR10+ dynamic metadata")
    p.add_argument("--recursive", action="store_true", help="Process all supported media recursively instead of legacy movie/Season discovery")
    p.add_argument("--min-size-mib", type=int, default=5, help="Ignore files smaller than this")
    p.add_argument("--include-filename-pattern", action="append", default=[], metavar="PATTERN", help="Include only media basenames matching at least one case-insensitive * or ? pattern")
    p.add_argument("--exclude-filename-pattern", action="append", default=[], metavar="PATTERN", help="Skip media basenames matching any case-insensitive * or ? pattern")
    p.add_argument("--selected-file", action="append", default=[], metavar="PATH", help="Limit the run to an absolute media path from the analyzed input; may be repeated")
    p.add_argument("--output-dir", default=None, help="Write rebuild copies to this existing folder, mirroring the input folder structure; sources remain in place")
    p.add_argument("--file-output-dirs-json", default=None, help="JSON object mapping absolute media paths to per-file output folders; each folder must already exist")
    p.add_argument("--secondary-video", choices=["encode","copy","drop"], default=None, help="Encode all selected videos by default; copy or drop secondaries for legacy workflows")
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
    p.add_argument("--metadata-only", action="store_true", help="Remux every source stream into a new MKV, apply reviewed metadata/artwork, and validate without video encoding")
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
    p.add_argument("--metadata-text-blacklist", action="append", default=None, metavar="PATTERN", help="Add a case-insensitive metadata substring pattern; '*' matches any sequence, e.g. 'Movie*Group'")
    p.add_argument("--replace-metadata-text-blacklist", action="store_true", help=argparse.SUPPRESS)
    p.add_argument("--embed-artwork", action=argparse.BooleanOptionalAction, default=None, help="Embed a selected local poster in the output Matroska file")
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


def canonical_output_directory(raw_path: str, label: str) -> str:
    if len(raw_path.encode("utf-8")) > 8192:
        raise RebuildError(f"{label} path must be 8192 UTF-8 bytes or fewer.")
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        raise RebuildError(f"{label} path must be absolute.")
    try:
        resolved = path.resolve(strict=True)
    except OSError as exc:
        raise RebuildError(f"{label} is unavailable: {path}") from exc
    if not resolved.is_dir():
        raise RebuildError(f"{label} is not an existing folder: {resolved}")
    return str(resolved)


def parse_file_output_directories(raw: str, root: Path) -> dict[str, str]:
    if len(raw.encode("utf-8")) > 65536:
        raise RebuildError("Per-file output folders exceed the 64 KiB limit.")
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"--file-output-dirs-json is invalid JSON: {exc}") from exc
    if not isinstance(parsed, dict) or len(parsed) > 512:
        raise RebuildError("Per-file output folders must be an object containing at most 512 media paths.")
    result: dict[str, str] = {}
    root_dir = root if root.is_dir() else root.parent
    for raw_source, raw_destination in parsed.items():
        if (
            not isinstance(raw_source, str)
            or not isinstance(raw_destination, str)
            or len(raw_source.encode("utf-8")) > 8192
        ):
            raise RebuildError("Each per-file output entry must map a media path to a folder path.")
        source_path = Path(raw_source).expanduser()
        if not source_path.is_absolute():
            raise RebuildError("Per-file output source paths must be absolute.")
        try:
            source = source_path.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"Per-file output source is unavailable: {source_path}") from exc
        if not source.is_file() or not source.is_relative_to(root_dir):
            raise RebuildError(f"Per-file output source must be an existing file inside the selected input: {source}")
        result[str(source)] = canonical_output_directory(raw_destination, "Per-file output folder")
    return result


def parse_file_poster_overrides(
    raw: str,
    root: Path,
    allowed_cache_root: str | Path | None = None,
) -> dict[str, Path]:
    if len(raw.encode("utf-8")) > 65536:
        raise RebuildError("Per-file poster selections exceed the 64 KiB limit.")
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"--file-poster-overrides-json is invalid JSON: {exc}") from exc
    if not isinstance(parsed, dict) or len(parsed) > 512:
        raise RebuildError("Per-file poster selections must be an object containing at most 512 media paths.")
    input_root = root if root.is_dir() else root.parent
    cache_root: Path | None = None
    if allowed_cache_root is not None:
        try:
            cache_root = Path(allowed_cache_root).expanduser().resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"The approved poster cache folder is unavailable: {exc}") from exc
        if not cache_root.is_dir():
            raise RebuildError("The approved poster cache root must be an existing folder.")
    result: dict[str, Path] = {}
    for raw_media, raw_poster in parsed.items():
        if (
            not isinstance(raw_media, str)
            or not isinstance(raw_poster, str)
            or len(raw_media.encode("utf-8")) > 8192
            or len(raw_poster.encode("utf-8")) > 8192
        ):
            raise RebuildError("Each poster entry must map an absolute media path to an absolute image path.")
        media_input = Path(raw_media).expanduser()
        poster_input = Path(raw_poster).expanduser()
        if not media_input.is_absolute() or not poster_input.is_absolute():
            raise RebuildError("Per-file media and poster paths must be absolute.")
        try:
            media = media_input.resolve(strict=True)
            poster = poster_input.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"A selected media file or poster image is unavailable: {exc}") from exc
        if not media.is_file() or not media.is_relative_to(input_root) or (root.is_file() and media != root):
            raise RebuildError(f"Poster selections must refer to media inside the selected input: {media}")
        if not poster.is_file() or not (
            poster.is_relative_to(input_root)
            or (cache_root is not None and poster.is_relative_to(cache_root))
        ):
            raise RebuildError("Selected poster images must be inside the selected input or the approved application artwork cache.")
        if poster.suffix.casefold() not in {".jpg", ".jpeg", ".png", ".webp"}:
            raise RebuildError("Selected poster images must use JPEG, PNG, or WebP format.")
        try:
            if poster.stat().st_size > 16 * 1024 * 1024:
                raise RebuildError("Selected poster images cannot exceed 16 MiB.")
            with poster.open("rb") as image:
                header = image.read(12)
        except OSError as exc:
            raise RebuildError(f"Could not inspect a selected poster image: {exc}") from exc
        suffix = poster.suffix.casefold()
        if suffix in {".jpg", ".jpeg"}:
            valid_header = header.startswith(b"\xff\xd8\xff")
        elif suffix == ".png":
            valid_header = header.startswith(b"\x89PNG\r\n\x1a\n")
        else:
            valid_header = len(header) >= 12 and header[:4] == b"RIFF" and header[8:12] == b"WEBP"
        if not valid_header:
            raise RebuildError("Selected poster image content does not match its JPEG, PNG, or WebP extension.")
        key = str(media)
        if key in result:
            raise RebuildError("A media file was included more than once in the poster selections.")
        result[key] = poster
    return result


def resolved_output_path(source: Path, root: Path, args) -> tuple[Path, bool]:
    """Return the output target and whether it is a separate, source-preserving copy."""
    source = source.resolve()
    per_file_dir = args.file_output_dirs.get(str(source))
    output_dir = per_file_dir or args.output_dir
    if output_dir is None:
        return output_path_for(source), False

    base = Path(output_dir)
    if per_file_dir:
        relative = Path(source.with_suffix(".mkv").name)
    elif root.is_dir():
        try:
            relative = source.relative_to(root).with_suffix(".mkv")
        except ValueError as exc:
            raise RebuildError(f"Source file is outside the selected input folder: {source}") from exc
    else:
        relative = Path(source.with_suffix(".mkv").name)

    target = base / relative
    if target.resolve(strict=False) == source:
        target = target.with_name(f"{source.stem}.hevc-rebuilt.mkv")
    resolved_target = target.resolve(strict=False)
    if not resolved_target.parent.is_relative_to(base.resolve()):
        raise RebuildError(f"Output folder resolves outside the selected destination: {target.parent}")
    return resolved_target, True


def test_output_path(output_base: Path) -> Path:
    candidate = output_base.with_name(f"Test_{output_base.stem}.mkv")
    if not candidate.exists():
        return candidate
    timestamp = int(time.time())
    for suffix in range(1000):
        tail = f"_{timestamp}" if suffix == 0 else f"_{timestamp}_{suffix}"
        candidate = output_base.with_name(f"Test_{output_base.stem}{tail}.mkv")
        if not candidate.exists():
            return candidate
    raise RebuildError(f"Could not find a free short-test output name for {output_base.name}.")


def metadata_output_path(source: Path, root: Path, args) -> Path:
    """Choose a new MKV path for metadata-only work; never resolve to the source."""
    output_base, _ = resolved_output_path(source, root, args)
    if output_base.resolve(strict=False) == source.resolve():
        return source.with_name(f"{source.stem}.metadata.mkv")
    return output_base


_METADATA_OVERRIDE_FIELDS = {
    "title", "show_title", "year", "airdate", "certification", "short_description",
    "plot", "genres", "rating", "season", "episode", "source_url", "comment",
}


def parse_file_metadata_overrides(raw: str, root: Path) -> dict[str, dict[str, object]]:
    if len(raw.encode("utf-8")) > 65536:
        raise RebuildError("Per-file reviewed metadata exceeds the 64 KiB limit.")
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"--file-metadata-overrides-json is invalid JSON: {exc}") from exc
    if not isinstance(parsed, dict) or len(parsed) > 512:
        raise RebuildError("Per-file reviewed metadata must be an object containing at most 512 media paths.")
    input_root = root if root.is_dir() else root.parent
    result: dict[str, dict[str, object]] = {}
    for raw_media, raw_fields in parsed.items():
        if not isinstance(raw_media, str) or len(raw_media.encode("utf-8")) > 8192:
            raise RebuildError("Each reviewed metadata entry must use an absolute media path no longer than 8192 bytes.")
        media_input = Path(raw_media).expanduser()
        if not media_input.is_absolute():
            raise RebuildError("Reviewed metadata media paths must be absolute.")
        try:
            media = media_input.resolve(strict=True)
        except OSError as exc:
            raise RebuildError(f"A reviewed metadata source is unavailable: {exc}") from exc
        if not media.is_file() or not media.is_relative_to(input_root) or (root.is_file() and media != root):
            raise RebuildError(f"Reviewed metadata must refer to media inside the selected input: {media}")
        if not isinstance(raw_fields, dict) or not raw_fields or len(raw_fields) > len(_METADATA_OVERRIDE_FIELDS):
            raise RebuildError("Each reviewed metadata entry must contain a non-empty object of supported fields.")
        unknown = set(raw_fields) - _METADATA_OVERRIDE_FIELDS
        if unknown:
            raise RebuildError("Unsupported reviewed metadata field(s): " + ", ".join(sorted(unknown)))
        fields: dict[str, object] = {}
        for name, value in raw_fields.items():
            if name == "genres":
                if not isinstance(value, list) or len(value) > 50 or any(not isinstance(item, str) for item in value):
                    raise RebuildError("Reviewed genres must be a list of at most 50 text values.")
                genres = [item.strip() for item in value]
                if any(len(item.encode("utf-8")) > 256 for item in genres):
                    raise RebuildError("Each reviewed genre is limited to 256 UTF-8 bytes.")
                fields[name] = genres
                continue
            if not isinstance(value, str):
                raise RebuildError(f"Reviewed metadata field {name!r} must be text.")
            limit = 8192 if name in {"plot", "short_description"} else 2048 if name == "source_url" else 500 if name == "comment" else 512
            if len(value.encode("utf-8")) > limit:
                raise RebuildError(f"Reviewed metadata field {name!r} exceeds its {limit}-byte limit.")
            normalized = value.strip()
            if name in {"season", "episode"} and normalized and (not normalized.isdecimal() or int(normalized) > 9999):
                raise RebuildError(f"Reviewed metadata field {name!r} must be a number from 0 to 9999.")
            if name == "rating" and normalized:
                try:
                    rating = float(normalized)
                except ValueError as exc:
                    raise RebuildError("Reviewed rating must be a number from 0 to 10.") from exc
                if not 0 <= rating <= 10:
                    raise RebuildError("Reviewed rating must be a number from 0 to 10.")
                normalized = str(rating)
            fields[name] = normalized
        key = str(media)
        if key in result:
            raise RebuildError("A media file was included more than once in the reviewed metadata selections.")
        result[key] = fields
    return result


def _gui_queue_percent(number: int, total: int, file_fraction: float) -> float:
    return ((number - 1) + max(0.0, min(1.0, file_fraction))) / max(1, total) * 100.0


def _gui_failure_fraction(phase: str) -> float:
    return {
        "metadata embedding": 0.90,
        "lossless output validation": 0.97,
        "output validation": 0.97,
        "validated output install": 0.99,
        "transaction commit": 0.99,
    }.get(phase, 0.0)


def _emit_file_stage(
    source: Path,
    *,
    phase: str,
    number: int,
    total: int,
    started: float,
    input_bytes: int | None,
    file_fraction: float = 0.0,
    file_percent: float | None = None,
    output_bytes: int | None = None,
    announce: bool = True,
) -> None:
    """Name the current pre-encode task in the desktop log and progress bar."""
    if announce:
        print_phase(phase)
    emit_gui_progress(
        file_name=source.name,
        file_path=str(source),
        phase=phase,
        batch_number=number,
        batch_total=total,
        file_percent=file_percent,
        queue_percent=_gui_queue_percent(number, total, file_fraction),
        elapsed_seconds=max(0.0, time.monotonic() - started),
        input_bytes=input_bytes,
        output_bytes=output_bytes,
    )


def _run_mkvmerge(tools: Toolchain, source: Path, output: Path, *, number: int, total: int) -> None:
    command = [tools.mkvmerge, "--gui-mode", "--output", str(output), str(source)]
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            bufsize=1,
        )
    except OSError as exc:
        raise RebuildError(f"Could not start MKVToolNix remux: {exc}") from exc

    stderr_tail: list[str] = []
    def drain_stderr() -> None:
        if process.stderr is None:
            return
        for line in process.stderr:
            stderr_tail.append(line[-2048:])
            if len(stderr_tail) > 100:
                del stderr_tail[0]

    stderr_thread = threading.Thread(target=drain_stderr, daemon=True)
    stderr_thread.start()
    last_percent = -1
    started = time.monotonic()
    try:
        input_bytes = source.stat().st_size
    except OSError:
        input_bytes = None
    try:
        assert process.stdout is not None
        for line in process.stdout:
            if line.startswith("#GUI#progress "):
                match = re.search(r"(\d{1,3})%", line)
                if match:
                    percent = max(0, min(100, int(match.group(1))))
                    if percent != last_percent:
                        print_phase(f"Copying streams without re-encoding · file {number}/{total} · {percent}%")
                        last_percent = percent
                    try:
                        output_bytes = max(0, output.stat().st_size)
                    except OSError:
                        output_bytes = None
                    emit_gui_progress(
                        file_name=source.name,
                        file_path=str(source),
                        phase="Copying streams",
                        batch_number=number,
                        batch_total=total,
                        file_percent=float(percent),
                        queue_percent=_gui_queue_percent(number, total, 0.85 * percent / 100.0),
                        elapsed_seconds=max(0.0, time.monotonic() - started),
                        input_bytes=input_bytes,
                        output_bytes=output_bytes,
                    )
            elif line.startswith("#GUI#"):
                logging.info("mkvmerge: %s", line.strip())
        return_code = process.wait()
        stderr_thread.join(timeout=5)
    except BaseException:
        process.kill()
        process.wait()
        stderr_thread.join(timeout=1)
        raise
    if return_code != 0:
        detail = "".join(stderr_tail).strip()
        raise RebuildError(
            f"MKVToolNix could not copy all streams (exit {return_code})."
            + (f" {detail[-4000:]}" if detail else "")
        )
    if not output.is_file() or output.stat().st_size == 0:
        raise RebuildError("MKVToolNix completed without producing a non-empty output file.")


def validate_output_destinations(files: list[Path], root: Path, args) -> None:
    planned: dict[str, Path] = {}
    for source in files:
        if getattr(args, "metadata_only", False):
            final_output = metadata_output_path(source, root, args)
            separate = True
        else:
            output_base, separate = resolved_output_path(source, root, args)
            final_output = test_output_path(output_base) if args.test else output_base
        if final_output.exists() and final_output.resolve() != source.resolve():
            raise RebuildError(f"Output already exists and will not be overwritten: {final_output}")
        if separate and final_output.exists():
            raise RebuildError(f"Output already exists and will not be overwritten: {final_output}")
        key = os.path.normcase(str(final_output.resolve(strict=False)))
        previous = planned.get(key)
        if previous is not None and previous != source:
            raise RebuildError(f"Two selected media files would write to the same output: {previous} and {source} -> {final_output}")
        planned[key] = source


def backup_path_for(source: Path, reprocess: bool) -> Path:
    old_dir = source.parent / "old"
    candidate = old_dir / source.name
    return unique_backup(candidate) if reprocess else candidate


def resource_retry_summary(plan: ResourcePlan) -> str:
    """Describe the first quality-neutral retry, or report that none is available."""
    next_plan = plan.reduced()
    if (
        next_plan.worker_threads != plan.worker_threads
        or next_plan.frame_threads != plan.frame_threads
    ):
        forecast = next_plan.memory_forecast
        risk = forecast.level.upper() if forecast else "UNKNOWN"
        return (
            f"  Pressure retry: first lower plan is {next_plan.worker_threads} worker(s) / "
            f"{next_plan.frame_threads} frame thread(s) · {risk}"
        )
    return (
        "  Pressure retry: no lower-parallelism plan is available; continued pressure "
        "will stop this file instead of repeating the same plan."
    )


def already_processed(source: Path) -> bool:
    return (source.parent / "old" / source.name).exists()


def storage_preflight(
    source: Path,
    *,
    output_directory: Path | None = None,
    duration: float,
    test_mode: bool,
    test_seconds: int,
    min_free_disk_gib: float,
    output_ratio: float,
    copy_only: bool = False,
) -> dict[str, int | float]:
    output_volume = output_directory or source.parent
    stats = disk_stats(output_volume)
    if stats is None:
        raise RebuildError(f"Cannot determine free space for output folder {output_volume}")
    source_size = source.stat().st_size
    reserve = int(max(0.0, min_free_disk_gib) * 1024**3)
    if copy_only:
        estimated_temp = max(64 * 1024**2, int(source_size * 1.05))
        minimum_temp = max(16 * 1024**2, int(source_size * 0.05))
    elif test_mode:
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


def process_metadata_only_one(
    tools: Toolchain,
    source: Path,
    root: Path,
    args,
    csv_path: Path,
    number: int,
    total: int,
) -> dict:
    """Copy every source stream to a new MKV, update metadata, then validate before install."""
    started = time.monotonic()
    phase = "media analysis"
    row = {
        "input": str(source),
        "status": "FAILED",
        "message": "",
        "metadata_text_scrub_count": 0,
        "metadata_text_scrub_details": "[]",
    }
    try:
        input_bytes = source.stat().st_size
    except OSError:
        input_bytes = None
    _emit_file_stage(
        source,
        phase="Reading media tracks",
        number=number,
        total=total,
        started=started,
        input_bytes=input_bytes,
    )
    temp_output: Path | None = None
    final_output: Path | None = None
    failure_detail: str | None = None
    work_dir = Path(tempfile.mkdtemp(prefix="hevc-metadata-only-"))
    try:
        native_profile = build_native_profile_set(
            tools,
            source,
            promote_8bit_to_10bit=False,
        )
        profile = native_profile or build_profile(tools, source, promote_8bit_to_10bit=False)
        selected_streams = stream_refs(profile.probe)
        unsupported = sorted({stream.codec_type for stream in selected_streams} - {"video", "audio", "subtitle", "attachment", "data"})
        if unsupported:
            raise RebuildError(
                "This source contains stream type(s) that cannot yet be verified for a lossless remux: "
                + ", ".join(unsupported)
                + ". The original was left untouched."
            )
        real_videos = [
            stream for stream in selected_streams
            if stream.codec_type == "video" and not stream.attached_pic and stream.width > 0 and stream.height > 0
        ]
        if not real_videos:
            raise RebuildError("No real video track was found; metadata-only MKV output requires a video stream.")
        if native_profile is not None:
            video_profiles = build_native_video_profiles(
                tools,
                profile,
                [stream.index for stream in real_videos],
                promote_8bit_to_10bit=False,
            )
        else:
            video_profiles = {
                stream.index: (
                    profile if stream.index == profile.primary_video.index else build_profile(
                        tools,
                        source,
                        promote_8bit_to_10bit=False,
                        video_stream_index=stream.index,
                        probe_data=profile.probe,
                        mediainfo_data=profile.mediainfo,
                    )
                )
                for stream in real_videos
            }
        if any(video.hdr10plus_hint for video in video_profiles.values()):
            _emit_file_stage(
                source,
                phase="Checking HDR10+ metadata",
                number=number,
                total=total,
                started=started,
                input_bytes=input_bytes,
                file_fraction=0.02,
            )
        for stream_index, video_profile in video_profiles.items():
            if video_profile.hdr10plus_hint:
                actual = verify_hdr10plus(tools, video_profile)
                if not actual:
                    raise RebuildError(
                        f"HDR10+ is indicated on video stream #{stream_index}, but it could not be verified before remux. "
                        "No output was installed."
                    )
                video_profile.hdr10plus = True

        row.update({
            "primary_video": profile.primary_video.codec_name,
            "resolution": f"{profile.primary_video.width}x{profile.primary_video.height}",
            "audio_count": sum(stream.codec_type == "audio" for stream in selected_streams),
            "subtitle_count": sum(stream.codec_type == "subtitle" for stream in selected_streams),
            "selected_stream_indices": ",".join(str(stream.index) for stream in selected_streams),
            "encoded_video_indices": "",
            "hdr10": any(video.hdr10 for video in video_profiles.values()),
            "hdr10plus": any(video.hdr10plus for video in video_profiles.values()),
            "dolby_vision": any(video.dolby_vision for video in video_profiles.values()),
            "input_bytes": source.stat().st_size,
            "content_tune": "not applicable · no video encoding",
        })

        final_output = metadata_output_path(source, root, args)
        if final_output.resolve(strict=False) == source.resolve():
            raise RebuildError("Metadata-only output must be a new file; the source path cannot be the destination.")
        if final_output.exists():
            raise RebuildError(f"Output already exists and will not be overwritten: {final_output}")
        final_output.parent.mkdir(parents=True, exist_ok=True)
        temp_output = final_output.with_name(f".{final_output.stem}.hevc-metadata.{uuid.uuid4().hex}.tmp.mkv")
        row["output"] = str(final_output)

        phase = "storage preflight"
        _emit_file_stage(
            source,
            phase="Checking free disk space",
            number=number,
            total=total,
            started=started,
            input_bytes=input_bytes,
            file_fraction=0.05,
        )
        storage_plan = storage_preflight(
            source,
            output_directory=final_output.parent,
            duration=profile.duration,
            test_mode=False,
            test_seconds=0,
            min_free_disk_gib=args.min_free_disk_gib,
            output_ratio=args.preflight_output_ratio,
            copy_only=True,
        )
        logging.info(
            "Metadata-only storage preflight: free=%s minimum=%s copy_budget=%s reserve=%s",
            human_bytes(storage_plan["free_bytes"]),
            human_bytes(storage_plan["required_bytes"]),
            human_bytes(storage_plan["estimated_temp_bytes"]),
            human_bytes(storage_plan["reserve_bytes"]),
        )
        print_phase(
            f"Metadata-only output: all {len(selected_streams)} source streams will be copied to {final_output}; no video encoder will run."
        )
        phase = "stream remux"
        _emit_file_stage(
            source,
            phase="Copying streams",
            number=number,
            total=total,
            started=started,
            input_bytes=input_bytes,
            file_fraction=0.08,
            announce=False,
        )
        _run_mkvmerge(tools, source, temp_output, number=number, total=total)

        phase = "metadata embedding"
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Embedding metadata and poster",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, 0.90),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=input_bytes,
            output_bytes=temp_output.stat().st_size if temp_output.exists() else None,
        )
        metadata = apply_metadata(
            tools,
            temp_output,
            profile,
            work_dir,
            comment=args.comment,
            text_blacklist=args.metadata_text_blacklist,
            poster_override=args.file_poster_overrides.get(str(profile.path.resolve())),
            metadata_override=args.file_metadata_overrides.get(str(profile.path.resolve())),
            embed_artwork=args.embed_artwork,
        )
        row["metadata_text_scrub_count"] = len(metadata.scrubbed_text_values)
        row["metadata_text_scrub_details"] = json.dumps(metadata.scrubbed_text_values, ensure_ascii=False)

        phase = "lossless output validation"
        print_phase("Checking every copied stream, packet payload, track flag, chapter, and HDR profile")
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Validating copied tracks and metadata",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, 0.97),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=input_bytes,
            output_bytes=temp_output.stat().st_size if temp_output.exists() else None,
        )
        validation = validate_output(
            tools,
            profile,
            temp_output,
            secondary_video_policy="copy",
            verify_copy_hashes=True,
            expect_poster=metadata.poster_path is not None,
            test_mode=False,
            expected_global_tags=metadata.expected_global_tags,
            scrubbed_text_values=metadata.scrubbed_text_values,
            chapter_text_edits=metadata.chapter_text_edits,
            preserve_hdr10plus=True,
            selected_streams=selected_streams,
            video_profiles=video_profiles,
            encode_video_indices=set(),
            copy_only=True,
        )
        row["copied_hashes_verified"] = validation.copied_hashes_verified
        if not validation.copied_hashes_verified:
            raise RebuildError("The output was not accepted because compressed source-stream payload hashes were not verified.")

        phase = "validated output install"
        install_output_copy(temp_output, final_output)
        temp_output = None
        row["status"] = "METADATA_OK"
        row["output"] = str(final_output)
        row["output_bytes"] = final_output.stat().st_size
        row["message"] = f"Metadata-only MKV validated; every source stream was copied and the original stayed in place."
        final_bytes = final_output.stat().st_size
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Validated output ready",
            batch_number=number,
            batch_total=total,
            file_percent=100.0,
            queue_percent=_gui_queue_percent(number, total, 1.0),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=input_bytes,
            output_bytes=final_bytes,
            estimated_output_bytes=final_bytes,
        )
        print_success(f"Metadata-only MKV ready: {final_output.name}")
    except KeyboardInterrupt as exc:
        row["status"] = "INTERRUPTED"
        row["message"] = "Interrupted before validated output install; source was left untouched."
        failure_detail = f"{type(exc).__name__}: {exc}".rstrip()
        raise
    except Exception as exc:
        row["message"] = str(exc)
        failure_detail = f"{type(exc).__name__}: {exc}"
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Failed safely",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, _gui_failure_fraction(phase)),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=input_bytes,
            output_bytes=temp_output.stat().st_size if temp_output and temp_output.exists() else None,
        )
        _log_failure_summary(source, phase, exc)
        logging.exception("Metadata-only output failed: %s", source)
        print_warning(f"File {number}/{total} failed safely during {phase}: {exc}")
        if args.stop_on_error:
            raise
    finally:
        _release_temporary_output(
            temp_output,
            phase,
            row,
            args.keep_work,
            final_output=final_output,
            source=source,
            failure_detail=failure_detail,
        )
        row["elapsed_seconds"] = round(time.monotonic() - started, 3)
        append_csv(csv_path, row)
        if args.keep_work:
            logging.info("Metadata work directory retained: %s", work_dir)
        else:
            shutil.rmtree(work_dir, ignore_errors=True)
    return row


_FINISHED_OUTPUT_PHASES = frozenset({
    "metadata embedding",
    "output validation",
    "lossless output validation",
    "validated output install",
    "transaction commit",
})


def _keep_finished_output(temp_output: Path, final_output: Path | None = None) -> Path:
    """Move a finished late-failure encode to a visible, marked output path.

    A completed output that failed a later validation or install step is never
    presented under the normal output name. The paired ``.error.mkv`` and
    ``.error.log`` names make the failed state obvious and are allocated
    together so an existing review artifact is never overwritten.
    """
    name = temp_output.name
    marker = ".hevc-rebuild."
    if name.startswith(".") and marker in name and name.endswith(".tmp.mkv"):
        stem = name[1:name.index(marker)]
    else:
        stem = temp_output.stem
    try:
        fsync_file(temp_output)
    except OSError as exc:
        # Keep the completed media even when a filesystem cannot confirm the
        # data flush; the adjacent diagnostic will make that boundary visible.
        logging.warning("Retained output file durability is unconfirmed: %s", exc)
    parent = final_output.parent if final_output is not None else temp_output.parent
    retained_stem = final_output.stem if final_output is not None else stem
    suffix = final_output.suffix if final_output is not None and final_output.suffix else ".mkv"
    candidates = [parent / f"{retained_stem}.error{suffix}"]
    candidates.extend(
        parent / f"{retained_stem}.error.{number}{suffix}"
        for number in range(1, 10000)
    )
    for candidate in candidates:
        if os.path.lexists(candidate) or os.path.lexists(_error_log_path(candidate)):
            continue
        try:
            rename_no_replace(temp_output, candidate)
        except FileExistsError:
            continue
        try:
            fsync_dir(candidate.parent)
        except OSError as exc:
            logging.warning("Retained output directory durability is unconfirmed: %s", exc)
        return candidate
    raise OSError(f"Unable to retain completed output without overwriting an existing file: {parent}")


def _error_log_path(final_output: Path) -> Path:
    """Return the user-facing sibling name for a failed completed output."""
    return final_output.with_suffix(".log")


def _write_retained_output_error_log(
    path: Path,
    *,
    source: Path | None,
    intended_output: Path | None,
    retained_output: Path | None,
    phase: str,
    status: str | None,
    message: str | None,
    failure_detail: str | None,
) -> Path | None:
    """Write one private, exclusive per-file diagnostic log.

    The requested unsuffixed name is used when available. An existing log is
    never overwritten; a numbered sibling preserves both diagnostic records.
    """
    if intended_output is None:
        return None
    resource_line = "resource metrics unavailable"
    if source is not None:
        try:
            resource_line = _resource_log_line(source.parent)
        except Exception as exc:  # Diagnostics must not mask output retention.
            resource_line = f"resource metrics unavailable ({type(exc).__name__}: {exc})"
    original_note = (
        "The original source was still protected because validation had not accepted this output."
        if phase in {"metadata embedding", "output validation", "lossless output validation"}
        else "Review the original and any old/backup path before retrying; this run did not report a successful replacement."
    )
    contents = "\n".join([
        "HEVC Rebuild retained-output diagnostic",
        f"Phase: {phase}",
        f"Status: {status or 'FAILED'}",
        f"Source: {source if source is not None else 'unknown'}",
        f"Intended output: {intended_output}",
        f"Retained output: {retained_output if retained_output is not None else 'not available'}",
        f"Error: {failure_detail or message or 'unknown failure'}",
        f"Message: {message or 'none'}",
        f"Resources: {resource_line}",
        "Outcome: the completed output was retained for investigation but was not accepted as a successful rebuild.",
        original_note,
        "Do not delete or retry over these files until this diagnostic and the media have been reviewed.",
        "",
    ])
    base = path
    for number in range(0, 10000):
        candidate = base if number == 0 else base.with_name(f"{base.stem}.{number}{base.suffix}")
        try:
            descriptor = os.open(
                candidate,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
                0o600,
            )
        except FileExistsError:
            continue
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                descriptor = None
                stream.write(contents)
                stream.flush()
                os.fsync(stream.fileno())
            try:
                fsync_dir(candidate.parent)
            except OSError as exc:
                logging.warning("Per-file error log durability is unconfirmed: %s", exc)
            return candidate
        except OSError as exc:
            if descriptor is not None:
                os.close(descriptor)
            logging.warning("Could not finish per-file error log %s: %s", candidate, exc)
            return None
    logging.warning("Could not allocate a per-file error log beside %s", intended_output)
    return None


def _release_temporary_output(
    temp_output: Path | None,
    phase: str,
    row: dict,
    keep_work: bool,
    *,
    final_output: Path | None = None,
    source: Path | None = None,
    failure_detail: str | None = None,
) -> None:
    """Remove unfinished work; retain completed late-failure output with a log."""
    failed_after_output = (
        phase in _FINISHED_OUTPUT_PHASES
        and row.get("status") not in {"OK", "METADATA_OK", "TEST_OK"}
    )
    if not failed_after_output:
        if temp_output is None or not temp_output.exists() or keep_work:
            return
        try:
            temp_output.unlink()
        except OSError:
            pass
        return

    retained: Path | None = None
    if temp_output is not None and temp_output.exists():
        try:
            retained = _keep_finished_output(temp_output, final_output)
        except OSError as exc:
            logging.exception("Could not move completed output to a visible retained path: %s", exc)
            retained = temp_output if temp_output.exists() else None
    elif (
        final_output is not None
        and os.path.lexists(final_output)
        and not (source is not None and final_output.resolve() == source.resolve())
    ):
        # Native install/commit can finish the move and then fail on a final
        # directory sync. Move the installed file to the same marked review
        # name used for a completed temporary output.
        try:
            retained = _keep_finished_output(final_output, final_output)
        except OSError as exc:
            logging.exception("Could not mark the installed output as a retained error artifact: %s", exc)
            retained = final_output if final_output.exists() else None

    if retained is not None and retained.exists():
        row["output"] = str(retained)
        try:
            row["output_bytes"] = retained.stat().st_size
        except OSError:
            pass
    intended_output = final_output or retained
    log_output = retained if retained is not None and retained != temp_output else intended_output
    error_log = (
        _write_retained_output_error_log(
            _error_log_path(log_output),
            source=source,
            intended_output=intended_output,
            retained_output=retained,
            phase=phase,
            status=row.get("status"),
            message=row.get("message"),
            failure_detail=failure_detail,
        )
        if intended_output is not None
        else None
    )
    if retained is not None:
        note = f" The completed output was retained at {retained}, but remains unvalidated."
    else:
        note = " The completed output could not be moved to a visible retained path."
    if error_log is not None:
        note += f" Details were written to {error_log}."
    else:
        note += " See the run diagnostics for the failure details."
    row["message"] = f"{row.get('message') or ''}".rstrip() + note
    logging.warning("Completed output handling after %s failed: retained=%s error_log=%s", phase, retained, error_log)


def _select_file_streams(profile: MediaProfile, args) -> tuple[list, set[int], bool]:
    """Resolve the per-file stream contract; GUI/CLI overrides are absolute FFprobe indices."""
    refs = stream_refs(profile.probe)
    by_index = {stream.index: stream for stream in refs}
    path_key = str(profile.path.resolve())
    overrides = getattr(args, "file_stream_overrides", {}) or {}
    explicit = path_key in overrides
    if explicit:
        indices = overrides[path_key]
        unknown = [index for index in indices if index not in by_index]
        if unknown:
            raise RebuildError(f"Selected stream index(es) are not present in {profile.path.name}: {unknown[:8]}")
        selected = [by_index[index] for index in indices]
    else:
        highest, secondary = choose_primary_video(refs)
        videos = [highest] if args.secondary_video == "drop" else [highest, *secondary]
        selected = [
            *videos,
            *[stream for stream in refs if stream.codec_type in {"audio", "subtitle", "attachment"}],
        ]
        if args.data_streams == "preserve":
            selected.extend(stream for stream in refs if stream.codec_type == "data")

    real_videos = [stream for stream in selected if stream.codec_type == "video" and not stream.attached_pic and stream.width > 0 and stream.height > 0]
    if not real_videos:
        raise RebuildError("Select at least one real video track for this media file.")
    selected.sort(key=lambda stream: (
        {"video": 0, "audio": 1, "subtitle": 2, "attachment": 3, "data": 4}.get(stream.codec_type, 5),
        int(stream.attached_pic),
        -stream.pixels if stream.codec_type == "video" else stream.index,
        stream.index,
    ))
    if explicit:
        encoded = {stream.index for stream in real_videos}
    elif args.secondary_video == "encode":
        encoded = {stream.index for stream in real_videos}
    else:
        encoded = {max(real_videos, key=lambda stream: (stream.pixels, int(stream.default), -stream.index)).index}
    return selected, encoded, explicit


def _hdr10plus_helper_available(tools: Toolchain) -> bool:
    path = Path(tools.hdr10plus_tool).expanduser()
    return bool(shutil.which(tools.hdr10plus_tool) or (path.is_file() and os.access(path, os.X_OK)))


def process_one(tools: Toolchain, source: Path, root: Path, args, csv_path: Path, number: int, total: int) -> dict:
    started = time.monotonic()
    phase = "media analysis"
    row = {
        "input": str(source),
        "status": "FAILED",
        "message": "",
        "metadata_text_scrub_count": 0,
        "metadata_text_scrub_details": "[]",
    }
    try:
        source_size = source.stat().st_size
    except OSError:
        source_size = None
    _emit_file_stage(
        source,
        phase="Reading media tracks",
        number=number,
        total=total,
        started=started,
        input_bytes=source_size,
    )
    temp_output: Path | None = None
    final_output: Path | None = None
    failure_detail: str | None = None
    work_dir = Path(tempfile.mkdtemp(prefix="hevc-rebuild-"))
    try:
        native_profile = build_native_profile_set(
            tools,
            source,
            promote_8bit_to_10bit=args.promote_8bit_to_10bit,
        )
        if native_profile is None:
            profile = build_profile(tools, source, promote_8bit_to_10bit=args.promote_8bit_to_10bit)
            native_video_profiles: dict[int, MediaProfile] = {}
        else:
            profile = native_profile
        selected_streams, encode_video_indices, explicit_stream_selection = _select_file_streams(profile, args)
        selected_real_videos = [stream for stream in selected_streams if stream.codec_type == "video" and not stream.attached_pic and stream.width > 0 and stream.height > 0]
        selected_primary = selected_real_videos[0]
        if native_profile is not None:
            native_video_profiles = build_native_video_profiles(
                tools,
                profile,
                [stream.index for stream in selected_real_videos],
                promote_8bit_to_10bit=args.promote_8bit_to_10bit,
            )
        if profile.primary_video.index != selected_primary.index:
            if native_video_profiles:
                try:
                    profile = native_video_profiles[selected_primary.index]
                except KeyError as exc:
                    raise RebuildError(
                        f"The Rust source profiler omitted selected video stream #{selected_primary.index}."
                    ) from exc
            else:
                profile = build_profile(
                    tools,
                    source,
                    promote_8bit_to_10bit=args.promote_8bit_to_10bit,
                    video_stream_index=selected_primary.index,
                    probe_data=profile.probe,
                    mediainfo_data=profile.mediainfo,
                )
        video_profiles: dict[int, MediaProfile] = {}
        hdr10plus_verification_skipped = False
        hdr10plus_scan_announced = False
        saw_hdr10plus_hint = False
        for stream in selected_real_videos:
            if native_video_profiles:
                try:
                    track_profile = native_video_profiles[stream.index]
                except KeyError as exc:
                    raise RebuildError(
                        f"The Rust source profiler omitted selected video stream #{stream.index}."
                    ) from exc
            else:
                track_profile = profile if stream.index == profile.primary_video.index else build_profile(
                    tools,
                    source,
                    promote_8bit_to_10bit=args.promote_8bit_to_10bit,
                    video_stream_index=stream.index,
                    probe_data=profile.probe,
                    mediainfo_data=profile.mediainfo,
                )
            if stream.index in encode_video_indices:
                if not track_profile.hdr10plus_hint:
                    actual_hdr10plus = False
                elif (
                    args.info_only
                    and (track_profile.primary_video.codec_name or "").lower() in {"hevc", "h265"}
                    and not _hdr10plus_helper_available(tools)
                ):
                    saw_hdr10plus_hint = True
                    actual_hdr10plus = False
                    hdr10plus_verification_skipped = True
                else:
                    saw_hdr10plus_hint = True
                    if not hdr10plus_scan_announced:
                        _emit_file_stage(
                            source,
                            phase="Checking HDR10+ metadata",
                            number=number,
                            total=total,
                            started=started,
                            input_bytes=source_size,
                            file_fraction=0.02,
                        )
                        hdr10plus_scan_announced = True
                    actual_hdr10plus = verify_hdr10plus(tools, track_profile)
                if (
                    not args.info_only
                    and args.preserve_hdr10plus
                    and track_profile.hdr10plus_hint
                    and not actual_hdr10plus
                ):
                    codec = (track_profile.primary_video.codec_name or "unknown").lower()
                    if codec not in {"hevc", "h265"}:
                        raise RebuildError(
                            f"HDR10+ is indicated on selected {stream.label}, but its {codec} codec cannot use the frame-aligned preservation path."
                        )
                    raise RebuildError(
                        f"HDR10+ is indicated on selected {stream.label}, but hdr10plus_tool could not verify it; refusing possible metadata loss."
                    )
                track_profile.hdr10plus = actual_hdr10plus
            video_profiles[stream.index] = track_profile
        if not saw_hdr10plus_hint:
            print_phase("No HDR10+ signaling, so the full metadata scan was skipped.")
        hdr10plus_jsons: dict[int, Path] = {}
        any_hdr10plus = any(item.hdr10plus for item in video_profiles.values())
        per_file_tune = (getattr(args, "file_tune_overrides", {}) or {}).get(str(profile.path.resolve()))
        content_tune = resolve_run_tune(
            profile.path,
            profile.mediainfo,
            args.tune,
            args.detect_animation,
            per_file_tune,
        )
        row["content_tune"] = content_tune or "normal"
        info_plan = None
        info_plan_error = None
        if args.info_only:
            print_profile(profile)
            print("  Selected tracks: " + ", ".join(f"#{stream.index} {stream.codec_type}" for stream in selected_streams))
            print("  Video encodes:  " + ", ".join(f"#{index}" for index in sorted(encode_video_indices)))
            print(f"  x265 tune:     {content_tune or 'normal'}")
            try:
                info_plan = choose_resource_plan(
                    max(video_profiles.values(), key=lambda item: (item.primary_video.pixels, item.source_bit_depth)),
                    threading=args.resource_threading,
                    reserved_cpu_threads=args.reserved_cpu_threads,
                    max_worker_threads=args.max_worker_threads,
                    frame_threads=args.frame_threads,
                    preset=args.preset,
                    min_free_memory_gib=args.min_free_memory_gib,
                    memory_reserve_percent=args.memory_reserve_percent,
                    concurrent_video_streams=len(encode_video_indices),
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
                print(resource_retry_summary(info_plan))
            except RebuildError as exc:
                info_plan_error = str(exc)
                print(f"  Memory plan:   BLOCKED · {info_plan_error}")
        row.update({
            "primary_video": profile.primary_video.codec_name,
            "resolution": f"{profile.primary_video.width}x{profile.primary_video.height}",
            "audio_count": sum(stream.codec_type == "audio" for stream in selected_streams),
            "subtitle_count": sum(stream.codec_type == "subtitle" for stream in selected_streams),
            "selected_stream_indices": ",".join(str(stream.index) for stream in selected_streams),
            "encoded_video_indices": ",".join(str(index) for index in sorted(encode_video_indices)),
            "hdr10": any(item.hdr10 for item in video_profiles.values()), "hdr10plus": any_hdr10plus,
            "dolby_vision": any(item.dolby_vision for item in video_profiles.values()),
            "input_bytes": source.stat().st_size,
        })
        if info_plan is not None:
            _record_memory_forecast(row, info_plan)
        elif info_plan_error:
            row["memory_forecast"] = "blocked"
            row["memory_forecast_message"] = info_plan_error

        preserve_dolbyvision_indices: set[int] = set()
        strip_dolbyvision_indices: set[int] = set()
        dovi_profiles = [(index, item) for index, item in video_profiles.items() if item.dolby_vision]
        if dovi_profiles:
            _emit_file_stage(
                source,
                phase="Checking Dolby Vision",
                number=number,
                total=total,
                started=started,
                input_bytes=source_size,
                file_fraction=0.04,
            )
            row["dovi_profile"] = ",".join(str(item.dovi_profile or "unknown") for _, item in dovi_profiles)
            row["dovi_source"] = "; ".join(item.dovi_source or "explicit signaling; source incomplete" for _, item in dovi_profiles)
            if args.dolby_vision_policy == "skip":
                row["status"] = "SKIPPED"
                row["dolby_vision_action"] = "skip"
                row["message"] = "A selected video track contains Dolby Vision; skipped by configured policy."
                emit_gui_progress(
                    file_name=source.name,
                    file_path=str(source),
                    phase="Skipped by Dolby Vision policy",
                    batch_number=number,
                    batch_total=total,
                    file_percent=100.0,
                    queue_percent=_gui_queue_percent(number, total, 1.0),
                    elapsed_seconds=time.monotonic() - started,
                    input_bytes=source_size,
                )
                logging.warning("%s", row["message"])
                return row
            for index, track_profile in dovi_profiles:
                if args.dolby_vision_policy == "hdr10-fallback":
                    fallback_reasons = dolbyvision_hdr10_fallback_reasons(track_profile)
                    if fallback_reasons:
                        raise RebuildError(
                            f"Dolby Vision HDR10 fallback is unsafe for source stream #{index}: "
                            + "; ".join(fallback_reasons)
                        )
                    strip_dolbyvision_indices.add(index)
                else:
                    reasons = dolbyvision_safe_preservation_reasons(
                        track_profile, tools, force_deinterlace=args.force_deinterlace
                    )
                    if reasons:
                        row["status"] = "SKIPPED"
                        row["dolby_vision_action"] = "skip-unsafe-preservation"
                        row["message"] = f"Dolby Vision safe-preservation requirements not met on source stream #{index}: " + "; ".join(reasons)
                        if not dolbyvision_hdr10_fallback_reasons(track_profile):
                            row["message"] += ". A verified HDR10 fallback is available using --dolby-vision-policy hdr10-fallback."
                        emit_gui_progress(
                            file_name=source.name,
                            file_path=str(source),
                            phase="Skipped: Dolby Vision cannot be safely preserved",
                            batch_number=number,
                            batch_total=total,
                            file_percent=100.0,
                            queue_percent=_gui_queue_percent(number, total, 1.0),
                            elapsed_seconds=time.monotonic() - started,
                            input_bytes=source_size,
                        )
                        logging.warning("%s", row["message"])
                        return row
                    preserve_dolbyvision_indices.add(index)
            if strip_dolbyvision_indices:
                row["dolby_vision_action"] = "hdr10-fallback"
                logging.warning("Dolby Vision is intentionally discarded only on the selected track(s) with independently verified HDR10 base layers.")
            elif preserve_dolbyvision_indices:
                row["dolby_vision_action"] = "preserve"
                logging.info("Verified Dolby Vision RPU will be preserved on each eligible selected video track.")
        preserve_dolbyvision = bool(preserve_dolbyvision_indices)
        strip_dolbyvision = bool(strip_dolbyvision_indices)

        if args.info_only:
            row["status"] = "INFO"
            row["message"] = "Analysis only; no changes made."
            emit_gui_progress(
                file_name=source.name,
                file_path=str(source),
                phase="Analysis complete",
                batch_number=number,
                batch_total=total,
                file_percent=100.0,
                queue_percent=_gui_queue_percent(number, total, 1.0),
                elapsed_seconds=time.monotonic() - started,
                input_bytes=source_size,
            )
            if hdr10plus_verification_skipped:
                row["message"] += " HDR10+ dynamic metadata was not checked because hdr10plus_tool is missing; install the helper before rebuilding HEVC tracks."
            if info_plan_error:
                row["message"] += " Memory forecast blocks encoding until available RAM recovers or the configured reserve is changed."
            return row
        output_base, separate_output = resolved_output_path(source, root, args)
        final_output = test_output_path(output_base) if args.test else output_base
        if final_output != source and final_output.exists():
            raise RebuildError(f"Output already exists and will not be overwritten: {final_output}")
        final_output.parent.mkdir(parents=True, exist_ok=True)
        if separate_output and not final_output.parent.resolve().is_relative_to(
            Path(args.file_output_dirs.get(str(source.resolve())) or args.output_dir).resolve()
        ):
            raise RebuildError(f"Output folder resolves outside the selected destination: {final_output.parent}")
        temp_output = final_output.with_name(f".{final_output.stem}.hevc-rebuild.{uuid.uuid4().hex}.tmp.mkv")
        row["output"] = str(final_output)
        if separate_output:
            print_phase(f"Validated output will be written to {final_output.parent}; source stays in place")
        phase = "storage preflight"
        _emit_file_stage(
            source,
            phase="Checking free disk space",
            number=number,
            total=total,
            started=started,
            input_bytes=source_size,
            file_fraction=0.05,
        )
        storage_plan = storage_preflight(
            source, output_directory=final_output.parent, duration=profile.duration, test_mode=args.test, test_seconds=args.test_seconds,
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
        if any_hdr10plus and args.preserve_hdr10plus:
            phase = "HDR10+ metadata extraction"
            _emit_file_stage(
                source,
                phase="Extracting HDR10+ metadata",
                number=number,
                total=total,
                started=started,
                input_bytes=source_size,
                file_fraction=0.06,
            )
            for index, track_profile in video_profiles.items():
                if track_profile.hdr10plus:
                    metadata_json = work_dir / f"hdr10plus-stream-{index}.json"
                    extract_hdr10plus(tools, track_profile, metadata_json)
                    hdr10plus_jsons[index] = metadata_json
        elif any_hdr10plus:
            logging.warning("HDR10+ source detected, but preservation is disabled in config/CLI; selected video tracks will not carry dynamic metadata into the re-encode.")

        encode_crf = args.test_crf if args.test else args.crf
        expected_duration = min(profile.duration, args.test_seconds) if args.test else profile.duration
        planning_profile = max(video_profiles.values(), key=lambda item: (item.primary_video.pixels, item.source_bit_depth))
        _emit_file_stage(
            source,
            phase="Planning encoder threads and memory",
            number=number,
            total=total,
            started=started,
            input_bytes=source_size,
            file_fraction=0.07,
        )
        plan = choose_resource_plan(
            planning_profile,
            threading=args.resource_threading,
            reserved_cpu_threads=args.reserved_cpu_threads,
            max_worker_threads=args.max_worker_threads,
            frame_threads=args.frame_threads,
            preset=args.preset,
            min_free_memory_gib=args.min_free_memory_gib,
            memory_reserve_percent=args.memory_reserve_percent,
            concurrent_video_streams=len(encode_video_indices),
        )
        demanding = is_demanding(
            profile,
            source.stat().st_size,
            large_file_gib=args.calibration_large_file_gib,
        )
        calibration_due = args.calibration_mode == "always" or (
            args.calibration_mode in {"auto", "ask"} and demanding
        )
        primary_hdr_json = hdr10plus_jsons.get(profile.primary_video.index)
        if calibration_due and args.resource_threading == "auto" and len(encode_video_indices) > 1:
            row["calibration_source"] = "skipped"
            row["calibration_note"] = "A multi-video job is planned as a concurrent aggregate; single-stream calibration would not represent its memory use."
            print_warning("Hardware calibration skipped for multiple selected video encoders; the aggregate memory plan and live pressure guards remain active.")
        elif calibration_due and args.resource_threading == "auto":
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
                    hdr10plus_enabled=bool(primary_hdr_json),
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
                        _emit_file_stage(
                            source,
                            phase="Calibrating the encoder",
                            number=number,
                            total=total,
                            started=started,
                            input_bytes=source_size,
                            file_fraction=0.08,
                            announce=False,
                        )
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
                                hdr10plus_json=primary_hdr_json,
                                preserve_dolbyvision=preserve_dolbyvision,
                                strip_dolbyvision=strip_dolbyvision,
                                preserve_source_level=args.preserve_source_level,
                                disable_hdr10_opt=args.disable_hdr10_opt,
                                force_deinterlace=args.force_deinterlace,
                                disable_deinterlace=args.disable_deinterlace,
                                cache_key=key,
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
        _emit_file_stage(
            source,
            phase="Starting the video encode",
            number=number,
            total=total,
            started=started,
            input_bytes=source_size,
            file_fraction=0.08,
        )
        max_attempts = 3
        attempt = 1
        run_resource_summary: dict[str, float | None] = {}
        while True:
            ffargs, _ = encode_args(
                tools, profile, temp_output,
                crf=encode_crf, preset=args.preset, tune=content_tune,
                hdr10plus_json=primary_hdr_json, preserve_dolbyvision=preserve_dolbyvision, strip_dolbyvision=strip_dolbyvision,
                test_seconds=(args.test_seconds if args.test else None),
                preserve_source_level=args.preserve_source_level, disable_hdr10_opt=args.disable_hdr10_opt,
                secondary_video_policy=args.secondary_video, data_policy=args.data_streams,
                force_deinterlace=args.force_deinterlace, disable_deinterlace=args.disable_deinterlace,
                resource_threading=args.resource_threading, reserved_cpu_threads=args.reserved_cpu_threads,
                max_worker_threads=args.max_worker_threads,
                frame_threads=args.frame_threads, min_free_memory_gib=args.min_free_memory_gib,
                memory_reserve_percent=args.memory_reserve_percent, resource_plan=plan,
                selected_streams=selected_streams, video_profiles=video_profiles,
                encode_video_indices=encode_video_indices, hdr10plus_jsons=hdr10plus_jsons,
                preserve_dolbyvision_indices=preserve_dolbyvision_indices,
                strip_dolbyvision_indices=strip_dolbyvision_indices,
            )
            logging.info(
                "[%d/%d] Encoding %s (attempt %d; workers=%s frame_threads=%s)",
                number, total, source.name, attempt, plan.worker_threads, plan.frame_threads,
            )
            try:
                encode_summary = run_encode(
                    ffargs, expected_duration=expected_duration, input_size=source.stat().st_size,
                    label=f"[{number}/{total}] {source.name}", memory_guard_gib=plan.memory_guard_gib,
                    file_path=str(source),
                    elapsed_seconds_before=max(0.0, time.monotonic() - started),
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
                _emit_file_stage(
                    source,
                    phase="Retrying the encode with fewer threads",
                    number=number,
                    total=total,
                    started=started,
                    input_bytes=source_size,
                    file_fraction=0.08,
                    announce=False,
                )
                print_phase(
                    f"Memory pressure detected; retrying with {next_plan.worker_threads} worker / "
                    f"{next_plan.frame_threads} frame thread(s). Quality settings are unchanged."
                )
                plan = next_plan
                attempt += 1

        phase = "metadata embedding"
        print_phase("Embedding and merging metadata")
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Embedding metadata and poster",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, 0.90),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=source_size,
            output_bytes=temp_output.stat().st_size if temp_output and temp_output.exists() else None,
        )
        meta = apply_metadata(
            tools,
            temp_output,
            profile,
            work_dir,
            comment=args.comment,
            text_blacklist=args.metadata_text_blacklist,
            poster_override=args.file_poster_overrides.get(str(profile.path.resolve())),
            metadata_override=args.file_metadata_overrides.get(str(profile.path.resolve())),
            embed_artwork=args.embed_artwork,
        )
        row["metadata_text_scrub_count"] = len(meta.scrubbed_text_values)
        row["metadata_text_scrub_details"] = json.dumps(meta.scrubbed_text_values, ensure_ascii=False)
        for change in meta.scrubbed_text_values:
            logging.info(
                "Cleaned metadata text [%s / %s]: %r -> %r (rules: %s)",
                change.get("scope", "metadata"),
                change.get("field", "text"),
                change.get("before", ""),
                change.get("after", ""),
                ", ".join(change.get("rules", [])),
            )
        scrub_note = (
            f" Metadata text cleaned in {len(meta.scrubbed_text_values)} field(s)."
            if meta.scrubbed_text_values else ""
        )
        phase = "output validation"
        print_phase("Verifying video, copied streams, metadata, and HDR signaling")
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Validating video, streams, metadata, and HDR",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, 0.97),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=source_size,
            output_bytes=temp_output.stat().st_size if temp_output and temp_output.exists() else None,
        )
        expect_poster = meta.poster_path is not None
        validate_output(
            tools, profile, temp_output, secondary_video_policy=args.secondary_video,
            verify_copy_hashes=args.verify_copy_hashes, expect_poster=expect_poster, test_mode=args.test,
            expected_global_tags=meta.expected_global_tags, preserve_hdr10plus=args.preserve_hdr10plus,
            scrubbed_text_values=meta.scrubbed_text_values,
            chapter_text_edits=meta.chapter_text_edits,
            preserve_dolbyvision=preserve_dolbyvision, strip_dolbyvision=strip_dolbyvision,
            preserve_dolbyvision_indices=preserve_dolbyvision_indices,
            strip_dolbyvision_indices=strip_dolbyvision_indices,
        )
        if args.test:
            install_output_copy(temp_output, final_output)
            temp_output = None
            row["status"] = "TEST_OK"
            row["output"] = str(final_output)
            row["output_bytes"] = final_output.stat().st_size
            row["message"] = f"Short test encode validated; source untouched.{scrub_note}"
            test_bytes = row["output_bytes"]
            emit_gui_progress(
                file_name=source.name,
                file_path=str(source),
                phase="Short test validated",
                batch_number=number,
                batch_total=total,
                file_percent=100.0,
                queue_percent=_gui_queue_percent(number, total, 1.0),
                elapsed_seconds=time.monotonic() - started,
                input_bytes=source_size,
                output_bytes=test_bytes,
                estimated_output_bytes=test_bytes,
            )
            logging.info("Test output: %s", final_output)
            print_success(f"Validated test ready: {final_output.name}")
            return row

        phase = "transaction commit"
        if separate_output:
            install_output_copy(temp_output, final_output)
            backup = None
        else:
            backup = backup_path_for(source, args.reprocess)
            commit_transaction(source, temp_output, final_output, backup)
        temp_output = None
        row["status"] = "OK"
        row["output"] = str(final_output)
        row["output_bytes"] = final_output.stat().st_size
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Validated output installed",
            batch_number=number,
            batch_total=total,
            file_percent=100.0,
            queue_percent=_gui_queue_percent(number, total, 1.0),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=source_size,
            output_bytes=row["output_bytes"],
            estimated_output_bytes=row["output_bytes"],
        )
        if separate_output:
            row["message"] = f"Validated copy saved; original left in place at {source}.{scrub_note}"
            logging.info("Validated copy %s; original left in place at %s", final_output, source)
            print_success(f"Validated copy saved: {final_output}")
        else:
            row["message"] = f"Validated and committed; original moved to {backup}.{scrub_note}"
            logging.info("Committed %s; original preserved at %s", final_output, backup)
            print_success(f"Validated rebuild complete; original preserved in {backup.parent.name}/")
    except ConsoleClosed as exc:
        row["status"] = "INTERRUPTED"
        row["message"] = str(exc)
        failure_detail = f"{type(exc).__name__}: {exc}"
        logging.warning("Encode stopped because the desktop progress pipe closed: %s", source)
        raise
    except KeyboardInterrupt as exc:
        row["status"] = "INTERRUPTED"
        row["message"] = "Interrupted; source was not committed/moved."
        failure_detail = f"{type(exc).__name__}: {exc}".rstrip()
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase="Interrupted safely",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, _gui_failure_fraction(phase)),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=source_size,
            output_bytes=temp_output.stat().st_size if temp_output and temp_output.exists() else None,
        )
        raise
    except Exception as exc:
        row["message"] = str(exc)
        failure_detail = f"{type(exc).__name__}: {exc}"
        emit_gui_progress(
            file_name=source.name,
            file_path=str(source),
            phase=f"Failed during {phase}",
            batch_number=number,
            batch_total=total,
            file_percent=None,
            queue_percent=_gui_queue_percent(number, total, _gui_failure_fraction(phase)),
            elapsed_seconds=time.monotonic() - started,
            input_bytes=source_size,
            output_bytes=temp_output.stat().st_size if temp_output and temp_output.exists() else None,
        )
        _log_failure_summary(source, phase, exc)
        logging.exception("Failed: %s", source)
        print_warning(f"File {number}/{total} failed safely during {phase}: {exc}")
        if args.stop_on_error:
            raise
    finally:
        _release_temporary_output(
            temp_output,
            phase,
            row,
            args.keep_work,
            final_output=final_output,
            source=source,
            failure_detail=failure_detail,
        )
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
    args.file_tune_overrides = {}
    args.file_stream_overrides = {}
    args.file_poster_overrides = {}
    args.file_metadata_overrides = {}
    args.file_output_dirs = {}
    args.tool_paths = {name: getattr(args, f"{name}_path", None) for name in TOOL_OVERRIDE_KEYS}
    if args.ffmpeg_path:
        args.tool_paths["ffmpeg"] = args.ffmpeg_path
    root = Path(args.root).expanduser().resolve()
    if not root.exists() or not (root.is_dir() or root.is_file()):
        print(f"ERROR: Media input does not exist or is not a supported file/folder: {root}", file=sys.stderr)
        return 2
    try:
        args.stop_request = StopRequest(args.stop_request_file)
        args.output_dir = canonical_output_directory(args.output_dir, "Default rebuild output folder") if args.output_dir else None
        if args.file_output_dirs_json:
            args.file_output_dirs = parse_file_output_directories(args.file_output_dirs_json, root)
        if args.file_poster_overrides_json:
            args.file_poster_overrides = parse_file_poster_overrides(
                args.file_poster_overrides_json,
                root,
                allowed_cache_root=args.allowed_poster_cache_root,
            )
        if args.file_metadata_overrides_json:
            args.file_metadata_overrides = parse_file_metadata_overrides(args.file_metadata_overrides_json, root)
    except (OSError, RebuildError) as exc:
        print(f"ERROR: invalid rebuild output folder: {exc}", file=sys.stderr)
        return 2
    if args.youtube_upload and (args.output_dir or args.file_output_dirs):
        print("ERROR: rebuild output folders do not apply to YouTube export mode; use --upload-output-dir.", file=sys.stderr)
        return 2
    config_root = root if root.is_dir() else root.parent
    try:
        cfg, config_files = load_config(config_root)
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
    if args.min_size_mib < 0:
        print("ERROR: --min-size-mib must be zero or greater.", file=sys.stderr)
        return 2
    if args.youtube_upload and (args.info_only or args.test or args.metadata_only):
        print("ERROR: YouTube upload preparation is its own output workflow; do not combine it with --info-only or --test.", file=sys.stderr)
        return 2
    if args.metadata_only and (args.info_only or args.test or args.setup_only):
        print("ERROR: --metadata-only cannot be combined with check-only, short-test, or setup-only mode.", file=sys.stderr)
        return 2
    if args.file_poster_overrides and (args.youtube_upload or args.info_only or args.setup_only):
        print("ERROR: selected poster images apply only to metadata-only, short-test, and full rebuild outputs.", file=sys.stderr)
        return 2
    if args.file_metadata_overrides and (args.youtube_upload or args.info_only or args.setup_only):
        print("ERROR: reviewed metadata applies only to metadata-only, short-test, and full rebuild outputs.", file=sys.stderr)
        return 2
    if args.metadata_only and args.file_stream_overrides:
        print("ERROR: metadata-only output always preserves every source stream; per-file stream selection is not allowed.", file=sys.stderr)
        return 2
    if args.youtube_upload and args.setup_only:
        print("ERROR: YouTube export mode cannot be combined with --setup-only.", file=sys.stderr)
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
    if args.file_tune_overrides_json:
        raw_overrides = args.file_tune_overrides_json
        if len(raw_overrides.encode("utf-8")) > 65536:
            print("ERROR: per-file tune overrides exceed the 64 KiB limit.", file=sys.stderr)
            return 2
        try:
            parsed_overrides = json.loads(raw_overrides)
        except json.JSONDecodeError as exc:
            print(f"ERROR: --file-tune-overrides-json is invalid JSON: {exc}", file=sys.stderr)
            return 2
        if not isinstance(parsed_overrides, dict) or len(parsed_overrides) > 512:
            print("ERROR: per-file tune overrides must be an object containing at most 512 paths.", file=sys.stderr)
            return 2
        try:
            for raw_path, tune in parsed_overrides.items():
                if not isinstance(raw_path, str) or not isinstance(tune, str) or tune not in {"animation", "none"}:
                    raise RebuildError("Each per-file tune entry must map an absolute path to 'animation' or 'none'.")
                path = Path(raw_path).expanduser()
                if not path.is_absolute():
                    raise RebuildError("Per-file tune override paths must be absolute.")
                resolved = path.resolve(strict=True)
                if not resolved.is_file():
                    raise RebuildError(f"Per-file tune override is not a file: {resolved}")
                args.file_tune_overrides[str(resolved)] = tune
        except (OSError, RebuildError) as exc:
            print(f"ERROR: invalid per-file tune override: {exc}", file=sys.stderr)
            return 2
        if args.youtube_upload and args.file_tune_overrides:
            print("ERROR: per-file x265 content tunes do not apply to YouTube export mode.", file=sys.stderr)
            return 2
    if args.file_stream_overrides_json:
        raw_overrides = args.file_stream_overrides_json
        if len(raw_overrides.encode("utf-8")) > 1_048_576:
            print("ERROR: per-file stream overrides exceed the 1 MiB limit.", file=sys.stderr)
            return 2
        try:
            parsed_overrides = json.loads(raw_overrides)
        except json.JSONDecodeError as exc:
            print(f"ERROR: --file-stream-overrides-json is invalid JSON: {exc}", file=sys.stderr)
            return 2
        if not isinstance(parsed_overrides, dict) or len(parsed_overrides) > 512:
            print("ERROR: per-file stream overrides must be an object containing at most 512 paths.", file=sys.stderr)
            return 2
        try:
            for raw_path, indices in parsed_overrides.items():
                if (
                    not isinstance(raw_path, str)
                    or not isinstance(indices, list)
                    or len(indices) > 1024
                    or any(not isinstance(index, int) or isinstance(index, bool) or index < 0 or index > 65535 for index in indices)
                    or len(set(indices)) != len(indices)
                ):
                    raise RebuildError("Each per-file stream entry must map an absolute path to unique FFprobe stream indices.")
                path = Path(raw_path).expanduser()
                if not path.is_absolute():
                    raise RebuildError("Per-file stream override paths must be absolute.")
                resolved = path.resolve(strict=True)
                if not resolved.is_file():
                    raise RebuildError(f"Per-file stream override is not a file: {resolved}")
                args.file_stream_overrides[str(resolved)] = indices
        except (OSError, RebuildError) as exc:
            print(f"ERROR: invalid per-file stream override: {exc}", file=sys.stderr)
            return 2
        if args.youtube_upload and args.file_stream_overrides:
            print("ERROR: per-file stream selections do not apply to YouTube export mode.", file=sys.stderr)
            return 2
        if args.metadata_only and args.file_stream_overrides:
            print("ERROR: metadata-only output always preserves every source stream; per-file stream selection is not allowed.", file=sys.stderr)
            return 2
    args.allow_youtube_hdr10plus_loss_paths = set()
    if args.allow_youtube_hdr10plus_loss:
        if not args.youtube_upload:
            print("ERROR: --allow-youtube-hdr10plus-loss is only available with --youtube-upload.", file=sys.stderr)
            return 2
        if len(args.allow_youtube_hdr10plus_loss) > 512:
            print("ERROR: HDR10+ loss consent is limited to 512 media files.", file=sys.stderr)
            return 2
        if sum(len(path.encode("utf-8")) for path in args.allow_youtube_hdr10plus_loss) > 65536:
            print("ERROR: HDR10+ loss consent exceeds the 64 KiB path limit.", file=sys.stderr)
            return 2
        try:
            for raw_path in args.allow_youtube_hdr10plus_loss:
                if len(raw_path.encode("utf-8")) > 8192:
                    raise RebuildError("HDR10+ loss consent paths are limited to 8192 characters each.")
                path = Path(raw_path).expanduser()
                if not path.is_absolute():
                    raise RebuildError("HDR10+ loss consent paths must be absolute.")
                resolved = path.resolve(strict=True)
                if not resolved.is_file():
                    raise RebuildError(f"HDR10+ loss consent path is not a file: {resolved}")
                canonical = str(resolved)
                if canonical in args.allow_youtube_hdr10plus_loss_paths:
                    raise RebuildError(f"HDR10+ loss consent was supplied more than once for {resolved}.")
                args.allow_youtube_hdr10plus_loss_paths.add(canonical)
            if sum(len(path.encode("utf-8")) for path in args.allow_youtube_hdr10plus_loss_paths) > 65536:
                raise RebuildError("Canonical HDR10+ loss consent paths exceed the 64 KiB limit.")
        except (OSError, RebuildError) as exc:
            print(f"ERROR: invalid YouTube HDR10+ loss consent: {exc}", file=sys.stderr)
            return 2
    log_dir = user_log_directory()
    log_path, csv_path = configure_logging(log_dir, args.verbose)
    logging.info("HEVC Rebuild Ubuntu/Linux Mint %s", __version__)
    logging.info("Root: %s", root)
    try:
        if args.youtube_upload:
            tools = resolve_youtube_toolchain(
                allow_unsupported_os=args.allow_unsupported_os,
                ffmpeg_path=args.ffmpeg_path,
                tool_paths=args.tool_paths,
            )
            logging.info("YouTube export tools: ffmpeg=%s ffprobe=%s mediainfo=%s", tools.ffmpeg, tools.ffprobe, tools.mediainfo)
        else:
            tools = resolve_toolchain(
                assume_yes=args.assume_yes,
                skip_update_check=args.skip_update_check,
                allow_unsupported_os=args.allow_unsupported_os,
                install_missing=not args.no_dependency_repair and not args.info_only,
                ffmpeg_path=args.ffmpeg_path,
                tool_paths=args.tool_paths,
                purpose="check" if args.info_only else "metadata" if args.metadata_only else "rebuild",
            )
            logging.info("Toolchain: ffmpeg=%s mediainfo=%s mkvpropedit=%s hdr10plus=%s x265_dolbyvision=%s", tools.ffmpeg, tools.mediainfo, tools.mkvpropedit, tools.hdr10plus_tool, tools.x265_dolbyvision)
        if args.setup_only:
            print("All required software is installed, current (unless update checks were disabled/offline), and capability-tested.")
            return 0
        if args.youtube_upload:
            from .youtube import youtube_upload_batch

            upload_output = Path(args.upload_output_dir).expanduser() if args.upload_output_dir else None
            status = youtube_upload_batch(
                tools,
                root,
                resolution=args.upload_resolution,
                quality_mode=args.upload_quality_mode,
                output_dir=upload_output,
                recursive=args.recursive,
                min_size_bytes=args.min_size_mib * 1024 * 1024,
                include_name_patterns=args.include_filename_pattern,
                exclude_name_patterns=args.exclude_filename_pattern,
                selected_paths=args.selected_file,
                csv_path=csv_path,
                args=args,
            )
            print(f"\nLog: {log_path}\nRun report: {csv_path}")
            return status
        files = discover(
            root,
            recursive=args.recursive,
            min_size=args.min_size_mib * 1024 * 1024,
            include_name_patterns=args.include_filename_pattern,
            exclude_name_patterns=args.exclude_filename_pattern,
            selected_paths=args.selected_file,
        )
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
        discovered_paths = {str(f.resolve()) for f in files}
        unknown_overrides = sorted(set(args.file_tune_overrides) - discovered_paths)
        if unknown_overrides:
            raise RebuildError("Per-file tune overrides do not belong to the selected media input: " + ", ".join(unknown_overrides[:5]))
        unknown_stream_overrides = sorted(set(args.file_stream_overrides) - discovered_paths)
        if unknown_stream_overrides:
            raise RebuildError("Per-file stream overrides do not belong to the selected media input: " + ", ".join(unknown_stream_overrides[:5]))
        unknown_poster_overrides = sorted(set(args.file_poster_overrides) - discovered_paths)
        if unknown_poster_overrides:
            raise RebuildError("Per-file poster selections do not belong to the selected media input: " + ", ".join(unknown_poster_overrides[:5]))
        unknown_metadata_overrides = sorted(set(args.file_metadata_overrides) - discovered_paths)
        if unknown_metadata_overrides:
            raise RebuildError("Reviewed metadata does not belong to the selected media input: " + ", ".join(unknown_metadata_overrides[:5]))
        unknown_output_overrides = sorted(set(args.file_output_dirs) - discovered_paths)
        if unknown_output_overrides:
            raise RebuildError("Per-file output folders do not belong to the selected media input: " + ", ".join(unknown_output_overrides[:5]))
        if pre_skipped:
            print_phase(f"Skipped {pre_skipped} already-processed file(s) with an existing old/ backup")
        if not eligible:
            print("No eligible files remain after safety filtering.")
            print(f"\nLog: {log_path}\nRun report: {csv_path}")
            return 0

        validate_output_destinations(eligible, root, args)

        run_started = time.monotonic()
        total_input = sum(f.stat().st_size for f in eligible)
        mem = memory_stats()
        disk = disk_stats(args.output_dir or root)
        effective_guard = automatic_memory_reserve_gib(
            mem.total_gib, floor_gib=args.min_free_memory_gib, percent=args.memory_reserve_percent
        )
        mode = "CHECK ONLY" if args.info_only else ("METADATA-ONLY MKV" if args.metadata_only else ("SHORT TESTS" if args.test else "FULL REBUILD"))
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
            if args.stop_request.requested():
                logging.info("Stop requested by operator; remaining files will stay untouched.")
                print_warning(f"Stopped before file {i}/{len(eligible)}; remaining files were not changed.")
                break
            processor = process_metadata_only_one if args.metadata_only else process_one
            results.append(processor(tools, source, root, args, csv_path, i, len(eligible)))

        counts = {"OK": 0, "METADATA_OK": 0, "TEST_OK": 0, "INFO": 0, "SKIPPED": pre_skipped, "FAILED": 0}
        for row in results:
            status = str(row.get("status", "FAILED"))
            counts[status] = counts.get(status, 0) + 1
        print_batch_summary(
            ok=counts.get("OK", 0), tests=counts.get("TEST_OK", 0), info=counts.get("INFO", 0),
            skipped=counts.get("SKIPPED", 0), failed=counts.get("FAILED", 0),
            elapsed=time.monotonic() - run_started,
            metadata=counts.get("METADATA_OK", 0),
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
    except Exception:
        logging.exception("Unhandled failure")
        return 1
