import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import xml.etree.ElementTree as ET

from hevc_rebuild import deps, metadata
from hevc_rebuild.model import Toolchain
from hevc_rebuild.util import RebuildError


class PlatformTests(unittest.TestCase):
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


class MetadataMergeTests(unittest.TestCase):
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
                xml_path, title, nfo, expected = metadata.build_tags(tools, media, root, comment="Test Comment")

            self.assertEqual(title, "New Title")
            self.assertEqual(nfo, media.with_suffix(".nfo"))
            self.assertEqual(expected["TITLE"], "New Title")
            self.assertEqual(expected["GENRE"], "Drama, Sci-Fi")
            out = ET.parse(xml_path).getroot()
            pairs = []
            for simple in out.findall("./Tag/Simple"):
                pairs.append((simple.findtext("Name"), simple.findtext("String")))
            self.assertIn(("ORIGINAL_MEDIA_TYPE", "Blu-ray"), pairs)
            self.assertIn(("TITLE", "New Title"), pairs)
            self.assertNotIn(("TITLE", "Old Title"), pairs)


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
