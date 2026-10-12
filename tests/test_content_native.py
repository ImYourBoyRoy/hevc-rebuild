from __future__ import annotations

import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from hevc_rebuild.content import resolve_run_tune


@unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
class NativeContentTuneParityTests(unittest.TestCase):
    def test_run_tune_decision_matches_python_for_nfo_and_overrides(self):
        helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        with TemporaryDirectory() as directory:
            root = Path(directory)
            media = root / "Animated film.mkv"
            media.write_bytes(b"fixture")
            media.with_suffix(".nfo").write_text(
                "<movie><genre>Drama</genre><genre>Animation</genre></movie>",
                encoding="utf-8",
            )
            mediainfo = {
                "media": {
                    "track": [
                        {"@type": "General", "Genre": "Adventure, Comedy"},
                        {"@type": "Video", "Genre": "Do not use"},
                    ]
                }
            }
            decisions = [
                ("auto", True, None),
                ("auto", True, "none"),
                ("none", True, "animation"),
                ("grain", False, None),
            ]
            for requested, detect, override in decisions:
                with self.subTest(requested=requested, override=override):
                    with patch.dict(os.environ):
                        os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                        expected = resolve_run_tune(media, mediainfo, requested, detect, override)
                    with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": helper}):
                        actual = resolve_run_tune(media, mediainfo, requested, detect, override)
                    self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
