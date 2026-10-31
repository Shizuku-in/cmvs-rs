"""Synthetic machine-code tests for the M1 lifter; python -I tools/test_contract_extract.py."""
import copy
import importlib.util
import struct
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("contract_extract", Path(__file__).with_name("contract_extract.py"))
ce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ce)
cc = ce.cc

BASE, DERIVED = "1" * 64, "2" * 64
EPILOGUE = ["4883c420", "5b", "c3"]                 # add rsp,0x20; pop rbx; ret
READ_SP4 = ["48638b70420100", "488b8368420100", "8b4c01fc"]   # ecx = SP-4
READ_SP8 = ["48638b70420100", "488b8368420100", "8b4c01f8"]   # ecx = SP-8


def ret(code):
    return ["b8" + struct.pack("<I", code).hex()]


def function(code, refs=None):
    """code: list of hex strings; refs: {index: [ref, ...]}. Fallthrough is sequential."""
    refs = refs or {}
    out, offset = [], 0
    for i, raw in enumerate(code):
        last = raw in ("c3",) or raw.startswith("e9")
        out.append({"bytes": raw, "refs": refs.get(i, []), "next": None if last else i + 1,
                    "offset": offset})
        offset += len(raw) // 2
    return {"entry_index": 0, "issues": [], "instructions": out}


def graph(sha, case, extra=None):
    functions = {"0x100": case}
    functions.update(extra or {})
    return {"schema": ce.pc.SCHEMA, "policy": ce.pc.POLICY, "profile": "synthetic-" + sha[0],
            "binary_sha256": sha, "arch": "x64", "image_base": "0x140000000", "table_rva": "0x2000",
            "commands": [{"opcode": 0x2000, "kind": "handler", "entry": "0x100"}], "functions": functions}


def context():
    side = {"arch": "x64", "image_base": "0x140000000", "table_rva": "0x2000",
            "vm_stack": {"sp_offset": "0x14270", "base_offset": "0x14268"}, "evidence": ["synthetic"]}
    return ce.check_context({"schema": ce.CONTEXT, "revision": "t", "sides": [
        dict(side, binary_sha256=BASE), dict(side, binary_sha256=DERIVED)]})


def mapping(entries):
    """entries: (id, object, field, width, {sha: offset}, points_to[, array count])."""
    doc = {"schema": ce.MAPPINGS, "revision": "t", "entries": []}
    for ident, obj, field, width, offsets, points, *count in entries:
        base_object = "image" if obj.startswith("image:") else obj     # globals: offset is an RVA
        obj = obj.split(":")[-1]
        target = {"region": "vm" if obj == "vm" else "object", "object": obj, "field": field, "width": width}
        if points:
            target["points_to"] = points
        if count:
            target["count"] = count[0]
        doc["entries"].append({"id": ident, "logical_target": target, "evidence": ["synthetic"], "scope": "t",
                               "review": {}, "sides": [dict({"binary_sha256": sha, "arch": "x64", "base_object": base_object,
                                                              "offset": off, "width": width},
                                                             **({"stride": width // 8} if count else {}))
                                                        for sha, off in offsets.items()]})
    return ce.check_mappings(doc)


FLAGS = ("flags", "vm", "flags", 32, {BASE: "0x180", DERIVED: "0x250"}, None)
ERRORS = ("errors", "vm", "errors", 32, {BASE: "0x2f18", DERIVED: "0x2f18"}, None)
RULES = {"extractor_revision": "a" * 64, "rules_sha256": "b" * 64, "mapping_sha256": "c" * 64,
         "summaries_sha256": "d" * 64}


def lift(sha, case, maps=(FLAGS, ERRORS), extra=None, summaries=None):
    return ce.Lifter(graph(sha, case, extra), context(), mapping(maps), summaries or {}).lift(0x2000)


def compare(base_case, derived_case, maps=(FLAGS, ERRORS), checked=True):
    sides = []
    for sha, case in ((BASE, base_case), (DERIVED, derived_case)):
        side = lift(sha, case, maps)
        if side["status"] != "extracted":
            return {"status": "unknown", "issues": side["issues"]}
        if checked:
            side["binary"] = {}
        sides.append(ce.envelope(graph(sha, case), "e" * 64, RULES, side))
    return cc.compare_envelopes(sides[0], sides[1], RULES)


def store(offset):
    return "898b" + struct.pack("<I", offset).hex()       # mov [rbx+offset], ecx


def guarded(offset, swap=False):
    """cmp ecx,0xc; jge error; mov [rbx+offset],ecx; ret 0x4004 / error: or [rbx+0x2f18],0x1000."""
    code = READ_SP4 + ["83f90c", "7d00", store(offset)] + ret(0x4004) + EPILOGUE
    error = len(code)
    code += ["818b182f000000100000"] + ret(0x4004) + EPILOGUE
    if swap:
        code[4] = "7c00"
    return function(code, {4: [{"kind": "local", "target": error}]})


class LiftTests(unittest.TestCase):
    def test_constant_return_and_release(self):
        side = lift(BASE, function(ret(0x4000) + EPILOGUE))
        self.assertEqual(side["status"], "extracted")
        leaf = side["contract"]["tree"]
        self.assertEqual(leaf["exit"]["code"], ["bv", 32, "00004000"])
        self.assertEqual(leaf["events"], [{"op": "release_stack", "bytes": 0, "id": "e0"}])

    def test_mapped_layout_move_is_equivalent(self):
        case = lambda off: function(READ_SP4 + [store(off)] + ret(0x4004) + EPILOGUE)
        result = compare(case(0x180), case(0x250))
        self.assertEqual(result["status"], "equivalent")
        self.assertEqual(compare(case(0x180), case(0x250), checked=False)["status"], "unknown")

    def test_unmapped_offset_is_unknown(self):
        case = lambda off: function(READ_SP4 + [store(off)] + ret(0x4004) + EPILOGUE)
        result = compare(case(0x180), case(0x254))
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(result["issues"][0]["issue"], "unproven-mapping")

    def test_machine_code_counterexamples(self):
        base = function(READ_SP4 + [store(0x180)] + ret(0x4004) + EPILOGUE)
        other_field = (("other", "vm", "other", 32, {BASE: "0x184", DERIVED: "0x254"}, None),)
        for derived, maps in (
                (function(READ_SP8 + [store(0x250)] + ret(0x4004) + EPILOGUE), ()),        # parameter order
                (function(READ_SP4 + [store(0x250)] + ret(0x4008) + EPILOGUE), ()),        # release bytes
                (function(READ_SP4 + [store(0x254)] + ret(0x4004) + EPILOGUE), other_field)):  # other field
            with self.subTest(derived=derived["instructions"][2]["bytes"]):
                result = compare(base, derived, (FLAGS, ERRORS) + maps)
                self.assertEqual(result["status"], "different", result)

    def test_conditional_error_branch(self):
        side = lift(BASE, guarded(0x180))
        self.assertEqual(side["contract"]["tree"]["kind"], "branch")
        self.assertEqual(compare(guarded(0x180), guarded(0x250))["status"], "equivalent")
        self.assertEqual(compare(guarded(0x180), guarded(0x250, swap=True))["status"], "different")

    def test_helper_tail_call_and_inline_call(self):
        helper = function(["c781d860000001000000"] + ret(0x4000) + ["c3"])
        maps = (("mgr", "vm", "mgr", 64, {BASE: "0x1298"}, "manager"),
                ("hint", "manager", "hint", 32, {BASE: "0x60d8"}, None))
        tail = function(["488b8b98120000", "4883c420", "5b", "e900000000"],
                        {3: [{"kind": "function", "target": "0x200"}]})
        side = lift(BASE, tail, maps, {"0x200": helper})
        self.assertEqual(side["status"], "extracted", side)
        call = function(["488b8b98120000", "e800000000"] + ret(0x4000) + EPILOGUE,
                        {1: [{"kind": "function", "target": "0x200"}]})
        inline = lift(BASE, call, maps, {"0x200": helper})
        tree = inline["contract"]["tree"]
        # The loaded manager pointer is unchecked: null is an explicit undefined path.
        self.assertEqual(tree["condition"], ["eq", ["ref", "e0"], ["bv", 64, "0" * 16]])
        self.assertEqual(tree["then"]["exit"]["kind"], "undefined")
        self.assertEqual(tree["else"]["events"][0]["target"]["field"], "hint")

    def test_checked_table_element(self):
        """cmp eax,12; jge err; mov r9,[rbx+rax*8+0x1020]; test r9,r9; je err; mov [r9+8],1."""
        def case(table, field):
            code = ["48638b70420100", "488b8368420100", "48634401fc", "83f80c", "7d00",
                    "4c8b8cc3" + struct.pack("<I", table).hex(), "4d85c9", "7400",
                    "41c741" + "%02x" % field + "01000000"] + ret(0x4004) + EPILOGUE
            err = len(code)
            code += ["818b182f000000100000"] + ret(0x4004) + EPILOGUE
            return function(code, {4: [{"kind": "local", "target": err}], 7: [{"kind": "local", "target": err}]})
        def maps(derived_count=12):
            return (ERRORS, ("slots", "vm", "slots", 64, {BASE: "0x1020"}, "text", 12),
                    ("slots-derived", "vm", "slots", 64, {DERIVED: "0x1040"}, "text", derived_count),
                    ("x", "text", "x", 32, {BASE: "0x8", DERIVED: "0xc"}, None))
        side = lift(BASE, case(0x1020, 8), maps())
        self.assertEqual(side["status"], "extracted", side)
        result = compare(case(0x1020, 8), case(0x1040, 0xc), maps())
        self.assertEqual(result["status"], "equivalent", result)
        # A negative slot passes the signed check and is out of bounds: undefined, not dropped.
        leaves = []
        def walk(n):
            if n["kind"] == "leaf":
                leaves.append(n["exit"]["kind"])
            else:
                walk(n["then"]), walk(n["else"])
        walk(side["contract"]["tree"])
        # One undefined leaf (negative slot); the code's own null check is a path fact.
        self.assertEqual(leaves.count("undefined"), 1)
        # Larger derived capacity: the signed slot check hides it, so no witness exists, but
        # the bounds differ in normal form and must not be accepted.
        self.assertEqual(compare(case(0x1020, 8), case(0x1040, 0xc), maps(16))["status"], "unknown")
        # A capacity below the checked range is observable.
        self.assertEqual(compare(case(0x1020, 8), case(0x1040, 0xc), maps(4))["status"], "different")

    def test_blockers(self):
        cases = {
            "unsupported-instruction": function(["0fafc1"] + ret(0x4000) + EPILOGUE),
            "cfg-gap": function(ret(0x4000) + ["4883c420", "c3"]),                       # rbx not restored
            "unsupported-effect": function(ret(0x8000) + EPILOGUE),                        # yield
            "loop": function(["eb00"] + ret(0x4000) + EPILOGUE, {0: [{"kind": "local", "target": 0}]}),
            "unknown-memory": function(["8b0500000000"] + ret(0x4000) + EPILOGUE),         # rip global
            "unsupported-expression": function(["0fb6c1"] + ret(0x4000) + EPILOGUE),       # scratch rcx
        }
        cases["loop"]["instructions"][0]["next"] = None
        for issue, case in cases.items():
            with self.subTest(issue=issue):
                side = lift(BASE, case)
                self.assertEqual(side["status"], "unknown")
                self.assertEqual(side["issues"][0]["issue"], issue, side)

    def test_summary_binding(self):
        helper = function(["0f0b", "c3"])                     # never decoded: summary replaces it
        call = function(["488b8b98120000", "e800000000"] + ret(0x4000) + EPILOGUE,
                        {1: [{"kind": "function", "target": "0x300"}]})
        g = graph(BASE, call, {"0x300": helper})
        entry = {"id": "set", "binary_sha256": BASE, "arch": "x64", "function_rva": "0x300",
                 "body_sha256": ce.body_digest(g, "0x300"), "abi": "win64",
                 "arguments": [{"id": "x", "register": "rcx", "width": 32}], "precondition": ["bool", True],
                 "tree": {"kind": "leaf", "events": [{"id": "w", "op": "write_state", "width": 32,
                                                      "target": {"region": "vm", "object": "vm", "field": "errors"},
                                                      "value": ["arg", "x"]}],
                          "exit": {"kind": "return", "code": ["bv", 32, "00000000"]}},
                 "coverage": {**{k: True for k in cc.CLOSED}, "issues": []}, "dependencies": [],
                 "evidence": ["synthetic"], "review": {}}
        doc = {"schema": ce.SUMMARIES, "revision": "t", "entries": [entry]}
        maps = mapping([FLAGS, ("mgr", "vm", "mgr", 64, {BASE: "0x1298"}, "manager")])
        lifter = ce.Lifter(g, context(), maps, ce.check_summaries(doc))
        self.assertEqual(lifter.lift(0x2000)["status"], "unknown")      # rcx is a pointer here
        call["instructions"][0]["bytes"] = "8b8b80010000"                 # ecx = flags instead
        side = ce.Lifter(g, context(), maps, ce.check_summaries(doc)).lift(0x2000)
        self.assertEqual(side["status"], "extracted", side)
        self.assertEqual(side["summaries"], ["set"])
        stale = copy.deepcopy(doc)
        stale["entries"][0]["body_sha256"] = "0" * 64
        with self.assertRaises(cc.InvalidEvidence):
            ce.Lifter(g, context(), maps, ce.check_summaries(stale)).lift(0x2000)

    def test_inputs_not_modified(self):
        case = guarded(0x180)
        before = copy.deepcopy(case)
        lift(BASE, case)
        self.assertEqual(case, before)


def pe(code_at, code):
    """Tiny PE32+ image: one RX section at RVA 0x1000 holding `code` at code_at."""
    section = bytearray(0x3000)
    section[code_at - 0x1000:code_at - 0x1000 + len(code)] = code
    header = bytearray(0x400)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 0x3c, 0x80)
    header[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", header, 0x84, 0x8664, 1)
    struct.pack_into("<H", header, 0x94, 0xf0)
    struct.pack_into("<H", header, 0x98, 0x20b)
    struct.pack_into("<Q", header, 0x98 + 24, 0x140000000)
    s = 0x98 + 0xf0
    header[s:s + 5] = b".text"
    struct.pack_into("<IIII", header, s + 8, len(section), 0x1000, len(section), 0x400)
    struct.pack_into("<I", header, s + 36, 0x60000020)
    return bytes(header + section)


class BinaryTests(unittest.TestCase):
    def build(self, case_bytes, table_entry=0x1800):
        dispatch = ce.DISPATCH_PROLOGUE + b"\x3d" + struct.pack("<I", 0) + b"\x77\x00"
        at = 0x1100 + len(dispatch)
        dispatch += b"\x48\x8d\x0d" + struct.pack("<i", -(at + 7)) + b"\x8b\x94\x81" + struct.pack("<I", 0x2000)
        dispatch += bytes.fromhex("4803d1ffe2")
        blob = bytearray(0x1000)
        blob[0x100:0x100 + len(dispatch)] = dispatch
        blob[0x800:0x800 + len(case_bytes)] = case_bytes
        image = bytearray(pe(0x1000, bytes(blob)))
        struct.pack_into("<I", image, 0x400 + 0x1000, table_entry)    # table at RVA 0x2000
        return bytes(image)

    def test_instruction_and_branch_binding(self):
        helper_rva = 0x1900
        case = bytes.fromhex("488b8b98120000e8") + struct.pack("<i", helper_rva - (0x1800 + 12))
        case += bytes.fromhex("b8004000004883c4205bc3")
        helper = bytes.fromhex("c781d860000001000000c3")
        raw = bytearray(self.build(case))
        raw[0x400 + 0x900:0x400 + 0x900 + len(helper)] = helper
        image = ce.Image(bytes(raw))
        g = graph(image.sha256, function(["488b8b98120000", "e800000000", "b800400000"] + EPILOGUE,
                                         {1: [{"kind": "function", "target": "0x1900"}]}),
                  {"0x1900": function(["c781d860000001000000", "c3"])})
        g["commands"][0]["entry"] = "0x1800"
        g["functions"]["0x1800"] = g["functions"].pop("0x100")
        ctx = ce.check_context({"schema": ce.CONTEXT, "revision": "t", "sides": [{
            "binary_sha256": image.sha256, "arch": "x64", "image_base": "0x140000000", "table_rva": "0x2000",
            "vm_stack": {"sp_offset": "0x14270", "base_offset": "0x14268"}, "evidence": []}]})
        maps = {(image.sha256, "vm", 0x1298, 64): {"id": "m", "logical_target": {
                    "region": "vm", "object": "vm", "field": "m", "width": 64, "points_to": "o"}},
                (image.sha256, "o", 0x60d8, 32): {"id": "h", "logical_target": {
                    "region": "object", "object": "o", "field": "h", "width": 32}}}
        side = ce.Lifter(g, ctx, maps, {}).lift(0x2000)
        self.assertEqual(side["status"], "extracted", side)
        ce.verify_binary(image, g, 0x2000, side["executed"])
        bad = [(fn, off, raw_hex, (target + 1 if target else target), data)
               for fn, off, raw_hex, target, data in side["executed"]]
        with self.assertRaises(cc.InvalidEvidence):
            ce.verify_binary(image, g, 0x2000, bad)
        g2 = copy.deepcopy(g)
        g2["binary_sha256"] = "f" * 64
        with self.assertRaises(cc.InvalidEvidence):
            ce.verify_binary(image, g2, 0x2000, side["executed"])

    def test_rip_relative_global_binding(self):
        case = bytes.fromhex("8b05") + struct.pack("<i", 0x2800 - (0x1800 + 6))
        case += bytes.fromhex("b8004000004883c4205bc3")
        image = ce.Image(self.build(case))
        g = graph(image.sha256, function(["8b0500000000", "b800400000"] + EPILOGUE, {0: [
            {"kind": "unknown-data", "target": "0x2800", "why": "writable"}]}))
        g["functions"]["0x100"]["instructions"][0]["issues"] = ["unresolved-or-mutable-data"]
        g["commands"][0]["entry"] = "0x1800"
        g["functions"]["0x1800"] = g["functions"].pop("0x100")
        ctx = ce.check_context({"schema": ce.CONTEXT, "revision": "t", "sides": [{
            "binary_sha256": image.sha256, "arch": "x64", "image_base": "0x140000000", "table_rva": "0x2000",
            "vm_stack": {"sp_offset": "0x14270", "base_offset": "0x14268"}, "evidence": []}]})
        maps = {(image.sha256, "image", 0x2800, 32): {"id": "g", "logical_target": {
            "region": "object", "object": "globals", "field": "g", "width": 32}}}
        side = ce.Lifter(g, ctx, maps, {}).lift(0x2000)
        self.assertEqual(side["status"], "extracted", side)
        ce.verify_binary(image, g, 0x2000, side["executed"])
        bad = [(fn, off, raw_hex, target, ("rva", 0x2804) if data else data)
               for fn, off, raw_hex, target, data in side["executed"]]
        with self.assertRaises(cc.InvalidEvidence):
            ce.verify_binary(image, g, 0x2000, bad)


GLOBAL = ("g", "image:globals", "g", 32, {BASE: "0x5000", DERIVED: "0x6000"}, None)


def global_ref(target):
    return {"kind": "unknown-data", "target": target, "why": "writable"}


def global_store(target):
    """mov [rip+X], ecx after reading SP-4; returns 0x4004."""
    case = function(READ_SP4 + ["890d00000000"] + ret(0x4004) + EPILOGUE, {3: [global_ref(target)]})
    case["instructions"][3]["issues"] = ["unresolved-or-mutable-data"]
    return case


class RegionTests(unittest.TestCase):
    def test_global_mapping_and_readonly_constant(self):
        maps = (FLAGS, ERRORS, GLOBAL)
        self.assertEqual(compare(global_store("0x5000"), global_store("0x6000"), maps)["status"], "equivalent")
        self.assertEqual(compare(global_store("0x5000"), global_store("0x6004"), maps)["issues"][0]["issue"],
                         "unproven-mapping")
        # Return code loaded from read-only data: a constant, not state.
        rodata = function(["8b0500000000"] + EPILOGUE, {0: [{"kind": "data", "offset": 4,
                                                             "bytes": "00000000" + "04400000"}]})
        side = lift(BASE, rodata)
        self.assertEqual(side["contract"]["tree"]["exit"]["code"], ["bv", 32, "00004004"])
        # Code pointers, immediates and other unresolved data stay blocked.
        other = global_store("0x5000")
        other["instructions"][3]["refs"][0]["why"] = "code-address"
        self.assertEqual(lift(BASE, other, maps)["issues"][0]["issue"], "unknown-memory")

    def test_stack_cookie_builtin(self):
        check = {"entry_index": 0, "issues": [], "library": "__security_check_cookie",
                 "instructions": [{"bytes": "c3", "refs": [], "next": None, "offset": 0}]}

        def case(second_xor=True):
            code = ["4883ec28", "488b0500000000", "4833c4", "4889442420", "488b4c2420"]
            code += (["4833cc"] if second_xor else ["90"]) + ["e800000000", "4883c428"]
            code += ret(0x4000) + EPILOGUE
            f = function(code, {1: [global_ref("0x7000")], 6: [{"kind": "function", "target": "0x300"}]})
            f["instructions"][1]["issues"] = ["unresolved-or-mutable-data"]
            return f
        g = graph(BASE, case(), {"0x300": check})
        side = {"binary_sha256": BASE, "arch": "x64", "image_base": "0x140000000", "table_rva": "0x2000",
                "vm_stack": {"sp_offset": "0x14270", "base_offset": "0x14268"}, "evidence": [],
                "stack_cookie": {"cookie_rva": "0x7000", "check_rva": "0x300",
                                 "check_body_sha256": ce.body_digest(g, "0x300")}}
        ctx = ce.check_context({"schema": ce.CONTEXT, "revision": "t", "sides": [side]})
        result = ce.Lifter(g, ctx, {}, {}).lift(0x2000)
        self.assertEqual(result["status"], "extracted", result)
        self.assertEqual(result["contract"]["tree"]["events"], [{"op": "release_stack", "bytes": 0, "id": "e0"}])
        bad = graph(BASE, case(second_xor=False), {"0x300": check})
        self.assertEqual(ce.Lifter(bad, ctx, {}, {}).lift(0x2000)["issues"][0]["issue"], "unsupported-effect")
        stale = copy.deepcopy(side)
        stale["stack_cookie"]["check_body_sha256"] = "0" * 64
        with self.assertRaises(cc.InvalidEvidence):
            ce.Lifter(g, ce.check_context({"schema": ce.CONTEXT, "revision": "t", "sides": [stale]}), {}, {})

    def test_proposals_pair_fields_but_are_not_mappings(self):
        lifters = [ce.Lifter(graph(sha, global_store(t)), context(), mapping((FLAGS, ERRORS)), {})
                   for sha, t in ((BASE, "0x5000"), (DERIVED, "0x6000"))]
        before = [dict(lifter.mappings) for lifter in lifters]
        result = ce.propose(lifters, [0x2000])
        self.assertEqual(result["aligned"], ["0x2000"])
        [candidate] = result["candidates"]
        self.assertEqual((candidate["base"]["offset"], candidate["derived"]["offset"]), ("0x5000", "0x6000"))
        self.assertFalse(candidate["conflict"])
        self.assertEqual([lifter.mappings for lifter in lifters], before)
        self.assertEqual(lifters[0].lift(0x2000)["issues"][0]["issue"], "unproven-mapping")


if __name__ == "__main__":
    unittest.main()
