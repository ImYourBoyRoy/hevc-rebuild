from __future__ import annotations

import os
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.hdr import (
    dolbyvision_hdr10_fallback_reasons,
    dolbyvision_safe_preservation_reasons,
    extract_hdr10plus,
    verify_hdr10plus,
)
from hevc_rebuild.model import Toolchain


@unittest.skipUnless(
    os.environ.get("HEVC_REBUILD_NATIVE_HELPER") and os.name == "posix",
    "Rust helper path and POSIX test scripts are required",
)
class NativeHdrParityTests(unittest.TestCase):
    def test_rust_dolby_vision_decisions_match_python_fallback(self):
        native_helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        profile = SimpleNamespace(
            dovi_profile=8,
            dovi_rpu_present=True,
            dovi_el_present=False,
            dovi_bl_present=True,
            hdr10=True,
            interlaced=False,
        )
        tools = Toolchain(
            "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool",
            x265_dolbyvision=True,
        )
        with patch.dict(os.environ):
            os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
            python_safe = dolbyvision_safe_preservation_reasons(profile, tools)
            python_fallback = dolbyvision_hdr10_fallback_reasons(profile)
        with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": native_helper}):
            rust_safe = dolbyvision_safe_preservation_reasons(profile, tools)
            rust_fallback = dolbyvision_hdr10_fallback_reasons(profile)
        self.assertEqual(rust_safe, python_safe)
        self.assertEqual(rust_fallback, python_fallback)

    def test_rust_hdr10plus_streaming_matches_python_fallback(self):
        native_helper = os.environ["HEVC_REBUILD_NATIVE_HELPER"]
        with TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.mkv"
            source.write_bytes(b"synthetic source")
            ffmpeg = root / "ffmpeg"
            parser = root / "hdr10plus_tool"
            ffmpeg.write_text("#!/bin/sh\nprintf 'annex-b-hevc-data\\n'\n", encoding="utf-8")
            parser.write_text(
                "#!/bin/sh\n"
                "if [ \"$1\" = \"--verify\" ]; then "
                "cat >/dev/null; printf 'Dynamic HDR10+ metadata detected\\n'; exit 0; fi\n"
                "while [ $# -gt 0 ]; do "
                "if [ \"$1\" = \"-o\" ]; then shift; output=$1; fi; shift; done\n"
                "cat >/dev/null\nprintf '{\"metadata\":true,\"frames\":[]}' > \"$output\"\n",
                encoding="utf-8",
            )
            ffmpeg.chmod(0o700)
            parser.chmod(0o700)
            tools = Toolchain(
                str(ffmpeg), "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", str(parser)
            )
            profile = SimpleNamespace(
                path=source,
                primary_video=SimpleNamespace(index=2, codec_name="hevc"),
            )
            python_output = root / "python.json"
            rust_output = root / "rust.json"

            with patch.dict(os.environ):
                os.environ.pop("HEVC_REBUILD_NATIVE_HELPER", None)
                python_verified = verify_hdr10plus(tools, profile)
                extract_hdr10plus(tools, profile, python_output)
            with patch.dict(os.environ, {"HEVC_REBUILD_NATIVE_HELPER": native_helper}):
                rust_verified = verify_hdr10plus(tools, profile)
                extract_hdr10plus(tools, profile, rust_output)

            self.assertTrue(python_verified)
            self.assertEqual(rust_verified, python_verified)
            self.assertEqual(rust_output.read_bytes(), python_output.read_bytes())


if __name__ == "__main__":
    unittest.main()
