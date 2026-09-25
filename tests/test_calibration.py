from __future__ import annotations

import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.calibration import (
    CalibrationResult,
    CalibrationSelection,
    _benchmark_args,
    _candidate_plans,
    cached_plan,
    choose_best_result,
    is_demanding,
    store_selection,
)
from hevc_rebuild.encode import ResourcePlan
from hevc_rebuild.model import Toolchain


def profile(width: int = 3840, height: int = 2160, duration: float = 3600.0):
    return SimpleNamespace(
        path=Path("/media/test.mkv"),
        duration=duration,
        primary_video=SimpleNamespace(index=2, width=width, height=height),
        source_bit_depth=10,
        source_chroma="4:2:0",
        target_pix_fmt="yuv420p10le",
        color_primaries="bt2020",
        transfer="smpte2084",
        color_matrix="bt2020nc",
        color_range_ffmpeg="tv",
        color_range_x265="limited",
        chroma_location=2,
        master_display=None,
        max_cll=None,
        max_fall=None,
        hdr10=True,
        hdr10plus=False,
        hlg=False,
        dolby_vision=False,
        dovi_profile=None,
        dovi_rpu_present=False,
        interlaced=False,
        format_level=None,
    )


class CalibrationPolicyTests(unittest.TestCase):
    def test_4k_is_eligible_even_when_small(self):
        self.assertTrue(is_demanding(profile(), 2 * 1024**3, large_file_gib=20.0))

    def test_large_1080p_is_eligible_and_small_1080p_is_not(self):
        self.assertTrue(is_demanding(profile(1920, 1080), 25 * 1024**3, large_file_gib=20.0))
        self.assertFalse(is_demanding(profile(1920, 1080), 5 * 1024**3, large_file_gib=20.0))

    def test_candidates_never_exceed_the_profile_aware_plan(self):
        base = ResourcePlan(8, 2, 12.0, 4.0, 16, "base", 3.0)
        plans = _candidate_plans(base, max_candidates=4)
        self.assertEqual([(p.worker_threads, p.frame_threads) for p in plans], [(8, 2), (8, 1), (6, 1), (5, 1)])

    def test_near_tie_prefers_lower_peak_memory(self):
        results = [
            CalibrationResult(15, 1, 10.0, 3.0, 0.300, 6.2, 20.0, 0.0, 0.0, 0.0, True),
            CalibrationResult(12, 1, 10.2, 3.0, 0.294, 4.0, 22.0, 0.0, 0.0, 0.0, True),
        ]
        self.assertEqual(choose_best_result(results, throughput_tolerance_percent=3.0).worker_threads, 12)

    def test_materially_faster_candidate_wins_and_unhealthy_is_excluded(self):
        results = [
            CalibrationResult(15, 1, 10.0, 3.0, 0.300, 6.2, 20.0, 0.0, 0.0, 0.0, True),
            CalibrationResult(12, 1, 12.0, 3.0, 0.250, 3.0, 25.0, 0.0, 0.0, 0.0, True),
            CalibrationResult(10, 1, 9.0, 3.0, 0.333, 5.0, 0.2, 40.0, 100.0, 100.0, False, "pressure"),
        ]
        self.assertEqual(choose_best_result(results).worker_threads, 15)

    def test_cache_expires_and_cannot_override_a_new_lower_cap(self):
        base = ResourcePlan(15, 2, 30.0, 2.0, 16, "base", 3.0)
        selection = CalibrationSelection(
            plan=ResourcePlan(12, 1, 30.0, 2.0, 16, "calibrated", 3.0),
            source="benchmark",
            results=(CalibrationResult(12, 1, 10.0, 3.0, 0.3, 4.0, 20.0, 0.0, 0.0, 0.0, True),),
        )
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp) / "calibration.json"
            with patch("hevc_rebuild.calibration.cache_path", return_value=cache):
                store_selection("same-settings", selection)
                hit = cached_plan("same-settings", base, cache_hours=24.0)
                self.assertIsNotNone(hit)
                self.assertEqual(hit.plan.worker_threads, 12)
                self.assertEqual(hit.plan.frame_threads, 1)
                self.assertIsNone(cached_plan("same-settings", base, cache_hours=0.0, now=time.time() + 1))
                lower_cap = ResourcePlan(8, 1, 30.0, 2.0, 16, "new cap", 3.0)
                self.assertIsNone(cached_plan("same-settings", lower_cap, cache_hours=24.0))

    def test_benchmark_reads_one_video_stream_and_writes_only_to_null(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        args = _benchmark_args(
            tools,
            profile(),
            preset="slow",
            tune=None,
            crf=21,
            plan=ResourcePlan(8, 1, 12.0, 4.0, 16, "base", 3.0),
            sample_seconds=3.0,
            hdr10plus_json=None,
            preserve_dolbyvision=False,
            strip_dolbyvision=False,
            preserve_source_level=False,
            disable_hdr10_opt=False,
            force_deinterlace=False,
            disable_deinterlace=False,
        )
        self.assertIn("0:2", args)
        joined_args = " ".join(args)
        self.assertIn("pools=8", joined_args)
        self.assertIn("frame-threads=1", joined_args)
        self.assertEqual(args[-2:], ["null", "-"])
        self.assertIn("-an", args)
        self.assertIn("-sn", args)


if __name__ == "__main__":
    unittest.main()
