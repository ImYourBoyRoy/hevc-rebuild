# ./tests/test_metadata_tags_native.py
"""Parity fixtures: Rust `internal metadata-tags` vs Python `build_tags`.

Run with the Rust helper built, e.g.:
    HEVC_REBUILD_NATIVE_HELPER=desktop/src-tauri/target/debug/hevc-rebuild-studio \
        python3 -m unittest tests.test_metadata_tags_native -v
Python stays authoritative until every fixture here agrees; the Rust coordinator
will consume the same logic. Both sides run with identical inputs and no media.
"""
from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.metadata import _is_global_tag, build_tags
from hevc_rebuild.model import Toolchain

TOOLS = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")

REVIEWED_MOVIE = {
    "title": "  Heat  ", "year": "1995", "certification": "R", "short_description": "Tag",
    "plot": "Line one & two", "genres": ["Crime", "Drama", "Crime"], "rating": "7.5",
    "source_url": "https://www.themoviedb.org/movie/949",
}

EXISTING_TAGS = """<Tags>
  <Tag><Targets/><Simple><Name>ENCODER</Name><String>mkvmerge v90</String></Simple>
       <Simple><Name>RELEASE_GROUP</Name><String>GROUP</String></Simple></Tag>
  <Tag><Targets><TrackUID>9</TrackUID></Targets><Simple><Name>BPS</Name><String>100</String></Simple></Tag>
  <Tag><Targets/><Simple><Name>GENRE</Name><String>Old Genre</String></Simple></Tag>
  <Tag><Targets/><Simple><Name>NOTE</Name><String>Film.1080p.WEB-DL.x265-GROUP</String>
       <Simple><Name>NESTED</Name><String>Nested.1080p.WEB-DL.x265-GROUP</String></Simple></Simple></Tag>
</Tags>"""

CASES: list[dict[str, object]] = [
    {"name": "plain stem", "stem": "Example", "comment": ""},
    {"name": "comment", "stem": "Example", "comment": "Brought To you By V1x0r"},
    {"name": "glob cleanup", "stem": "Example MovieReleaseGroup", "comment": "Keep Movieshowgroup", "blacklist": ["Movie*Group"]},
    {"name": "reviewed movie", "stem": "Heat", "comment": "c", "override": REVIEWED_MOVIE},
    {"name": "reviewed movie with existing tags", "stem": "Heat", "comment": "", "override": REVIEWED_MOVIE, "existing": EXISTING_TAGS},
    {"name": "existing tags only", "stem": "Film.2024.1080p.WEB-DL.x265-TEPES", "comment": "", "existing": EXISTING_TAGS},
    {"name": "override all", "stem": "X", "comment": "global", "override": {
        "title": " New ", "show_title": "S", "certification": "TV-MA", "short_description": "Short", "plot": "P",
        "season": "3", "episode": "4", "source_url": "https://example.test", "year": "2001", "airdate": "",
        "genres": ["A", " B ", "A", ""], "rating": "9", "comment": "Own"}},
    {"name": "airdate wins", "stem": "X", "comment": "", "override": {"airdate": "2020-02-02", "year": "2019"}},
    {"name": "empty rating clears", "stem": "X", "comment": "", "override": {**REVIEWED_MOVIE, "rating": ""}},
    {"name": "fractional ratings", "stem": "X", "comment": "", "override": {"rating": "0.25"}},
    {"name": "rating 10", "stem": "X", "comment": "", "override": {"rating": "10"}},
    {"name": "unicode and xml chars", "stem": "A & B <C> é", "comment": "q\"uote 'x'"},
]


def _helper() -> str | None:
    value = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    return value if value and os.name == "posix" else None


def _shape(node: ET.Element) -> tuple[str, str | None, list[object]]:
    children = [_shape(child) for child in node]
    text = node.text
    if children and (text is None or not text.strip()):
        text = None
    return node.tag, text, children


def _global_only(xml: str) -> ET.Element:
    root = ET.Element("Tags")
    parsed = ET.fromstring(xml)
    for tag in parsed.findall("Tag"):
        if _is_global_tag(tag):
            root.append(tag)
    return root


@unittest.skipUnless(_helper(), "Rust helper path and POSIX are required")
class NativeMetadataTagParityTests(unittest.TestCase):
    def _python(self, case: dict[str, object], media: Path, work: Path):
        existing = ET.Element("Tags") if "existing" not in case else _global_only(str(case["existing"]))
        with patch.dict(os.environ):
            os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
            with patch("hevc_rebuild.metadata._read_existing_global_root", return_value=existing):
                return build_tags(
                    TOOLS, media, work,
                    comment=str(case["comment"]),
                    text_blacklist=list(case.get("blacklist", [])),  # type: ignore[arg-type]
                    metadata_override=case.get("override"),  # type: ignore[arg-type]
                )

    def _rust(self, case: dict[str, object], media: Path) -> dict[str, object]:
        payload: dict[str, object] = {
            "source_stem": media.stem,
            "comment": case["comment"],
            "text_blacklist": case.get("blacklist", []),
        }
        if case.get("override") is not None:
            payload["metadata_override"] = case["override"]
        if "existing" in case:
            payload["existing_tags_xml"] = case["existing"]
        result = subprocess.run(
            [str(_helper()), "internal", "metadata-tags"],
            input=json.dumps(payload), capture_output=True, text=True, check=False, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_rust_matches_python_for_every_fixture(self):
        for case in CASES:
            with self.subTest(case=case["name"]), tempfile.TemporaryDirectory() as temporary:
                media = Path(temporary) / f"{case['stem']}.mkv"
                work = Path(temporary) / "work"
                work.mkdir()
                # A sidecar must never influence either implementation.
                media.with_suffix(".nfo").write_text("<movie><title>Ignored</title></movie>", encoding="utf-8")
                xml_path, title, expected, changes = self._python(case, media, work)
                native = self._rust(case, media)
                self.assertEqual(native["title"], title)
                self.assertEqual(native["expected_tags"], expected)
                self.assertEqual(native["changes"], changes)
                self.assertEqual(
                    _shape(ET.fromstring(str(native["tags_xml"]))),
                    _shape(ET.parse(xml_path).getroot()),
                )

    def test_rust_rejects_the_same_invalid_reviewed_input(self):
        for override in ({"rating": "11"}, {"rating": "abc"}, {"season": 2}, {"genres": "A"}, {"comment": 5}):
            with self.subTest(override=override), tempfile.TemporaryDirectory() as temporary:
                media = Path(temporary) / "X.mkv"
                work = Path(temporary) / "work"
                work.mkdir()
                case: dict[str, object] = {"name": "bad", "stem": "X", "comment": "", "override": override}
                with self.assertRaises(Exception):
                    self._python(case, media, work)
                result = subprocess.run(
                    [str(_helper()), "internal", "metadata-tags"],
                    input=json.dumps({"source_stem": "X", "metadata_override": override}),
                    capture_output=True, text=True, check=False, timeout=30,
                )
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
