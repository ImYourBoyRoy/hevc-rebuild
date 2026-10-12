from __future__ import annotations

import os
import unittest
from pathlib import Path
from types import SimpleNamespace

from hevc_rebuild.encode import ResourcePlan, encode_args
from hevc_rebuild.model import StreamRef


class RustEncodePlanParityTests(unittest.TestCase):
    def test_rust_and_python_multi_video_argument_vectors_match(self):
        helper = os.environ.get("HEVC_REBUILD_NATIVE_HELPER")
        if not helper:
            self.skipTest("Rust helper path not configured")

        video_4k = StreamRef(
            0, 0, "video", "hevc", 3840, 2160, "yuv420p10le", 10,
            True, False, {}, {"default": 1, "original": 1},
        )
        video_1080 = StreamRef(
            1, 1, "video", "hevc", 1920, 1080, "yuv420p", 8,
            False, False, {}, {"forced": 1},
        )
        audio = StreamRef(2, 0, "audio", "truehd", default=True, disposition={"default": 1})
        subtitle = StreamRef(3, 0, "subtitle", "hdmv_pgs_subtitle", disposition={"forced": 1})
        attachment = StreamRef(4, 0, "attachment", "ttf")
        data = StreamRef(5, 0, "data", "bin_data")
        shared = {
            "path": Path("/fixture/source.mkv"),
            "primary_video": video_4k,
            "secondary_videos": [video_1080],
            "audio": [audio],
            "subtitles": [subtitle],
            "attachments": [attachment],
            "data": [data],
            "source_bit_depth": 10,
            "source_chroma": "4:2:0",
            "target_pix_fmt": "yuv420p10le",
            "color_primaries": "bt2020",
            "transfer": "smpte2084",
            "color_matrix": "bt2020nc",
            "color_range_x265": "limited",
            "color_range_ffmpeg": "tv",
            "chroma_location": 2,
            "master_display": "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50)",
            "max_cll": 1000,
            "max_fall": 400,
            "hdr10": True,
            "hlg": False,
            "dolby_vision": False,
            "interlaced": False,
            "format_level": "L5.1",
        }
        hdr_profile = SimpleNamespace(**shared)
        sdr_profile = SimpleNamespace(**{
            **shared,
            "primary_video": video_1080,
            "target_pix_fmt": "yuv420p",
            "source_bit_depth": 8,
            "color_primaries": None,
            "transfer": None,
            "color_matrix": None,
            "color_range_x265": None,
            "color_range_ffmpeg": None,
            "chroma_location": None,
            "master_display": None,
            "max_cll": None,
            "max_fall": None,
            "hdr10": False,
            "format_level": "L4.1",
        })
        tools = SimpleNamespace(
            ffmpeg="ffmpeg-test",
            x265_pixel_formats={"yuv420p10le", "yuv420p"},
            x265_dolbyvision=True,
        )
        plan = ResourcePlan(
            worker_threads=8,
            frame_threads=2,
            mem_available_gib=18,
            swap_free_gib=8,
            cpu_threads=16,
            reason="synthetic parity case",
            memory_guard_gib=3,
        )
        kwargs = {
            "crf": 20,
            "preset": "slow",
            "tune": "animation",
            "hdr10plus_json": Path("/tmp/hdr10plus.json"),
            "preserve_dolbyvision": False,
            "strip_dolbyvision": False,
            "test_seconds": None,
            "preserve_source_level": True,
            "disable_hdr10_opt": False,
            "secondary_video_policy": "encode",
            "data_policy": "preserve",
            "force_deinterlace": False,
            "disable_deinterlace": False,
            "resource_plan": plan,
            "selected_streams": [video_1080, video_4k, audio, subtitle, attachment, data],
            "video_profiles": {0: hdr_profile, 1: sdr_profile},
            "encode_video_indices": {0, 1},
            "hdr10plus_jsons": {0: Path("/tmp/hdr10plus.json")},
            "preserve_dolbyvision_indices": set(),
            "strip_dolbyvision_indices": set(),
        }

        os.environ.pop("HEVC_REBUILD_NATIVE_HELPER")
        try:
            python_plan = encode_args(tools, hdr_profile, Path("/fixture/output.mkv"), **kwargs)
        finally:
            os.environ["HEVC_REBUILD_NATIVE_HELPER"] = helper
        rust_plan = encode_args(tools, hdr_profile, Path("/fixture/output.mkv"), **kwargs)

        self.assertEqual(rust_plan, python_plan)
        self.assertEqual(rust_plan[1], {0: 0, 1: 1})
        self.assertIn("0:5", rust_plan[0])


if __name__ == "__main__":
    unittest.main()
