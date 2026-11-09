"""Synthetic filename precedence, character allocation and input guards."""
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
import warnings
import zipfile

import convert_realive_patch as patch


class ConverterTests(unittest.TestCase):
    def test_explicit_picture_wins_without_losing_extensionless_resources(self):
        names = ["lt101.mai", "lt101.pb3.mai", "fontcolor.mai", "config_bg.pb3.mai"]
        expected = ({"lt101.pb3": "lt101.pb3.mai", "fontcolor.pb3": "fontcolor.mai",
                     "config_bg.pb3": "config_bg.pb3.mai"}, ["lt101.mai"])
        self.assertEqual(patch.picture_names(names), expected)
        self.assertEqual(patch.picture_names(list(reversed(names))), expected)
        with self.assertRaises(ValueError):
            patch.picture_names(["fontcolor.mai", "fontcolor.mai"])
        with self.assertRaises(ValueError):
            patch.picture_names(["fontcolor.png"])

    def test_character_map_preserves_existing_and_allocates_missing_unicode(self):
        flags = bytearray(65536)
        sjis = [63] * 65536
        sjis[ord("中")] = 0x889F
        for char in "中简体":
            flags[ord(char)] = 1
        mapping = patch.character_map(bytes(flags), tuple(sjis))
        self.assertEqual(mapping[0x889F], ord("中"))
        self.assertEqual(mapping[0x88A0], ord("体"))
        self.assertEqual(mapping[0x88A1], ord("简"))
        self.assertEqual(mapping[ord("A")], ord("A"))
        with self.assertRaises(ValueError):
            patch.character_map(bytes(65535), tuple(sjis))

    def test_unsafe_or_duplicate_zip_members_fail_before_extraction(self):
        for names in (["mai/pic/../bad.mai"], ["mai/sc/evil\\bad.ps3"],
                      ["/mai/sc/bad.ps3"], ["mai/sc/a.ps3"] * 2):
            with self.subTest(names=names), tempfile.TemporaryDirectory() as temp:
                stream = BytesIO()
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    with zipfile.ZipFile(stream, "w") as archive:
                        for name in names:
                            archive.writestr(patch.ROOT + name, b"x")
                with zipfile.ZipFile(stream) as archive, self.assertRaises(ValueError):
                    patch.extract_patch(archive, Path(temp))
                self.assertEqual(list(Path(temp).iterdir()), [])

    def test_reject_inputs_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaisesRegex(ValueError, "encoding"):
                patch.convert(root, root / "patch.zip", root / "output", "utf16")
            with self.assertRaisesRegex(ValueError, "outside"):
                patch.convert(root, root / "patch.zip", root / "output")
            with self.assertRaises(OSError):
                patch.convert(root / "missing", root / "patch.zip", root / "output")
            self.assertFalse((root / "output").exists())


if __name__ == "__main__":
    unittest.main()
