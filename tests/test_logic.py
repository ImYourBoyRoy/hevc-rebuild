from __future__ import annotations

import unittest

from hevc_rebuild.encode import _disposition_value
from hevc_rebuild.model import StreamRef
from hevc_rebuild.probe import choose_primary_video, _target_pix_fmt


class SelectionTests(unittest.TestCase):
    def test_4k_beats_default_1080p(self):
        v1080 = StreamRef(index=0, type_ordinal=0, codec_type="video", width=1920, height=1080, codec_name="h264", default=True)
        v4k = StreamRef(index=1, type_ordinal=1, codec_type="video", width=3840, height=2160, codec_name="hevc", default=False)
        primary, secondary = choose_primary_video([v1080, v4k])
        self.assertEqual(primary.index, 1)
        self.assertEqual([v.index for v in secondary], [0])

    def test_attached_picture_never_selected(self):
        cover = StreamRef(index=0, type_ordinal=0, codec_type="video", width=6000, height=9000, codec_name="mjpeg", attached_pic=True)
        movie = StreamRef(index=1, type_ordinal=1, codec_type="video", width=1920, height=1080, codec_name="h264")
        primary, _ = choose_primary_video([cover, movie])
        self.assertEqual(primary.index, 1)


class PixelFormatTests(unittest.TestCase):
    def test_source_bit_depth_is_preserved_by_default(self):
        self.assertEqual(_target_pix_fmt(8, "4:2:0", False), "yuv420p")
        self.assertEqual(_target_pix_fmt(10, "4:2:2", False), "yuv422p10le")
        self.assertEqual(_target_pix_fmt(12, "4:4:4", False), "yuv444p12le")

    def test_8bit_can_be_promoted_explicitly(self):
        self.assertEqual(_target_pix_fmt(8, "4:2:0", True), "yuv420p10le")

    def test_odd_bit_depth_is_not_silently_changed(self):
        from hevc_rebuild.util import RebuildError
        with self.assertRaises(RebuildError):
            _target_pix_fmt(9, "4:2:0", False)


class DispositionTests(unittest.TestCase):
    def test_exact_audio_disposition(self):
        self.assertEqual(_disposition_value({"default": 1, "forced": 1}), "default+forced")
        self.assertEqual(_disposition_value({"default": 0}), "0")
        self.assertEqual(_disposition_value({"default": 1, "original": 1}, force_default=False), "original")
        self.assertEqual(_disposition_value({"default": 0, "original": 1}, force_default=True), "default+original")


if __name__ == "__main__":
    unittest.main()

class EncodeInvariantTests(unittest.TestCase):
    def test_only_primary_video_is_transcoded(self):
        from pathlib import Path
        from hevc_rebuild.encode import encode_args
        from hevc_rebuild.model import MediaProfile, Toolchain

        primary = StreamRef(index=2, type_ordinal=1, codec_type="video", width=3840, height=2160, codec_name="h264", pix_fmt="yuv420p10le", bit_depth=10)
        secondary = StreamRef(index=0, type_ordinal=0, codec_type="video", width=1920, height=1080, codec_name="h264")
        audio = StreamRef(index=1, type_ordinal=0, codec_type="audio", codec_name="truehd", disposition={"default": 1})
        sub = StreamRef(index=3, type_ordinal=0, codec_type="subtitle", codec_name="hdmv_pgs_subtitle", disposition={"forced": 1})
        profile = MediaProfile(
            path=Path("input.mkv"), probe={}, mediainfo={}, primary_video=primary,
            secondary_videos=[secondary], audio=[audio], subtitles=[sub], attachments=[], data=[],
            chapters_count=0, duration=100.0, source_bit_depth=10, source_chroma="4:2:0",
            target_pix_fmt="yuv420p10le", color_primaries=None, transfer=None, color_matrix=None,
            color_range_x265=None, color_range_ffmpeg=None, chroma_location=None, master_display=None,
            master_display_source=None, max_cll=None, max_fall=None, hdr10=False, hdr10plus=False,
            hdr10plus_hint=False, hlg=False, dolby_vision=False, dovi_profile=None, dovi_level=None,
            dovi_rpu_present=False, dovi_el_present=False, dovi_bl_present=False, dovi_compatibility_id=None, dovi_source=None,
            interlaced=False, format_level=None,
        )
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool", {"yuv420p10le"})
        args, _ = encode_args(
            tools, profile, Path("out.mkv"), crf=21, preset="slow", tune="animation", hdr10plus_json=None,
            preserve_dolbyvision=False, strip_dolbyvision=False, test_seconds=None, preserve_source_level=False, disable_hdr10_opt=False,
            secondary_video_policy="copy", data_policy="preserve", force_deinterlace=False,
            disable_deinterlace=False,
        )
        joined = " ".join(args)
        self.assertIn("-c copy", joined)
        self.assertIn("-c:v:0 libx265", joined)
        self.assertIn("-preset:v:0 slow", joined)
        self.assertIn("-tune:v:0 animation", joined)
        self.assertNotIn("-c:a", joined)
        self.assertNotIn("-c:s", joined)
        self.assertEqual(args[args.index("-map") + 1], "0:2")  # selected 4K stream first


class DolbyVisionEncodeArgTests(unittest.TestCase):
    def test_native_dolbyvision_switch_is_explicit(self):
        from pathlib import Path
        from hevc_rebuild.encode import encode_args
        from hevc_rebuild.model import MediaProfile, Toolchain

        primary = StreamRef(index=0, type_ordinal=0, codec_type="video", width=3840, height=2160, codec_name="hevc", pix_fmt="yuv420p10le", bit_depth=10)
        profile = MediaProfile(
            path=Path("dv.mkv"), probe={}, mediainfo={}, primary_video=primary, secondary_videos=[], audio=[], subtitles=[], attachments=[], data=[],
            chapters_count=0, duration=10.0, source_bit_depth=10, source_chroma="4:2:0", target_pix_fmt="yuv420p10le",
            color_primaries="bt2020", transfer="smpte2084", color_matrix="bt2020nc", color_range_x265="limited", color_range_ffmpeg="tv",
            chroma_location=2, master_display=None, master_display_source=None, max_cll=None, max_fall=None, hdr10=True, hdr10plus=False,
            hdr10plus_hint=False, hlg=False, dolby_vision=True, dovi_profile=8, dovi_level=6, dovi_rpu_present=True,
            dovi_el_present=False, dovi_bl_present=True, dovi_compatibility_id=1, dovi_source="FFprobe DOVI configuration record", interlaced=False, format_level=None,
        )
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool", {"yuv420p10le"}, True)
        args, _ = encode_args(
            tools, profile, Path("out.mkv"), crf=21, preset="slow", tune=None, hdr10plus_json=None, preserve_dolbyvision=True, strip_dolbyvision=False,
            test_seconds=None, preserve_source_level=False, disable_hdr10_opt=False, secondary_video_policy="copy", data_policy="preserve",
            force_deinterlace=False, disable_deinterlace=False,
        )
        joined = " ".join(args)
        self.assertIn("-dolbyvision:v:0 1", joined)

        fallback_args, _ = encode_args(
            tools, profile, Path("fallback.mkv"), crf=21, preset="slow", tune=None, hdr10plus_json=None,
            preserve_dolbyvision=False, strip_dolbyvision=True, test_seconds=None, preserve_source_level=False,
            disable_hdr10_opt=False, secondary_video_policy="copy", data_policy="preserve",
            force_deinterlace=False, disable_deinterlace=False,
        )
        self.assertIn("-dolbyvision:v:0 0", " ".join(fallback_args))


if __name__ == "__main__":
    unittest.main()
