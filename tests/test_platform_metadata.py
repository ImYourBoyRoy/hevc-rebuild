import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import xml.etree.ElementTree as ET

from hevc_rebuild import deps, metadata
from hevc_rebuild.model import Toolchain
from hevc_rebuild.util import RebuildError


class PlatformTests(unittest.TestCase):
    def setUp(self):
        managed = tempfile.TemporaryDirectory()
        self.addCleanup(managed.cleanup)
        self.managed_bin = Path(managed.name)
        patcher = patch.object(deps, "MANAGED_BIN", self.managed_bin)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_metadata_helper_discovery_managed_path_and_missing_precedence(self):
        executable = self.managed_bin / "hdr10plus_tool"
        with patch.object(deps, "_native_toolchain", return_value=None), patch.object(deps, "validate_platform"), patch.object(deps, "run", return_value=type("Result", (), {"returncode": 0})()), patch.object(deps, "get_x265_help", side_effect=AssertionError("metadata must not probe x265")), patch.object(deps, "which") as lookup:
            lookup.side_effect = lambda name: f"/path/{name}"
            executable.write_text("#!/bin/sh\nexit 99\n", encoding="utf-8")
            executable.chmod(0o700)
            self.assertEqual(deps.resolve_toolchain(install_missing=False, purpose="metadata").hdr10plus_tool, str(executable))
            executable.chmod(0o600)
            self.assertEqual(deps.resolve_toolchain(install_missing=False, purpose="metadata").hdr10plus_tool, "/path/hdr10plus_tool")
            executable.unlink()
            lookup.side_effect = lambda name: None if name == "hdr10plus_tool" else f"/path/{name}"
            self.assertEqual(deps.resolve_toolchain(install_missing=False, purpose="metadata").hdr10plus_tool, "hdr10plus_tool")

    @patch.object(deps, "native_json_action")
    def test_metadata_toolchain_does_not_require_x265_or_hdr10plus_helper(self, native_action):
        native_action.return_value = {
            "ffmpeg": "/tools/ffmpeg",
            "ffprobe": "/tools/ffprobe",
            "mediainfo": "/tools/mediainfo",
            "mkvpropedit": "/tools/mkvpropedit",
            "mkvmerge": "/tools/mkvmerge",
            "mkvextract": "/tools/mkvextract",
            "x265_pixel_formats": [],
            "x265_dolbyvision": False,
        }
        resolved = deps._native_toolchain(
            "metadata",
            allow_unsupported_os=False,
            ffmpeg_path=None,
            tool_paths=None,
        )
        self.assertEqual(resolved.ffmpeg, "/tools/ffmpeg")
        self.assertEqual(resolved.mkvmerge, "/tools/mkvmerge")
        self.assertEqual(resolved.x265_pixel_formats, set())
        self.assertEqual(resolved.hdr10plus_tool, "hdr10plus_tool")
        request = native_action.call_args.args[1]
        self.assertEqual(request["purpose"], "metadata")

    @patch.object(deps, "_native_toolchain", return_value=None)
    @patch.object(deps, "get_x265_help", side_effect=AssertionError("metadata mode must not probe x265"))
    @patch.object(deps, "run", return_value=type("Result", (), {"returncode": 0, "stdout": "", "stderr": ""})())
    @patch.object(deps, "validate_platform")
    @patch.object(deps, "which")
    def test_direct_metadata_toolchain_keeps_hdr_helper_optional_and_skips_x265(
        self, which, _validate, _run, _x265, _native
    ):
        available = {
            "ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract",
        }
        which.side_effect = lambda name: f"/usr/bin/{name}" if name in available else None
        resolved = deps.resolve_toolchain(install_missing=False, purpose="metadata")
        self.assertEqual(resolved.hdr10plus_tool, "hdr10plus_tool")
        self.assertEqual(resolved.x265_pixel_formats, set())

    @patch.object(deps, "which")
    @patch.object(deps, "os_release")
    def test_linux_mint_22_ubuntu_base_is_supported(self, osr, which):
        osr.return_value = {"ID": "linuxmint", "VERSION_ID": "22.3", "ID_LIKE": "ubuntu debian", "UBUNTU_CODENAME": "noble"}
        which.side_effect = lambda name: f"/usr/bin/{name}"
        deps.validate_platform()

    @patch.object(deps, "which")
    @patch.object(deps, "os_release")
    def test_ubuntu_24_is_supported(self, osr, which):
        osr.return_value = {"ID": "ubuntu", "VERSION_ID": "24.04", "ID_LIKE": "debian"}
        which.side_effect = lambda name: f"/usr/bin/{name}"
        deps.validate_platform()

    @patch.object(deps, "which")
    @patch.object(deps, "os_release")
    def test_lmde_is_not_mistaken_for_ubuntu_mint(self, osr, which):
        osr.return_value = {"ID": "linuxmint", "VERSION_ID": "7", "ID_LIKE": "debian"}
        which.side_effect = lambda name: f"/usr/bin/{name}"
        with self.assertRaises(RebuildError):
            deps.validate_platform()

    @patch.object(deps, "_brew_program", return_value="/opt/homebrew/bin/brew")
    @patch.object(deps.platform, "system", return_value="Darwin")
    def test_macos_uses_homebrew_and_universal_hdr_asset(self, _system, _brew):
        deps.validate_platform()
        self.assertEqual(deps.arch_asset_token(), "universal-macOS")


class MetadataMergeTests(unittest.TestCase):
    def test_reviewed_metadata_overrides_nfo_and_episode_airdate_takes_precedence_over_series_year(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            media = root / "Episode.mkv"
            media.touch()
            media.with_suffix(".nfo").write_text(
                "<episodedetails><title>Old title</title><showtitle>Old series</showtitle>"
                "<year>2010</year><aired>2010-01-01</aired><userrating>6</userrating></episodedetails>",
                encoding="utf-8",
            )
            with patch.object(metadata, "_read_existing_global_root", return_value=ET.Element("Tags")):
                xml_path, title, _nfo, expected, _changes = metadata.build_tags(
                    tools,
                    media,
                    root,
                    comment="",
                    metadata_override={
                        "title": "Reviewed episode",
                        "show_title": "Reviewed series",
                        "year": "2024",
                        "airdate": "2024-05-12",
                        "genres": ["Animation", "Adventure"],
                        "rating": "8.5",
                    },
                )
            self.assertEqual(title, "Reviewed episode")
            self.assertEqual(expected["SHOWTITLE"], "Reviewed series")
            self.assertEqual(expected["DATE_RELEASED"], "2024-05-12")
            self.assertEqual(expected["GENRE"], "Animation, Adventure")
            self.assertEqual(expected["RATING"], "4.25")
            self.assertIn(("SHOWTITLE", "Reviewed series"), {
                (node.findtext("Name"), node.findtext("String"))
                for node in ET.parse(xml_path).findall("./Tag/Simple")
            })

    def test_existing_global_tags_survive_and_kodi_values_win(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            media = root / "Movie.mkv"
            media.write_bytes(b"placeholder")
            media.with_suffix(".nfo").write_text(
                "<movie><title>New Title</title><plot>Plot text</plot><genre>Drama</genre><genre>Sci-Fi</genre></movie>",
                encoding="utf-8",
            )
            existing = ET.Element("Tags")
            tag = ET.SubElement(existing, "Tag")
            ET.SubElement(tag, "Targets")
            for name, value in (("ORIGINAL_MEDIA_TYPE", "Blu-ray"), ("TITLE", "Old Title")):
                simple = ET.SubElement(tag, "Simple")
                ET.SubElement(simple, "Name").text = name
                ET.SubElement(simple, "String").text = value

            with patch.object(metadata, "_read_existing_global_root", return_value=existing):
                xml_path, title, nfo, expected, changes = metadata.build_tags(tools, media, root, comment="Test Comment")

            self.assertEqual(title, "New Title")
            self.assertEqual(nfo, media.with_suffix(".nfo"))
            self.assertEqual(expected["TITLE"], "New Title")
            self.assertEqual(expected["GENRE"], "Drama, Sci-Fi")
            self.assertEqual(changes, [])
            out = ET.parse(xml_path).getroot()
            pairs = []
            for simple in out.findall("./Tag/Simple"):
                pairs.append((simple.findtext("Name"), simple.findtext("String")))
            self.assertIn(("ORIGINAL_MEDIA_TYPE", "Blu-ray"), pairs)
            self.assertIn(("TITLE", "New Title"), pairs)
            self.assertNotIn(("TITLE", "Old Title"), pairs)

    def test_app_managed_nfo_overrides_matching_fields_and_maps_rating_to_matroska_scale(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            media = root / "Film.mkv"
            media.write_bytes(b"placeholder")
            media.with_suffix(".nfo").write_text(
                "<movie><title>Old title</title><genre>Drama</genre></movie>",
                encoding="utf-8",
            )
            managed = root / "Film.hevc-rebuild.nfo"
            managed.write_text(
                "<movie><title>Reviewed title</title><year>2011</year><mpaa>G</mpaa>"
                "<tagline>Short copy</tagline><plot>Long copy</plot><genre>Animation</genre>"
                "<userrating>7.50</userrating><source>https://www.themoviedb.org/movie/1</source></movie>",
                encoding="utf-8",
            )
            with patch.object(metadata, "_read_existing_global_root", return_value=ET.Element("Tags")):
                xml_path, title, nfo, expected, _changes = metadata.build_tags(tools, media, root, comment="")

            self.assertEqual(title, "Reviewed title")
            self.assertEqual(nfo, managed)
            self.assertEqual(expected["DATE_RELEASED"], "2011")
            self.assertEqual(expected["LAW_RATING"], "G")
            self.assertEqual(expected["GENRE"], "Animation")
            self.assertEqual(expected["RATING"], "3.75")
            self.assertEqual(expected["SOURCE"], "https://www.themoviedb.org/movie/1")
            values = {(node.findtext("Name"), node.findtext("String")) for node in ET.parse(xml_path).findall("./Tag/Simple")}
            self.assertIn(("RATING", "3.75"), values)

    def test_episode_nfo_maps_series_episode_plot_and_air_date(self):
        tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            season = root / "Season 01"
            season.mkdir()
            media = season / "S01E02.mkv"
            media.write_bytes(b"placeholder")
            (root / "tvshow.nfo").write_text(
                "<tvshow><genre>Animation</genre><userrating>8.00</userrating><source>https://www.tvmaze.com/shows/1</source></tvshow>",
                encoding="utf-8",
            )
            media.with_name("S01E02.hevc-rebuild.nfo").write_text(
                "<episodedetails><title>The Frog Prince</title><showtitle>Faerie Tale Theatre</showtitle>"
                "<season>1</season><episode>2</episode><aired>1982-09-12</aired><plot>Reviewed episode plot</plot></episodedetails>",
                encoding="utf-8",
            )
            with patch.object(metadata, "_read_existing_global_root", return_value=ET.Element("Tags")):
                xml_path, _title, _nfo, expected, _changes = metadata.build_tags(tools, media, root, comment="")
            self.assertEqual(expected["TITLE"], "The Frog Prince")
            self.assertEqual(expected["SHOWTITLE"], "Faerie Tale Theatre")
            self.assertEqual(expected["SEASON"], "1")
            self.assertEqual(expected["EPISODE"], "2")
            self.assertEqual(expected["DATE_RELEASED"], "1982-09-12")
            self.assertEqual(expected["SYNOPSIS"], "Reviewed episode plot")
            self.assertEqual(expected["GENRE"], "Animation")
            self.assertEqual(expected["RATING"], "4")
            self.assertEqual(expected["SOURCE"], "https://www.tvmaze.com/shows/1")


class PosterSelectionTests(unittest.TestCase):
    def _profile(self, path: Path):
        from types import SimpleNamespace
        return SimpleNamespace(path=path)

    def test_no_matching_poster_returns_none(self):
        with tempfile.TemporaryDirectory() as td:
            media = Path(td) / "Movie.mkv"
            media.write_bytes(b"x")
            self.assertIsNone(metadata.choose_poster(self._profile(media)))

    def test_matching_movie_poster_is_selected(self):
        with tempfile.TemporaryDirectory() as td:
            media = Path(td) / "Movie.mkv"
            poster = Path(td) / "Movie-poster.jpg"
            media.write_bytes(b"x")
            poster.write_bytes(b"poster")
            self.assertEqual(metadata.choose_poster(self._profile(media)), poster)

    def test_unrelated_image_is_not_selected(self):
        with tempfile.TemporaryDirectory() as td:
            media = Path(td) / "Movie.mkv"
            unrelated = Path(td) / "poster.jpg"
            media.write_bytes(b"x")
            unrelated.write_bytes(b"poster")
            self.assertIsNone(metadata.choose_poster(self._profile(media)))


if __name__ == "__main__":
    unittest.main()
