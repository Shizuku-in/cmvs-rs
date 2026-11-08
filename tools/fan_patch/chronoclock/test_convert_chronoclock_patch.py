"""Synthetic version selection and input guards; no game data required."""
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
import zipfile

import convert_chronoclock_patch as patch


def fixture(extra=None):
    buffer = BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for version in ("1.0", "1.05"):
            prefix = f"release/柯罗的怀表{version}版补丁/"
            archive.writestr(prefix + "Project/trans.txt", "1|原文|译文")
            for i in range(92):
                archive.writestr(prefix + f"Project/picture{i}.pb3.bmp", version)
            archive.writestr(prefix + "补丁使用说明.txt", "source notice")
            archive.writestr(prefix + "patch.dll", "never install")
            if extra:
                archive.writestr(prefix + extra, "invalid")
    buffer.seek(0)
    return zipfile.ZipFile(buffer)


class ConverterTests(unittest.TestCase):
    def test_selects_each_version_without_installing_executables(self):
        with fixture() as archive, tempfile.TemporaryDirectory() as temp:
            for version in ("1.0", "1.05"):
                stage = Path(temp) / version
                stage.mkdir()
                patch.extract_patch(archive, stage, version)
                self.assertEqual((stage / "Project/picture0.pb3.bmp").read_text(), version)
                self.assertEqual(len(list((stage / "Project").iterdir())), 93)
                self.assertEqual((stage / "汉化补丁说明.txt").read_text(), "source notice")
                self.assertFalse((stage / "patch.dll").exists())

    def test_rejects_traversal_and_unexpected_assets_before_extracting(self):
        for extra in ("Project/../escape.bmp", "Project/wrong.png", "Project/C:bad.pb3.bmp"):
            with self.subTest(extra=extra), fixture(extra) as archive, tempfile.TemporaryDirectory() as temp:
                with self.assertRaises(ValueError):
                    patch.extract_patch(archive, Path(temp), "1.0")
                self.assertEqual(list(Path(temp).iterdir()), [])

    def test_rejects_inputs_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with self.assertRaises(OSError):
                patch.convert(root / "missing", root / "patch.zip", root / "output")
            self.assertFalse((root / "output").exists())
            with self.assertRaisesRegex(ValueError, "outside"):
                patch.convert(root, root / "patch.zip", root / "output")
            with self.assertRaisesRegex(ValueError, "version"):
                patch.convert(root / "missing", root / "patch.zip", root / "output", "2.0")


if __name__ == "__main__":
    unittest.main()
