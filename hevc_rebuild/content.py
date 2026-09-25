from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

from .metadata import nfo_path_for


def _split_genres(value: str) -> list[str]:
    return [x.strip() for x in re.split(r"[,;/|]", value) if x.strip()]


def metadata_genres(path: Path, mediainfo: dict[str, Any]) -> set[str]:
    genres: set[str] = set()
    nfo = nfo_path_for(path)
    if nfo:
        try:
            root = ET.parse(nfo).getroot()
            for node in root.findall("genre"):
                if node.text:
                    genres.update(x.lower() for x in _split_genres(node.text))
        except (ET.ParseError, OSError):
            # NFO parsing for naming/tags remains strict later. Tune detection itself
            # must never make the encode fail or guess from malformed metadata.
            pass

    for track in ((mediainfo.get("media") or {}).get("track") or []):
        if str(track.get("@type", "")).lower() != "general":
            continue
        for key in ("Genre", "Genres"):
            value = track.get(key)
            if value:
                genres.update(x.lower() for x in _split_genres(str(value)))
    return genres


def resolve_tune(path: Path, mediainfo: dict[str, Any], requested: str, detect_animation: bool) -> str | None:
    requested = (requested or "auto").lower()
    if requested == "none":
        return None
    if requested in {"animation", "grain"}:
        return requested
    if requested != "auto":
        return None
    if detect_animation and "animation" in metadata_genres(path, mediainfo):
        return "animation"
    return None
