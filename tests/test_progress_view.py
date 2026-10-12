from __future__ import annotations

import io
import json
import os
import unittest
from contextlib import redirect_stderr
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.ui import ProgressView, emit_gui_progress
from hevc_rebuild.util import ConsoleClosed


def empty_resources() -> SimpleNamespace:
    return SimpleNamespace(
        cpu_percent=None,
        memory=SimpleNamespace(
            available_gib=None,
            swap_used_gib=None,
            swap_total_gib=None,
            swap_used_percent=None,
        ),
        swap_activity=None,
        process_rss_gib=None,
        process_swap_gib=None,
        disk=None,
        temperature_c=None,
        pressure=SimpleNamespace(io_full_avg10=None),
    )


class ProgressViewTests(unittest.TestCase):
    def test_machine_progress_is_silent_outside_the_desktop_engine(self) -> None:
        output = io.StringIO()
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "0"}), redirect_stderr(output):
            emit_gui_progress(
                file_name="Film.mkv",
                phase="Encoding",
                batch_number=1,
                batch_total=1,
                file_percent=25.0,
                queue_percent=25.0,
            )
        self.assertEqual(output.getvalue(), "")

    def test_machine_progress_is_bounded_and_includes_exact_path(self) -> None:
        output = io.StringIO()
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "1"}), redirect_stderr(output):
            emit_gui_progress(
                file_name="Film.mkv",
                file_path="/media/Film.mkv",
                phase="Encoding",
                batch_number=2,
                batch_total=4,
                file_percent=40.0,
                queue_percent=35.0,
                speed=float("nan"),
                eta_seconds=60.0,
                elapsed_seconds=25.0,
                input_bytes=1000,
                output_bytes=400,
                estimated_output_bytes=1000,
            )
        line = output.getvalue().strip()
        self.assertTrue(line.startswith("#HEVC_REBUILD_PROGRESS\t"))
        event = json.loads(line.partition("\t")[2])
        self.assertEqual(event["filePath"], "/media/Film.mkv")
        self.assertEqual(event["fileName"], "Film.mkv")
        self.assertEqual(event["batchNumber"], 2)
        self.assertEqual(event["queuePercent"], 35.0)
        self.assertIsNone(event["speed"])
        self.assertEqual(event["estimatedOutputBytes"], 1000)

    def test_progress_reports_source_current_output_and_live_size_estimate(self) -> None:
        output = io.StringIO()
        view = ProgressView(
            label="Film.mkv",
            expected_duration=100,
            memory_guard_gib=3,
            input_size=500 * 1024**2,
            batch_number=1,
            batch_total=1,
        )
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "0"}), redirect_stderr(output):
            view.render(
                done=20,
                speed=0.5,
                size=80 * 1024**2,
                resources=empty_resources(),
            )

        rendered = output.getvalue()
        self.assertIn("Input 500.00 MiB", rendered)
        self.assertIn("Output 80.00 MiB", rendered)
        self.assertIn("Live estimate 400.00 MiB", rendered)
        self.assertIn("ETA", rendered)

    def test_progress_labels_projection_as_calculating_before_enough_data(self) -> None:
        output = io.StringIO()
        view = ProgressView(
            label="Film.mkv",
            expected_duration=100,
            memory_guard_gib=3,
            input_size=500 * 1024**2,
        )
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "0"}), redirect_stderr(output):
            view.render(
                done=5,
                speed=0.5,
                size=8 * 1024**2,
                resources=empty_resources(),
            )

        self.assertIn("Live estimate calculating", output.getvalue())

    def test_desktop_activity_log_keeps_percent_milestones_only(self) -> None:
        output = io.StringIO()
        view = ProgressView(
            label="Film.mkv",
            expected_duration=100,
            memory_guard_gib=3,
            input_size=500 * 1024**2,
            batch_number=1,
            batch_total=1,
        )
        view.enabled = False
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "1"}), redirect_stderr(output):
            for done in (0.2, 0.4, 5.0, 5.2):
                view._last_gui_update_at = 0.0
                view.render(
                    done=done,
                    speed=0.12,
                    size=4 * 1024**2,
                    resources=empty_resources(),
                )

        rendered = output.getvalue()
        self.assertNotIn("Input ", rendered)
        self.assertNotIn("Live estimate", rendered)
        self.assertEqual(rendered.count("  0%"), 1)
        self.assertEqual(rendered.count("  5%"), 1)
        self.assertEqual(rendered.count("#HEVC_REBUILD_PROGRESS"), 4)

    def test_closed_desktop_pipe_raises_console_closed(self) -> None:
        view = ProgressView(
            label="Film.mkv",
            expected_duration=100,
            memory_guard_gib=3,
            input_size=500 * 1024**2,
        )
        view.enabled = False
        with patch.dict(os.environ, {"HEVC_REBUILD_GUI_EVENTS": "1"}):
            with patch("hevc_rebuild.ui.print", side_effect=BrokenPipeError(32, "Broken pipe")):
                with self.assertRaises(ConsoleClosed):
                    view.render(
                        done=1,
                        speed=0.1,
                        size=1024,
                        resources=empty_resources(),
                    )


if __name__ == "__main__":
    unittest.main()
