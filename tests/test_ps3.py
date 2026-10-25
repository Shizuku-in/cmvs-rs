import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from ps3_inspect import FormatError, inspect, lzss_decode


def fixture(payload=None, compressed=True):
    # One label, two-byte code segment and an independently constructed text segment.
    payload = payload if payload is not None else struct.pack("<I", 0) + b"\x00\x20" + b"hello\0"
    packed = b"".join(b"\xff" + payload[i:i+8] for i in range(0, len(payload), 8)) if compressed else payload
    key = 0x5D3F3020
    xor, shift = ((key >> 3) + (key >> 24)) & 255, (key >> 20) % 5 + 1
    encoded = bytearray()
    for b in packed:
        encoded.append(((((b << shift) | (b >> (8 - shift))) & 255) ^ xor) + 124 & 255)
    return b"PS2A" + struct.pack("<11I", 48, 0, key, 1, 2, 0, 6, 0,
                                len(encoded) if compressed else 0, len(payload), 0) + encoded


class ScriptTests(unittest.TestCase):
    def test_compressed_script(self):
        info, payload = inspect(fixture())
        self.assertEqual(info["labels"], [0])
        self.assertEqual(info["strings"][0]["text"], "hello")
        self.assertEqual(len(payload), 12)

    def test_uncompressed_but_encoded_script(self):
        self.assertEqual(inspect(fixture(compressed=False))[0]["strings"][0]["text"], "hello")

    def test_overlapping_backreference(self):
        # 'A' stored at 0x7df; copy three bytes beginning there, including newly written bytes.
        self.assertEqual(lzss_decode(b"\x01A\xdf\xe1", 4), b"AAAA")

    def test_output_overflow(self):
        with self.assertRaises(FormatError):
            lzss_decode(b"\x01A\xdf\xe1", 3)

    def test_truncated_reference(self):
        with self.assertRaises(FormatError):
            lzss_decode(b"\x00\xdf", 4)

    def test_truncated_payload(self):
        with self.assertRaises(FormatError):
            inspect(fixture()[:-1])

    def test_invalid_label(self):
        with self.assertRaises(FormatError):
            inspect(fixture(struct.pack("<I", 200) + b"\x00\x20hello\0"))

    def test_size_limit(self):
        with self.assertRaises(FormatError):
            lzss_decode(b"", 2**32)


if __name__ == "__main__":
    unittest.main()
