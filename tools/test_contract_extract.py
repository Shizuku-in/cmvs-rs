"""Synthetic machine graph tests; python -I tools/test_contract_extract.py."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("contract_extract", Path(__file__).with_name("contract_extract.py"))
ce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ce)


def graph():
    return {"schema": ce.pc.SCHEMA, "policy": ce.pc.POLICY, "profile": "synthetic",
            "binary_sha256": "1" * 64, "arch": "x64", "image_base": "0x140000000",
            "commands": [{"opcode": 0x2000, "kind": "handler", "entry": "0x100"}],
            "functions": {"0x100": {"entry_index": 0, "issues": [], "instructions": [
                {"bytes": "b800400000", "refs": [], "next": 1},
                {"bytes": "c3", "refs": [], "next": None}]}}}


class ExtractTests(unittest.TestCase):
    def test_constant_return_fragment_is_never_command_proof(self):
        data = graph()
        before = copy.deepcopy(data)
        result = ce.extract(data, 0x2000)
        self.assertEqual(result["return_eax"], ["bv", 32, "00004000"])
        self.assertEqual(result["status"], "extracted-fragment")
        self.assertFalse(result["command_contract_complete"])
        self.assertEqual(data, before)

    def test_effectful_or_unknown_instruction_blocks(self):
        for raw in ("c70101000000", "31c0", "ff10", "0f05"):
            data = graph()
            data["functions"]["0x100"]["instructions"][0]["bytes"] = raw
            result = ce.extract(data, 0x2000)
            self.assertEqual(result["status"], "unknown")
            self.assertEqual(result["issues"][0]["reason"], "unsupported-instruction")

    def test_unknown_initial_register_not_zero(self):
        data = graph()
        data["functions"]["0x100"]["instructions"][0]["bytes"] = "90"
        self.assertEqual(ce.extract(data, 0x2000)["issues"][0]["reason"], "unknown-return-value")

    def test_direct_pure_helper_and_tail_call(self):
        for opcode in ("e8", "e9"):
            data = graph()
            data["functions"]["0x200"] = copy.deepcopy(data["functions"]["0x100"])
            data["functions"]["0x100"]["instructions"][0] = {
                "bytes": opcode + "00000000", "refs": [{"kind": "function", "target": "0x200"}],
                "next": 1 if opcode == "e8" else None}
            self.assertEqual(ce.extract(data, 0x2000)["status"], "extracted-fragment")
            data["functions"]["0x200"]["instructions"][0]["bytes"] = "c70101000000"
            self.assertEqual(ce.extract(data, 0x2000)["status"], "unknown")

    def test_cycle_budget_library_and_missing_cfg_fail_closed(self):
        data = graph()
        node = data["functions"]["0x100"]
        node["instructions"][0]["next"] = 0
        self.assertEqual(ce.extract(data, 0x2000)["issues"][0]["reason"], "loop")
        self.assertEqual(ce.extract(graph(), 0x2000, instruction_budget=1)["issues"][0]["reason"], "budget-exceeded")
        data = graph()
        data["functions"]["0x100"]["library"] = "named_runtime"
        self.assertEqual(ce.extract(data, 0x2000)["issues"][0]["reason"], "opaque-library")
        data = graph()
        data["functions"]["0x100"]["instructions"][0]["next"] = None
        self.assertEqual(ce.extract(data, 0x2000)["issues"][0]["reason"], "cfg-gap")

    def test_recursion_and_cross_arch(self):
        data = graph()
        data["functions"]["0x100"]["instructions"][0] = {"bytes": "e800000000",
            "refs": [{"kind": "function", "target": "0x100"}], "next": 1}
        self.assertEqual(ce.extract(data, 0x2000)["issues"][0]["reason"], "recursion")
        data = graph()
        data["arch"] = "x86"
        with self.assertRaises(ValueError):
            ce.extract(data, 0x2000)


if __name__ == "__main__":
    unittest.main()
