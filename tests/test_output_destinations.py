from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from hevc_rebuild.app import (
    parse_file_output_directories,
    resolved_output_path,
    validate_output_destinations,
)
from hevc_rebuild.util import RebuildError


class OutputDestinationTests(unittest.TestCase):
    def test_default_folder_mirrors_input_tree_without_creating_it_during_planning(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            source = root / "Movies" / "Film.mkv"
            source.parent.mkdir(parents=True)
            source.write_bytes(b"source")
            output = base / "rebuilt"
            output.mkdir()
            args = SimpleNamespace(output_dir=str(output), file_output_dirs={})

            destination, separate = resolved_output_path(source, root, args)

            self.assertTrue(separate)
            self.assertEqual(destination, output / "Movies" / "Film.mkv")
            self.assertFalse((output / "Movies").exists())

    def test_per_file_folder_overrides_default_and_uses_flat_filename(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            source = root / "Season 1" / "Episode.mkv"
            source.parent.mkdir(parents=True)
            source.write_bytes(b"source")
            default = base / "default"
            custom = base / "custom"
            default.mkdir()
            custom.mkdir()
            args = SimpleNamespace(
                output_dir=str(default),
                file_output_dirs={str(source.resolve()): str(custom.resolve())},
            )

            destination, separate = resolved_output_path(source, root, args)

            self.assertTrue(separate)
            self.assertEqual(destination, custom / "Episode.mkv")

    def test_existing_destination_named_like_source_uses_a_distinct_copy_name(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "Film.mkv"
            source.write_bytes(b"source")
            args = SimpleNamespace(output_dir=str(root.resolve()), file_output_dirs={})

            destination, separate = resolved_output_path(source, root, args)

            self.assertTrue(separate)
            self.assertEqual(destination, root / "Film.hevc-rebuilt.mkv")

    def test_per_file_output_mapping_requires_existing_input_and_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            source = root / "Film.mkv"
            root.mkdir()
            source.write_bytes(b"source")
            destination = base / "output"
            destination.mkdir()
            parsed = parse_file_output_directories(
                f'{{"{source}": "{destination}"}}', root
            )
            self.assertEqual(parsed[str(source.resolve())], str(destination.resolve()))
            with self.assertRaises(RebuildError):
                parse_file_output_directories(
                    f'{{"{source}": "{base / "missing"}"}}', root
                )

    def test_batch_preflight_rejects_colliding_per_file_outputs_before_encoding(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            first = root / "one" / "Film.mp4"
            second = root / "two" / "Film.avi"
            first.parent.mkdir(parents=True)
            second.parent.mkdir(parents=True)
            first.write_bytes(b"one")
            second.write_bytes(b"two")
            output = base / "shared-output"
            output.mkdir()
            args = SimpleNamespace(
                output_dir=None,
                file_output_dirs={
                    str(first.resolve()): str(output.resolve()),
                    str(second.resolve()): str(output.resolve()),
                },
                test=False,
            )

            with self.assertRaisesRegex(RebuildError, "same output"):
                validate_output_destinations([first, second], root, args)

    def test_batch_preflight_rejects_existing_separate_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            source = root / "Film.mp4"
            root.mkdir()
            source.write_bytes(b"source")
            output = base / "output"
            output.mkdir()
            (output / "Film.mkv").write_bytes(b"older result")
            args = SimpleNamespace(output_dir=str(output), file_output_dirs={}, test=False)

            with self.assertRaisesRegex(RebuildError, "will not be overwritten"):
                validate_output_destinations([source], root, args)


if __name__ == "__main__":
    unittest.main()
