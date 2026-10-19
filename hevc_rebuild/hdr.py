from __future__ import annotations

import logging
import os
import time
import re
import subprocess
import tempfile
from pathlib import Path

from .model import MediaProfile, Toolchain
from .util import MAX_HELPER_TIMEOUT_SECONDS, RebuildError, format_command, native_json_action, terminate_process_group


_HDR10PLUS_DETECTED = re.compile(
    r"(?:dynamic\s+hdr10\+\s+metadata|hdr10\+.*metadata).*detected",
    re.I,
)


def _hevc_pipe(tools: Toolchain, profile: MediaProfile) -> list[str]:
    return [
        tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(profile.path),
        "-map", f"0:{profile.primary_video.index}", "-c", "copy", "-bsf:v", "hevc_mp4toannexb", "-f", "hevc", "-",
    ]


def _pipe_close_is_expected(stderr: str) -> bool:
    """Identify FFmpeg's normal SIGPIPE-style noise when the verifier exits first.

    hdr10plus_tool is allowed to stop reading once it has reached its conclusion. When
    that happens FFmpeg can report a non-zero status because stdout was closed by the
    consumer. That is a pipeline shutdown detail, not evidence that the media is bad.
    """
    text = (stderr or "").lower()
    if "broken pipe" not in text:
        return False
    suspicious = (
        "invalid data found",
        "error while decoding",
        "corrupt",
        "input/output error",
        "no such file",
        "permission denied",
    )
    return not any(token in text for token in suspicious)


def _producer_stderr_file():
    # A temporary file avoids a stderr PIPE deadlock while also keeping expected
    # Broken-pipe chatter out of the console and normal debug log stream.
    return tempfile.TemporaryFile(mode="w+b")


def _read_stderr_file(handle) -> str:
    handle.flush()
    handle.seek(0)
    return handle.read().decode("utf-8", errors="replace")


def _native_hdr_tool_payload(tools: Toolchain) -> dict[str, object]:
    return {
        "tools": {
            "ffmpeg": tools.ffmpeg,
            "hdr10plus_tool": tools.hdr10plus_tool,
            "x265_dolbyvision": tools.x265_dolbyvision,
        }
    }


def _native_dolbyvision_profile(profile: MediaProfile) -> dict[str, object]:
    return {
        "dovi_profile": getattr(profile, "dovi_profile", None),
        "dovi_rpu_present": bool(getattr(profile, "dovi_rpu_present", False)),
        "dovi_el_present": bool(getattr(profile, "dovi_el_present", False)),
        "dovi_bl_present": bool(getattr(profile, "dovi_bl_present", False)),
        "hdr10": bool(getattr(profile, "hdr10", False)),
        "interlaced": bool(getattr(profile, "interlaced", False)),
    }


def _native_reasons(payload: dict[str, object]) -> list[str] | None:
    response = native_json_action("hdr-metadata", payload, timeout=15.0)
    if response is None:
        return None
    reasons = response.get("reasons")
    if not isinstance(reasons, list) or not all(isinstance(reason, str) for reason in reasons):
        raise RebuildError("The Rust HDR metadata helper returned invalid Dolby Vision reasons.")
    return reasons


def _run_hdr_pipeline(ffmpeg: list[str], parser: list[str], fferr):
    started = time.monotonic()
    p1 = subprocess.Popen(ffmpeg, stdout=subprocess.PIPE, stderr=fferr, start_new_session=os.name == "posix")
    p2 = None
    try:
        p2 = subprocess.Popen(parser, stdin=p1.stdout, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=os.name == "posix")
        assert p1.stdout is not None
        p1.stdout.close()
        out2, err2 = p2.communicate(timeout=MAX_HELPER_TIMEOUT_SECONDS)
        rc1 = p1.wait(timeout=max(0.1, MAX_HELPER_TIMEOUT_SECONDS - (time.monotonic() - started)))
        return out2, err2, rc1, p2.returncode
    except BaseException as exc:
        if p2 is not None:
            terminate_process_group(p2)
        terminate_process_group(p1)
        if isinstance(exc, (OSError, subprocess.TimeoutExpired)):
            raise RebuildError(f"HDR10+ pipeline failed ({ffmpeg[0]} / {parser[0]}): {exc}") from exc
        raise
    finally:
        for proc in (p1, p2):
            if proc is not None:
                for stream in (proc.stdout, proc.stderr):
                    if stream is not None:
                        stream.close()


def verify_hdr10plus(tools: Toolchain, profile: MediaProfile, path_override: Path | None = None) -> bool:
    # For an encoded output where primary video is guaranteed v:0, use a lightweight
    # path override rather than rebuilding the source profile.
    if path_override is not None:
        ffmpeg = [
            tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-nostdin", "-i", str(path_override),
            "-map", "0:v:0", "-c", "copy", "-bsf:v", "hevc_mp4toannexb", "-f", "hevc", "-",
        ]
    else:
        if (profile.primary_video.codec_name or "").lower() not in {"hevc", "h265"}:
            return False
        ffmpeg = _hevc_pipe(tools, profile)

    native_payload = {
        "operation": "verify_hdr10plus",
        **_native_hdr_tool_payload(tools),
        "input_path": str(profile.path),
        "stream_index": profile.primary_video.index,
        "codec_name": profile.primary_video.codec_name,
        "path_override": str(path_override) if path_override is not None else None,
    }
    native = native_json_action("hdr-metadata", native_payload, timeout=None)
    if native is not None:
        verified = native.get("verified")
        if not isinstance(verified, bool):
            raise RebuildError("The Rust HDR metadata helper returned an invalid verification result.")
        return verified

    parser = [tools.hdr10plus_tool, "--verify", "extract", "-"]
    logging.debug("HDR10+ verify pipeline: %s | %s", format_command(ffmpeg), format_command(parser))

    with _producer_stderr_file() as fferr:
        out2, err2, rc1, parser_status = _run_hdr_pipeline(ffmpeg, parser, fferr)
        err1 = _read_stderr_file(fferr)

    parser_text = (out2 or "") + "\n" + (err2 or "")
    detected = bool(_HDR10PLUS_DETECTED.search(parser_text))
    expected_pipe_close = rc1 != 0 and _pipe_close_is_expected(err1)

    if parser_status == 0 and detected:
        if rc1 != 0 and not expected_pipe_close:
            logging.debug("HDR10+ verifier detected metadata, but FFmpeg failed unexpectedly: %s", err1.strip())
            return False
        if expected_pipe_close:
            logging.debug("HDR10+ verifier finished before FFmpeg producer; expected pipe closure suppressed.")
        return True

    # A verifier that concludes early can close stdin and make FFmpeg emit Broken pipe.
    # Keep that expected pipeline detail out of the alarming diagnostics.
    if rc1 != 0:
        if expected_pipe_close:
            logging.debug("HDR10+ verifier closed the stream after reaching its conclusion; no HDR10+ metadata verified.")
        else:
            logging.debug("HDR10+ verify FFmpeg failed: %s", err1.strip())
    if parser_status != 0:
        logging.debug("HDR10+ verification reported no usable metadata: %s", parser_text.strip())
    elif not detected:
        logging.debug("HDR10+ verifier completed without an explicit metadata-detected result.")
    return False


def extract_hdr10plus(tools: Toolchain, profile: MediaProfile, output_json: Path) -> None:
    if (profile.primary_video.codec_name or "").lower() not in {"hevc", "h265"}:
        raise RebuildError("HDR10+ preservation currently requires an HEVC source video stream.")
    native_payload = {
        "operation": "extract_hdr10plus",
        **_native_hdr_tool_payload(tools),
        "input_path": str(profile.path),
        "stream_index": profile.primary_video.index,
        "codec_name": profile.primary_video.codec_name,
        "output_json": str(output_json),
    }
    native = native_json_action("hdr-metadata", native_payload, timeout=None)
    if native is not None:
        if native.get("extracted") is not True:
            raise RebuildError("The Rust HDR metadata helper did not confirm extraction.")
        return
    ffmpeg = _hevc_pipe(tools, profile)
    parser = [tools.hdr10plus_tool, "extract", "-o", str(output_json), "-"]
    logging.info("Extracting HDR10+ dynamic metadata from selected primary video stream.")

    with _producer_stderr_file() as fferr:
        out2, err2, rc1, parser_status = _run_hdr_pipeline(ffmpeg, parser, fferr)
        err1 = _read_stderr_file(fferr)

    if rc1 != 0 or parser_status != 0 or not output_json.exists() or output_json.stat().st_size < 16:
        if rc1 != 0 and _pipe_close_is_expected(err1) and parser_status != 0:
            ffmpeg_detail = "FFmpeg producer stopped after hdr10plus_tool closed the pipe."
        else:
            ffmpeg_detail = err1.strip()
        raise RebuildError(
            "HDR10+ extraction failed; refusing to continue because dynamic metadata would be lost.\n"
            f"FFmpeg: {ffmpeg_detail}\nhdr10plus_tool: {(err2 or out2).strip()}"
        )


def dolbyvision_safe_preservation_reasons(
    profile: MediaProfile,
    tools: Toolchain,
    *,
    force_deinterlace: bool = False,
) -> list[str]:
    """Return reasons the source cannot use native safe Dolby Vision preservation."""
    native = _native_reasons({
        "operation": "dolbyvision_safe_reasons",
        **_native_hdr_tool_payload(tools),
        "profile": _native_dolbyvision_profile(profile),
        "force_deinterlace": force_deinterlace,
    })
    if native is not None:
        return native
    reasons: list[str] = []
    if profile.dovi_profile != 8:
        reasons.append(f"Profile {profile.dovi_profile or 'unknown'} is not the safe single-layer Profile 8 path")
    if not profile.dovi_rpu_present:
        reasons.append("RPU presence was not verified")
    if profile.dovi_el_present:
        reasons.append("an enhancement layer is present")
    if not profile.dovi_bl_present:
        reasons.append("base-layer presence was not verified")
    if not tools.x265_dolbyvision:
        reasons.append("installed FFmpeg/libx265 does not expose native Dolby Vision RPU transcoding")
    if profile.interlaced or force_deinterlace:
        reasons.append("video filtering/deinterlacing would make per-frame RPU preservation unsafe")
    return reasons


def dolbyvision_hdr10_fallback_reasons(profile: MediaProfile) -> list[str]:
    """Return reasons an intentional DV -> HDR10 base-layer fallback is not safe enough.

    This path intentionally discards Dolby Vision metadata/enhancement data. It is only
    allowed when FFprobe verified a base layer and the selected video independently
    identifies as HDR10. Profiles outside the known Profile 7/8 families are rejected
    rather than guessed.
    """
    native = _native_reasons({
        "operation": "dolbyvision_fallback_reasons",
        "profile": _native_dolbyvision_profile(profile),
    })
    if native is not None:
        return native
    reasons: list[str] = []
    if profile.dovi_profile not in {7, 8}:
        reasons.append(f"Dolby Vision Profile {profile.dovi_profile or 'unknown'} is not approved for this HDR10 fallback path")
    if not profile.dovi_bl_present:
        reasons.append("FFprobe did not verify a Dolby Vision base layer")
    if not profile.hdr10:
        reasons.append("the selected video was not independently identified as HDR10-compatible")
    return reasons
