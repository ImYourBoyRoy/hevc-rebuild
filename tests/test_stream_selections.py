from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from hevc_rebuild.app import _select_file_streams
from hevc_rebuild.util import RebuildError


class FileStreamSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.media = Path(self.temp_dir.name) / "series.mkv"
        self.media.write_bytes(b"fixture")
        self.profile = SimpleNamespace(
            path=self.media,
            probe={
                "streams": [
                    {"index": 0, "codec_type": "video", "width": 1920, "height": 1080, "disposition": {"default": 1}},
                    {"index": 1, "codec_type": "video", "width": 3840, "height": 2160, "disposition": {"default": 0}},
                    {"index": 2, "codec_type": "audio", "codec_name": "truehd"},
                    {"index": 3, "codec_type": "subtitle", "codec_name": "hdmv_pgs_subtitle"},
                    {"index": 4, "codec_type": "attachment", "codec_name": "ttf"},
                    {"index": 5, "codec_type": "data", "codec_name": "bin_data"},
                ]
            },
        )

    def test_explicit_4k_and_1080_selection_encodes_both_and_copies_other_tracks(self):
        args = SimpleNamespace(
            file_stream_overrides={str(self.media.resolve()): [0, 1, 2, 3, 4, 5]},
            secondary_video="drop",
            data_streams="drop",
        )

        selected, encode_indices, explicit = _select_file_streams(self.profile, args)

        self.assertTrue(explicit)
        self.assertEqual(encode_indices, {0, 1})
        self.assertEqual(
            [(stream.index, stream.codec_type) for stream in selected],
            [(1, "video"), (0, "video"), (2, "audio"), (3, "subtitle"), (4, "attachment"), (5, "data")],
        )

    def test_override_cannot_select_unknown_stream_index(self):
        args = SimpleNamespace(
            file_stream_overrides={str(self.media.resolve()): [99]},
            secondary_video="encode",
            data_streams="preserve",
        )

        with self.assertRaisesRegex(RebuildError, "not present"):
            _select_file_streams(self.profile, args)


if __name__ == "__main__":
    unittest.main()
