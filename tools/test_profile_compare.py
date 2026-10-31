"""Synthetic-only regression tests: python -I tools/test_profile_compare.py."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("profile_compare", Path(__file__).with_name("profile_compare.py"))
pc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pc)


def graph(identity, entry="0x100", callee="0x200"):
    return {"schema": pc.SCHEMA, "policy": pc.POLICY, "profile": identity, "arch": "x64",
            "binary_sha256": ("1" if identity == "base" else "2")*64,
            "commands": [{"opcode": 0x2000, "kind": "handler", "entry": entry}],
            "functions": {
                entry: {"entry_index": 0, "issues": [], "instructions": [
                    {"bytes": "e800000000", "refs": [{"kind": "function", "target": callee}], "next": 1},
                    {"bytes": "c3", "refs": [], "next": None}]},
                callee: {"entry_index": 0, "issues": [], "instructions": [
                    {"bytes": "b801000000", "refs": [], "next": 1},
                    {"bytes": "c3", "refs": [], "next": None}]}}}


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.base = graph("base")
        self.derived = graph("derived", "0x500", "0x900")

    def status(self):
        return pc.compare(self.base, self.derived)[0]["status"]

    def test_relocated_callees_match_without_using_addresses(self):
        self.assertEqual(self.status(), "inherited")

    def test_callee_immediate_register_and_field_changes_do_not_match(self):
        for raw in ("b802000000", "b901000000", "8b4110", "8b4114"):
            with self.subTest(raw=raw):
                changed = copy.deepcopy(self.derived)
                changed["functions"]["0x900"]["instructions"][0]["bytes"] = raw
                self.assertEqual(pc.compare(self.base, changed)[0]["status"], "unreviewed")

    def test_data_contents_and_import_names_are_semantic_inputs(self):
        for left, right in [
            ({"kind": "data", "bytes": "0000803f", "offset": 0}, {"kind": "data", "bytes": "00000040", "offset": 0}),
            ({"kind": "import", "name": "kernel32!GetTickCount"}, {"kind": "import", "name": "kernel32!ExitProcess"}),
        ]:
            self.base["functions"]["0x200"]["instructions"][0]["refs"] = [left]
            self.derived["functions"]["0x900"]["instructions"][0]["refs"] = [right]
            self.assertEqual(self.status(), "unreviewed")

    def test_equal_but_incomplete_subtrees_never_inherit(self):
        for issue in ("unresolved-or-mutable-data", "indirect-control-flow", "relocated-data", "instruction-limit"):
            self.base["functions"]["0x200"]["issues"] = [issue]
            self.derived["functions"]["0x900"]["issues"] = [issue]
            self.assertEqual(self.status(), "unreviewed")

    def test_depth_limit_and_recursive_cycles_never_inherit(self):
        self.assertEqual(pc.compare(self.base, self.derived, 0)[0]["status"], "unreviewed")
        for g, callee, entry in [(self.base, "0x200", "0x100"), (self.derived, "0x900", "0x500")]:
            g["functions"][callee]["instructions"][0]["refs"] = [{"kind": "function", "target": entry}]
        self.assertEqual(self.status(), "unreviewed")

    def test_entry_and_control_flow_changes_cannot_match(self):
        self.derived["functions"]["0x900"]["entry_index"] = 1
        self.assertEqual(self.status(), "unreviewed")
        self.derived["functions"]["0x900"]["entry_index"] = 0
        self.derived["functions"]["0x900"]["instructions"][0]["next"] = 0
        self.assertEqual(self.status(), "unreviewed")

    def make_library(self, name="_malloc", raw="b801000000"):
        for g, callee in [(self.base, "0x200"), (self.derived, "0x900")]:
            g["functions"][callee]["library"] = name
            g["functions"][callee]["instructions"][0]["bytes"] = raw
            # Library internals stay opaque: no walk, no data or indirect-flow issues.
            g["functions"][callee]["instructions"][0]["refs"] = [{"kind": "opaque"}]

    def test_library_leaves_match_by_name_and_own_body(self):
        self.make_library()
        self.assertEqual(self.status(), "inherited")
        self.derived["functions"]["0x900"]["library"] = "_free"
        self.assertEqual(self.status(), "unreviewed")
        self.make_library()
        self.derived["functions"]["0x900"]["instructions"][0]["bytes"] = "b802000000"
        self.assertEqual(self.status(), "unreviewed")
        # The same body without the library mark is not a library match.
        self.make_library()
        del self.derived["functions"]["0x900"]["library"]
        with self.assertRaises(ValueError):
            self.status()

    def test_placeholder_names_cannot_identify_library_leaves(self):
        for name in ("sub_140038BA0", "unknown_libname_2", "nullsub_1", "j_free"):
            with self.subTest(name=name):
                self.make_library(name=name)
                if name == "j_free":
                    self.assertEqual(self.status(), "inherited")  # a real name after `j_`
                    continue
                with self.assertRaises(ValueError):
                    self.status()

    def test_library_leaves_still_report_their_own_issues(self):
        self.make_library()
        for g, callee in [(self.base, "0x200"), (self.derived, "0x900")]:
            g["functions"][callee]["issues"] = ["undecoded-instruction"]
        self.assertEqual(self.status(), "unreviewed")

    def test_opaque_and_foreign_references_are_confined(self):
        self.derived["functions"]["0x900"]["instructions"][0]["refs"] = [{"kind": "opaque"}]
        with self.assertRaises(ValueError):
            self.status()
        self.make_library()
        self.derived["functions"]["0x900"]["instructions"][0]["refs"] = [{"kind": "function", "target": "0x500"}]
        with self.assertRaises(ValueError):
            self.status()

    def test_diagnostic_fields_never_change_fingerprints(self):
        plain = pc.compare(self.base, self.derived)[0]
        self.make_library()
        library = pc.compare(self.base, self.derived)[0]
        for g in (self.base, self.derived):
            for node in g["functions"].values():
                node["undecoded"] = [7]
                for k, ins in enumerate(node["instructions"]):
                    ins["offset"], ins["issues"] = 3*k, []
        self.assertEqual(pc.compare(self.base, self.derived)[0], library)
        self.setUp()
        for g in (self.base, self.derived):
            for k, ins in enumerate(g["functions"]["0x100" if g is self.base else "0x500"]["instructions"]):
                ins["offset"] = 5*k
        self.assertEqual(pc.compare(self.base, self.derived)[0], plain)

    def test_explain_names_the_blocking_instruction_and_data_pair(self):
        for g, callee, target in [(self.base, "0x200", "0x1010"), (self.derived, "0x900", "0x2020")]:
            ins = g["functions"][callee]["instructions"][0]
            ins.update(offset=0, issues=["unresolved-or-mutable-data"])
            ins["refs"] = [{"kind": "unknown-data", "target": target, "why": "writable"}]
            g["functions"][callee]["issues"] = ["unresolved-or-mutable-data"]
        text = "\n".join(pc.explain(self.base, self.derived, 0x2000))
        self.assertIn("0x2000 unreviewed", text)
        self.assertIn("0x200 / 0x900: incomplete", text)
        self.assertIn("base +0x0 b801000000: unresolved-or-mutable-data", text)
        self.assertIn("data 0x1010 / 0x2020: writable", text)

    def test_explain_marks_same_subtrees_and_the_first_difference(self):
        self.derived["functions"]["0x900"]["instructions"][0]["bytes"] = "b802000000"
        text = pc.explain(self.base, self.derived, 0x2000)
        self.assertEqual(text[0], "0x2000 unreviewed: different")
        self.assertIn("  0x200 / 0x900: different", text)
        self.assertIn("    first difference: base #0 b801000000 / derived #0 b802000000", text)
        self.setUp()
        self.assertIn("0x100 / 0x500: same", pc.explain(self.base, self.derived, 0x2000))

    def test_architecture_schema_and_missing_dependencies_rejected(self):
        for key, value in [("arch", "x86"), ("policy", "old"), ("schema", "unknown")]:
            changed = copy.deepcopy(self.derived)
            changed[key] = value
            with self.assertRaises(ValueError):
                pc.compare(self.base, changed)
        del self.derived["functions"]["0x900"]
        with self.assertRaises(ValueError):
            self.status()

    def test_noop_absent_and_new_handlers_remain_separate(self):
        self.derived["commands"] += [
            {"opcode": 0x2001, "kind": "noop", "entry": "0xa00"},
            {"opcode": 0x2002, "kind": "absent", "entry": None},
            {"opcode": 0x2003, "kind": "handler", "entry": "0x500"}]
        self.assertEqual([r["status"] for r in pc.compare(self.base, self.derived)],
                         ["inherited", "noop", "absent", "unreviewed"])

    def test_profile_binding_rejects_hash_table_and_case_mismatches(self):
        self.base.update(image_base="0x140000000", table_rva="0x300")
        good = [["profile", "base"], ["base", "-"],
                ["binary", "x64", "sha256="+"1"*64, "image-base=0x140000000", "table=0x300"],
                ["0x2000", "base", "0x100"]]
        self.assertEqual(len(pc.bind_profile(good, self.base)), 1)
        for old, new in [("1"*64, "3"*64), ("table=0x300", "table=0x301"), ("0x100", "0x101")]:
            changed = [[word.replace(old, new) for word in row] for row in good]
            with self.assertRaises(ValueError):
                pc.bind_profile(changed, self.base)

    def test_candidate_profile_names_the_root_exe_once_per_architecture(self):
        for g in (self.base, self.derived):
            g.update(image_base="0x140000000", table_rva="0x300")
        binary = "binary x64 sha256={} image-base=0x140000000 table=0x300"
        with tempfile.TemporaryDirectory() as root:
            base_path, derived_path = Path(root, "base.profile"), Path(root, "derived.profile")
            base_path.write_text(f"profile base\nbase -\n{binary.format('1'*64)}\n0x2000 base 0x100\n")
            derived_path.write_text(f"profile derived\nbase base\n{binary.format('2'*64)}\n"
                                    f"inherit x64 base-sha256={'4'*64}\n0x2000 unreviewed 0x500\n")
            rows = pc.compare(self.base, self.derived)
            lines = pc.apply_profile(self.base, self.derived, rows, base_path, derived_path).splitlines()
        self.assertEqual(lines[3], "inherit x64 base-sha256=" + "1"*64)
        self.assertEqual(lines[4], f"0x2000 inherited 0x500 inherit=x64:{rows[0]['fingerprint']}")
        self.assertEqual(len(lines), 5)

    def test_candidate_profile_keeps_review_records_verbatim(self):
        for g in (self.base, self.derived):
            g.update(image_base="0x140000000", table_rva="0x300")
        binary = "binary x64 sha256={} image-base=0x140000000 table=0x300"
        reviewed = "0x2000 reviewed 0x500 review=games/x/vm-semantics.md"
        with tempfile.TemporaryDirectory() as root:
            base_path, derived_path = Path(root, "base.profile"), Path(root, "derived.profile")
            base_path.write_text(f"profile base\nbase -\n{binary.format('1'*64)}\n0x2000 base 0x100\n")
            derived_path.write_text(f"profile derived\nbase base\n{binary.format('2'*64)}\n{reviewed}\n")
            rows = pc.compare(self.base, self.derived)
            self.assertEqual(rows[0]["status"], "inherited")
            lines = pc.apply_profile(self.base, self.derived, rows, base_path, derived_path).splitlines()
        # No inherited row is written, so no stale `inherit` header either.
        self.assertEqual(lines[3:], [reviewed])


if __name__ == "__main__":
    unittest.main()
