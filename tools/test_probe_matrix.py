"""Synthetic-only regression tests: python -I tools/test_probe_matrix.py."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("probe_matrix", Path(__file__).with_name("probe_matrix.py"))
pm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pm)


def write(directory, target, checks, options=None):
    directory.mkdir()
    summary = {"target": target, "options": options or {}, "checks": checks}
    (directory / "summary.json").write_text(json.dumps(summary), encoding="utf-8")


class MatrixTests(unittest.TestCase):
    def test_rows_columns_and_cells(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp) / "a", Path(tmp) / "b"
            write(a, {"record": "game-a", "profile": "p1"},
                  [{"id": "S1", "result": "pass", "level": "L0"},
                   {"id": "R1", "result": "blocked", "blocked": {"command": "0x2092"}}])
            write(b, {"root": "D:\\games\\2009_TITLE"},
                  [{"id": "S1", "result": "fail"}, {"id": "D1", "result": "skipped"}],
                  {"allow_presumed": True})
            columns, rows = pm.matrix([a, b])
        self.assertEqual(columns, ["S1", "R1", "D1"])
        self.assertEqual(rows[0], ("game-a", "default", {"S1": "pass L0", "R1": "BLOCKED 0x2092"}))
        self.assertEqual(rows[1][:2], ("2009_TITLE", "allow_presumed"))
        text = pm.render(columns, rows, markdown=True).splitlines()
        self.assertEqual(text[0], "| game | policy | S1 | R1 | D1 |")
        self.assertEqual(text[3], "| 2009_TITLE | allow_presumed | FAIL |  | skip |")


if __name__ == "__main__":
    unittest.main()
