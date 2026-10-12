from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from hevc_rebuild.discovery import (
    filter_by_filename_patterns,
    filter_by_selected_files,
    validate_filename_patterns,
)
from hevc_rebuild.util import RebuildError


class FilenameFilterTests(unittest.TestCase):
    def test_globs_are_case_insensitive_and_exclusions_take_precedence(self):
        files = [Path("GreatMovieReleaseGroup.MKV"), Path("Special1.mp4"), Path("GreatMovieSampleGroup.mkv"), Path("Other.mkv")]
        filtered = filter_by_filename_patterns(
            files,
            include=("*Movie*Group.mkv", "Special?.mp4"),
            exclude=("*sample*",),
        )
        self.assertEqual([path.name for path in filtered], ["GreatMovieReleaseGroup.MKV", "Special1.mp4"])
        self.assertEqual(filter_by_filename_patterns(files), files)

    def test_invalid_rules_are_rejected(self):
        for invalid in ("", "***", "?*", "../*.mkv", "bad\\name", " padded", "x" * 129):
            with self.subTest(invalid=invalid), self.assertRaises(RebuildError):
                validate_filename_patterns((invalid,), "include")
        with self.assertRaises(RebuildError):
            validate_filename_patterns(("Film*", "film*"), "exclude")

    def test_selected_paths_must_be_present_and_unique(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.mkv"
            second = root / "second.mkv"
            first.write_bytes(b"media")
            second.write_bytes(b"media")
            self.assertEqual(filter_by_selected_files([first, second], [str(second)]), [second])
            with self.assertRaises(RebuildError):
                filter_by_selected_files([first, second], [str(first), str(first)])
            with self.assertRaises(RebuildError):
                filter_by_selected_files([first], [str(second)])
            with self.assertRaises(RebuildError):
                filter_by_selected_files([first], ["relative.mkv"])


if __name__ == "__main__":
    unittest.main()
