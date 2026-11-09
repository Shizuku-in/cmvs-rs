"""Synthetic PKP/PS2A tests, no game assets required."""
from pathlib import Path
import struct
import tempfile
import unittest
import warnings
import zipfile
import zlib

import convert_amatsutsumi_patch as patch
from ps3_inspect import decode_script


def pkp(entries):
    out = bytearray(b"PKP\0\0\0\0\0" + struct.pack("<I", len(entries)))
    data = bytearray()
    offset = 12 + 176 * len(entries)
    for name, raw, compress in entries:
        if compress:
            encoder = zlib.compressobj(wbits=-15)
            stored = encoder.compress(raw) + encoder.flush()
        else:
            stored = raw
        record = bytearray(176)
        struct.pack_into("<III", record, 0, offset, len(raw), len(stored))
        field = (name + "\0").encode("utf-16le")
        record[48:48 + len(field)] = field
        out.extend(record)
        data.extend(stored)
        offset += len(stored)
    return bytes(out + data)


class ConverterTests(unittest.TestCase):
    def test_pkp_stored_and_raw_deflate(self):
        self.assertEqual(patch.unpack_pkp(pkp([("a.ps3", b"A" * 4096, True),
                                              ("b.png", b"XYZ", False)])),
                         {"a.ps3": b"A" * 4096, "b.png": b"XYZ"})

    def test_pkp_rejects_unsafe_duplicate_names(self):
        for name in ("../a", "a/b", "a\\b", "a:b", ".", "..", "中文"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                patch.unpack_pkp(pkp([(name, b"A", False)]))
        with self.assertRaises(ValueError):
            patch.unpack_pkp(pkp([("a", b"A", False), ("a", b"B", False)]))

    def test_pkp_rejects_truncation_bounds_and_excess_decompression(self):
        raw = pkp([("a", b"A" * 4096, True)])
        for end in (0, 8, 12, 187, len(raw) - 1):
            with self.subTest(end=end), self.assertRaises(ValueError):
                patch.unpack_pkp(raw[:end])
        for offset, value in ((12, 0), (16, patch.LIMIT + 1), (16, 4), (20, patch.LIMIT + 1)):
            bad = bytearray(raw)
            struct.pack_into("<I", bad, offset, value)
            with self.subTest(offset=offset, value=value), self.assertRaises(ValueError):
                patch.unpack_pkp(bad)
        with self.assertRaises(ValueError):
            patch.unpack_pkp(raw + b"tail")

    def test_plain_lzss_script_becomes_standard_encrypted_ps2a(self):
        payload = b"\x30\x04hello\0"
        header = b"PS2A" + struct.pack("<11I", 48, 1, 0x12345678, 0, 2, 0, 6, 0, 9, 8, 0)
        raw = header + b"\xff" + payload
        out = patch.encode_script(raw)
        info, decoded = decode_script(out)
        self.assertEqual(decoded, payload)
        self.assertEqual(info["compressed_size"], 0)
        with self.assertRaises(ValueError):
            patch.encode_script(raw[:-1])

    def test_unsupported_encoding_does_not_create_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaises(ValueError):
                patch.convert(root / "game", root / "patch.zip", root / "out", "utf-16")
            self.assertFalse((root / "out").exists())

    def test_zip_requires_pinned_members_and_rejects_duplicates(self):
        import io
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            archive.writestr("readme.txt", "fixture")
        buffer.seek(0)
        with tempfile.TemporaryDirectory() as temp, zipfile.ZipFile(buffer) as archive:
            with self.assertRaisesRegex(ValueError, "Missing"):
                patch.extract_patch(archive, Path(temp))
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive, warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            archive.writestr("duplicate", "A")
            archive.writestr("duplicate", "B")
        buffer.seek(0)
        with tempfile.TemporaryDirectory() as temp, zipfile.ZipFile(buffer) as archive:
            with self.assertRaisesRegex(ValueError, "Duplicate"):
                patch.extract_patch(archive, Path(temp))


if __name__ == "__main__":
    unittest.main()
