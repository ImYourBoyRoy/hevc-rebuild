from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from hevc_rebuild.metadata import (
    _scrub_chapter_tree,
    _scrub_tag_tree,
    apply_metadata,
    build_tags,
    clean_metadata_text,
    choose_poster,
    _poster_attachment_description,
    scrub_release_group_suffix,
)
from hevc_rebuild.app import parse_file_metadata_overrides, parse_file_poster_overrides
from hevc_rebuild.model import Toolchain
from hevc_rebuild.util import RebuildError


class ReleaseGroupSuffixTests(unittest.TestCase):
    def test_removes_clear_scene_suffix(self):
        value = "Film.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HEVC-TEPES"
        self.assertEqual(
            scrub_release_group_suffix(value),
            "Film.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HEVC",
        )

    def test_removes_bracketed_release_group_after_release_markers(self):
        self.assertEqual(
            scrub_release_group_suffix("Film 1080p WEB-DL x265 [YTS.MX]"),
            "Film 1080p WEB-DL x265",
        )

    def test_keeps_hyphenated_titles_and_non_release_names(self):
        for value in (
            "Spider-Man (2002)",
            "Film.1080p.WEB-DL.x265",
            "A story about WEB-DL distribution",
            "A title - Criterion Collection",
            "Film x265 - Resurrection",
        ):
            with self.subTest(value=value):
                self.assertEqual(scrub_release_group_suffix(value), value)

    def test_known_lowercase_group_is_scrubbed_conservatively(self):
        self.assertEqual(scrub_release_group_suffix("Film.1080p.WEB-DL.x265-yify"), "Film.1080p.WEB-DL.x265")

    def test_movie_group_glob_matches_camel_case_without_removing_surrounding_text(self):
        cleaned, rules = clean_metadata_text("Keep [movieReleaseGroup] and Movieshowgroup titles", ["Movie*Group"])
        self.assertEqual(cleaned, "Keep [] and titles")
        self.assertEqual(rules, ["Movie*Group"])
        self.assertEqual(
            clean_metadata_text("Keep   these  spaces movieReleaseGroup after", ["Movie*Group"])[0],
            "Keep   these  spaces after",
        )


class MatroskaTextScrubTests(unittest.TestCase):
    def test_build_tags_applies_glob_to_comment_and_filename_title(self):
        with tempfile.TemporaryDirectory() as temporary:
            media = Path(temporary) / "Example MovieReleaseGroup.mkv"
            work = Path(temporary) / "work"
            work.mkdir()
            tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
            with patch("hevc_rebuild.metadata._read_existing_global_root", return_value=ET.Element("Tags")):
                xml_path, title, _, expected, changes = build_tags(
                    tools,
                    media,
                    work,
                    comment="Keep Movieshowgroup metadata",
                    text_blacklist=["Movie*Group"],
                )
            self.assertEqual(title, "Example")
            self.assertEqual(expected, {"COMMENT": "Keep metadata", "TITLE": "Example"})
            self.assertEqual([change["field"] for change in changes], ["COMMENT", "TITLE", "title"])
            self.assertEqual([node.text for node in ET.parse(xml_path).getroot().iter("String")], ["Keep metadata", "Example"])

    def test_file_comment_override_replaces_the_global_comment(self):
        with tempfile.TemporaryDirectory() as temporary:
            media = Path(temporary) / "Example.mkv"
            work = Path(temporary) / "work"
            work.mkdir()
            tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
            with patch("hevc_rebuild.metadata._read_existing_global_root", return_value=ET.Element("Tags")):
                _, _, _, expected, _ = build_tags(
                    tools,
                    media,
                    work,
                    comment="Global comment",
                    metadata_override={"comment": "Episode comment"},
                )
            self.assertEqual(expected["COMMENT"], "Episode comment")

    def test_replaced_source_tags_do_not_leave_empty_tag_elements(self):
        with tempfile.TemporaryDirectory() as temporary:
            media = Path(temporary) / "Chaos Theory (2008 PG-13).mkv"
            work = Path(temporary) / "work"
            work.mkdir()
            tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
            existing = ET.fromstring(
                """<Tags>
                  <Tag><Simple><Name>TITLE</Name><String>Chaos Theory</String></Simple><Targets><TargetTypeValue>50</TargetTypeValue></Targets></Tag>
                  <Tag><Simple><Name>GENRE</Name><String>Comedy</String></Simple><Targets><TargetTypeValue>50</TargetTypeValue></Targets></Tag>
                </Tags>"""
            )
            with patch("hevc_rebuild.metadata._read_existing_global_root", return_value=existing):
                xml_path, _, _, expected, _ = build_tags(
                    tools,
                    media,
                    work,
                    comment="Brought To you By V1x0r",
                    metadata_override={"title": "Chaos Theory"},
                )
            tags = list(ET.parse(xml_path).getroot())
            self.assertTrue(tags)
            self.assertTrue(all(tag.find("Simple") is not None for tag in tags))
            self.assertEqual(expected["TITLE"], "Chaos Theory")
            self.assertIn("Comedy", [node.text for node in ET.parse(xml_path).getroot().iter("String")])

    def test_tags_scrub_global_and_track_text_but_preserve_technical_values(self):
        root = ET.fromstring(
            """<Tags>
              <Tag><Targets/><Simple><Name>COMMENT</Name><String>Film.1080p.WEB-DL.x265-GROUP</String></Simple>
                <Simple><Name>RELEASE_GROUP</Name><String>GROUP</String></Simple>
                <Simple><Name>ENCODER</Name><String>libx265</String></Simple>
              </Tag>
              <Tag><Targets><TrackUID>99</TrackUID></Targets>
                <Simple><Name>TITLE</Name><String>Audio.1080p.WEB-DL-GROUP</String></Simple>
              </Tag>
            </Tags>"""
        )

        changes = _scrub_tag_tree(root)
        values = [node.text for node in root.iter("String")]

        self.assertEqual(values, ["Film.1080p.WEB-DL.x265", "libx265", "Audio.1080p.WEB-DL"])
        self.assertEqual(len(changes), 3)
        self.assertEqual(changes[0]["scope"], "global")
        self.assertEqual(changes[1]["field"], "RELEASE_GROUP")
        self.assertEqual(changes[2]["scope"], "track_tag")

    def test_custom_pattern_scrubs_technical_tag_text_but_preserves_other_data(self):
        root = ET.fromstring(
            """<Tags><Tag><Targets/>
              <Simple><Name>COMMENT</Name><String>Keep movieReleaseGroup text</String></Simple>
              <Simple><Name>ENCODER</Name><String>libx265 movieReleaseGroup encoder identifier</String></Simple>
              <Simple><Name>STATISTICS_WRITING_APP</Name><String>MovieReleaseGroup tool fps=24</String></Simple>
            </Tag></Tags>"""
        )
        changes = _scrub_tag_tree(root, ["Movie*Group"])
        values = {node.findtext("Name"): node.findtext("String") for node in root.iter("Simple")}
        self.assertEqual(values["COMMENT"], "Keep text")
        self.assertEqual(values["ENCODER"], "libx265 encoder identifier")
        self.assertEqual(values["STATISTICS_WRITING_APP"], "tool fps=24")
        self.assertEqual(len(changes), 3)
        self.assertTrue(all(change["rules"] == ["Movie*Group"] for change in changes))

    def test_custom_pattern_applies_to_chapter_titles(self):
        root = ET.fromstring(
            """<Chapters><EditionEntry><ChapterAtom><ChapterTimeStart>00:00:01.000000000</ChapterTimeStart>
              <ChapterDisplay><ChapterString>Keep Movieshowgroup title</ChapterString></ChapterDisplay>
            </ChapterAtom></EditionEntry></Chapters>"""
        )
        changes, edits = _scrub_chapter_tree(root, ["Movie*Group"])
        self.assertEqual(root.findtext(".//ChapterString"), "Keep title")
        self.assertEqual(changes[0]["rules"], ["Movie*Group"])
        self.assertEqual(edits[0]["after"], "Keep title")

    def test_chapters_scrub_display_names_and_retain_timing(self):
        root = ET.fromstring(
            """<Chapters><EditionEntry><ChapterAtom><ChapterTimeStart>00:00:01.000000000</ChapterTimeStart>
              <ChapterDisplay><ChapterString>Film.1080p.WEB-DL-GROUP</ChapterString><ChapterLanguage>eng</ChapterLanguage></ChapterDisplay>
              <ChapterAtom><ChapterTimeStart>00:00:02.000000000</ChapterTimeStart>
                <ChapterDisplay><ChapterString>Next scene</ChapterString></ChapterDisplay>
              </ChapterAtom>
            </ChapterAtom></EditionEntry></Chapters>"""
        )

        changes, edits = _scrub_chapter_tree(root)

        self.assertEqual([node.text for node in root.iter("ChapterString")], ["Film.1080p.WEB-DL", "Next scene"])
        self.assertEqual(changes[0]["chapter_index"], 0)
        self.assertEqual(edits, [{"scope": "chapter", "chapter_index": 0, "before": "Film.1080p.WEB-DL-GROUP", "after": "Film.1080p.WEB-DL"}])

    def test_output_scrub_preserves_all_unmodified_tag_xml_and_records_commands(self):
        with tempfile.TemporaryDirectory() as temporary:
            work_dir = Path(temporary)
            output = work_dir / "output.mkv"
            output.write_bytes(b"temporary output")
            global_tags = work_dir / "global-tags.xml"
            global_tags.write_text("<Tags/>", encoding="utf-8")
            profile = SimpleNamespace(path=work_dir / "source.mkv")
            tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
            calls: list[list[str]] = []

            def fake_run(args, **kwargs):
                command = list(args)
                calls.append(command)
                if command[0] == "mkvextract":
                    destination = Path(command[3])
                    if command[2] == "tags":
                        destination.write_text(
                            """<Tags>
                              <Tag><Targets/><Simple><Name>COMMENT</Name><String>Film.1080p.WEB-DL.x265-GROUP</String></Simple>
                                <Simple><Name>ENCODER</Name><String>libx265</String></Simple></Tag>
                            </Tags>""",
                            encoding="utf-8",
                        )
                    elif command[2] == "chapters":
                        destination.write_text(
                            """<Chapters><EditionEntry><ChapterAtom><ChapterTimeStart>00:00:00.000000000</ChapterTimeStart>
                              <ChapterDisplay><ChapterString>Film.1080p.WEB-DL-GROUP</ChapterString></ChapterDisplay>
                            </ChapterAtom></EditionEntry></Chapters>""",
                            encoding="utf-8",
                        )
                return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

            with (
                patch("hevc_rebuild.metadata.build_tags", return_value=(global_tags, "Film", None, {"TITLE": "Film"}, [])),
                patch("hevc_rebuild.metadata.choose_poster", return_value=None),
                patch(
                    "hevc_rebuild.metadata.mkv_identify",
                    return_value={"tracks": [{"type": "video", "properties": {"track_name": "Film.1080p.WEB-DL-GROUP"}}]},
                ),
                patch("hevc_rebuild.metadata.run", side_effect=fake_run),
            ):
                result = apply_metadata(tools, output, profile, work_dir, comment="")

            self.assertEqual([change["scope"] for change in result.scrubbed_text_values], ["global", "chapter", "track_header"])
            self.assertEqual(len(result.chapter_text_edits), 1)
            tags_call = next(call for call in calls if "--tags" in call and "all:" in call[call.index("--tags") + 1])
            sanitized_tags = Path(tags_call[tags_call.index("--tags") + 1].split(":", 1)[1])
            values = [node.text for node in ET.parse(sanitized_tags).getroot().iter("String")]
            self.assertEqual(values, ["Film.1080p.WEB-DL.x265", "libx265"])
            self.assertTrue(any("--chapters" in call for call in calls))
            track_call = next(call for call in calls if "--edit" in call and "track:1" in call)
            self.assertIn("name=Film.1080p.WEB-DL", track_call)

    def test_tag_extraction_failure_aborts_metadata_write(self):
        with tempfile.TemporaryDirectory() as temporary:
            work_dir = Path(temporary)
            output = work_dir / "output.mkv"
            output.write_bytes(b"temporary output")
            global_tags = work_dir / "global-tags.xml"
            global_tags.write_text("<Tags/>", encoding="utf-8")
            profile = SimpleNamespace(path=work_dir / "source.mkv")
            tools = Toolchain("ffmpeg", "ffprobe", "mediainfo", "mkvpropedit", "mkvmerge", "mkvextract", "hdr10plus_tool")
            failed = subprocess.CompletedProcess([], 2, stdout="", stderr="unable to read tags")

            with (
                patch("hevc_rebuild.metadata.build_tags", return_value=(global_tags, "Film", None, {"TITLE": "Film"}, [])),
                patch("hevc_rebuild.metadata.choose_poster", return_value=None),
                patch("hevc_rebuild.metadata.run", return_value=failed),
            ):
                with self.assertRaisesRegex(RebuildError, "Could not safely inspect Matroska tags"):
                    apply_metadata(tools, output, profile, work_dir, comment="")


class ArtworkSidecarTests(unittest.TestCase):
    def test_explicit_local_poster_selection_precedes_adjacent_art(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            episode = root / "Film.mkv"
            episode.touch()
            default_poster = root / "Film-poster.jpg"
            selected_poster = root / "My Poster.png"
            default_poster.touch()
            selected_poster.touch()

            self.assertEqual(choose_poster(SimpleNamespace(path=episode), selected_poster), selected_poster)

    def test_poster_override_parser_accepts_scoped_valid_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            media = root / "Film.mkv"
            poster = root / "Artwork" / "poster.png"
            media.touch()
            poster.parent.mkdir()
            poster.write_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x00")

            parsed = parse_file_poster_overrides(
                json.dumps({str(media): str(poster)}), root.resolve()
            )

            self.assertEqual(parsed[str(media.resolve())], poster.resolve())

    def test_poster_override_parser_accepts_only_explicit_cache_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            cache = base / "profile" / ".cache" / "artwork"
            root.mkdir()
            cache.mkdir(parents=True)
            media = root / "Film.mkv"
            media.touch()
            poster = cache / "poster.webp"
            poster.write_bytes(b"RIFF0000WEBPVP8 ")

            parsed = parse_file_poster_overrides(
                json.dumps({str(media): str(poster)}), root.resolve(), cache.resolve()
            )

            self.assertEqual(parsed[str(media.resolve())], poster.resolve())
            with self.assertRaisesRegex(RebuildError, "inside the selected input"):
                parse_file_poster_overrides(json.dumps({str(media): str(poster)}), root.resolve())

    def test_poster_override_parser_rejects_outside_paths_and_bad_image_headers(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            root.mkdir()
            media = root / "Film.mkv"
            media.touch()
            outside = base / "poster.jpg"
            outside.write_bytes(b"\xff\xd8\xff" + b"0" * 12)
            with self.assertRaisesRegex(RebuildError, "inside the selected input"):
                parse_file_poster_overrides(json.dumps({str(media): str(outside)}), root.resolve())

            bad = root / "poster.jpg"
            bad.write_bytes(b"not an image".ljust(12, b" "))
            with self.assertRaisesRegex(RebuildError, "does not match"):
                parse_file_poster_overrides(json.dumps({str(media): str(bad)}), root.resolve())

    def test_episode_prefers_season_poster_and_falls_back_to_series_folder_art(self):
        with tempfile.TemporaryDirectory() as temporary:
            series = Path(temporary) / "Example Series"
            season = series / "Season 01"
            season.mkdir(parents=True)
            episode = season / "S01E01.Pilot.mkv"
            episode.touch()
            series_poster = series / "folder.jpg"
            series_poster.touch()

            profile = SimpleNamespace(path=episode)
            self.assertEqual(choose_poster(profile), series_poster)

            season_poster = season / "poster.png"
            season_poster.touch()
            self.assertEqual(choose_poster(profile), season_poster)

            episode_poster = season / "S01E01.Pilot-poster.webp"
            episode_poster.touch()
            self.assertEqual(choose_poster(profile), episode_poster)

    def test_season_poster_beats_a_series_poster_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            series = Path(temporary) / "Example Series"
            season = series / "Season 01"
            season.mkdir(parents=True)
            episode = season / "S01E01.Pilot.mkv"
            episode.touch()
            series_poster = series / "poster.jpg"
            season_poster = season / "poster.jpg"
            episode_poster = season / "S01E01.Pilot-poster.jpg"
            series_poster.touch()
            season_poster.touch()
            profile = SimpleNamespace(path=episode)

            self.assertEqual(choose_poster(profile, series_poster), season_poster)

            episode_poster.touch()
            self.assertEqual(choose_poster(profile, series_poster), episode_poster)
            self.assertEqual(choose_poster(profile, episode_poster), episode_poster)

    def test_artwork_attribution_is_preserved_for_the_matroska_attachment(self):
        with tempfile.TemporaryDirectory() as temporary:
            poster = Path(temporary) / "folder.jpg"
            attribution = poster.with_name(poster.name + ".attribution.json")
            attribution.write_text(
                '{"title":"Series poster","creator":"An Artist","attribution":"Credit line",'
                '"license":"CC BY-SA","sourceUrl":"https://www.tvmaze.com/shows/1/example",'
                '"licenseUrl":"https://www.tvmaze.com/api"}',
                encoding="utf-8",
            )
            description = _poster_attachment_description(poster)
            self.assertIn("Artwork: Series poster", description)
            self.assertIn("Creator: An Artist", description)
            self.assertIn("License: CC BY-SA", description)
            self.assertIn("Source: https://www.tvmaze.com/shows/1/example", description)

    def test_invalid_artwork_attribution_fails_before_embedding(self):
        with tempfile.TemporaryDirectory() as temporary:
            poster = Path(temporary) / "poster.jpg"
            poster.with_name(poster.name + ".attribution.json").write_text("not-json", encoding="utf-8")
            with self.assertRaisesRegex(RebuildError, "attribution could not be read safely"):
                _poster_attachment_description(poster)


class MetadataOverrideParserTests(unittest.TestCase):
    def test_reviewed_metadata_accepts_scoped_bounded_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            media = root / "Film.mkv"
            media.touch()
            parsed = parse_file_metadata_overrides(
                json.dumps({str(media): {
                    "title": "Reviewed title",
                    "year": "2011",
                    "genres": ["Drama", "Animation"],
                    "rating": "7.5",
                }}),
                root,
            )
            self.assertEqual(parsed[str(media.resolve())]["title"], "Reviewed title")
            self.assertEqual(parsed[str(media.resolve())]["genres"], ["Drama", "Animation"])
            self.assertEqual(parsed[str(media.resolve())]["rating"], "7.5")

    def test_reviewed_metadata_rejects_unknown_fields_invalid_values_and_external_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / "library"
            root.mkdir()
            media = root / "Film.mkv"
            media.touch()
            outside = base / "Outside.mkv"
            outside.touch()
            cases = (
                (json.dumps({str(media): {"unknown": "value"}}), "Unsupported reviewed metadata"),
                (json.dumps({str(media): {"rating": "11"}}), "rating must be a number"),
                (json.dumps({str(outside): {"title": "Outside"}}), "inside the selected input"),
                (json.dumps({str(media): {"genres": "Drama"}}), "genres must be a list"),
            )
            for raw, message in cases:
                with self.subTest(message=message), self.assertRaisesRegex(RebuildError, message):
                    parse_file_metadata_overrides(raw, root)


if __name__ == "__main__":
    unittest.main()
