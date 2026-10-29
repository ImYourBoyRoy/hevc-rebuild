# ./tests/test_metadata_verify_native.py
"""Parity: Rust `internal metadata-verify` vs Python `verify_expected_output`.

Run with the Rust helper plus ffmpeg and MKVToolNix (otherwise skipped):
    HEVC_REBUILD_NATIVE_HELPER=desktop/src-tauri/target/debug/hevc-rebuild-studio \
        python3 -m unittest tests.test_metadata_verify_native -v
Uses a synthetic clip, plus tests/fixtures/local/clip15.mkv (gitignored) or
HEVC_REBUILD_TEST_CLIP when present. Files are copied; the verifier is read-only,
so each case also asserts the checked file is byte-identical afterwards.
"""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from hevc_rebuild.metadata import attach_cover
from hevc_rebuild.model import Toolchain
from hevc_rebuild.util import RebuildError
from hevc_rebuild.validate import verify_expected_output

TOOLS = {name: shutil.which(name) for name in ("ffmpeg", "mkvmerge", "mkvextract", "mkvpropedit")}

TAGS = """<?xml version="1.0" encoding="UTF-8"?>
<Tags>
  <Tag><Targets/>
    <Simple><Name>Title</Name><String>  Spaced Title  </String></Simple>
    <Simple><Name>ACTOR</Name><String>One</String></Simple>
    <Simple><Name>ACTOR</Name><String>Two</String></Simple>
    <Simple><Name>QUOTE</Name><String>it's "fine"</String></Simple>
    <Simple><Name>UNI</Name><String>Éloïse “Q” 😀</String></Simple>
  </Tag>
  <Tag><Targets><TrackUID>@@UID@@</TrackUID></Targets>
    <Simple><Name>TRACKONLY</Name><String>nope</String></Simple></Tag>
</Tags>"""


def _helper() -> str | None:
    value = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
    return value if value and os.name == "posix" else None


def _real_clip() -> Path | None:
    explicit = os.environ.get("HEVC_REBUILD_TEST_CLIP")
    candidate = Path(explicit) if explicit else Path(__file__).parent / "fixtures" / "local" / "clip15.mkv"
    return candidate if candidate.is_file() else None


def _run(args: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=120)


def _toolchain() -> Toolchain:
    return Toolchain(
        str(TOOLS["ffmpeg"]), "ffprobe", "mediainfo", str(TOOLS["mkvpropedit"]),
        str(TOOLS["mkvmerge"]), str(TOOLS["mkvextract"]), "hdr10plus_tool",
    )


def _digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _prepare(source: Path, root: Path, *, with_cover: bool) -> Path:
    target = root / "checked.mkv"
    shutil.copyfile(source, target)
    identify = json.loads(_run([str(TOOLS["mkvmerge"]), "-J", str(target)]).stdout)
    track_uid = identify["tracks"][0]["properties"]["uid"]
    (root / "tags.xml").write_text(TAGS.replace("@@UID@@", str(track_uid)), encoding="utf-8")
    _run([str(TOOLS["mkvpropedit"]), str(target), "--tags", f"all:{root / 'tags.xml'}"])
    if with_cover:
        _run([
            str(TOOLS["ffmpeg"]), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=64x96:rate=1:duration=1",
            "-frames:v", "1", str(root / "cover.jpg"),
        ])
        attach_cover(_toolchain(), target, root / "cover.jpg", "Cover art")
    return target


def _synthetic(path: Path) -> None:
    raw = path.with_suffix(".raw.mkv")
    _run([
        str(TOOLS["ffmpeg"]), "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=10:duration=2",
        "-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=none", str(raw),
    ])
    _run([str(TOOLS["mkvmerge"]), "-o", str(path), str(raw)])
    raw.unlink()


CASES: list[tuple[str, bool, dict[str, str]]] = [
    ("nothing expected", False, {}),
    ("poster only", True, {}),
    ("title trimmed and case-folded name", False, {"title": "Spaced Title"}),
    ("second of repeated values", False, {"actor": "Two"}),
    ("quotes in value", False, {"QUOTE": "it's \"fine\""}),
    ("unicode value", False, {"UNI": "Éloïse “Q” 😀"}),
    ("all together", True, {"TITLE": "Spaced Title", "ACTOR": "One"}),
    ("wrong value", False, {"TITLE": "Other"}),
    ("untrimmed expected value", False, {"TITLE": "  Spaced Title  "}),
    ("missing tag", False, {"NOPE": "x"}),
    ("track-targeted tag is not global", False, {"TRACKONLY": "nope"}),
    ("wrong quoted value is reported with repr", False, {"QUOTE": "it's"}),
    ("first failure wins in order", False, {"TITLE": "Spaced Title", "ZZZ": "a", "AAA": "b"}),
    ("unicode mismatch repr", False, {"UNI": "Éloïse\u200b"}),
]


@unittest.skipUnless(_helper() and all(TOOLS.values()), "Rust helper path, POSIX, ffmpeg, and MKVToolNix are required")
class NativeVerifyParityTests(unittest.TestCase):
    def _python(self, path: Path, poster: bool, tags: dict[str, str]) -> str | None:
        with patch.dict(os.environ):
            os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
            try:
                verify_expected_output(_toolchain(), path, expect_poster=poster, expected_global_tags=tags or None)
            except RebuildError as exc:
                return str(exc)
        return None

    def _native(self, path: Path, work: Path, poster: bool, tags: dict[str, str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(_helper()), "internal", "metadata-verify"],
            input=json.dumps({
                "tools": {
                    "mkvmerge": TOOLS["mkvmerge"], "mkvextract": TOOLS["mkvextract"],
                    "mkvpropedit": TOOLS["mkvpropedit"],
                },
                "output": str(path), "work_dir": str(work), "expect_poster": poster,
                "expected_global_tags": [[name, value] for name, value in tags.items()],
            }),
            capture_output=True, text=True, check=False, timeout=120,
        )

    def _compare_all(self, source: Path, label: str) -> None:
        failures_seen = successes_seen = 0
        for with_cover in (True, False):
            with tempfile.TemporaryDirectory(prefix=f"hevc-verify-{label}-") as temporary:
                root = Path(temporary)
                target = _prepare(source, root, with_cover=with_cover)
                work = root / "work"
                work.mkdir()
                before = _digest(target)
                for name, wants_poster, tags in CASES:
                    with self.subTest(label=label, cover=with_cover, case=name):
                        expected = self._python(target, wants_poster, tags)
                        completed = self._native(target, work, wants_poster, tags)
                        if expected is None:
                            successes_seen += 1
                            self.assertEqual(completed.returncode, 0, completed.stderr)
                            self.assertTrue(json.loads(completed.stdout)["ok"])
                        else:
                            failures_seen += 1
                            self.assertEqual(completed.returncode, 0, completed.stderr)
                            reply = json.loads(completed.stdout)
                            self.assertFalse(reply["ok"], "native accepted what Python rejects")
                            self.assertEqual(reply["message"], expected)
                self.assertEqual(_digest(target), before, "verification must be read-only")
        self.assertGreater(failures_seen, 10)
        self.assertGreater(successes_seen, 10)

    def test_synthetic_clip_matches_python(self):
        with tempfile.TemporaryDirectory(prefix="hevc-verify-source-") as temporary:
            source = Path(temporary) / "synthetic.mkv"
            _synthetic(source)
            self._compare_all(source, "synthetic")

    @unittest.skipUnless(_real_clip(), "put a short MKV at tests/fixtures/local/clip15.mkv or set HEVC_REBUILD_TEST_CLIP")
    def test_real_clip_matches_python(self):
        source = _real_clip()
        assert source is not None
        self._compare_all(source, "real")

    def test_file_without_tags_reads_as_unverified(self):
        with tempfile.TemporaryDirectory(prefix="hevc-verify-bare-") as temporary:
            root = Path(temporary)
            target = root / "bare.mkv"
            _synthetic(target)
            work = root / "work"
            work.mkdir()
            expected = self._python(target, False, {"TITLE": "x"})
            completed = self._native(target, work, False, {"TITLE": "x"})
            self.assertIsNotNone(expected)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(json.loads(completed.stdout), {"ok": False, "message": expected})

    def test_bad_requests_fail_cleanly(self):
        with tempfile.TemporaryDirectory(prefix="hevc-verify-bad-") as temporary:
            root = Path(temporary)
            result = self._native(root / "missing.mkv", root, True, {})
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("existing file", result.stderr)
            unknown = subprocess.run(
                [str(_helper()), "internal", "metadata-verify"], input='{"surprise":1}',
                capture_output=True, text=True, check=False, timeout=30,
            )
            self.assertNotEqual(unknown.returncode, 0)


if __name__ == "__main__":
    unittest.main()
