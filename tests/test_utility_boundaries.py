import json
import copy
import csv
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild import util


class FiniteNumberTests(unittest.TestCase):
    def test_probe_duration_rejects_nonfinite_tool_metadata(self):
        from hevc_rebuild import probe
        for value in (float("nan"), float("inf"), "inf", "nan"):
            metadata = {"streams": [{"index": 0, "codec_type": "video", "width": 64, "height": 64}], "format": {"duration": value}}
            with patch.object(probe, "_frame_probe", return_value={"frames": []}), self.assertRaises(util.RebuildError):
                probe.build_profile(None, Path("synthetic.mkv"), probe_data=metadata, mediainfo_data={})
    def test_probe_dimensions_native_resources_and_config_reject_nonfinite(self):
        from hevc_rebuild import config, probe
        from hevc_rebuild.encode import _native_optional_number, _native_resource_snapshot
        for value in (float("nan"), float("inf"), -float("inf")):
            for field in ("width", "height", "index"):
                with self.subTest(field=field, value=value), self.assertRaises(util.RebuildError):
                    probe.stream_refs({"streams": [{"codec_type": "video", field: value}]})
            with self.assertRaises(util.RebuildError):
                _native_optional_number(value, "available_gib")
            with self.assertRaises(util.RebuildError):
                _native_resource_snapshot({"memory_available_gib": value})
            for field in ("min_free_memory_gib", "min_free_disk_gib", "preflight_output_ratio", "disk_projection_margin_percent"):
                cfg = copy.deepcopy(config.DEFAULTS)
                cfg["resources"][field] = value
                with self.subTest(config=field), self.assertRaises(util.RebuildError):
                    config.validate_config(cfg)

    def test_invalid_progress_and_duration_forecasts_fail_without_plausible_estimates(self):
        from hevc_rebuild.ui import ProgressView
        from hevc_rebuild.youtube import youtube_output_estimate
        for value in (float("nan"), float("inf"), -float("inf")):
            view = ProgressView(label="synthetic", expected_duration=10, memory_guard_gib=0, input_size=1)
            with self.assertRaises(util.RebuildError):
                view.render(done=1, speed=value, size=1, resources=None)
            with self.assertRaises(util.RebuildError):
                youtube_output_estimate(SimpleNamespace(duration=value), "source")

    def test_reports_represent_nonfinite_numbers_as_empty_values(self):
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": ""}):
            report = Path(directory) / "hevc-rebuild-numeric.csv"
            util.append_csv(report, {"elapsed_seconds": float("nan"), "input_bytes": float("inf")})
            with report.open(newline="") as stream:
                row = next(csv.DictReader(stream))
            self.assertEqual(row["elapsed_seconds"], "")
            self.assertEqual(row["input_bytes"], "")

    def test_invalid_values_are_unknown_and_finite_values_are_preserved(self):
        for value in (None, "bad", "nan", "inf", "-inf", float("nan"), float("inf"), -float("inf"), "inf/2", "1/0", "0/0"):
            with self.subTest(value=value):
                self.assertIsNone(util.safe_float(value))
                self.assertEqual(util.duration_text(value), "?")
                self.assertEqual(util.human_bytes(value), "unknown")
        self.assertEqual(util.safe_float("30000/1001"), 30000/1001)
        self.assertEqual(util.safe_float(0), 0)
        self.assertEqual(util.duration_text(3661.1), "01:01:01")
        self.assertEqual(util.human_bytes(1024), "1.00 KiB")

    def test_json_serialization_rejects_nonfinite_nested_numbers(self):
        with self.assertRaises(ValueError):
            util.dataclass_json({"memory": {"available": float("inf")}})
        self.assertEqual(json.loads(util.dataclass_json({"memory": None})), {"memory": None})


class JsonBoundaryTests(unittest.TestCase):
    def test_dependency_release_shape_is_validated_without_network(self):
        from hevc_rebuild import deps
        from unittest.mock import MagicMock
        for document in ([], None, {}, {"assets": {}, "tag_name": "v1"}):
            with patch.object(deps.urllib.request, "urlopen", return_value=MagicMock()), patch.object(deps.json, "load", return_value=document), self.assertRaises(util.RebuildError):
                deps.github_latest_hdr10plus()
        with patch.object(deps.urllib.request, "urlopen", return_value=MagicMock()), patch.object(deps.json, "load", return_value={"assets": [], "tag_name": "v1"}):
            self.assertEqual(deps.github_latest_hdr10plus()["assets"], [])

    def test_object_shape_and_required_keys(self):
        for text in ("[]", "null", "1", '"string"', "broken"):
            with self.subTest(text=text), patch.object(util, "run", return_value=SimpleNamespace(stdout=text)):
                with self.assertRaisesRegex(util.RebuildError, "fixture-tool"):
                    util.run_json_object(["fixture-tool"], required_keys=("streams",))
        with patch.object(util, "run", return_value=SimpleNamespace(stdout='{}')):
            with self.assertRaisesRegex(util.RebuildError, "missing required fields: streams"):
                util.run_json_object(["fixture-tool"], required_keys=("streams",))
        with patch.object(util, "run", return_value=SimpleNamespace(stdout='{"streams": []}')):
            self.assertEqual(util.run_json_object(["fixture-tool"], required_keys=("streams",)), {"streams": []})
        with patch.object(util, "run", return_value=SimpleNamespace(stdout='[]')):
            self.assertEqual(util.run_json(["fixture-tool"]), [])

    def test_ffprobe_mediainfo_and_mkv_nested_shapes(self):
        from hevc_rebuild import probe, metadata
        tools = SimpleNamespace(ffprobe="fixture-probe", mediainfo="fixture-mediainfo", mkvmerge="fixture-mkvmerge")
        for document in ({"streams": {}}, {"streams": [None]}, {"streams": [], "format": []}):
            with patch.object(util, "run", return_value=SimpleNamespace(stdout=json.dumps(document))), self.assertRaises(util.RebuildError):
                probe.ffprobe_json(tools, Path("synthetic.mkv"))
        for document in ([], {"media": []}, {"media": {"track": {}}}):
            with patch.object(probe, "run", return_value=SimpleNamespace(stdout=json.dumps(document))), self.assertRaises(util.RebuildError):
                probe.mediainfo_json(tools, Path("synthetic.mkv"))
        for document in ([], {}, {"tracks": {}}, {"tracks": [None]}):
            with patch.object(util, "run", return_value=SimpleNamespace(stdout=json.dumps(document))), self.assertRaises(util.RebuildError):
                metadata.mkv_identify(tools, Path("synthetic.mkv"))
