from __future__ import annotations

import json
import logging
import math
import re
from pathlib import Path
from typing import Any

from .hdr import verify_hdr10plus
from .metadata import mkv_identify, read_global_simple_tags
from .model import MediaProfile, StreamRef, Toolchain, ValidationResult
from .probe import build_profile, ffprobe_json, stream_refs
from .util import RebuildError, run


def stream_copy_hash(tools: Toolchain, path: Path, specifier: str) -> str:
    # Explicit -c copy makes FFmpeg hash compressed packet payloads, not decoded PCM/video.
    cp = run([
        tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path),
        "-map", f"0:{specifier}", "-c", "copy", "-f", "hash", "-hash", "sha256", "-",
    ])
    m = re.search(r"SHA256=([0-9a-fA-F]{64})", cp.stdout)
    if not m:
        raise RebuildError(f"Could not compute compressed stream hash for {path} stream {specifier}")
    return m.group(1).lower()




def packet_fingerprints(tools: Toolchain, path: Path, specifiers: list[str]) -> list[dict[str, Any]]:
    """Return packet-boundary SHA-256 fingerprints for mapped streams without decoding.

    FFmpeg framehash with explicit -c copy hashes each *compressed packet* and also
    reports codec extradata. We deliberately ignore container timestamp units here;
    timing is validated separately in seconds so MP4->Matroska time-base conversion
    does not create false corruption alarms.
    """
    if not specifiers:
        return []
    args = [tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path)]
    for spec in specifiers:
        args += ["-map", f"0:{spec}"]
    args += ["-c", "copy", "-f", "framehash", "-hash", "sha256", "-"]
    cp = run(args)
    streams: list[dict[str, Any]] = [{"extradata": None, "packets": [], "seen": False} for _ in specifiers]
    for line in cp.stdout.splitlines():
        m_type = re.match(r"^#media_type\s+(\d+):", line)
        if m_type:
            idx = int(m_type.group(1))
            if 0 <= idx < len(streams):
                streams[idx]["seen"] = True
            continue
        m_extra = re.match(r"^#extradata\s+(\d+),\s*\d+,\s*([0-9a-fA-F]{64})\s*$", line)
        if m_extra:
            idx = int(m_extra.group(1))
            if 0 <= idx < len(streams):
                streams[idx]["extradata"] = m_extra.group(2).lower()
            continue
        m_packet = re.match(
            r"^\s*(\d+),\s*-?\d+,\s*-?\d+,\s*-?\d+,\s*(\d+),\s*([0-9a-fA-F]{64})\s*$",
            line,
        )
        if m_packet:
            idx = int(m_packet.group(1))
            if 0 <= idx < len(streams):
                streams[idx]["packets"].append((int(m_packet.group(2)), m_packet.group(3).lower()))
    if any(not item["seen"] for item in streams):
        raise RebuildError(f"Could not obtain packet fingerprints for all mapped streams in {path}")
    for item in streams:
        item.pop("seen", None)
    return streams


def _active_dispositions(value: dict[str, Any]) -> set[str]:
    return {str(k) for k, v in (value or {}).items() if int(v or 0) == 1}


def _raw_streams(probe: dict[str, Any], kind: str) -> list[dict[str, Any]]:
    return [s for s in probe.get("streams", []) or [] if s.get("codec_type") == kind]


def _compare_codec_parameters(source_probe: dict[str, Any], output_probe: dict[str, Any], kind: str) -> None:
    src = _raw_streams(source_probe, kind)
    dst = _raw_streams(output_probe, kind)
    if kind == "audio":
        keys = ("profile", "sample_rate", "channels", "channel_layout", "bits_per_sample", "bits_per_raw_sample", "extradata_hash")
    elif kind == "subtitle":
        # Subtitle codec-private data (especially ASS/SSA headers/styles) matters just as much as packets.
        keys = ("profile", "width", "height", "extradata_hash")
    else:
        return
    for n, (a, b) in enumerate(zip(src, dst)):
        for key in keys:
            av, bv = a.get(key), b.get(key)
            # Missing/empty in both is equivalent. If the source has a value, require it unchanged.
            if av not in {None, "", 0, "0"} and str(av) != str(bv):
                raise RebuildError(f"{kind} stream {n} codec parameter {key} changed: {av!r} -> {bv!r}")


def _compare_copied_type(tools: Toolchain, source: Path, output: Path, kind: str, source_refs: list[StreamRef], output_refs: list[StreamRef], verify_hashes: bool) -> None:
    if len(source_refs) != len(output_refs):
        raise RebuildError(f"{kind} stream count changed: input={len(source_refs)} output={len(output_refs)}")
    short = {"audio": "a", "subtitle": "s", "video": "v"}[kind]
    source_fingerprints = packet_fingerprints(tools, source, [f"{short}:{n}" for n in range(len(source_refs))]) if verify_hashes else []
    output_fingerprints = packet_fingerprints(tools, output, [f"{short}:{n}" for n in range(len(output_refs))]) if verify_hashes else []
    for n, (src, dst) in enumerate(zip(source_refs, output_refs)):
        if src.codec_name != dst.codec_name:
            raise RebuildError(f"{kind} stream {n} codec changed: {src.codec_name} -> {dst.codec_name}")
        # Preserve language/title metadata and all disposition flags for audio/subtitle streams.
        if kind in {"audio", "subtitle"}:
            for tag in ("language", "title"):
                if (src.tags.get(tag) or "") != (dst.tags.get(tag) or ""):
                    raise RebuildError(f"{kind} stream {n} {tag} metadata changed.")
            if _active_dispositions(src.disposition) != _active_dispositions(dst.disposition):
                raise RebuildError(
                    f"{kind} stream {n} disposition flags changed: "
                    f"{sorted(_active_dispositions(src.disposition))} -> {sorted(_active_dispositions(dst.disposition))}"
                )
        if verify_hashes:
            left = source_fingerprints[n]
            right = output_fingerprints[n]
            if left != right:
                raise RebuildError(
                    f"{kind} stream {n} compressed packet sequence/extradata differs. The output is rejected; "
                    "no audio/subtitle transcoding or packet mutation is allowed."
                )


def _timing_value(stream: dict[str, Any], key: str) -> float | None:
    try:
        return float(stream.get(key)) if stream.get(key) not in {None, "N/A"} else None
    except (TypeError, ValueError):
        return None


def _validate_timing(source_probe: dict[str, Any], output_probe: dict[str, Any], kind: str, tolerance: float = 0.010) -> None:
    src = [s for s in source_probe.get("streams", []) if s.get("codec_type") == kind]
    out = [s for s in output_probe.get("streams", []) if s.get("codec_type") == kind]
    for n, (a, b) in enumerate(zip(src, out)):
        for key in ("start_time", "duration"):
            av, bv = _timing_value(a, key), _timing_value(b, key)
            if av is not None and bv is not None and abs(av - bv) > tolerance:
                raise RebuildError(f"{kind} stream {n} {key} shifted by {abs(av-bv):.6f}s; refusing possible A/V/subtitle sync change.")


def validate_output(
    tools: Toolchain,
    source_profile: MediaProfile,
    output: Path,
    *,
    secondary_video_policy: str,
    verify_copy_hashes: bool,
    expect_poster: bool,
    test_mode: bool,
    expected_global_tags: dict[str, str] | None = None,
    preserve_hdr10plus: bool = True,
    preserve_dolbyvision: bool = False,
    strip_dolbyvision: bool = False,
) -> ValidationResult:
    if not output.exists() or output.stat().st_size < 1024 * 1024:
        raise RebuildError(f"Output is missing or implausibly small: {output}")
    out_probe = ffprobe_json(tools, output)
    out_refs = stream_refs(out_probe)
    out_videos = [s for s in out_refs if s.codec_type == "video" and not s.attached_pic]
    if not out_videos:
        raise RebuildError("Output contains no usable video stream.")
    primary = out_videos[0]
    if (primary.codec_name or "").lower() not in {"hevc", "h265"}:
        raise RebuildError(f"Output primary video codec is {primary.codec_name!r}; expected HEVC.")
    if (primary.pix_fmt or "") != source_profile.target_pix_fmt:
        raise RebuildError(f"Output primary pixel format {primary.pix_fmt!r} != planned {source_profile.target_pix_fmt!r}.")
    if primary.width != source_profile.primary_video.width or primary.height != source_profile.primary_video.height:
        raise RebuildError(
            f"Output resolution {primary.width}x{primary.height} != selected primary {source_profile.primary_video.width}x{source_profile.primary_video.height}."
        )
    # Highest-resolution stream must be first/default after rebuild.
    if not primary.default:
        raise RebuildError("Selected highest-resolution video is not marked as the output default video track.")

    src_duration = source_profile.duration
    out_duration = float((out_probe.get("format") or {}).get("duration") or 0)
    if not test_mode:
        tolerance = max(2.0, src_duration * 0.005)
        if abs(out_duration - src_duration) > tolerance:
            raise RebuildError(f"Output duration differs by {abs(out_duration-src_duration):.3f}s (tolerance {tolerance:.3f}s).")

    out_audio = [s for s in out_refs if s.codec_type == "audio"]
    out_subs = [s for s in out_refs if s.codec_type == "subtitle"]
    if not test_mode:
        _compare_copied_type(tools, source_profile.path, output, "audio", source_profile.audio, out_audio, verify_copy_hashes)
        _compare_copied_type(tools, source_profile.path, output, "subtitle", source_profile.subtitles, out_subs, verify_copy_hashes)
        _compare_codec_parameters(source_profile.probe, out_probe, "audio")
        _compare_codec_parameters(source_profile.probe, out_probe, "subtitle")
        _validate_timing(source_profile.probe, out_probe, "audio")
        _validate_timing(source_profile.probe, out_probe, "subtitle", tolerance=0.020)

    if secondary_video_policy == "copy":
        expected = len(source_profile.secondary_videos) + 1
        if len(out_videos) != expected:
            raise RebuildError(f"Video stream count changed unexpectedly: expected {expected}, output {len(out_videos)}")
        if not test_mode and verify_copy_hashes and source_profile.secondary_videos:
            # Input primary was reordered to output v:0. Every source secondary follows as output v:1..N.
            left = packet_fingerprints(
                tools, source_profile.path, [f"v:{src.type_ordinal}" for src in source_profile.secondary_videos]
            )
            right = packet_fingerprints(
                tools, output, [f"v:{out_ord}" for out_ord in range(1, len(source_profile.secondary_videos) + 1)]
            )
            for src, a, b in zip(source_profile.secondary_videos, left, right):
                if a != b:
                    raise RebuildError(f"Secondary video {src.label} was not copied packet-for-packet exactly.")

    if not test_mode:
        src_chapters = source_profile.probe.get("chapters") or []
        out_chapters = out_probe.get("chapters") or []
        if len(out_chapters) != source_profile.chapters_count:
            raise RebuildError(f"Chapter count changed: {source_profile.chapters_count} -> {len(out_chapters)}")
        for n, (src_ch, dst_ch) in enumerate(zip(src_chapters, out_chapters)):
            for key in ("start_time", "end_time"):
                a, b = _timing_value(src_ch, key), _timing_value(dst_ch, key)
                if a is not None and b is not None and abs(a - b) > 0.002:
                    raise RebuildError(f"Chapter {n} {key} changed by {abs(a-b):.6f}s.")
            src_tags = {str(k).lower(): str(v) for k, v in (src_ch.get("tags") or {}).items()}
            dst_tags = {str(k).lower(): str(v) for k, v in (dst_ch.get("tags") or {}).items()}
            for key, value in src_tags.items():
                if key in {"encoder", "duration"}:
                    continue
                if dst_tags.get(key) != value:
                    raise RebuildError(f"Chapter {n} metadata {key!r} changed: {value!r} -> {dst_tags.get(key)!r}")

    # Re-analyze the encoded primary video so HDR10 static mastering metadata is validated, not merely color labels.
    out_profile = build_profile(tools, output, promote_8bit_to_10bit=False)
    expected_color = {
        "color primaries": (source_profile.color_primaries, out_profile.color_primaries),
        "transfer characteristic": (source_profile.transfer, out_profile.transfer),
        "color matrix": (source_profile.color_matrix, out_profile.color_matrix),
    }
    for label, (expected_value, actual_value) in expected_color.items():
        if expected_value and actual_value and str(expected_value) != str(actual_value):
            raise RebuildError(f"Primary video {label} changed: expected {expected_value}, output {actual_value}")
    if source_profile.hdr10 and not out_profile.hdr10:
        raise RebuildError("HDR10 input no longer identifies as HDR10 after encode.")
    if source_profile.hlg and not out_profile.hlg:
        raise RebuildError("HLG input no longer identifies as HLG after encode.")
    if source_profile.master_display and not out_profile.master_display:
        raise RebuildError("Source carried mastering-display metadata but output does not.")
    if source_profile.master_display and out_profile.master_display and source_profile.master_display != out_profile.master_display:
        raise RebuildError(f"Mastering-display metadata changed: input={source_profile.master_display!r} output={out_profile.master_display!r}")
    if source_profile.max_cll is not None and out_profile.max_cll != source_profile.max_cll:
        raise RebuildError(f"MaxCLL changed: input={source_profile.max_cll} output={out_profile.max_cll}")
    if source_profile.max_fall is not None and out_profile.max_fall != source_profile.max_fall:
        raise RebuildError(f"MaxFALL changed: input={source_profile.max_fall} output={out_profile.max_fall}")

    if source_profile.hdr10plus and preserve_hdr10plus:
        if not verify_hdr10plus(tools, source_profile, path_override=output):
            raise RebuildError("HDR10+ source was encoded but dynamic HDR10+ metadata was not verified in the output.")

    if preserve_dolbyvision and strip_dolbyvision:
        raise RebuildError("Internal validation error: Dolby Vision cannot be preserved and stripped simultaneously.")

    if preserve_dolbyvision:
        if not out_profile.dolby_vision:
            raise RebuildError("Dolby Vision preservation was requested but output no longer identifies as Dolby Vision.")
        if out_profile.dovi_profile != source_profile.dovi_profile:
            raise RebuildError(f"Dolby Vision profile changed: input={source_profile.dovi_profile} output={out_profile.dovi_profile}")
        if not out_profile.dovi_rpu_present:
            raise RebuildError("Dolby Vision output does not report an RPU; refusing metadata loss.")
        if out_profile.dovi_el_present:
            raise RebuildError("Dolby Vision output unexpectedly reports an enhancement layer.")
        if source_profile.dovi_compatibility_id is not None and out_profile.dovi_compatibility_id is not None:
            if source_profile.dovi_compatibility_id != out_profile.dovi_compatibility_id:
                raise RebuildError(
                    f"Dolby Vision base-layer compatibility ID changed: input={source_profile.dovi_compatibility_id} "
                    f"output={out_profile.dovi_compatibility_id}"
                )


    if strip_dolbyvision:
        if not source_profile.hdr10:
            raise RebuildError("Dolby Vision HDR10 fallback was requested for a source that was not verified as HDR10-compatible.")
        if not out_profile.hdr10:
            raise RebuildError("Dolby Vision HDR10 fallback output no longer identifies as HDR10.")
        if out_profile.dolby_vision:
            raise RebuildError(
                "Dolby Vision HDR10 fallback was requested, but the output still carries Dolby Vision signaling; "
                "refusing an ambiguous result."
            )

    if expect_poster:
        ident = mkv_identify(tools, output)
        covers = [a for a in ident.get("attachments", []) or [] if str(a.get("file_name", "")).lower() == "cover.jpg" and str(a.get("content_type", "")).lower() == "image/jpeg"]
        if not covers:
            raise RebuildError("Expected cover.jpg attachment is missing from output.")

    if expected_global_tags:
        actual_tags = read_global_simple_tags(tools, output)
        for name, value in expected_global_tags.items():
            values = actual_tags.get(name.upper(), [])
            if value not in values:
                raise RebuildError(
                    f"Expected embedded Matroska tag {name}={value!r} was not verified in the output."
                )

    return ValidationResult(output=output, output_probe=out_probe, copied_hashes_verified=(verify_copy_hashes and not test_mode))
