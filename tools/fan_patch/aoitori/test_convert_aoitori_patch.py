"""Synthetic Longinus boundaries and omitted empty-string terminators."""
import struct
import unittest

import convert_aoitori_patch as patch


def archive(entries):
    offset = 16 + 12 * len(entries)
    index = bytearray(b"Longinus\x01\x04\x00\x01" + struct.pack("<I", len(entries)))
    data = bytearray()
    for key, value in entries:
        index.extend(struct.pack("<III", key, len(value), offset))
        data.extend(value)
        offset += len(value)
    return index + data


class ConverterTests(unittest.TestCase):
    def test_longinus_ranges_duplicates_and_trailing_bytes(self):
        raw = archive([(1, b"abc"), (2, b"defg")])
        self.assertEqual(patch.longinus_entries(raw), {1: b"abc", 2: b"defg"})
        overlap = bytearray(raw)
        struct.pack_into("<I", overlap, 36, 40)
        for bad in (raw[:10], raw[:20], raw[:-1], raw + b"x", overlap,
                    archive([(1, b"a"), (1, b"b")])):
            with self.subTest(raw=bad), self.assertRaises(ValueError):
                patch.longinus_entries(bad)

    def test_only_missing_trailing_empty_strings_are_restored(self):
        raw = b"PS2A" + struct.pack("<11I", 48, 1, 1, 0, 2, 0, 5, 0, 0, 7, 0)
        raw += b"\x30\x04a\0\0\0\0"
        self.assertEqual(patch.normalize_script(raw), raw)
        for missing in (1, 2):
            self.assertEqual(patch.normalize_script(raw[:-missing]), raw)
        for bad in (raw[:20], raw[:-3], raw[:-2] + b"x", raw + b"x"):
            with self.subTest(raw=bad), self.assertRaises(ValueError):
                patch.normalize_script(bad)


if __name__ == "__main__":
    unittest.main()
