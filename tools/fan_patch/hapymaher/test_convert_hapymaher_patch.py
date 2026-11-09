"""Synthetic character allocation and input guards; no game data required."""
from pathlib import Path
import tempfile
import unittest

import convert_hapymaher_patch as patch


class ConverterTests(unittest.TestCase):
    def test_existing_sjis_and_allocated_chinese_codes(self):
        flags = bytearray(65536)
        sjis = [63] * 65536
        # Preserve an existing SJIS mapping, then allocate new Unicode in order.
        sjis[ord("中")] = 0x889F
        for char in "中简体":
            flags[ord(char)] = 1
        mapping = patch.character_map(bytes(flags), tuple(sjis))
        self.assertEqual(mapping[0x889F], ord("中"))
        self.assertEqual(mapping[0x88A0], ord("体"))
        self.assertEqual(mapping[0x88A1], ord("简"))
        self.assertEqual(mapping[127], 127)
        self.assertEqual(mapping[ord("A")], ord("A"))
        self.assertNotIn(0x889E, mapping)

    def test_allocation_skips_excluded_trail_byte(self):
        flags = bytearray(65536)
        for code in range(0x4000, 0x4100):
            flags[code] = 1
        mapping = patch.character_map(bytes(flags), (63,) * 65536)
        self.assertNotIn(0x899F, mapping)
        self.assertIn(0x89A0, mapping)
        for bad in (bytes(65535), bytes([2]) + bytes(65535)):
            with self.assertRaises(ValueError):
                patch.character_map(bad, (63,) * 65536)

    def test_reject_inputs_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaises(OSError):
                patch.convert(root / "missing", root / "patch.zip", root / "output")
            self.assertFalse((root / "output").exists())
            with self.assertRaisesRegex(ValueError, "outside"):
                patch.convert(root, root / "patch.zip", root / "output")


if __name__ == "__main__":
    unittest.main()
