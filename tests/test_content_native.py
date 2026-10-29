from __future__ import annotations

import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from hevc_rebuild.content import resolve_run_tune


@unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
class NativeContentTuneParityTests(unittest.TestCase):
    def test_run_tune_decision_matches_python_for_reviewed_genres_and_overrides(self):
        helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        with TemporaryDirectory() as directory:
            root = Path(directory)
            media = root / "Animated film.mkv"
            media.write_bytes(b"fixture")
            media.with_suffix(".nfo").write_text(
                "<movie><genre>Animation</genre></movie>",
                encoding="utf-8",
            )  # sidecars must be ignored by both implementations
            mediainfo = {
                "media": {
                    "track": [
                        {"@type": "General", "Genre": "Adventure, Comedy"},
                        {"@type": "Video", "Genre": "Do not use"},
                    ]
                }
            }
            decisions = [
                ("auto", True, None, ()),
                ("auto", True, None, ("Drama, Animation",)),
                ("auto", True, "none", ("Animation",)),
                ("none", True, "animation", ()),
                ("none", True, None, ("Animation",)),
                ("auto", False, None, ("Animation",)),
                ("grain", False, None, ()),
            ]
            for requested, detect, override, reviewed in decisions:
                with self.subTest(requested=requested, override=override, reviewed=reviewed):
                    with patch.dict(os.environ):
                        os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                        expected = resolve_run_tune(media, mediainfo, requested, detect, override, reviewed)
                    with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": helper}):
                        actual = resolve_run_tune(media, mediainfo, requested, detect, override, reviewed)
                    self.assertEqual(actual, expected)
            self.assertEqual(resolve_run_tune(media, mediainfo, "auto", True, None, ("Animation",)), "animation")
            self.assertIsNone(resolve_run_tune(media, mediainfo, "auto", True, None, ()))


if __name__ == "__main__":
    unittest.main()
