from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass(slots=True)
class Toolchain:
    ffmpeg: str
    ffprobe: str
    mediainfo: str
    mkvpropedit: str
    mkvmerge: str
    mkvextract: str
    hdr10plus_tool: str
    x265_pixel_formats: set[str] = field(default_factory=set)
    x265_dolbyvision: bool = False


@dataclass(slots=True)
class StreamRef:
    index: int
    type_ordinal: int
    codec_type: str
    codec_name: str | None = None
    width: int = 0
    height: int = 0
    pix_fmt: str | None = None
    bit_depth: int = 0
    default: bool = False
    attached_pic: bool = False
    tags: dict[str, Any] = field(default_factory=dict)
    disposition: dict[str, Any] = field(default_factory=dict)

    @property
    def pixels(self) -> int:
        return self.width * self.height

    @property
    def label(self) -> str:
        if self.codec_type == "video":
            return f"#{self.index} v:{self.type_ordinal} {self.width}x{self.height} {self.codec_name or '?'}"
        return f"#{self.index} {self.codec_type}:{self.type_ordinal} {self.codec_name or '?'}"


@dataclass(slots=True)
class MediaProfile:
    path: Path
    probe: dict[str, Any]
    mediainfo: dict[str, Any]
    primary_video: StreamRef
    secondary_videos: list[StreamRef]
    audio: list[StreamRef]
    subtitles: list[StreamRef]
    attachments: list[StreamRef]
    data: list[StreamRef]
    chapters_count: int
    duration: float
    source_bit_depth: int
    source_chroma: str
    target_pix_fmt: str
    color_primaries: str | None
    transfer: str | None
    color_matrix: str | None
    color_range_x265: str | None
    color_range_ffmpeg: str | None
    chroma_location: int | None
    master_display: str | None
    master_display_source: str | None
    max_cll: int | None
    max_fall: int | None
    hdr10: bool
    hdr10plus: bool
    hdr10plus_hint: bool
    hlg: bool
    dolby_vision: bool
    dovi_profile: int | None
    dovi_level: int | None
    dovi_rpu_present: bool
    dovi_el_present: bool
    dovi_bl_present: bool
    dovi_compatibility_id: int | None
    dovi_source: str | None
    interlaced: bool
    format_level: str | None


@dataclass(slots=True)
class MetadataResult:
    xml_path: Path
    title: str
    nfo_path: Path | None
    poster_path: Path | None
    expected_global_tags: dict[str, str] = field(default_factory=dict)
    scrubbed_text_values: list[dict[str, Any]] = field(default_factory=list)
    chapter_text_edits: list[dict[str, Any]] = field(default_factory=list)


@dataclass(slots=True)
class ValidationResult:
    output: Path
    output_probe: dict[str, Any]
    copied_hashes_verified: bool
