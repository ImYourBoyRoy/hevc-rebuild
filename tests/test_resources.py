from __future__ import annotations

import json
import os
import tempfile
import unittest
import shutil
import subprocess
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.app import resource_retry_summary, storage_preflight
from hevc_rebuild.encode import ResourcePlan, _NativeRuntimeGuard, choose_resource_plan, run_encode
from hevc_rebuild.resources import DiskStats, MemoryStats, PressureStats, automatic_memory_reserve_gib
from hevc_rebuild.util import RebuildError


def profile(width=3840, height=2160):
    return SimpleNamespace(
        primary_video=SimpleNamespace(width=width, height=height),
        source_bit_depth=8,
        source_chroma="4:2:0",
        target_pix_fmt="yuv420p",
    )


def mem(total=16.0, available=12.0, swap_free=4.0):
    return MemoryStats(total_gib=total, available_gib=available, swap_total_gib=8.0, swap_free_gib=swap_free)


class ResourcePlanTests(unittest.TestCase):
    @unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
    def test_native_run_planner_reads_host_resources_in_rust(self):
        with patch("hevc_rebuild.encode.effective_cpu_threads", side_effect=AssertionError("Python CPU sample used")), \
                patch("hevc_rebuild.encode.memory_stats", side_effect=AssertionError("Python memory sample used")), \
                patch("hevc_rebuild.encode.pressure_stats", side_effect=AssertionError("Python PSI sample used")):
            plan = choose_resource_plan(
                profile(1920, 1080),
                threading="auto",
                preset="ultrafast",
                min_free_memory_gib=0.0,
                memory_reserve_percent=0.0,
            )
        self.assertGreaterEqual(plan.cpu_threads, 1)
        self.assertIsNotNone(plan.mem_available_gib)
        self.assertIsNotNone(plan.memory_forecast)

    @unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
    def test_rust_run_planner_and_retry_plan_are_used_for_native_runs(self):
        with patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16):
            plan = choose_resource_plan(
                profile(),
                threading="auto",
                reserved_cpu_threads=1,
                max_worker_threads=0,
                frame_threads=0,
                preset="slow",
                mem_stats=MemoryStats(
                    total_gib=40.0,
                    available_gib=32.0,
                    swap_total_gib=8.0,
                    swap_free_gib=5.0,
                    swap_used_gib=3.0,
                    swap_used_percent=37.5,
                ),
                pressure=SimpleNamespace(memory_full_avg10=0.0),
            )
        self.assertEqual((plan.worker_threads, plan.frame_threads), (13, 2))
        retry = plan.reduced()
        self.assertEqual((retry.worker_threads, retry.frame_threads), (10, 1))

    def test_retry_summary_reports_reduced_or_unavailable_parallelism(self):
        minimum = ResourcePlan(1, 1, 8.0, 0.0, 8, "minimum", 3.0)
        message = resource_retry_summary(minimum)
        self.assertIn("no lower-parallelism plan is available", message)
        self.assertIn("instead of repeating the same plan", message)

        adjustable = ResourcePlan(7, 2, 12.0, 4.0, 8, "initial", 3.0)
        message = resource_retry_summary(adjustable)
        self.assertIn("first lower plan is 5 worker(s) / 1 frame thread(s)", message)

    def test_shared_resource_plan_cases_match_python_policy(self):
        fixture_path = Path(__file__).parent / "fixtures" / "resource_plan_cases.json"
        fixture = json.loads(fixture_path.read_text(encoding="utf-8"))

        for case in fixture["cases"]:
            with self.subTest(case=case["name"]):
                video = case["profile"]["video"]
                media_profile = SimpleNamespace(
                    primary_video=SimpleNamespace(
                        width=video["width"], height=video["height"]
                    ),
                    source_bit_depth=video["source_bit_depth"],
                    source_chroma=video["source_chroma"],
                    target_pix_fmt=video["target_pixel_format"],
                )
                resources = case["settings"]["resources"]
                hardware = case["host"]["hardware"]
                memory = MemoryStats(
                    total_gib=hardware["memory_total_gib"],
                    available_gib=hardware["memory_available_gib"],
                    swap_total_gib=(
                        hardware["swap_free_gib"] + hardware["swap_used_gib"]
                        if hardware["swap_free_gib"] is not None
                        and hardware["swap_used_gib"] is not None
                        else None
                    ),
                    swap_free_gib=hardware["swap_free_gib"],
                    swap_used_gib=hardware["swap_used_gib"],
                    swap_used_percent=hardware["swap_used_percent"],
                )
                target_dimensions = case["target_dimensions"]
                if target_dimensions is not None:
                    target_dimensions = tuple(target_dimensions)
                preset = case["preset_override"] or case["settings"]["video"]["preset"]
                frame_threads = case["frame_threads_override"]
                if frame_threads is None:
                    frame_threads = resources["frame_threads"]

                with patch(
                    "hevc_rebuild.encode.effective_cpu_threads",
                    return_value=hardware["logical_cpus"],
                ):
                    if case["expected"]["level"] == "blocked":
                        with self.assertRaisesRegex(RebuildError, "dynamic safety reserve"):
                            choose_resource_plan(
                                media_profile,
                                target_dimensions=target_dimensions,
                                threading=resources["threading"],
                                reserved_cpu_threads=resources["reserved_cpu_threads"],
                                max_worker_threads=resources["max_worker_threads"],
                                frame_threads=frame_threads,
                                preset=preset,
                                min_free_memory_gib=resources["min_free_memory_gib"],
                                memory_reserve_percent=resources["memory_reserve_percent"],
                                mem_stats=memory,
                                pressure=PressureStats(
                                    memory_full_avg10=case["memory_psi_full_avg10"]
                                ),
                                concurrent_video_streams=case[
                                    "concurrent_video_streams"
                                ],
                            )
                        self.assertIsNone(case["retry_expected"])
                        continue

                    plan = choose_resource_plan(
                        media_profile,
                        target_dimensions=target_dimensions,
                        threading=resources["threading"],
                        reserved_cpu_threads=resources["reserved_cpu_threads"],
                        max_worker_threads=resources["max_worker_threads"],
                        frame_threads=frame_threads,
                        preset=preset,
                        min_free_memory_gib=resources["min_free_memory_gib"],
                        memory_reserve_percent=resources["memory_reserve_percent"],
                        mem_stats=memory,
                        pressure=PressureStats(
                            memory_full_avg10=case["memory_psi_full_avg10"]
                        ),
                        concurrent_video_streams=case["concurrent_video_streams"],
                    )
                forecast = plan.memory_forecast
                self.assertIsNotNone(forecast)
                for key, expected in case["expected"].items():
                    actual = getattr(forecast, key)
                    if isinstance(expected, float):
                        self.assertAlmostEqual(actual, expected, places=6, msg=key)
                    else:
                        self.assertEqual(actual, expected, key)

                retry = plan.reduced()
                retry_forecast = retry.memory_forecast
                self.assertIsNotNone(retry_forecast)
                for key, expected in case["retry_expected"].items():
                    actual = (
                        retry.worker_threads != plan.worker_threads
                        or retry.frame_threads != plan.frame_threads
                        if key == "reduces_parallelism"
                        else getattr(retry_forecast, key)
                    )
                    if isinstance(expected, float):
                        self.assertAlmostEqual(actual, expected, places=6, msg=key)
                    else:
                        self.assertEqual(actual, expected, key)

    @patch("hevc_rebuild.encode.pressure_stats", return_value=PressureStats())
    @patch("hevc_rebuild.encode.memory_stats", return_value=mem())
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_auto_respects_memory_budget_below_cpu_ceiling(self, _cpu, _mem, _pressure):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
            mem_stats=mem(), pressure=PressureStats(),
        )
        self.assertEqual(p.worker_threads, 2)
        self.assertEqual(p.frame_threads, 1)
        self.assertEqual(p.memory_guard_gib, 3.0)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(total=64.0, available=12.0))
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_dynamic_memory_percent_can_exceed_floor(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
            mem_stats=mem(total=64.0, available=12.0), pressure=PressureStats(),
        )
        self.assertAlmostEqual(p.memory_guard_gib, 5.12, places=2)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(available=5.0, swap_free=0.0))
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_memory_budget_caps_threads_even_above_reserve(self, _cpu, _mem):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=0, frame_threads=0,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
            mem_stats=mem(available=5.0, swap_free=0.0), pressure=PressureStats(),
        )
        self.assertEqual(p.worker_threads, 1)
        self.assertEqual(p.frame_threads, 1)

    @patch("hevc_rebuild.encode.memory_stats", return_value=mem(available=2.0, swap_free=0.0))
    def test_refuses_when_below_memory_reserve(self, _mem):
        with self.assertRaises(Exception):
            choose_resource_plan(
                profile(), threading="auto", max_worker_threads=0, frame_threads=0,
                min_free_memory_gib=3.0, memory_reserve_percent=8.0,
                mem_stats=mem(available=2.0, swap_free=0.0), pressure=PressureStats(),
            )

    @patch("hevc_rebuild.encode.pressure_stats", return_value=PressureStats())
    @patch("hevc_rebuild.encode.memory_stats", return_value=mem())
    @patch("hevc_rebuild.encode.effective_cpu_threads", return_value=16)
    def test_manual_caps_win(self, _cpu, _mem, _pressure):
        p = choose_resource_plan(
            profile(), threading="auto", max_worker_threads=4, frame_threads=1,
            min_free_memory_gib=3.0, memory_reserve_percent=8.0,
            mem_stats=mem(), pressure=PressureStats(),
        )
        self.assertEqual(p.worker_threads, 2)
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


@unittest.skipUnless(os.environ.get("HEVC_REBUILD_NATIVE_HELPER"), "Rust helper path not configured")
class NativeRuntimeGuardTests(unittest.TestCase):
    def test_native_runtime_guard_samples_resources_over_json_lines(self):
        guard = _NativeRuntimeGuard.from_environment()
        self.assertIsNotNone(guard)
        assert guard is not None
        with tempfile.TemporaryDirectory() as td:
            request = {
                "pid": os.getpid(),
                "disk_path": td,
                "memory_guard_gib": 0.0,
                "disk_guard_bytes": 0,
                "projection_margin_percent": 15.0,
                "progress_seconds": 0.0,
                "expected_duration": 100.0,
                "output_bytes": 0,
            }
            try:
                decision, snapshot = guard.decide(request)
                self.assertEqual(decision["action"], "continue")
                self.assertIsNotNone(snapshot.memory.available_gib)
                self.assertIsNotNone(snapshot.disk)
            finally:
                guard.close()


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
            run_kwargs = {
                "expected_duration": 1.0,
                "input_size": 1024,
                "label": "test",
                "memory_guard_gib": 0.0,
                "output_path": out,
                "disk_guard_gib": 0.0,
                "batch_number": 1,
                "batch_total": 2,
            }
            if os.environ.get("HEVC_REBUILD_NATIVE_HELPER"):
                with patch(
                    "hevc_rebuild.encode.snapshot",
                    side_effect=AssertionError("Rust-launched encoding should use the Rust resource sampler"),
                ):
                    run_encode(args, **run_kwargs)
            else:
                run_encode(args, **run_kwargs)
            self.assertTrue(out.exists())
            self.assertGreater(out.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
