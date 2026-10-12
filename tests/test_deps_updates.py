import contextlib
import io
import subprocess
import unittest
from unittest.mock import patch

from hevc_rebuild import bridge, deps
from hevc_rebuild.util import RebuildError


class AptUpdateTests(unittest.TestCase):
    def setUp(self):
        self.status = {
            "packages": [
                {"name": "ffmpeg", "installed_version": "1", "candidate_version": "2"},
                {"name": "mediainfo", "installed_version": "1", "candidate_version": "2"},
            ],
            "catalog_stale": False,
        }

    def test_confirmation_requires_the_exact_current_allowlisted_list(self):
        self.assertEqual(
            deps.validate_apt_update_confirmation(self.status, ["mediainfo", "ffmpeg"]),
            ["ffmpeg", "mediainfo"],
        )
        for supplied in (["ffmpeg"], ["ffmpeg", "mediainfo", "mediainfo"], ["ffmpeg", "other"]):
            with self.subTest(supplied=supplied), self.assertRaises(RebuildError):
                deps.validate_apt_update_confirmation(self.status, list(supplied))

    def test_confirmation_requires_fresh_catalog_and_nonempty_updates(self):
        stale = {**self.status, "catalog_stale": True}
        empty = {**self.status, "packages": []}
        for status, supplied in ((stale, ["ffmpeg", "mediainfo"]), (empty, [])):
            with self.subTest(status=status), self.assertRaises(RebuildError):
                deps.validate_apt_update_confirmation(status, supplied)

    def test_simulated_plan_must_match_exactly_and_remove_nothing(self):
        packages = ["ffmpeg", "mediainfo"]
        plan = "Inst ffmpeg [1] (2 Ubuntu:24.04/stable)\nInst mediainfo:amd64 [1] (2 Ubuntu:24.04/stable)\n"
        deps.validate_apt_upgrade_plan(packages, plan)
        with self.assertRaisesRegex(RebuildError, "beyond the confirmed"):
            deps.validate_apt_upgrade_plan(packages, plan + "Inst libc6 [1] (2 Ubuntu:24.04/stable)\n")
        with self.assertRaisesRegex(RebuildError, "removals"):
            deps.validate_apt_upgrade_plan(packages, plan + "Rem unrelated [1]\n")
        with self.assertRaises(RebuildError):
            deps.validate_apt_upgrade_plan(packages, "", returncode=100)

    @patch.object(deps, "apt_update_status")
    @patch.object(deps, "sudo", side_effect=lambda command: command)
    @patch.object(deps, "run")
    def test_upgrade_simulates_before_applying_and_returns_json_safe_summary(self, run, _sudo, status):
        status.return_value = self.status
        run.side_effect = [
            subprocess.CompletedProcess([], 0, "Inst ffmpeg [1] (2 Ubuntu:24.04/stable)\nInst mediainfo [1] (2 Ubuntu:24.04/stable)\n", ""),
            subprocess.CompletedProcess([], 0, "", ""),
        ]

        result = deps.upgrade_apt_packages(["ffmpeg", "mediainfo"])

        self.assertEqual(result["upgraded"], ["ffmpeg", "mediainfo"])
        self.assertEqual(run.call_count, 2)
        self.assertIn("-s", run.call_args_list[0].args[0])
        actual = run.call_args_list[1].args[0]
        self.assertIn("--only-upgrade", actual)
        self.assertIn("--no-remove", actual)
        self.assertIn("--yes", actual)

    @patch.object(bridge, "upgrade_apt_packages", return_value={"upgraded": ["ffmpeg"]})
    def test_json_bridge_requires_explicit_confirmation_and_emits_one_json_document(self, upgrade):
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors):
            self.assertEqual(bridge.main(["apt-upgrade", "--package", "ffmpeg"]), 1)
        self.assertIn("require --confirm", errors.getvalue())
        upgrade.assert_not_called()

        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(
                bridge.main(["apt-upgrade", "--confirm", "--package", "ffmpeg"]),
                0,
            )
        self.assertEqual(output.getvalue(), '{"upgraded": ["ffmpeg"]}\n')
        upgrade.assert_called_once_with(["ffmpeg"])


class ReadOnlyCheckToolchainTests(unittest.TestCase):
    @patch.object(deps, "native_json_action")
    def test_check_purpose_does_not_require_optional_or_encoding_helpers(self, native_action):
        native_action.return_value = {
            "ffmpeg": "/usr/bin/ffmpeg",
            "ffprobe": "/usr/bin/ffprobe",
            "x265_pixel_formats": [],
            "x265_dolbyvision": False,
        }

        toolchain = deps._native_toolchain(
            "check",
            allow_unsupported_os=False,
            ffmpeg_path=None,
            tool_paths=None,
        )

        self.assertEqual(toolchain.ffmpeg, "/usr/bin/ffmpeg")
        self.assertEqual(toolchain.ffprobe, "/usr/bin/ffprobe")
        self.assertEqual(toolchain.mediainfo, "mediainfo")
        self.assertEqual(toolchain.hdr10plus_tool, "hdr10plus_tool")
        self.assertEqual(native_action.call_args.args[1]["purpose"], "check")

    @patch.object(deps, "ensure_apt_dependencies")
    @patch.object(deps, "_native_toolchain")
    def test_check_mode_never_attempts_dependency_repair(self, native_toolchain, install):
        expected = object()
        native_toolchain.return_value = expected

        result = deps.resolve_toolchain(purpose="check", install_missing=True)

        self.assertIs(result, expected)
        native_toolchain.assert_called_once()
        install.assert_not_called()


if __name__ == "__main__":
    unittest.main()
