"""Synthetic IR tests: python -I tools/test_command_contract.py."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("command_contract", Path(__file__).with_name("command_contract.py"))
cc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cc)
VECTORS = json.loads((Path(__file__).resolve().parents[1] / "docs/design/command-contracts-v1-cases.json").read_text(encoding="utf-8"))


def change(value, changes):
    for patch in changes:
        assert patch["op"] == "replace"
        parts = patch["path"].strip("/").split("/")
        node = value
        for key in parts[:-1]:
            node = node[int(key)] if isinstance(node, list) else node[key]
        key = int(parts[-1]) if isinstance(node, list) else parts[-1]
        node[key] = copy.deepcopy(patch["value"])


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.base = copy.deepcopy(VECTORS["contract"])

    def test_m0_semantic_vectors(self):
        for vector in VECTORS["cases"]:
            # M1a has no evidence-envelope verifier. Do not pretend to execute
            # missing/stale proof or cross-architecture scenarios at IR level.
            if vector["expected"] == "invalid-evidence":
                continue
            with self.subTest(vector=vector["id"]):
                base = copy.deepcopy(self.base)
                change(base, vector.get("base_changes", []))
                derived = copy.deepcopy(base)
                change(derived, vector["derived_changes"])
                if vector["expected"] == "invalid-input":
                    with self.assertRaises(cc.Invalid):
                        cc.compare(base, derived)
                else:
                    self.assertEqual(cc.compare(base, derived)["status"], vector["expected"])

    @unittest.skip("M1a compares supplied IR only; binary evidence verifier is not implemented")
    def test_m0_evidence_vectors(self):
        pass

    def test_alpha_renaming(self):
        other = copy.deepcopy(self.base)
        other["parameters"][0]["id"] = "new_param"
        other["tree"]["events"][0]["parameter"] = "new_param"
        other["tree"]["events"][1]["value"] = ["arg", "new_param"]
        for i, event in enumerate(other["tree"]["events"]):
            event["id"] = "renamed%d" % i
        self.assertEqual(cc.compare(self.base, other)["status"], "equivalent")

    def test_commutative_only(self):
        a = ["arg", "flag"]
        b = ["bv", 32, "00000001"]
        for op, expected in (("add", "equivalent"), ("sub", "different")):
            self.base["tree"]["events"][1]["value"] = [op, a, b]
            other = copy.deepcopy(self.base)
            other["tree"]["events"][1]["value"] = [op, b, a]
            self.assertEqual(cc.compare(self.base, other)["status"], expected)

    def test_no_witness_does_not_prove_equality(self):
        other = copy.deepcopy(self.base)
        other["tree"]["events"][1]["value"] = ["add", ["arg", "flag"], ["bv", 32, "00000000"]]
        self.assertEqual(cc.compare(self.base, other)["status"], "unknown")
        other["tree"]["exit"]["code"] = ["bv", 32, "00000000"]
        self.assertEqual(cc.compare(self.base, other, witness_budget=0)["status"], "unknown")

    def test_shared_byte_stack_for_overlapping_parameters(self):
        other = copy.deepcopy(self.base)
        other["parameters"][0]["stack_offset"] = -8
        result = cc.compare(self.base, other)
        self.assertEqual(result["status"], "different")
        stack = {int(k): v for k, v in result["witness"]["stack_bytes"].items()}
        initial = result["witness"]["initial_state"]
        self.assertNotEqual(cc.execute(self.base, stack, initial), cc.execute(other, stack, initial))

    def test_reads_after_writes_observe_updated_state(self):
        events = self.base["tree"]["events"]
        target = copy.deepcopy(events[1]["target"])
        events.insert(2, {"id": "r", "op": "read_state", "target": target, "width": 32})
        self.base["tree"]["exit"]["code"] = ["ref", "r"]
        cc.validate(self.base)
        result = cc.execute(self.base, {-4: 5}, {cc.encode(target): 99})
        self.assertEqual(result["exit"], 5)

    def test_ordered_effects_are_not_sorted_away(self):
        self.base["tree"]["events"].insert(2, {"id": "write2", "op": "write_state",
            "target": copy.deepcopy(self.base["tree"]["events"][1]["target"]), "width": 32,
            "value": ["bv", 32, "00000007"]})
        other = copy.deepcopy(self.base)
        other["tree"]["events"][1:3] = reversed(other["tree"]["events"][1:3])
        self.assertEqual(cc.compare(self.base, other)["status"], "different")

    def test_same_unknown_stays_unknown(self):
        self.base["coverage"]["issues"] = ["unknown-memory"]
        self.assertEqual(cc.compare(self.base, self.base)["status"], "unknown")

    def test_invalid_fields_types_and_use_before_definition(self):
        for patches in ([{"op": "replace", "path": "/precondition", "value": ["bool", 1]}],
                        [{"op": "replace", "path": "/coverage/cfg_closed", "value": 1}],
                        [{"op": "replace", "path": "/tree/events/1/value", "value": ["ref", "future"]}],
                        [{"op": "replace", "path": "/tree/events/1/id", "value": "p0"}],
                        [{"op": "replace", "path": "/tree/events/2/bytes", "value": True}]):
            other = copy.deepcopy(self.base)
            change(other, patches)
            with self.assertRaises(cc.Invalid):
                cc.compare(self.base, other)
        other = copy.deepcopy(self.base)
        other["coverage"]["trusted"] = True
        with self.assertRaises(cc.Invalid):
            cc.compare(self.base, other)

    def test_unsupported_region_and_second_input_validation(self):
        self.base["tree"]["events"][1]["target"]["region"] = "save"
        self.assertEqual(cc.compare(self.base, self.base)["status"], "unknown")
        bad = copy.deepcopy(self.base)
        bad["tree"]["events"][1]["value"] = ["bv", 8, "01"]
        with self.assertRaises(cc.Invalid):
            cc.compare(self.base, bad)

    def test_signed_and_wrapping_expressions(self):
        def ev(expr):
            return cc.evaluate(expr, {}, {})
        self.assertEqual(ev(["add", ["bv", 8, "ff"], ["bv", 8, "01"]]), (0, 8))
        self.assertEqual(ev(["slt", ["bv", 8, "ff"], ["bv", 8, "00"]]), (True, "bool"))
        self.assertEqual(ev(["ult", ["bv", 8, "ff"], ["bv", 8, "00"]]), (False, "bool"))
        self.assertEqual(ev(["sext", 32, ["bv", 8, "ff"]]), (0xffffffff, 32))
        self.assertEqual(ev(["trunc", 8, ["bv", 32, "00000100"]]), (0, 8))

    def test_cli_is_read_only_and_rejects_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "contract.json"
            path.write_text(json.dumps(self.base), encoding="utf-8")
            before = path.read_bytes()
            command = [sys.executable, "-I", str(Path(cc.__file__)), str(path), str(path)]
            run = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertEqual(json.loads(run.stdout)["status"], "equivalent")
            self.assertEqual(path.read_bytes(), before)
            path.write_text('{"contract":1,"contract":2}', encoding="utf-8")
            run = subprocess.run(command, capture_output=True, text=True, check=False)
            self.assertEqual(run.returncode, 3)
            self.assertEqual(json.loads(run.stdout)["status"], "invalid-input")


if __name__ == "__main__":
    unittest.main()
