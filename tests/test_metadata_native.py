from __future__ import annotations

import os
import unittest
from unittest.mock import patch

from hevc_rebuild.metadata import clean_metadata_text, clean_metadata_texts


@unittest.skipUnless(
    os.environ.get("HEVC_REBUILD_NATIVE_HELPER") and os.name == "posix",
    "Rust helper path and POSIX test scripts are required",
)
class NativeMetadataTextParityTests(unittest.TestCase):
    def test_rust_blacklist_globs_match_python_results(self):
        native_helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        cases = [
            (
                "Keep [movieReleaseGroup] and Movieshowgroup titles",
                ["Movie*Group"],
            ),
            (
                "Keep   these  spaces movieReleaseGroup after",
                ["Movie*Group"],
            ),
            (
                "Before Movie\nReleaseGroup after; Explicit content remains",
                ["Movie*Group", "Explicit"],
            ),
            (
                "A Film.1080p.WEB-DL.x265-yify example",
                [],
            ),
            (
                "Film.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HEVC-TEPES",
                [],
            ),
            (
                "Film 1080p WEB-DL x265 [YTS.MX]",
                [],
            ),
            (
                "Film.1080p.WEB-DL.x265-GROUP  \n",
                [],
            ),
            (
                "Film.1080p.WEB-DL.x265-HEVC",
                [],
            ),
        ]
        for value, patterns in cases:
            with self.subTest(value=value, patterns=patterns):
                with patch.dict(os.environ):
                    os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                    python_result = clean_metadata_text(value, patterns)
                with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": native_helper}):
                    rust_result = clean_metadata_text(value, patterns)
                self.assertEqual(rust_result, python_result)

    def test_rust_batch_cleanup_matches_python_for_multiple_fields(self):
        native_helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        values = [
            "Keep [movieReleaseGroup] and Movieshowgroup titles",
            "Keep   these  spaces movieReleaseGroup after",
            "Before Movie\nReleaseGroup after; Explicit remains",
        ]
        patterns = ["Movie*Group", "Explicit"]
        with patch.dict(os.environ):
            os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
            python_results = clean_metadata_texts(values, patterns)
        with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": native_helper}):
            rust_results = clean_metadata_texts(values, patterns)
        self.assertEqual(rust_results, python_results)


if __name__ == "__main__":
    unittest.main()
