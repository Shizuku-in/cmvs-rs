"""Synthetic patch member validation and unused resource selection."""
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
import warnings
import zipfile

import convert_kunado_patch as patch


class ConverterTests(unittest.TestCase):
    def test_selects_scripts_and_images_without_installing_hook(self):
        stream = BytesIO()
        with zipfile.ZipFile(stream, "w") as archive:
            for i in range(58):
                archive.writestr(patch.ROOT + f"FileHook/sn{i}.ps3", b"script")
            for i in range(48):
                archive.writestr(patch.ROOT + f"FileHook/image{i}.pb3", b"png")
            archive.writestr(patch.ROOT + "FileHook/ed_bg.pb3", b"unused")
            archive.writestr(patch.ROOT + "cmvs32_cn.dll", b"hook")
        with tempfile.TemporaryDirectory() as temp, zipfile.ZipFile(stream) as archive:
            root = Path(temp)
            pictures, ignored = patch.extract_patch(archive, root)
            self.assertEqual(len(list((root / "sc").iterdir())), 58)
            self.assertEqual(len(pictures), 48)
            self.assertEqual(ignored, ["ed_bg.pb3"])
            self.assertFalse((root / "pic/ed_bg.pb3").exists())
            self.assertFalse((root / "cmvs32_cn.dll").exists())

    def test_unsafe_and_duplicate_members_fail_before_extraction(self):
        for names in (["FileHook/../bad.pb3"], ["FileHook/a\\bad.ps3"],
                      ["FileHook/a.ps3"] * 2):
            stream = BytesIO()
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                with zipfile.ZipFile(stream, "w") as archive:
                    for name in names:
                        archive.writestr(patch.ROOT + name, b"x")
            with tempfile.TemporaryDirectory() as temp, zipfile.ZipFile(stream) as archive:
                with self.assertRaises(ValueError):
                    patch.extract_patch(archive, Path(temp))
                self.assertEqual(list(Path(temp).iterdir()), [])


if __name__ == "__main__":
    unittest.main()
