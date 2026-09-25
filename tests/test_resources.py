from __future__ import annotations

import tempfile
import unittest
import shutil
import subprocess
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.app import storage_preflight
from hevc_rebuild.encode import ResourcePlan, choose_resource_plan, run_encode
from hevc_rebuild.resources import DiskStats, MemoryStats, automatic_memory_reserve_gib


def profile(width=3840, height=2160):
    return SimpleNamespace(primary_video=SimpleNamespace(width=width, height=height))


def mem(total=16.0, available=12.0, swap_free=4.0):
    return MemoryStats(total_gib=total, available_gib=available, swap_total_gib=8.0, swap_free_gib=swap_free)


class ResourcePlanTests(unittest.TestCase):
    @patch("hevc_rebuild.encode.memory_stats", return_value=mem())
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_auto_uses_all_but_one_cpu_thread(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
        )
        self.assertEqual(p.worker_threads, 15)
        self.assertEqual(p.frame_threads, 2)
        self.assertEqual(p.memory_guard_gib, 3.0)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(total=64.0, available=12.0))
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_dynamic_memory_percent_can_exceed_floor(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
        )
        self.assertAlmostEqual(p.memory_guard_gib, 5.12, places=2)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(available=5.0, swap_free=0.0))
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_above_reserve_still_starts_all_but_one(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
        )
        self.assertEqual(p.worker_threads, 15)
        self.assertEqual(p.frame_threads, 2)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(available=2.0, swap_free=0.0))
    def test_refuses_when_below_memory_reserve(self, _mem):
        with self.assertRaises(Exception):
            choose_resource_plan(
                profile(), threading="auto", max_worker_threads=0, frame_threads=0,
                min_free_memory_gib=3.0, memory_reserve_percent=8.0,
            )

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem())
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_manual_caps_win(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=4, frame_threads=1,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
        )
        self.assertEqual(p.worker_threads, 4)
        self.assertEqual(p.frame_threads, 1)

    def test_adaptive_retry_reduces_parallelism_without_quality_knobs(self):
        p = ResourcePlan(15, 2, 12.0, 4.0, 16, "initial", 3.0)
        p2 = p.reduced()
        self.assertLess(p2.worker_threads, p.worker_threads)
        self.assertEqual(p2.frame_threads, 1)
        self.assertEqual(p2.memory_guard_gib, 3.0)
        p3 = p2.reduced()
        self.assertLess(p3.worker_threads, p2.worker_threads)
        self.assertEqual(p3.frame_threads, 1)

    def test_memory_reserve_uses_larger_floor_or_percent(self):
        self.assertEqual(automatic_memory_reserve_gib(16.0, floor_gib=3.0, percent=8.0), 3.0)
        self.assertAlmostEqual(automatic_memory_reserve_gib(64.0, floor_gib=3.0, percent=8.0), 5.12)


class StorageTests(unittest.TestCase):
    @patch("hevc_rebuild.app.disk_stats")
    def test_full_encode_preflight_includes_temp_budget_and_disk_reserve(self, ds):
        ds.return_value = DiskStats(total_bytes=500 * 1024**3, used_bytes=100 * 1024**3, free_bytes=400 * 1024**3)
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "movie.mkv"
            source.write_bytes(b"x" * 1024)
            with patch.object(Path, "stat") as stat:
                stat.return_value = SimpleNamespace(st_size=100 * 1024**3)
                plan = storage_preflight(
                    source, duration=7200, test_mode=False, test_seconds=5,
                    min_free_disk_gib=8.0, output_ratio=1.30,
                )
        self.assertEqual(plan["estimated_temp_bytes"], 130 * 1024**3)
        self.assertEqual(plan["reserve_bytes"], 8 * 1024**3)
        self.assertEqual(plan["required_bytes"], 13 * 1024**3)
        self.assertEqual(plan["recommended_bytes"], 138 * 1024**3)
        self.assertTrue(plan["conservative_ok"])

    @patch("hevc_rebuild.app.disk_stats")
    def test_storage_preflight_refuses_unsafe_space(self, ds):
        ds.return_value = DiskStats(total_bytes=200 * 1024**3, used_bytes=190 * 1024**3, free_bytes=10 * 1024**3)
        with tempfile.TemporaryDirectory() as td:
            source = Path(td) / "movie.mkv"
            source.write_bytes(b"x")
            with patch.object(Path, "stat") as stat:
                stat.return_value = SimpleNamespace(st_size=100 * 1024**3)
                with self.assertRaises(Exception):
                    storage_preflight(
                        source, duration=7200, test_mode=False, test_seconds=5,
                        min_free_disk_gib=8.0, output_ratio=1.30,
                    )


FFMPEG = shutil.which("ffmpeg")


@unittest.skipUnless(FFMPEG, "FFmpeg not installed")
class LiveProgressIntegrationTests(unittest.TestCase):
    def test_run_encode_machine_progress_and_resource_sampling(self):
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            out = td / "out.mkv"
            args = [
                FFMPEG, "-hide_banner", "-nostdin", "-y", "-loglevel", "error",
                "-stats_period", "0.25", "-progress", "pipe:1",
                "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=10:duration=1",
                "-c:v", "libx265", "-preset", "ultrafast", "-crf", "35",
                "-f", "matroska", str(out),
            ]
            run_encode(
                args, expected_duration=1.0, input_size=1024, label="test",
                memory_guard_gib=0.0, output_path=out, disk_guard_gib=0.0,
                batch_number=1, batch_total=2,
            )
            self.assertTrue(out.exists())
            self.assertGreater(out.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
