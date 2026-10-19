"""Preserve the return contract of historical encode-module resource helpers."""
import unittest
from unittest.mock import patch

from hevc_rebuild import encode, resources


class ResourceCompatibilityTests(unittest.TestCase):
    def test_meminfo_parser_delegates_to_canonical_implementation(self):
        with patch.object(resources, "_meminfo_values", return_value={"MemAvailable": 123}) as parser:
            self.assertEqual(encode._meminfo_values(), {"MemAvailable": 123})
            parser.assert_called_once_with()

    def test_historical_cgroup_alias_means_effective_available_memory(self):
        stats = resources.MemoryStats(16, 3, 4, 2)
        with patch.object(encode, "memory_stats", return_value=stats):
            self.assertEqual(encode._cgroup_available_gib(), 3)
            self.assertEqual(encode._meminfo_gib(), (3, 2))

    def test_unknown_values_remain_none(self):
        with patch.object(encode, "memory_stats", return_value=resources.MemoryStats(None, None, None, None)):
            self.assertIsNone(encode._cgroup_available_gib())
            self.assertEqual(encode._meminfo_gib(), (None, None))
