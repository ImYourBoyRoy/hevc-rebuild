from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild import config, deps
from hevc_rebuild.probe import _dovi_configuration
from hevc_rebuild.hdr import _pipe_close_is_expected, dolbyvision_hdr10_fallback_reasons
from hevc_rebuild.content import resolve_tune
from hevc_rebuild.util import RebuildError


class ConfigTests(unittest.TestCase):
    def test_defaults_are_source_faithful_and_slow(self):
        self.assertEqual(config.DEFAULTS["video"]["crf"], 21)
        self.assertEqual(config.DEFAULTS["video"]["preset"], "slow")
        self.assertFalse(config.DEFAULTS["video"]["promote_8bit_to_10bit"])
        self.assertEqual(config.DEFAULTS["hdr"]["dolby_vision"], "safe")
        self.assertTrue(config.DEFAULTS["hdr"]["preserve_hdr10plus"])
        self.assertEqual(config.DEFAULTS["metadata"]["text_blacklist"], [])

    def test_metadata_blacklist_validates_as_individual_patterns(self):
        self.assertEqual(config.validate_metadata_text_blacklist(["Movie*Group", "Explicit"]), ["Movie*Group", "Explicit"])
        for invalid in ("", "***", " Movie", "Movie ", "x" * 129):
            with self.subTest(invalid=invalid), self.assertRaises(RebuildError):
                config.validate_metadata_text_blacklist([invalid])
        with self.assertRaises(RebuildError):
            config.validate_metadata_text_blacklist(["Movie*Group", "movie*group"])

    def test_cli_blacklist_patterns_add_to_config_patterns(self):
        from hevc_rebuild.app import parser
        args = parser().parse_args([".", "--metadata-text-blacklist", "Movie*Group", "--metadata-text-blacklist", "SpoilerTag"])
        cfg = {**config.DEFAULTS, "metadata": {**config.DEFAULTS["metadata"], "text_blacklist": ["ReleaseGroup"]}}
        config.apply_to_args(args, cfg)
        self.assertEqual(args.metadata_text_blacklist, ["ReleaseGroup", "Movie*Group", "SpoilerTag"])

    def test_internal_settings_forwarding_replaces_the_loaded_pattern_list(self):
        from hevc_rebuild.app import parser
        args = parser().parse_args([".", "--replace-metadata-text-blacklist", "--metadata-text-blacklist", "Movie*Group"])
        cfg = {**config.DEFAULTS, "metadata": {**config.DEFAULTS["metadata"], "text_blacklist": ["ReleaseGroup"]}}
        config.apply_to_args(args, cfg)
        self.assertEqual(args.metadata_text_blacklist, ["Movie*Group"])
        cleared = parser().parse_args([".", "--replace-metadata-text-blacklist"])
        config.apply_to_args(cleared, cfg)
        self.assertEqual(cleared.metadata_text_blacklist, [])

    def test_local_config_overrides_user_defaults(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            user = root / "user.toml"
            user.write_text('[video]\npreset="slow"\ncrf=21\n', encoding="utf-8")
            (root / ".hevc-rebuild.toml").write_text('[video]\npreset="slower"\n', encoding="utf-8")
            with patch.object(config, "ensure_user_config", return_value=user):
                cfg, loaded = config.load_config(root)
            self.assertEqual(cfg["video"]["preset"], "slower")
            self.assertEqual(cfg["video"]["crf"], 21)
            self.assertEqual(loaded, [user, root / ".hevc-rebuild.toml"])

    def test_default_config_text_is_valid_toml(self):
        import tomllib
        parsed = tomllib.loads(config.default_config_text())
        self.assertEqual(parsed["calibration"]["mode"], "auto")
        self.assertEqual(parsed["calibration"]["cache_hours"], 24.0)
        self.assertEqual(parsed["calibration"]["max_candidates"], 2)
        self.assertEqual(parsed["resources"]["reserved_cpu_threads"], 1)
        self.assertEqual(parsed["resources"]["memory_reserve_percent"], 8.0)
        self.assertEqual(parsed["resources"]["min_free_disk_gib"], 0.0)
        self.assertEqual(parsed["metadata"]["text_blacklist"], [])

    def test_old_resource_config_migrates_without_overwriting_choices(self):
        import tomllib
        old = """[resources]
threading = "auto"
max_worker_threads = 4
frame_threads = 1
min_free_memory_gib = 3.0

[video]
crf = 19
"""
        updated, changed = config._insert_resource_options(old)
        self.assertTrue(changed)
        parsed = tomllib.loads(updated)
        self.assertEqual(parsed["resources"]["max_worker_threads"], 4)
        self.assertEqual(parsed["resources"]["frame_threads"], 1)
        self.assertEqual(parsed["video"]["crf"], 19)
        self.assertEqual(parsed["resources"]["reserved_cpu_threads"], 1)
        self.assertEqual(parsed["resources"]["min_free_disk_gib"], 0.0)
        self.assertEqual(parsed["calibration"]["mode"], "auto")

    def test_migration_preserves_existing_calibration_choices(self):
        import tomllib
        old = '''[calibration]
mode = "off"
sample_seconds = 5.0

[resources]
threading = "auto"
max_worker_threads = 8
frame_threads = 1
min_free_memory_gib = 3.0
memory_reserve_percent = 8.0
min_free_disk_gib = 8.0
preflight_output_ratio = 1.30
disk_projection_margin_percent = 15.0
'''
        updated, changed = config._insert_resource_options(old)
        parsed = tomllib.loads(updated)
        self.assertTrue(changed)
        self.assertEqual(parsed["calibration"]["mode"], "off")
        self.assertEqual(parsed["calibration"]["sample_seconds"], 5.0)
        self.assertEqual(parsed["resources"]["max_worker_threads"], 8)

    def test_cli_wins_over_config(self):
        args = SimpleNamespace(
            crf=19, preset=None, promote_8bit_to_10bit=None, tune=None, detect_animation=None,
            preserve_hdr10plus=None, dolby_vision_policy=None, secondary_video=None, data_streams=None,
            comment=None, metadata_text_blacklist=None, verify_copy_hashes=None,
        )
        config.apply_to_args(args, config.DEFAULTS)
        self.assertEqual(args.crf, 19)
        self.assertEqual(args.preset, "slow")
        self.assertFalse(args.promote_8bit_to_10bit)


class ContentTuneTests(unittest.TestCase):
    def test_kodi_animation_genre_enables_animation_tune(self):
        with tempfile.TemporaryDirectory() as td:
            media = Path(td) / "Film.mkv"
            media.write_bytes(b"x")
            media.with_suffix(".nfo").write_text("<movie><genre>Animation</genre></movie>", encoding="utf-8")
            self.assertEqual(resolve_tune(media, {}, "auto", True), "animation")

    def test_auto_does_not_guess_grain(self):
        with tempfile.TemporaryDirectory() as td:
            media = Path(td) / "Old Film.mkv"
            media.write_bytes(b"x")
            self.assertIsNone(resolve_tune(media, {}, "auto", True))
            self.assertEqual(resolve_tune(media, {}, "grain", True), "grain")


class DolbyVisionCapabilityTests(unittest.TestCase):
    def test_encoder_option_detection(self):
        help_text = "  -dolbyvision       <boolean>    E..V....... Enable Dolby Vision RPU coding"
        self.assertTrue(deps.x265_supports_dolbyvision(help_text))
        self.assertFalse(deps.x265_supports_dolbyvision("Encoder libx265"))

    def test_ffprobe_dovi_record_is_parsed_conservatively(self):
        side = [{
            "side_data_type": "DOVI configuration record",
            "dv_profile": 8, "dv_level": 6, "rpu_present_flag": 1,
            "el_present_flag": 0, "bl_present_flag": 1,
            "dv_bl_signal_compatibility_id": 1,
        }]
        self.assertEqual(_dovi_configuration(side, {}), (8, 6, True, False, True, 1, "FFprobe DOVI configuration record"))

    def test_generic_rpu_text_does_not_invent_dolby_vision(self):
        side = [{"side_data_type": "Unrelated RPU statistics"}]
        self.assertEqual(_dovi_configuration(side, {}), (None, None, False, False, False, None, None))

    def test_profile7_hdr10_base_is_eligible_for_explicit_fallback(self):
        profile = SimpleNamespace(dovi_profile=7, dovi_bl_present=True, hdr10=True)
        self.assertEqual(dolbyvision_hdr10_fallback_reasons(profile), [])
        profile.dovi_bl_present = False
        self.assertTrue(dolbyvision_hdr10_fallback_reasons(profile))

    def test_expected_hdr10plus_broken_pipe_is_classified_as_pipeline_shutdown(self):
        stderr = "Error submitting a packet to the muxer: Broken pipe\nError muxing a packet\nError writing trailer: Broken pipe\n"
        self.assertTrue(_pipe_close_is_expected(stderr))
        self.assertFalse(_pipe_close_is_expected("Invalid data found when processing input"))


if __name__ == "__main__":
    unittest.main()
