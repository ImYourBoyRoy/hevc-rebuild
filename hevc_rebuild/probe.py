from __future__ import annotations

import json
import logging
import math
import re
import subprocess
from pathlib import Path
from typing import Any

from .model import MediaProfile, StreamRef, Toolchain
from .util import RebuildError, native_json_action, run, run_json, safe_float, safe_int


def ffprobe_json(tools: Toolchain, path: Path) -> dict[str, Any]:
    return run_json([
        tools.ffprobe, "-v", "error", "-show_streams", "-show_format", "-show_chapters",
        "-show_data_hash", "sha256", "-of", "json", str(path),
    ])


def mediainfo_json(tools: Toolchain, path: Path) -> dict[str, Any]:
    cp = run([tools.mediainfo, "--Output=JSON", str(path)])
    try:
        return json.loads(cp.stdout)
    except json.JSONDecodeError as exc:
        raise RebuildError(f"MediaInfo produced invalid JSON for {path}: {exc}") from exc


def stream_refs(probe: dict[str, Any]) -> list[StreamRef]:
    ordinals: dict[str, int] = {}
    result: list[StreamRef] = []
    for s in probe.get("streams", []):
        kind = str(s.get("codec_type", "unknown"))
        ordinal = ordinals.get(kind, 0)
        ordinals[kind] = ordinal + 1
        disp = s.get("disposition") or {}
        tags = s.get("tags") or {}
        bit_depth = safe_int(s.get("bits_per_raw_sample")) or safe_int(s.get("bits_per_sample")) or 0
        pix = str(s.get("pix_fmt") or "")
        if not bit_depth:
            if "12" in pix:
                bit_depth = 12
            elif "10" in pix:
                bit_depth = 10
            elif kind == "video":
                bit_depth = 8
        result.append(StreamRef(
            index=int(s.get("index", -1)),
            type_ordinal=ordinal,
            codec_type=kind,
            codec_name=s.get("codec_name"),
            width=int(s.get("width") or 0),
            height=int(s.get("height") or 0),
            pix_fmt=s.get("pix_fmt"),
            bit_depth=bit_depth,
            default=bool(disp.get("default", 0)),
            attached_pic=bool(disp.get("attached_pic", 0)),
            tags=tags,
            disposition=disp,
        ))
    return result


def choose_primary_video(refs: list[StreamRef], preferred_index: int | None = None) -> tuple[StreamRef, list[StreamRef]]:
    videos = [s for s in refs if s.codec_type == "video" and not s.attached_pic and s.width > 0 and s.height > 0]
    if not videos:
        raise RebuildError("No usable video stream was found.")
    if preferred_index is not None:
        selected = next((stream for stream in videos if stream.index == preferred_index), None)
        if selected is None:
            raise RebuildError(f"Selected video stream #{preferred_index} is not a usable video stream.")
        return selected, [stream for stream in videos if stream.index != selected.index]
    # Resolution dominates. This explicitly makes 3840x2160 win over 1920x1080 even if 1080p is marked default.
    selected = max(videos, key=lambda s: (s.pixels, s.width, s.height, int(s.default), s.bit_depth, -s.index))
    secondary = [s for s in videos if s.index != selected.index]
    return selected, secondary


def _selected_media_video(mi: dict[str, Any], primary: StreamRef) -> dict[str, Any]:
    """Match MediaInfo's video track to the FFprobe-selected stream.

    Resolution-only matching is not sufficient when a file contains two same-resolution
    tracks (for example SDR/HDR variants). Prefer explicit stream order, then video-track
    ordinal, and use dimensions only as a final fallback.
    """
    tracks = ((mi.get("media") or {}).get("track") or [])
    videos = [t for t in tracks if str(t.get("@type", "")).lower() == "video"]
    if not videos:
        return {}

    def parse_intish(v: Any) -> int | None:
        text = str(v or "").strip().replace(" ", "").replace(",", "")
        m = re.search(r"-?\d+", text)
        return int(m.group()) if m else None

    def dims(track: dict[str, Any]) -> tuple[int, int]:
        return parse_intish(track.get("Width")) or 0, parse_intish(track.get("Height")) or 0

    # MediaInfo StreamOrder normally tracks container stream order and therefore FFprobe index.
    for track in videos:
        if parse_intish(track.get("StreamOrder")) == primary.index:
            return track

    # Video-track order is the next strongest association available across containers.
    if 0 <= primary.type_ordinal < len(videos):
        ordinal_match = videos[primary.type_ordinal]
        if dims(ordinal_match) == (primary.width, primary.height):
            return ordinal_match

    exact = [t for t in videos if dims(t) == (primary.width, primary.height)]
    if len(exact) == 1:
        return exact[0]
    if exact:
        # Prefer a codec-compatible exact-resolution track when possible.
        codec = (primary.codec_name or "").lower()
        codec_aliases = {
            "hevc": {"hevc", "h.265", "h265"},
            "h264": {"avc", "h.264", "h264"},
            "av1": {"av1"},
            "vp9": {"vp9"},
        }.get(codec, {codec})
        compatible = [t for t in exact if str(t.get("Format") or "").strip().lower() in codec_aliases]
        if compatible:
            return compatible[0]
        return exact[0]
    return max(videos, key=lambda t: dims(t)[0] * dims(t)[1])


def _norm_primaries(value: Any) -> str | None:
    if value is None:
        return None
    v = str(value).strip().lower()
    mappings = {
        "bt.709": "bt709", "bt709": "bt709", "bt.2020": "bt2020", "bt2020": "bt2020",
        "bt.601 ntsc": "smpte170m", "smpte170m": "smpte170m", "bt.601 pal": "bt470bg",
        "bt470bg": "bt470bg", "bt.470 system m": "bt470m", "bt470m": "bt470m",
        "generic film": "film", "film": "film", "display p3": "smpte432", "dci p3": "smpte431",
    }
    if v in {"unknown", "unspecified", ""}:
        return None
    return mappings.get(v, v)


def _norm_transfer(value: Any) -> str | None:
    if value is None:
        return None
    v = str(value).strip().lower()
    mappings = {
        "pq": "smpte2084", "smpte2084": "smpte2084", "hlg": "arib-std-b67",
        "arib-std-b67": "arib-std-b67", "bt.709": "bt709", "bt709": "bt709",
        "bt.470 system b/g": "bt470bg", "bt470bg": "bt470bg", "bt.470 system m": "bt470m",
        "bt470m": "bt470m", "smpte170m": "smpte170m", "linear": "linear",
        "iec61966-2-1": "iec61966-2-1", "srgb": "iec61966-2-1",
    }
    if v in {"unknown", "unspecified", ""}:
        return None
    return mappings.get(v, v)


def _norm_matrix(value: Any) -> str | None:
    if value is None:
        return None
    v = str(value).strip().lower()
    if "bt.2020" in v and "non-constant" in v:
        return "bt2020nc"
    if "bt.2020" in v and "constant" in v:
        return "bt2020c"
    mappings = {
        "bt2020nc": "bt2020nc", "bt2020c": "bt2020c", "bt.709": "bt709", "bt709": "bt709",
        "bt.601": "smpte170m", "smpte170m": "smpte170m", "bt.470 system b/g": "bt470bg",
        "bt470bg": "bt470bg", "fcc": "fcc", "ycgco": "ycgco", "gbr": "gbr", "rgb": "gbr",
    }
    if v in {"unknown", "unspecified", ""}:
        return None
    return mappings.get(v, v)


def _color_range(value: Any) -> tuple[str | None, str | None]:
    v = str(value or "").strip().lower()
    if v in {"tv", "limited"}:
        return "limited", "tv"
    if v in {"pc", "full"}:
        return "full", "pc"
    return None, None


def _target_pix_fmt(source_depth: int, source_chroma: str, promote_8bit_to_10bit: bool = False) -> str:
    # Preserve source precision by default. Optional 8->10 promotion is an explicit
    # compression/banding trade-off; it does not create color information absent
    # from an 8-bit source. Never silently reduce 10/12-bit sources.
    if source_depth not in {8, 10, 12}:
        raise RebuildError(
            f"Source bit depth {source_depth} cannot be reproduced exactly by this x265 workflow; "
            "refusing a silent precision change."
        )
    depth = 10 if source_depth == 8 and promote_8bit_to_10bit else source_depth
    chroma = "444" if "4:4:4" in source_chroma else "422" if "4:2:2" in source_chroma else "420"
    return f"yuv{chroma}p" if depth == 8 else f"yuv{chroma}p{depth}le"


def _frame_probe(tools: Toolchain, path: Path, video_ordinal: int) -> dict[str, Any]:
    return run_json([
        tools.ffprobe, "-v", "error", "-select_streams", f"v:{video_ordinal}", "-read_intervals", "%+1",
        "-show_frames", "-show_entries",
        "frame=color_range,color_space,color_primaries,color_transfer,interlaced_frame:frame_side_data",
        "-of", "json", str(path),
    ])


def _side_data_entries(stream: dict[str, Any], frame_probe: dict[str, Any]) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    result.extend(stream.get("side_data_list") or [])
    for frame in frame_probe.get("frames", []) or []:
        result.extend(frame.get("side_data_list") or [])
    return [x for x in result if isinstance(x, dict)]


def _side_data(side: list[dict[str, Any]], pattern: str) -> dict[str, Any] | None:
    rx = re.compile(pattern, re.I)
    return next((x for x in side if rx.search(str(x.get("side_data_type", "")))), None)


def _master_display_from_side(data: dict[str, Any] | None) -> str | None:
    if not data:
        return None
    names = ["red_x", "red_y", "green_x", "green_y", "blue_x", "blue_y", "white_point_x", "white_point_y", "min_luminance", "max_luminance"]
    values = {n: safe_float(data.get(n)) for n in names}
    if any(values[n] is None for n in names):
        return None
    def xy(x: str, y: str) -> str:
        return f"{round(values[x] * 50000)},{round(values[y] * 50000)}"
    return (
        f"G({xy('green_x','green_y')})B({xy('blue_x','blue_y')})R({xy('red_x','red_y')})"
        f"WP({xy('white_point_x','white_point_y')})"
        f"L({round(values['max_luminance'] * 10000)},{round(values['min_luminance'] * 10000)})"
    )


def _fallback_master_display(mi_video: dict[str, Any]) -> str | None:
    label = str(mi_video.get("MasteringDisplay_ColorPrimaries") or "")
    if not label:
        return None
    lum = str(mi_video.get("MasteringDisplay_Luminance") or "")
    min_l, max_l = 0.005, 1000.0
    m_min = re.search(r"min:\s*([0-9.]+)", lum, re.I)
    m_max = re.search(r"max:\s*([0-9.]+)", lum, re.I)
    if m_min:
        min_l = float(m_min.group(1))
    if m_max:
        max_l = float(m_max.group(1))
    light = f"L({round(max_l * 10000)},{round(min_l * 10000)})"
    if re.search(r"BT\.?2020", label, re.I):
        return f"G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450){light}"
    if re.search(r"Display P3", label, re.I):
        return f"G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450){light}"
    if re.search(r"BT\.?709", label, re.I):
        return f"G(15000,30000)B(7500,3000)R(32000,16500)WP(15635,16450){light}"
    return None


def _cd_int(value: Any) -> int | None:
    if value is None:
        return None
    m = re.search(r"([0-9]+(?:\.[0-9]+)?)", str(value))
    return round(float(m.group(1))) if m else None




def _dovi_configuration(side: list[dict[str, Any]], mi_video: dict[str, Any]) -> tuple[int | None, int | None, bool, bool, bool, int | None, str | None]:
    data = _side_data(side, r"DOVI configuration record|Dolby Vision configuration")
    if data:
        profile = safe_int(data.get("dv_profile"))
        level = safe_int(data.get("dv_level"))
        rpu = bool(safe_int(data.get("rpu_present_flag")) or 0)
        el = bool(safe_int(data.get("el_present_flag")) or 0)
        bl = bool(safe_int(data.get("bl_present_flag")) or 0)
        compat = safe_int(data.get("dv_bl_signal_compatibility_id"))
        return profile, level, rpu, el, bl, compat, "FFprobe DOVI configuration record"

    # MediaInfo fallback is intentionally conservative. It can establish that Dolby
    # Vision is advertised, and sometimes its profile/level, but without FFprobe's
    # configuration record we do not claim EL/BL/RPU flags.
    text = " ".join(str(mi_video.get(k) or "") for k in (
        "HDR_Format", "HDR_Format_Profile", "HDR_Format_Compatibility", "HDR_Format_Settings"
    ))
    if not re.search(r"Dolby Vision|(?:dvhe|dvh1)[.]", text, re.I):
        return None, None, False, False, False, None, None
    m = re.search(r"(?:dvhe|dvh1)[.]0?(\d+)[.]0?(\d+)", text, re.I)
    if m:
        return int(m.group(1)), int(m.group(2)), False, False, False, None, "MediaInfo Dolby Vision metadata"
    m = re.search(r"Dolby Vision[^\n]*?Profile\s*(\d+)", text, re.I)
    return (int(m.group(1)) if m else None), None, False, False, False, None, "MediaInfo Dolby Vision metadata"

def _chroma_location(stream: dict[str, Any], mi_video: dict[str, Any]) -> int | None:
    loc = str(stream.get("chroma_location") or "").lower()
    mapping = {"left": 0, "center": 1, "topleft": 2, "top-left": 2, "top": 3, "bottomleft": 4, "bottom-left": 4, "bottom": 5}
    if loc in mapping:
        return mapping[loc]
    text = str(mi_video.get("ChromaSubsampling_Position") or "")
    m = re.search(r"\b([0-5])\b", text)
    return int(m.group(1)) if m else None


def build_profile(
    tools: Toolchain,
    path: Path,
    *,
    promote_8bit_to_10bit: bool = False,
    video_stream_index: int | None = None,
    probe_data: dict[str, Any] | None = None,
    mediainfo_data: dict[str, Any] | None = None,
) -> MediaProfile:
    probe = probe_data if probe_data is not None else ffprobe_json(tools, path)
    refs = stream_refs(probe)
    primary, secondary = choose_primary_video(refs, preferred_index=video_stream_index)
    mi = mediainfo_data if mediainfo_data is not None else mediainfo_json(tools, path)
    mi_video = _selected_media_video(mi, primary)

    streams_by_index = {int(s.get("index", -1)): s for s in probe.get("streams", [])}
    primary_raw = streams_by_index[primary.index]
    frame_probe = _frame_probe(tools, path, primary.type_ordinal)
    first_frame = (frame_probe.get("frames") or [{}])[0] if frame_probe.get("frames") else {}
    side = _side_data_entries(primary_raw, frame_probe)

    duration = safe_float((probe.get("format") or {}).get("duration")) or safe_float(primary_raw.get("duration")) or safe_float(mi_video.get("Duration"))
    if not duration or duration <= 0:
        raise RebuildError(f"Unable to determine duration for {path}")

    bit_text = str(mi_video.get("BitDepth") or "")
    bit_match = re.search(r"(8|10|12|14|16)", bit_text)
    source_depth = (int(bit_match.group(1)) if bit_match else None) or primary.bit_depth or 8
    chroma = str(mi_video.get("ChromaSubsampling") or "")
    if not chroma:
        pix = primary.pix_fmt or ""
        chroma = "4:4:4" if "444" in pix else "4:2:2" if "422" in pix else "4:2:0"
    target = _target_pix_fmt(source_depth, chroma, promote_8bit_to_10bit)

    prim = _norm_primaries(primary_raw.get("color_primaries") or first_frame.get("color_primaries") or mi_video.get("colour_primaries"))
    transfer = _norm_transfer(primary_raw.get("color_transfer") or first_frame.get("color_transfer") or mi_video.get("transfer_characteristics"))
    matrix = _norm_matrix(primary_raw.get("color_space") or first_frame.get("color_space") or mi_video.get("matrix_coefficients"))
    range_x265, range_ffmpeg = _color_range(primary_raw.get("color_range") or first_frame.get("color_range") or mi_video.get("colour_range"))

    master_side = _side_data(side, r"Mastering display metadata")
    master = _master_display_from_side(master_side)
    master_source = "ffprobe-side-data" if master else None
    if not master:
        master = _fallback_master_display(mi_video)
        master_source = "MediaInfo-label-fallback" if master else None
    light = _side_data(side, r"Content light level metadata")
    max_cll = _cd_int(light.get("max_content")) if light else None
    max_fall = _cd_int(light.get("max_average")) if light else None
    max_cll = max_cll if max_cll is not None else _cd_int(mi_video.get("MaxCLL"))
    max_fall = max_fall if max_fall is not None else _cd_int(mi_video.get("MaxFALL"))

    hdr_text = f"{mi_video.get('HDR_Format','')} {mi_video.get('HDR_Format_Compatibility','')}"
    side_text = " ".join(str(x.get("side_data_type", "")) for x in side)
    hdr10plus_hint = bool(re.search(r"HDR10\+|SMPTE2094-40|HDR Dynamic Metadata", hdr_text + " " + side_text, re.I))
    dovi_profile, dovi_level, dovi_rpu, dovi_el, dovi_bl, dovi_compat, dovi_source = _dovi_configuration(side, mi_video)
    # Only explicit Dolby Vision evidence counts. A generic token such as "RPU" is
    # not sufficient by itself because that can produce false positives in unrelated
    # side-data text.
    dolby = dovi_source is not None or bool(re.search(r"Dolby Vision|DOVI configuration", hdr_text + " " + side_text, re.I))
    hdr10 = (transfer == "smpte2084" and prim == "bt2020") or bool(re.search(r"HDR10", hdr_text, re.I))
    hlg = transfer == "arib-std-b67" or bool(re.search(r"\bHLG\b", hdr_text, re.I))

    scan = str(mi_video.get("ScanType") or "")
    field_order = str(primary_raw.get("field_order") or "")
    interlaced = bool(re.search(r"interlaced", scan, re.I)) or (field_order not in {"", "progressive", "unknown"})
    if not interlaced:
        interlaced = any(int(f.get("interlaced_frame") or 0) == 1 for f in frame_probe.get("frames", []) or [])

    audio = [s for s in refs if s.codec_type == "audio"]
    subtitles = [s for s in refs if s.codec_type == "subtitle"]
    attachments = [s for s in refs if s.codec_type == "attachment"]
    data = [s for s in refs if s.codec_type == "data"]

    return MediaProfile(
        path=path, probe=probe, mediainfo=mi, primary_video=primary, secondary_videos=secondary,
        audio=audio, subtitles=subtitles, attachments=attachments, data=data,
        chapters_count=len(probe.get("chapters") or []), duration=float(duration),
        source_bit_depth=int(source_depth), source_chroma=chroma, target_pix_fmt=target,
        color_primaries=prim, transfer=transfer, color_matrix=matrix,
        color_range_x265=range_x265, color_range_ffmpeg=range_ffmpeg,
        chroma_location=_chroma_location(primary_raw, mi_video),
        master_display=master, master_display_source=master_source,
        max_cll=max_cll, max_fall=max_fall,
        hdr10=hdr10, hdr10plus=False, hdr10plus_hint=hdr10plus_hint, hlg=hlg,
        dolby_vision=dolby, dovi_profile=dovi_profile, dovi_level=dovi_level,
        dovi_rpu_present=dovi_rpu, dovi_el_present=dovi_el, dovi_bl_present=dovi_bl,
        dovi_compatibility_id=dovi_compat, dovi_source=dovi_source, interlaced=interlaced,
        format_level=str(mi_video.get("Format_Level") or "") or None,
    )


def _native_media_profile(
    source: Path,
    raw_probe: dict[str, Any],
    media_info: dict[str, Any],
    data: dict[str, Any],
) -> MediaProfile:
    video = data.get("video")
    hdr = data.get("hdr")
    if not isinstance(video, dict) or not isinstance(hdr, dict):
        raise RebuildError("The Rust source-profile helper omitted video or HDR metadata.")
    for name, value in (
        ("hdr10", hdr.get("hdr10")),
        ("hdr10plus_hint", hdr.get("hdr10plus_hint")),
        ("hlg", hdr.get("hlg")),
        ("dolby_vision", hdr.get("dolby_vision")),
        ("dolby_vision_rpu", hdr.get("dolby_vision_rpu")),
        ("dolby_vision_enhancement_layer", hdr.get("dolby_vision_enhancement_layer")),
        ("dolby_vision_base_layer", hdr.get("dolby_vision_base_layer")),
        ("interlaced", data.get("interlaced")),
    ):
        if not isinstance(value, bool):
            raise RebuildError(f"The Rust source-profile helper returned an invalid {name} flag.")
    index = data.get("primary_stream_index")
    if not isinstance(index, int) or isinstance(index, bool):
        raise RebuildError("The Rust source-profile helper returned an invalid video stream index.")
    refs = stream_refs(raw_probe)
    streams_by_index = {stream.index: stream for stream in refs}
    primary = streams_by_index.get(index)
    if (
        primary is None
        or primary.codec_type != "video"
        or primary.attached_pic
        or primary.width <= 0
        or primary.height <= 0
    ):
        raise RebuildError("The Rust source-profile helper selected an unusable video stream.")
    duration = data.get("duration_seconds")
    depth = video.get("source_bit_depth")
    chroma = video.get("source_chroma")
    target = video.get("target_pixel_format")
    if (
        not isinstance(duration, (int, float))
        or isinstance(duration, bool)
        or not math.isfinite(duration)
        or duration <= 0
        or not isinstance(depth, int)
        or isinstance(depth, bool)
        or depth not in {8, 10, 12, 14, 16}
        or not isinstance(chroma, str)
        or not chroma
        or not isinstance(target, str)
        or not target
    ):
        raise RebuildError("The Rust source-profile helper returned invalid video parameters.")
    secondaries = [
        stream
        for stream in refs
        if stream.codec_type == "video"
        and not stream.attached_pic
        and stream.width > 0
        and stream.height > 0
        and stream.index != primary.index
    ]
    optional_integer_fields = {
        "chroma_location": data.get("chroma_location"),
        "max_cll": data.get("max_cll"),
        "max_fall": data.get("max_fall"),
    }
    for field_name, value in optional_integer_fields.items():
        if value is not None and (not isinstance(value, int) or isinstance(value, bool)):
            raise RebuildError(f"The Rust source-profile helper returned an invalid {field_name}.")
    dovi_values = {
        "dovi_profile": hdr.get("dolby_vision_profile"),
        "dovi_level": hdr.get("dolby_vision_level"),
        "dovi_compatibility_id": hdr.get("dolby_vision_compatibility_id"),
    }
    for field_name, value in dovi_values.items():
        if value is not None and (not isinstance(value, int) or isinstance(value, bool)):
            raise RebuildError(f"The Rust source-profile helper returned an invalid {field_name}.")

    def optional_text(value: object) -> str | None:
        if value is None:
            return None
        return str(value)

    return MediaProfile(
        path=source,
        probe=raw_probe,
        mediainfo=media_info,
        primary_video=primary,
        secondary_videos=secondaries,
        audio=[stream for stream in refs if stream.codec_type == "audio"],
        subtitles=[stream for stream in refs if stream.codec_type == "subtitle"],
        attachments=[stream for stream in refs if stream.codec_type == "attachment"],
        data=[stream for stream in refs if stream.codec_type == "data"],
        chapters_count=len(raw_probe.get("chapters") or []),
        duration=float(duration),
        source_bit_depth=depth,
        source_chroma=chroma,
        target_pix_fmt=target,
        color_primaries=optional_text(data.get("color_primaries")),
        transfer=optional_text(data.get("color_transfer")),
        color_matrix=optional_text(data.get("color_matrix")),
        color_range_x265=optional_text(data.get("color_range_x265")),
        color_range_ffmpeg=optional_text(data.get("color_range_ffmpeg")),
        chroma_location=optional_integer_fields["chroma_location"],
        master_display=optional_text(data.get("mastering_display")),
        master_display_source=optional_text(data.get("mastering_display_source")),
        max_cll=optional_integer_fields["max_cll"],
        max_fall=optional_integer_fields["max_fall"],
        hdr10=hdr["hdr10"],
        hdr10plus=False,
        hdr10plus_hint=hdr["hdr10plus_hint"],
        hlg=hdr["hlg"],
        dolby_vision=hdr["dolby_vision"],
        dovi_profile=dovi_values["dovi_profile"],
        dovi_level=dovi_values["dovi_level"],
        dovi_rpu_present=hdr["dolby_vision_rpu"],
        dovi_el_present=hdr["dolby_vision_enhancement_layer"],
        dovi_bl_present=hdr["dolby_vision_base_layer"],
        dovi_compatibility_id=dovi_values["dovi_compatibility_id"],
        dovi_source=optional_text(hdr.get("dolby_vision_evidence")),
        interlaced=data["interlaced"],
        format_level=optional_text(data.get("format_level")),
    )


def build_native_profile_set(
    tools: Toolchain,
    path: Path,
    *,
    promote_8bit_to_10bit: bool = False,
) -> MediaProfile | None:
    """Use Rust's source analyzer for app-launched rebuild profiles."""
    response = native_json_action(
        "engine-profile",
        {
            "path": str(path.expanduser()),
            "ffprobe": tools.ffprobe,
            "mediainfo": tools.mediainfo,
            "promote_8bit_to_10bit": bool(promote_8bit_to_10bit),
        },
        timeout=300.0,
    )
    if response is None:
        return None
    raw_probe = response.get("probe")
    media_info = response.get("mediainfo")
    profile_data = response.get("profile")
    native_path = response.get("path")
    if (
        not isinstance(raw_probe, dict)
        or not isinstance(media_info, dict)
        or not isinstance(profile_data, dict)
        or not isinstance(native_path, str)
    ):
        raise RebuildError("The Rust source-profile helper returned an incomplete response.")
    return _native_media_profile(Path(native_path), raw_probe, media_info, profile_data)


def build_native_video_profiles(
    tools: Toolchain,
    source_profile: MediaProfile,
    video_stream_indices: list[int],
    *,
    promote_8bit_to_10bit: bool = False,
) -> dict[int, MediaProfile]:
    """Return Rust-derived profiles for just the tracks selected by the job."""
    requested = sorted(set(video_stream_indices))
    if len(requested) != len(video_stream_indices) or any(index < 0 for index in requested):
        raise RebuildError("Selected video stream indices must be unique and nonnegative.")
    profiles: dict[int, MediaProfile] = {}
    remaining = requested
    if source_profile.primary_video.index in requested:
        profiles[source_profile.primary_video.index] = source_profile
        remaining = [index for index in requested if index != source_profile.primary_video.index]
    if not remaining:
        return profiles
    timeout = max(300.0, 60.0 + 30.0 * len(remaining))
    response = native_json_action(
        "engine-profile-tracks",
        {
            "path": str(source_profile.path),
            "ffprobe": tools.ffprobe,
            "promote_8bit_to_10bit": bool(promote_8bit_to_10bit),
            "video_stream_indices": remaining,
            "probe": source_profile.probe,
            "mediainfo": source_profile.mediainfo,
        },
        timeout=timeout,
    )
    if response is None:
        raise RebuildError("The Rust selected-track profiler is unavailable for this app-launched run.")
    video_data = response.get("video_profiles")
    if not isinstance(video_data, dict):
        raise RebuildError("The Rust selected-track profiler returned an invalid response.")
    for raw_index, variant in video_data.items():
        try:
            index = int(raw_index)
        except (TypeError, ValueError) as exc:
            raise RebuildError("The Rust selected-track profiler returned a nonnumeric stream key.") from exc
        if not isinstance(variant, dict) or index in profiles:
            raise RebuildError("The Rust selected-track profiler returned an invalid profile map.")
        video_profile = _native_media_profile(
            source_profile.path,
            source_profile.probe,
            source_profile.mediainfo,
            variant,
        )
        if video_profile.primary_video.index != index:
            raise RebuildError("The Rust selected-track profiler mismatched a video-profile key.")
        profiles[index] = video_profile
    if set(profiles) != set(requested):
        missing = ", ".join(f"#{index}" for index in requested if index not in profiles)
        raise RebuildError(f"The Rust selected-track profiler omitted video stream(s) {missing}.")
    return profiles
