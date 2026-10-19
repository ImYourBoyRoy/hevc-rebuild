from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from hevc_rebuild.encode import encode_args
from hevc_rebuild.model import MediaProfile, Toolchain
from hevc_rebuild.probe import choose_primary_video, stream_refs
from hevc_rebuild.validate import stream_copy_hash


FFMPEG = shutil.which("ffmpeg")
FFPROBE = shutil.which("ffprobe")


def has_encoder(name: str) -> bool:
    if not FFMPEG:
        return False
    cp = subprocess.run([FFMPEG, "-hide_banner", "-h", f"encoder={name}"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return cp.returncode == 0 and name in cp.stdout


@unittest.skipUnless(FFMPEG and FFPROBE and has_encoder("libx265"), "FFmpeg/libx265 not installed")
class EncodeIntegrationTests(unittest.TestCase):
    def test_highest_video_first_and_audio_subtitles_packet_exact(self):
        self._assert_video_and_copied_tracks("flac")

    def test_copied_aac_priming_timestamps_and_packets_remain_exact(self):
        self._assert_video_and_copied_tracks("aac")

    def _assert_video_and_copied_tracks(self, audio_codec):
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            srt = td / "sub.srt"
            srt.write_text("1\n00:00:00,000 --> 00:00:00,800\nPreserve me exactly\n", encoding="utf-8")
            src = td / "input.mkv"
            out = td / "output.mkv"
            # Put the lower-resolution video first/default and higher-resolution video second.
            subprocess.run([
                FFMPEG, "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=10:duration=1",
                "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=10:duration=1",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
                "-i", str(srt),
                "-map", "0:v", "-map", "1:v", "-map", "2:a", "-map", "3:s",
                "-c:v", "mpeg4", "-q:v", "5", "-c:a", audio_codec, "-c:s", "srt", "-avoid_negative_ts", "disabled",
                "-disposition:v:0", "default", "-disposition:v:1", "0",
                "-metadata:s:a:0", "language=eng", "-disposition:s:0", "forced",
                str(src),
            ], check=True)
            probe = json.loads(subprocess.run([
                FFPROBE, "-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json", str(src)
            ], check=True, stdout=subprocess.PIPE, text=True).stdout)
            refs = stream_refs(probe)
            primary, secondary = choose_primary_video(refs)
            self.assertEqual((primary.width, primary.height), (640, 360))
            profile = MediaProfile(
                path=src, probe=probe, mediainfo={}, primary_video=primary, secondary_videos=secondary,
                audio=[r for r in refs if r.codec_type == "audio"], subtitles=[r for r in refs if r.codec_type == "subtitle"],
                attachments=[], data=[], chapters_count=0, duration=1.0, source_bit_depth=8, source_chroma="4:2:0",
                target_pix_fmt="yuv420p", color_primaries=None, transfer=None, color_matrix=None,
                color_range_x265=None, color_range_ffmpeg=None, chroma_location=None, master_display=None,
                master_display_source=None, max_cll=None, max_fall=None, hdr10=False, hdr10plus=False,
                hdr10plus_hint=False, hlg=False, dolby_vision=False, dovi_profile=None, dovi_level=None,
                dovi_rpu_present=False, dovi_el_present=False, dovi_bl_present=False, dovi_compatibility_id=None, dovi_source=None,
                interlaced=False, format_level=None,
            )
            cp = subprocess.run([FFMPEG, "-hide_banner", "-h", "encoder=libx265"], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            pix_line = next(line for line in cp.stdout.splitlines() if "Supported pixel formats:" in line)
            pixels = set(pix_line.split("Supported pixel formats:", 1)[1].split())
            tools = Toolchain(FFMPEG, FFPROBE, "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool", pixels)
            args, _ = encode_args(
                tools, profile, out, crf=30, preset="ultrafast", tune="animation", hdr10plus_json=None,
                preserve_dolbyvision=False, strip_dolbyvision=False, test_seconds=None,
                preserve_source_level=False, disable_hdr10_opt=False, secondary_video_policy="copy",
                data_policy="preserve", force_deinterlace=False, disable_deinterlace=False,
            )
            # Remove progress output so the test can run as a normal subprocess.
            progress_i = args.index("-progress")
            del args[progress_i:progress_i + 2]
            stats_i = args.index("-stats_period")
            del args[stats_i:stats_i + 2]
            subprocess.run(args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

            out_probe = json.loads(subprocess.run([
                FFPROBE, "-v", "error", "-show_streams", "-of", "json", str(out)
            ], check=True, stdout=subprocess.PIPE, text=True).stdout)
            source_audio = next(s for s in probe["streams"] if s["codec_type"] == "audio")
            output_audio = next(s for s in out_probe["streams"] if s["codec_type"] == "audio")
            if audio_codec == "aac":
                self.assertLess(float(source_audio["start_time"]), 0, "Fixture must include AAC negative priming")
            self.assertAlmostEqual(float(source_audio["start_time"]), float(output_audio["start_time"]), places=3)
            out_refs = stream_refs(out_probe)
            videos = [r for r in out_refs if r.codec_type == "video" and not r.attached_pic]
            self.assertEqual((videos[0].width, videos[0].height), (640, 360))
            self.assertEqual(videos[0].codec_name, "hevc")
            self.assertTrue(videos[0].default)
            self.assertEqual((videos[1].width, videos[1].height), (320, 180))
            self.assertEqual(videos[1].codec_name, "mpeg4")
            self.assertEqual(stream_copy_hash(tools, src, "a:0"), stream_copy_hash(tools, out, "a:0"))
            self.assertEqual(stream_copy_hash(tools, src, "s:0"), stream_copy_hash(tools, out, "s:0"))
            self.assertEqual(stream_copy_hash(tools, src, "v:0"), stream_copy_hash(tools, out, "v:1"))

            # Explicit per-file selection can encode both the 4K-like primary and secondary video
            # in one MKV while the selected audio and subtitle tracks remain packet-identical.
            out_both = td / "output-both.mkv"
            selected_videos = [primary, *secondary]
            selected_streams = [*selected_videos, *profile.audio, *profile.subtitles]
            both_args, _ = encode_args(
                tools, profile, out_both, crf=30, preset="ultrafast", tune=None, hdr10plus_json=None,
                preserve_dolbyvision=False, strip_dolbyvision=False, test_seconds=None,
                preserve_source_level=False, disable_hdr10_opt=False, secondary_video_policy="copy",
                data_policy="preserve", force_deinterlace=False, disable_deinterlace=False,
                selected_streams=selected_streams,
                video_profiles={stream.index: profile for stream in selected_videos},
                encode_video_indices={stream.index for stream in selected_videos},
            )
            for progress_option in ("-progress", "-stats_period"):
                option_index = both_args.index(progress_option)
                del both_args[option_index:option_index + 2]
            subprocess.run(both_args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            both_probe = json.loads(subprocess.run([
                FFPROBE, "-v", "error", "-show_streams", "-of", "json", str(out_both)
            ], check=True, stdout=subprocess.PIPE, text=True).stdout)
            both_videos = [r for r in stream_refs(both_probe) if r.codec_type == "video" and not r.attached_pic]
            self.assertEqual([(stream.width, stream.height, stream.codec_name) for stream in both_videos], [
                (640, 360, "hevc"), (320, 180, "hevc"),
            ])
            self.assertEqual(stream_copy_hash(tools, src, "a:0"), stream_copy_hash(tools, out_both, "a:0"))
            self.assertEqual(stream_copy_hash(tools, src, "s:0"), stream_copy_hash(tools, out_both, "s:0"))


if __name__ == "__main__":
    unittest.main()
