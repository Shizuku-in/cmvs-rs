"""Synthetic compressed vectors; no game or translation data required."""
from pathlib import Path
import struct
import tempfile
import unittest

import convert_miranosu_patch as patch

# LE32 control words interleaved with literals, an end marker, and two matches.
LITERALS = bytes.fromhex("0c00000003000000499224e141424300004025ff")
MATCHES = bytes.fromhex("0d00000009000000499270ee4142430400a09224ff")


class ConverterTests(unittest.TestCase):
    def test_literals_new_offset_and_repeated_offset(self):
        self.assertEqual(patch.decode_mirai(LITERALS), b"ABC")
        self.assertEqual(patch.decode_mirai(MATCHES), b"ABCABCABC")

    def test_truncated_streams_and_declared_lengths(self):
        for vector in (LITERALS, MATCHES):
            for end in range(len(vector)):
                raw = bytearray(vector[:end])
                if end >= 8:
                    struct.pack_into("<I", raw, 0, end - 8)
                with self.subTest(end=end), self.assertRaises(ValueError):
                    patch.decode_mirai(bytes(raw))
            for expected in (1, 2, 100, patch.LIMIT + 1):
                raw = bytearray(vector)
                struct.pack_into("<I", raw, 4, expected)
                with self.assertRaises(ValueError):
                    patch.decode_mirai(bytes(raw))

    def test_match_before_output_and_trailing_bytes(self):
        raw = bytearray(MATCHES)
        raw[15] = 250  # first match now points before the initial three literals
        with self.assertRaisesRegex(ValueError, "bounds"):
            patch.decode_mirai(bytes(raw))
        raw = bytearray(LITERALS + b"unused")
        struct.pack_into("<I", raw, 0, len(raw) - 8)
        with self.assertRaisesRegex(ValueError, "length mismatch"):
            patch.decode_mirai(bytes(raw))

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
