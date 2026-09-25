from __future__ import annotations

import logging
import mimetypes
import re
import shutil
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any

from .model import MediaProfile, MetadataResult, Toolchain
from .util import RebuildError, run, run_json

TAG_MAP = {
    "title": "TITLE", "showtitle": "SHOWTITLE", "season": "SEASON", "episode": "EPISODE",
    "plot": "SYNOPSIS", "tagline": "SUMMARY", "mpaa": "LAW_RATING", "premiered": "DATE_RELEASED",
    "studio": "PRODUCTION_STUDIO", "genre": "GENRE",
}


def _flatten_text(node: ET.Element | None) -> str | None:
    if node is None:
        return None
    values: list[str] = []
    if node.text and node.text.strip():
        values.append(node.text.strip())
    for child in node:
        if child.text and child.text.strip():
            values.append(child.text.strip())
    value = ", ".join(dict.fromkeys(values)).strip()
    return value or None


def nfo_path_for(path: Path) -> Path | None:
    candidate = path.with_suffix(".nfo")
    return candidate if candidate.exists() else None


def _is_global_tag(tag: ET.Element) -> bool:
    targets = tag.find("Targets")
    if targets is None:
        return True
    targeted_ids = {"TrackUID", "ChapterUID", "AttachmentUID", "EditionUID"}
    return not any(child.tag in targeted_ids and (child.text or "").strip() for child in targets)


def _read_existing_global_root(tools: Toolchain, path: Path, work_dir: Path) -> ET.Element:
    """Extract true global Matroska tags so rebuilding does not discard source metadata."""
    root = ET.Element("Tags")
    if path.suffix.lower() != ".mkv":
        return root
    extracted = work_dir / "source-tags.xml"
    cp = run([tools.mkvextract, str(path), "tags", str(extracted)], check=False)
    if cp.returncode != 0 or not extracted.exists() or extracted.stat().st_size == 0:
        return root
    try:
        parsed = ET.parse(extracted).getroot()
    except (ET.ParseError, OSError) as exc:
        logging.warning("Existing Matroska tags could not be parsed and will not be merged: %s", exc)
        return root
    for tag in list(parsed.findall("Tag")):
        if _is_global_tag(tag):
            root.append(tag)
    return root


def _remove_conflicting_simple_tags(root: ET.Element, names: set[str]) -> None:
    wanted = {n.upper() for n in names}
    for tag in root.findall("Tag"):
        if not _is_global_tag(tag):
            continue
        for simple in list(tag.findall("Simple")):
            name = simple.findtext("Name", default="").strip().upper()
            if name in wanted:
                tag.remove(simple)


def build_tags(tools: Toolchain, path: Path, work_dir: Path, *, comment: str) -> tuple[Path, str, Path | None, dict[str, str]]:
    nfo = nfo_path_for(path)
    values: dict[str, str] = {}
    if nfo:
        try:
            root = ET.parse(nfo).getroot()
            for source, target in TAG_MAP.items():
                # Kodi may repeat elements such as <genre> and <studio>. Preserve all
                # values in source order instead of silently taking only the first.
                found = root.findall(source)
                parts = [v for node in found if (v := _flatten_text(node))]
                value = ", ".join(dict.fromkeys(parts)).strip() if parts else None
                if value:
                    if source == "mpaa":
                        value = re.sub(r"^Rated\s+", "", value, flags=re.I).strip()
                    values[target] = value
        except (ET.ParseError, OSError) as exc:
            raise RebuildError(f"Kodi NFO exists but cannot be parsed safely ({nfo}): {exc}") from exc

    title = values.get("TITLE") or path.stem
    values.setdefault("TITLE", title)
    if comment:
        values["COMMENT"] = comment

    # Better than v0.107: preserve pre-existing *global* Matroska tags, while Kodi
    # values intentionally win for the fields this utility manages.
    tags = _read_existing_global_root(tools, path, work_dir)
    _remove_conflicting_simple_tags(tags, set(values))
    tag = ET.SubElement(tags, "Tag")
    ET.SubElement(tag, "Targets")
    for name, value in values.items():
        simple = ET.SubElement(tag, "Simple")
        ET.SubElement(simple, "Name").text = name
        ET.SubElement(simple, "String").text = value

    xml_path = work_dir / "merged-global-tags.xml"
    ET.ElementTree(tags).write(xml_path, encoding="utf-8", xml_declaration=True)
    return xml_path, title, nfo, values


def read_global_simple_tags(tools: Toolchain, path: Path) -> dict[str, list[str]]:
    """Read global Matroska Simple tags for post-write validation."""
    with tempfile.TemporaryDirectory(prefix="hevc-rebuild-tags-") as tmp:
        out = Path(tmp) / "tags.xml"
        cp = run([tools.mkvextract, str(path), "tags", str(out)], check=False)
        if cp.returncode != 0 or not out.exists():
            return {}
        try:
            root = ET.parse(out).getroot()
        except (ET.ParseError, OSError):
            return {}
        result: dict[str, list[str]] = {}
        for tag in root.findall("Tag"):
            if not _is_global_tag(tag):
                continue
            for simple in tag.findall("Simple"):
                name = simple.findtext("Name", default="").strip().upper()
                value = simple.findtext("String", default="").strip()
                if name and value:
                    result.setdefault(name, []).append(value)
        return result

def _poster_candidates(profile: MediaProfile) -> list[Path]:
    path = profile.path
    candidates: list[Path] = []
    for ext in (".jpg", ".jpeg", ".png", ".webp"):
        candidates.append(path.with_name(path.stem + "-poster" + ext))
    # TV/season conventions inherited from the original utility plus common modern names.
    parent = path.parent
    if re.match(r"(?i)^season\s*\d+", parent.name):
        series = parent.parent
        normalized = re.sub(r"\s+", "", parent.name).lower()
        for ext in (".jpg", ".jpeg", ".png", ".webp"):
            candidates.extend([
                series / f"{normalized}-poster{ext}",
                series / f"{parent.name}-poster{ext}",
                parent / f"poster{ext}", parent / f"folder{ext}",
            ])
    return candidates


def choose_poster(profile: MediaProfile) -> Path | None:
    """Return only library artwork that explicitly matches the media/season.

    HEVC Rebuild never invents or supplies fallback artwork. If no recognized local
    poster exists, None is returned so any already-embedded attachment copied from
    the source remains untouched.
    """
    for p in _poster_candidates(profile):
        if p.exists() and p.is_file():
            return p
    return None


def poster_to_jpeg(tools: Toolchain, poster: Path, work_dir: Path) -> Path:
    out = work_dir / "cover.jpg"
    # Decode and re-encode even JPEG input so corrupt/mislabeled artwork cannot be
    # attached under a false image/jpeg MIME type. This also normalizes PNG/WebP.
    run([
        tools.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-i", str(poster),
        "-frames:v", "1", "-q:v", "2", str(out),
    ])
    if not out.exists() or out.stat().st_size < 128:
        raise RebuildError(f"Failed to prepare poster artwork from {poster}")
    return out


def mkv_identify(tools: Toolchain, path: Path) -> dict[str, Any]:
    return run_json([tools.mkvmerge, "-J", str(path)])


def apply_metadata(
    tools: Toolchain,
    output: Path,
    profile: MediaProfile,
    work_dir: Path,
    *,
    comment: str,
) -> MetadataResult:
    xml_path, title, nfo, expected_tags = build_tags(tools, profile.path, work_dir, comment=comment)
    poster = choose_poster(profile)
    cover = poster_to_jpeg(tools, poster, work_dir) if poster else None

    # Replace only global tags; preserve track-specific tags/statistics copied from the source.
    args = [tools.mkvpropedit, str(output), "--edit", "info", "--set", f"title={title}", "--tags", f"global:{xml_path}"]
    run(args)

    if cover:
        ident = mkv_identify(tools, output)
        # Better than v0.107: remove only cover/poster-like image attachments, not unrelated image attachments.
        delete_ids: list[int] = []
        for att in ident.get("attachments", []) or []:
            name = str(att.get("file_name") or "")
            mime = str(att.get("content_type") or "")
            if mime.startswith("image/") and re.search(r"(?i)(cover|poster|folder|front)", name):
                try:
                    delete_ids.append(int(att["id"]))
                except (KeyError, TypeError, ValueError):
                    pass
        if delete_ids:
            run([tools.mkvpropedit, str(output), *sum((["--delete-attachment", str(i)] for i in delete_ids), [])])
        run([
            tools.mkvpropedit, str(output), "--attachment-name", "cover.jpg",
            "--attachment-mime-type", "image/jpeg", "--attachment-description", "Cover art",
            "--add-attachment", str(cover),
        ])
    return MetadataResult(xml_path=xml_path, title=title, nfo_path=nfo, poster_path=poster, expected_global_tags=expected_tags)
