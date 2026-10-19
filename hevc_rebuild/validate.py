from __future__ import annotations

import json
import hashlib
import os
import re
import subprocess
import threading
from collections import deque
from pathlib import Path
from typing import Any

from .hdr import verify_hdr10plus
from .metadata import mkv_identify, read_global_simple_tags
from .model import MediaProfile, StreamRef, Toolchain, ValidationResult
from .probe import build_profile, ffprobe_json, stream_refs
from .util import ExternalCommandError, RebuildError, run


def stream_copy_hash(tools: Toolchain, path: Path, specifier: str) -> str:
    # Explicit -c copy makes FFmpeg hash compressed packet payloads, not decoded PCM/video.
    native = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native:
        result = _native_stream_verification(
            native, "stream-copy-hash", tools, path, [specifier],
        )
        digest = result.get("sha256")
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise RebuildError("Rust stream verifier returned an invalid SHA-256 digest.")
        return digest
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
    native = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native:
        result = _native_stream_verification(
            native, "packet-fingerprints", tools, path, specifiers,
        )
        fingerprints = result.get("fingerprints")
        if not isinstance(fingerprints, list) or len(fingerprints) != len(specifiers):
            raise RebuildError("Rust packet verifier returned an incomplete stream fingerprint set.")
        for item in fingerprints:
            extradata = item.get("extradata") if isinstance(item, dict) else None
            if (
                not isinstance(item, dict)
                or "extradata" not in item
                or (
                    extradata is not None
                    and (
                        not isinstance(extradata, str)
                        or not re.fullmatch(r"[0-9a-f]{64}", extradata)
                    )
                )
                or not isinstance(item.get("packet_sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", item["packet_sha256"])
                or type(item.get("packet_count")) is not int
                or type(item.get("packet_bytes")) is not int
                or item["packet_count"] < 0
                or item["packet_bytes"] < 0
            ):
                raise RebuildError("Rust packet verifier returned malformed stream fingerprints.")
        return fingerprints
    args = [tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path)]
    for spec in specifiers:
        args += ["-map", f"0:{spec}"]
    args += ["-c", "copy", "-f", "framehash", "-hash", "sha256", "-"]
    try:
        proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace", bufsize=1)
    except OSError as exc:
        raise RebuildError(f"Could not start FFmpeg packet verification for {path}: {exc}") from exc

    # Keep only a bounded diagnostic tail. A full packet list (or captured framehash
    # stdout) grows with the media duration and can exhaust RAM on long 4K sources.
    stderr_tail: deque[str] = deque(maxlen=160)

    def drain_stderr() -> None:
        if proc.stderr is None:
            return
        for raw in proc.stderr:
            stderr_tail.append(raw[-2048:])
        proc.stderr.close()

    stderr_thread = threading.Thread(target=drain_stderr, daemon=True)
    stderr_thread.start()
    streams: list[dict[str, Any]] = [
        {"extradata": None, "digest": hashlib.sha256(), "packet_count": 0, "packet_bytes": 0, "seen": False}
        for _ in specifiers
    ]
    try:
        assert proc.stdout is not None
        for line in proc.stdout:
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
            if not m_packet:
                continue
            idx = int(m_packet.group(1))
            if not 0 <= idx < len(streams):
                continue
            item = streams[idx]
            packet_size = int(m_packet.group(2))
            item["digest"].update(packet_size.to_bytes(8, "big"))
            item["digest"].update(bytes.fromhex(m_packet.group(3)))
            item["packet_count"] += 1
            item["packet_bytes"] += packet_size
    except KeyboardInterrupt:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
        stderr_thread.join(timeout=3)
        raise
    except Exception:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        stderr_thread.join(timeout=3)
        raise
    finally:
        if proc.stdout is not None:
            proc.stdout.close()

    return_code = proc.wait()
    stderr_thread.join(timeout=3)
    if return_code != 0:
        raise ExternalCommandError(args, return_code, "".join(stderr_tail))
    if any(not item["seen"] for item in streams):
        raise RebuildError(f"Could not obtain packet fingerprints for all mapped streams in {path}")
    return [
        {
            "extradata": item["extradata"],
            "packet_sha256": item["digest"].hexdigest(),
            "packet_count": item["packet_count"],
            "packet_bytes": item["packet_bytes"],
        }
        for item in streams
    ]


def _native_stream_verification(
    executable: str,
    action: str,
    tools: Toolchain,
    path: Path,
    specifiers: list[str],
) -> dict[str, Any]:
    args = [
        executable,
        "internal",
        action,
        "--ffmpeg",
        tools.ffmpeg,
        "--file",
        str(path),
    ]
    for specifier in specifiers:
        args.extend(["--stream", specifier])
    try:
        cp = subprocess.run(
            args,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            check=False,
        )
    except OSError as exc:
        raise RebuildError(f"Could not start the Rust stream verifier: {exc}") from exc
    if cp.returncode != 0:
        detail = cp.stderr.strip()
        raise RebuildError(
            f"Rust stream verifier exited with status {cp.returncode}."
            + (f"\n{detail}" if detail else "")
        )
    try:
        result = json.loads(cp.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"Rust stream verifier returned invalid JSON: {exc}") from exc
    if not isinstance(result, dict):
        raise RebuildError("Rust stream verifier returned a non-object result.")
    return result


def _native_output_stream_validation(
    executable: str,
    tools: Toolchain,
    source_profile: MediaProfile,
    output: Path,
    *,
    selected_streams: list[StreamRef],
    video_profiles: dict[int, MediaProfile],
    encode_video_indices: set[int],
    verify_copy_hashes: bool,
    test_mode: bool,
    expect_poster: bool,
    scrubbed_text_values: list[dict[str, Any]],
    chapter_text_edits: list[dict[str, Any]],
    preserve_hdr10plus: bool,
    preserve_dolbyvision_indices: set[int],
    strip_dolbyvision_indices: set[int],
    copy_only: bool,
) -> dict[str, Any]:
    request = {
        "ffmpeg": tools.ffmpeg,
        "ffprobe": tools.ffprobe,
        "mediainfo": tools.mediainfo,
        "hdr10plus_tool": tools.hdr10plus_tool,
        "source_path": str(source_profile.path),
        "output_path": str(output),
        "source_probe": source_profile.probe,
        "selected_stream_indices": [stream.index for stream in selected_streams],
        "encode_video_indices": sorted(encode_video_indices),
        "video_target_formats": {
            str(index): profile.target_pix_fmt
            for index, profile in video_profiles.items()
        },
        "video_hdr_expectations": {
            str(index): {
                "color_primaries": profile.color_primaries,
                "transfer": profile.transfer,
                "color_matrix": profile.color_matrix,
                "master_display": profile.master_display,
                "max_cll": profile.max_cll,
                "max_fall": profile.max_fall,
                "hdr10": profile.hdr10,
                "hdr10plus": preserve_hdr10plus and profile.hdr10plus,
                "hlg": profile.hlg,
                "dolby_vision": profile.dolby_vision,
                "dovi_profile": profile.dovi_profile,
                "dovi_rpu_present": profile.dovi_rpu_present,
                "dovi_el_present": profile.dovi_el_present,
                "dovi_compatibility_id": profile.dovi_compatibility_id,
                "preserve_dolbyvision": index in preserve_dolbyvision_indices,
                "strip_dolbyvision": index in strip_dolbyvision_indices,
            }
            for index, profile in video_profiles.items()
            if index in encode_video_indices or copy_only
        },
        "verify_copy_hashes": verify_copy_hashes,
        "test_mode": test_mode,
        "copy_only": copy_only,
        "expect_poster": expect_poster,
        "source_duration": source_profile.duration,
        "source_chapters_count": source_profile.chapters_count,
        "scrubbed_text_values": scrubbed_text_values,
        "chapter_text_edits": chapter_text_edits,
    }
    try:
        payload = json.dumps(request, ensure_ascii=False, separators=(",", ":"))
        if len(payload.encode("utf-8")) > 16 * 1024 * 1024:
            raise RebuildError("Rust output-validation request exceeds the 16 MiB limit.")
        cp = subprocess.run(
            [executable, "internal", "validate-streams"],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            errors="replace",
            check=False,
        )
    except OSError as exc:
        raise RebuildError(f"Could not start Rust output stream validation: {exc}") from exc
    if cp.returncode != 0:
        detail = cp.stderr.strip()
        raise RebuildError(
            f"Rust output stream validator exited with status {cp.returncode}."
            + (f"\n{detail}" if detail else "")
        )
    if len(cp.stdout.encode("utf-8")) > 20 * 1024 * 1024:
        raise RebuildError("Rust output stream validator returned more than 20 MiB of JSON.")
    try:
        result = json.loads(cp.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"Rust output stream validator returned invalid JSON: {exc}") from exc
    if (
        not isinstance(result, dict)
        or not isinstance(result.get("output_probe"), dict)
        or type(result.get("copied_hashes_verified")) is not bool
    ):
        raise RebuildError("Rust output stream validator returned an incomplete response.")
    return result


def _active_dispositions(value: dict[str, Any]) -> set[str]:
    return {str(k) for k, v in (value or {}).items() if int(v or 0) == 1}


def _raw_streams(probe: dict[str, Any], kind: str) -> list[dict[str, Any]]:
    return [s for s in probe.get("streams", []) or [] if s.get("codec_type") == kind]


def _compare_codec_parameters(
    source_probe: dict[str, Any],
    output_probe: dict[str, Any],
    kind: str,
    source_refs: list[StreamRef],
    output_refs: list[StreamRef],
) -> None:
    src = {int(stream.get("index", -1)): stream for stream in source_probe.get("streams", []) or []}
    dst = {int(stream.get("index", -1)): stream for stream in output_probe.get("streams", []) or []}
    if kind == "audio":
        keys = ("profile", "sample_rate", "channels", "channel_layout", "bits_per_sample", "bits_per_raw_sample", "extradata_hash")
    elif kind == "subtitle":
        # Subtitle codec-private data (especially ASS/SSA headers/styles) matters just as much as packets.
        keys = ("profile", "width", "height", "extradata_hash")
    else:
        return
    for n, (source_ref, output_ref) in enumerate(zip(source_refs, output_refs)):
        a, b = src.get(source_ref.index, {}), dst.get(output_ref.index, {})
        for key in keys:
            av, bv = a.get(key), b.get(key)
            # Missing/empty in both is equivalent. If the source has a value, require it unchanged.
            if av not in {None, "", 0, "0"} and str(av) != str(bv):
                raise RebuildError(f"{kind} stream {n} codec parameter {key} changed: {av!r} -> {bv!r}")


def _approved_scrub_change(
    edits: list[dict[str, Any]],
    *,
    scopes: set[str],
    before: str,
    after: str,
    chapter_index: int | None = None,
) -> bool:
    return any(
        edit.get("scope") in scopes
        and str(edit.get("field", "title")).strip().casefold() in {"title", "name"}
        and edit.get("before") == before
        and edit.get("after") == after
        and (chapter_index is None or edit.get("chapter_index") == chapter_index)
        for edit in edits
    )


def _validate_copy_hdr_profiles(
    tools: Toolchain,
    output_path: Path,
    source_profiles: dict[int, MediaProfile],
    source_videos: list[StreamRef],
    output_videos: list[StreamRef],
) -> None:
    if len(source_videos) != len(output_videos):
        raise RebuildError("Video stream count changed during metadata-only remux.")
    for source_stream, output_stream in zip(source_videos, output_videos):
        source_profile = source_profiles.get(source_stream.index)
        if source_profile is None:
            raise RebuildError(f"No HDR profile was retained for copied video stream #{source_stream.index}.")
        output_profile = build_profile(
            tools,
            output_path,
            promote_8bit_to_10bit=False,
            video_stream_index=output_stream.index,
        )
        for label, before, after in (
            ("color primaries", source_profile.color_primaries, output_profile.color_primaries),
            ("transfer characteristic", source_profile.transfer, output_profile.transfer),
            ("color matrix", source_profile.color_matrix, output_profile.color_matrix),
            ("mastering-display metadata", source_profile.master_display, output_profile.master_display),
            ("MaxCLL", source_profile.max_cll, output_profile.max_cll),
            ("MaxFALL", source_profile.max_fall, output_profile.max_fall),
            ("Dolby Vision profile", source_profile.dovi_profile, output_profile.dovi_profile),
            ("Dolby Vision RPU state", source_profile.dovi_rpu_present, output_profile.dovi_rpu_present),
            ("Dolby Vision enhancement-layer state", source_profile.dovi_el_present, output_profile.dovi_el_present),
            ("Dolby Vision compatibility ID", source_profile.dovi_compatibility_id, output_profile.dovi_compatibility_id),
        ):
            if before is not None and before != after:
                raise RebuildError(
                    f"Copied video stream #{source_stream.index} {label} changed: {before!r} -> {after!r}."
                )
        for label, before, after in (
            ("HDR10", source_profile.hdr10, output_profile.hdr10),
            ("HDR10+", source_profile.hdr10plus, output_profile.hdr10plus),
            ("HLG", source_profile.hlg, output_profile.hlg),
            ("Dolby Vision", source_profile.dolby_vision, output_profile.dolby_vision),
        ):
            if before != after:
                raise RebuildError(
                    f"Copied video stream #{source_stream.index} {label} state changed: {before} -> {after}."
                )
        if source_profile.hdr10plus and not verify_hdr10plus(tools, source_profile, path_override=output_path):
            raise RebuildError(f"Copied video stream #{source_stream.index} HDR10+ dynamic metadata was not verified in output.")


def _compare_copied_type(
    tools: Toolchain,
    source: Path,
    output: Path,
    kind: str,
    source_refs: list[StreamRef],
    output_refs: list[StreamRef],
    verify_hashes: bool,
    scrubbed_text_values: list[dict[str, Any]],
) -> None:
    if len(source_refs) != len(output_refs):
        raise RebuildError(f"{kind} stream count changed: input={len(source_refs)} output={len(output_refs)}")
    short = {"audio": "a", "subtitle": "s", "video": "v", "attachment": "t", "data": "d"}[kind]
    if verify_hashes and kind == "attachment":
        source_fingerprints = [stream_copy_hash(tools, source, f"{short}:{ref.type_ordinal}") for ref in source_refs]
        output_fingerprints = [stream_copy_hash(tools, output, f"{short}:{ref.type_ordinal}") for ref in output_refs]
    else:
        source_fingerprints = packet_fingerprints(tools, source, [f"{short}:{ref.type_ordinal}" for ref in source_refs]) if verify_hashes else []
        output_fingerprints = packet_fingerprints(tools, output, [f"{short}:{ref.type_ordinal}" for ref in output_refs]) if verify_hashes else []
    for n, (src, dst) in enumerate(zip(source_refs, output_refs)):
        if src.codec_name != dst.codec_name:
            raise RebuildError(f"{kind} stream {n} codec changed: {src.codec_name} -> {dst.codec_name}")
        # Preserve language/title metadata and all disposition flags for audio/subtitle streams.
        if kind in {"audio", "subtitle", "attachment", "data"}:
            for tag in ("language", "title"):
                source_value = src.tags.get(tag) or ""
                output_value = dst.tags.get(tag) or ""
                if source_value != output_value and not (
                    tag == "title"
                    and _approved_scrub_change(
                        scrubbed_text_values,
                        scopes={"track_tag", "track_header"},
                        before=source_value,
                        after=output_value,
                    )
                ):
                    raise RebuildError(f"{kind} stream {n} {tag} metadata changed.")
            if kind == "attachment":
                for tag in ("filename", "mimetype"):
                    if (src.tags.get(tag) or "") != (dst.tags.get(tag) or ""):
                        raise RebuildError(f"attachment stream {n} {tag} metadata changed.")
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


def _is_replaceable_cover(stream: StreamRef) -> bool:
    is_attachment = stream.codec_type == "attachment"
    is_attached_picture = stream.codec_type == "video" and stream.attached_pic
    if not is_attachment and not is_attached_picture:
        return False
    tags = {str(key).casefold(): str(value) for key, value in (stream.tags or {}).items()}
    filename = tags.get("filename", "").casefold()
    mime = tags.get("mimetype", "").casefold()
    return mime.startswith("image/") and any(marker in filename for marker in ("cover", "poster", "folder", "front"))


def _timing_value(stream: dict[str, Any], key: str) -> float | None:
    try:
        return float(stream.get(key)) if stream.get(key) not in {None, "N/A"} else None
    except (TypeError, ValueError):
        return None


def _validate_timing(
    source_probe: dict[str, Any],
    output_probe: dict[str, Any],
    kind: str,
    source_refs: list[StreamRef],
    output_refs: list[StreamRef],
    tolerance: float = 0.010,
) -> None:
    src = {int(stream.get("index", -1)): stream for stream in source_probe.get("streams", []) or []}
    out = {int(stream.get("index", -1)): stream for stream in output_probe.get("streams", []) or []}
    for n, (source_ref, output_ref) in enumerate(zip(source_refs, output_refs)):
        a, b = src.get(source_ref.index, {}), out.get(output_ref.index, {})
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
    scrubbed_text_values: list[dict[str, Any]] | None = None,
    chapter_text_edits: list[dict[str, Any]] | None = None,
    preserve_hdr10plus: bool = True,
    preserve_dolbyvision: bool = False,
    strip_dolbyvision: bool = False,
    selected_streams: list[StreamRef] | None = None,
    video_profiles: dict[int, MediaProfile] | None = None,
    encode_video_indices: set[int] | None = None,
    preserve_dolbyvision_indices: set[int] | None = None,
    strip_dolbyvision_indices: set[int] | None = None,
    copy_only: bool = False,
) -> ValidationResult:
    if selected_streams is None:
        selected_videos = [source_profile.primary_video]
        if secondary_video_policy != "drop":
            selected_videos.extend(source_profile.secondary_videos)
        selected_streams = [*selected_videos, *source_profile.audio, *source_profile.subtitles, *source_profile.attachments]
        if secondary_video_policy != "drop":
            selected_streams.extend(source_profile.data)
        encode_video_indices = {source_profile.primary_video.index}
        if secondary_video_policy == "encode":
            encode_video_indices.update(stream.index for stream in selected_videos)
    if encode_video_indices is None:
        encode_video_indices = {source_profile.primary_video.index}
    video_profiles = video_profiles or {source_profile.primary_video.index: source_profile}
    preserve_dolbyvision_indices = preserve_dolbyvision_indices or set()
    strip_dolbyvision_indices = strip_dolbyvision_indices or set()
    scrubbed_text_values = scrubbed_text_values or []
    chapter_text_edits = chapter_text_edits or []
    native_validation = None
    native = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    if native:
        native_validation = _native_output_stream_validation(
            native,
            tools,
            source_profile,
            output,
            selected_streams=selected_streams,
            video_profiles=video_profiles,
            encode_video_indices=encode_video_indices,
            verify_copy_hashes=verify_copy_hashes,
            test_mode=test_mode,
            expect_poster=expect_poster,
            scrubbed_text_values=scrubbed_text_values,
            chapter_text_edits=chapter_text_edits,
            preserve_hdr10plus=preserve_hdr10plus,
            preserve_dolbyvision_indices=preserve_dolbyvision_indices,
            strip_dolbyvision_indices=strip_dolbyvision_indices,
            copy_only=copy_only,
        )
        out_probe = native_validation["output_probe"]
    else:
        minimum_output_size = 1 if copy_only else 1024 * 1024
        if not output.exists() or output.stat().st_size < minimum_output_size:
            raise RebuildError(f"Output is missing or implausibly small: {output}")
        out_probe = ffprobe_json(tools, output)
    out_refs = stream_refs(out_probe)
    selected_videos = [stream for stream in selected_streams if stream.codec_type == "video"]
    selected_real_videos = sorted(
        (stream for stream in selected_videos if not stream.attached_pic and stream.width > 0 and stream.height > 0),
        key=(lambda stream: stream.index) if copy_only else (lambda stream: (-stream.pixels, -int(stream.default), stream.index)),
    )
    selected_attached_pictures = [stream for stream in selected_videos if stream.attached_pic]
    out_videos = [stream for stream in out_refs if stream.codec_type == "video" and not stream.attached_pic]
    out_attached_pictures = [stream for stream in out_refs if stream.codec_type == "video" and stream.attached_pic]
    if expect_poster:
        # Matroska cover attachments can also appear in FFprobe as attached-picture
        # video streams. The selected cover is checked separately through MKVToolNix.
        selected_attached_pictures = [stream for stream in selected_attached_pictures if not _is_replaceable_cover(stream)]
        out_attached_pictures = [stream for stream in out_attached_pictures if not _is_replaceable_cover(stream)]
    if native_validation is None:
        if not out_videos:
            raise RebuildError("Output contains no usable video stream.")
        if len(out_videos) != len(selected_real_videos):
            raise RebuildError(f"Selected real video stream count changed: expected {len(selected_real_videos)}, output {len(out_videos)}")
        if len(out_attached_pictures) != len(selected_attached_pictures):
            raise RebuildError(f"Selected attached-picture count changed: expected {len(selected_attached_pictures)}, output {len(out_attached_pictures)}")
        primary = out_videos[0]
        if not copy_only and not primary.default:
            raise RebuildError("Selected highest-resolution video is not marked as the output default video track.")
        out_video_by_source = dict(zip((stream.index for stream in selected_real_videos), out_videos))
        for source_stream, output_stream in zip(selected_real_videos, out_videos):
            if source_stream.index in encode_video_indices:
                track_profile = video_profiles.get(source_stream.index)
                if track_profile is None:
                    raise RebuildError(f"No source profile was retained for selected video stream #{source_stream.index}.")
                if (output_stream.codec_name or "").lower() not in {"hevc", "h265"}:
                    raise RebuildError(f"Selected video stream #{source_stream.index} output codec is {output_stream.codec_name!r}; expected HEVC.")
                if (output_stream.pix_fmt or "") != track_profile.target_pix_fmt:
                    raise RebuildError(f"Video stream #{source_stream.index} pixel format {output_stream.pix_fmt!r} != planned {track_profile.target_pix_fmt!r}.")
            elif output_stream.codec_name != source_stream.codec_name:
                raise RebuildError(f"Copied video stream #{source_stream.index} codec changed: {source_stream.codec_name} -> {output_stream.codec_name}.")
            if output_stream.width != source_stream.width or output_stream.height != source_stream.height:
                raise RebuildError(f"Video stream #{source_stream.index} resolution changed from {source_stream.width}x{source_stream.height} to {output_stream.width}x{output_stream.height}.")

    src_duration = source_profile.duration
    out_duration = float((out_probe.get("format") or {}).get("duration") or 0)
    if not test_mode and native_validation is None:
        tolerance = max(2.0, src_duration * 0.005)
        if abs(out_duration - src_duration) > tolerance:
            raise RebuildError(f"Output duration differs by {abs(out_duration-src_duration):.3f}s (tolerance {tolerance:.3f}s).")

    selected_audio = [stream for stream in selected_streams if stream.codec_type == "audio"]
    selected_subtitles = [stream for stream in selected_streams if stream.codec_type == "subtitle"]
    selected_attachments = [stream for stream in selected_streams if stream.codec_type == "attachment"]
    selected_data = [stream for stream in selected_streams if stream.codec_type == "data"]
    out_audio = [stream for stream in out_refs if stream.codec_type == "audio"]
    out_subs = [stream for stream in out_refs if stream.codec_type == "subtitle"]
    out_attachments = [stream for stream in out_refs if stream.codec_type == "attachment"]
    out_data = [stream for stream in out_refs if stream.codec_type == "data"]
    if not test_mode and native_validation is None:
        _compare_copied_type(tools, source_profile.path, output, "audio", selected_audio, out_audio, verify_copy_hashes, scrubbed_text_values)
        _compare_copied_type(tools, source_profile.path, output, "subtitle", selected_subtitles, out_subs, verify_copy_hashes, scrubbed_text_values)
        _compare_codec_parameters(source_profile.probe, out_probe, "audio", selected_audio, out_audio)
        _compare_codec_parameters(source_profile.probe, out_probe, "subtitle", selected_subtitles, out_subs)
        _validate_timing(source_profile.probe, out_probe, "audio", selected_audio, out_audio)
        _validate_timing(source_profile.probe, out_probe, "subtitle", selected_subtitles, out_subs, tolerance=0.020)
        source_attachments = selected_attachments
        copied_output_attachments = out_attachments
        if expect_poster:
            source_attachments = [stream for stream in source_attachments if not _is_replaceable_cover(stream)]
            copied_output_attachments = [stream for stream in copied_output_attachments if not _is_replaceable_cover(stream)]
        _compare_copied_type(tools, source_profile.path, output, "attachment", source_attachments, copied_output_attachments, verify_copy_hashes, scrubbed_text_values)
        _compare_copied_type(tools, source_profile.path, output, "data", selected_data, out_data, verify_copy_hashes, scrubbed_text_values)
        copied_videos = [stream for stream in selected_real_videos if stream.index not in encode_video_indices]
        copied_outputs = [out_video_by_source[stream.index] for stream in copied_videos]
        _compare_copied_type(tools, source_profile.path, output, "video", copied_videos, copied_outputs, verify_copy_hashes, scrubbed_text_values)
        _compare_copied_type(tools, source_profile.path, output, "video", selected_attached_pictures, out_attached_pictures, verify_copy_hashes, scrubbed_text_values)

    if not test_mode and native_validation is None:
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
                    if key == "title" and _approved_scrub_change(
                        chapter_text_edits,
                        scopes={"chapter"},
                        before=value,
                        after=dst_tags.get(key, ""),
                        chapter_index=n,
                    ):
                        continue
                    raise RebuildError(f"Chapter {n} metadata {key!r} changed: {value!r} -> {dst_tags.get(key)!r}")

    if copy_only and native_validation is None:
        _validate_copy_hdr_profiles(tools, output, video_profiles, selected_real_videos, out_videos)
    elif native_validation is None or not native_validation.get("hdr_validation_complete"):
        # The direct Python engine keeps its complete validation reference. Rust
        # validates each encoded stream's color/HDR/Dolby Vision policy on native
        # application runs before returning the output probe.
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

    copied_hashes_verified = (
        native_validation["copied_hashes_verified"]
        if native_validation is not None
        else (verify_copy_hashes and not test_mode)
    )
    return ValidationResult(output=output, output_probe=out_probe, copied_hashes_verified=copied_hashes_verified)
