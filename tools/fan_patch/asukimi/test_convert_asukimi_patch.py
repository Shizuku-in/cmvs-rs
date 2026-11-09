"""Synthetic converter tests; no game or translation assets are required."""
import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

import convert_asukimi_patch as patch


FIXTURES = Path(__file__).resolve().parents[3] / "tests/fixtures"


class ConverterTests(unittest.TestCase):
    def test_cpz_cipher_against_existing_independent_vectors(self):
        for i in range(8):
            with self.subTest(i=i), tempfile.TemporaryDirectory() as temp:
                raw = bytes.fromhex((FIXTURES / f"cpz3-{i}.hex").read_text())
                path = Path(temp) / "source.cpz"
                path.write_bytes(raw)
                name = "fixture.bin" + "y" * (i % 4)
                # Fixtures 4..7 contain duplicate names: converter deliberately refuses.
                if i >= 4:
                    with self.assertRaisesRegex(ValueError, "duplicate"):
                        patch.Cpz3(path)
                    continue
                source = patch.Cpz3(path)
                expected = b"synthetic CPZ3 payload" + b"!" * (i % 4)
                self.assertEqual(source.read(name), expected)
                self.assertEqual(patch.cipher(expected, source.entries[name][3], encrypt=True),
                                 source.encrypted(name))

    def test_writer_preserves_and_replaces_verified_resources(self):
        with tempfile.TemporaryDirectory() as temp:
            original = Path(temp) / "in.cpz"
            original.write_bytes(bytes.fromhex((FIXTURES / "cpz3-3.hex").read_text()))
            source = patch.Cpz3(original)
            name = next(iter(source.entries))
            out = Path(temp) / "out.cpz"
            patch.write_cpz3(out, source, {})
            self.assertEqual(patch.Cpz3(out).read(name), source.read(name))
            changed = Path(temp) / "changed.cpz"
            patch.write_cpz3(changed, source, {name: b"translated\0"})
            self.assertEqual(patch.Cpz3(changed).read(name), b"translated\0")
            with self.assertRaises(FileExistsError):
                patch.write_cpz3(out, source, {})

    def test_stale_checksum_normalization_does_not_weaken_reader(self):
        raw = bytearray(bytes.fromhex((FIXTURES / "cpz3-0.hex").read_text()))
        raw[-1] ^= 1
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "test.cpz"
            path.write_bytes(raw)
            reader = patch.Cpz3(path)
            with self.assertRaisesRegex(ValueError, "resource checksum"):
                reader.read(next(iter(reader.entries)))
            fixed, count = patch.normalize_script_archive(bytes(raw))
            self.assertEqual(count, 1)
            path.write_bytes(fixed)
            reader = patch.Cpz3(path)
            reader.read(next(iter(reader.entries)))
            broken = bytearray(fixed)
            broken[12] ^= 1
            path.write_bytes(broken)
            with self.assertRaisesRegex(ValueError, "index checksum"):
                patch.Cpz3(path)

    def test_bmp_orientation_alpha_and_pb3_planes(self):
        # Asymmetric 2x2 BGRA image, including transparent and partial alpha.
        pixels = bytes([1, 2, 3, 0, 4, 5, 6, 64, 7, 8, 9, 128, 10, 11, 12, 255])
        expected_rgba = bytes([3, 2, 1, 0, 6, 5, 4, 64, 9, 8, 7, 128, 12, 11, 10, 255])
        outputs = []
        for height in (-2, 2):
            raw = bytearray(54)
            raw[:2] = b"BM"
            struct.pack_into("<I", raw, 2, 70)
            struct.pack_into("<I", raw, 10, 54)
            struct.pack_into("<IiiHHI", raw, 14, 40, 2, height, 1, 32, 0)
            raw.extend(pixels if height < 0 else pixels[8:] + pixels[:8])
            pb3, detail = patch.bmp_to_pb3(bytes(raw))
            self.assertEqual(detail["rgba_sha256"], hashlib.sha256(expected_rgba).hexdigest())
            self.assertEqual(pb3[:4], b"PB3B")
            self.assertEqual(struct.unpack_from("<4H", pb3, 28), (1, 2, 2, 32))
            at = struct.unpack_from("<I", pb3, 48)[0] + 16
            self.assertEqual(pb3[at:at + 16], b"".join(pixels[n::4] for n in range(4)))
            outputs.append(pb3)
        self.assertEqual(*outputs)
        pb3, detail = patch.bmp_to_pb3(bytes(raw), 24)
        opaque = bytearray(expected_rgba)
        opaque[3::4] = bytes([255]) * 4
        self.assertEqual(detail["rgba_sha256"], hashlib.sha256(opaque).hexdigest())
        self.assertEqual(struct.unpack_from("<H", pb3, 34)[0], 24)
        with self.assertRaises(ValueError):
            patch.bmp_to_pb3(bytes(raw[:-1]))

    def test_unsupported_inputs_do_not_create_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            out = root / "out"
            with self.assertRaises((ValueError, OSError)):
                patch.convert(root / "missing-game", root / "patch.zip", out)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
