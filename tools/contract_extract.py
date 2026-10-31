"""Read-only bootstrap extractor for pure constant-return x64 graph fragments.

Consumes existing cmvs-command-graph-v3 exports. This is NOT a command contract:
dispatcher cleanup/PC semantics and binary proof verification are still pending.
Unknown instructions, library leaves and unresolved effects are never skipped.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


def sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pc = sibling("profile_compare")


class Blocked(Exception):
    def __init__(self, reason, function, index=None):
        self.issue = {"reason": reason, "function": function, "instruction_index": index}


def extract(graph, opcode, instruction_budget=4096, depth_limit=8):
    pc.validate(graph)
    if graph["arch"] != "x64":
        raise ValueError("bootstrap extractor supports x64 only")
    if type(opcode) is not int or type(instruction_budget) is not int or not 1 <= instruction_budget <= 4096:
        raise ValueError("invalid opcode or instruction budget")
    if type(depth_limit) is not int or not 0 <= depth_limit <= 8:
        raise ValueError("invalid call depth")
    command = next((c for c in graph["commands"] if c["opcode"] == opcode), None)
    if command is None:
        raise ValueError("opcode not in export")
    result = {"schema": "cmvs-return-fragment-v1", "opcode": opcode,
              "arch": "x64", "binary_sha256": graph["binary_sha256"],
              "graph_sha256": pc.digest(graph), "instruction_budget": instruction_budget,
              "depth_limit": depth_limit, "scope": "exported-function-fragment-only",
              "command_contract_complete": False}
    if command["kind"] != "handler":
        return {**result, "status": "unknown", "issues": [{"reason": "not-handler"}]}
    entry = command["entry"]
    result["entry"] = entry
    visited = []
    remaining = instruction_budget

    def walk(function, active):
        nonlocal remaining
        if function in active:
            raise Blocked("recursion", function)
        if len(active) > depth_limit:
            raise Blocked("budget-exceeded", function)
        node = graph["functions"][function]
        if node.get("library") is not None:
            raise Blocked("opaque-library", function)
        if node["issues"]:
            raise Blocked("incomplete-export", function)
        index = node.get("entry_index")
        value = None
        seen = set()
        while True:
            if type(index) is not int or not 0 <= index < len(node["instructions"]):
                raise Blocked("cfg-gap", function, index)
            if index in seen:
                raise Blocked("loop", function, index)
            if remaining == 0:
                raise Blocked("budget-exceeded", function, index)
            seen.add(index)
            remaining -= 1
            insn = node["instructions"][index]
            visited.append({"function": function, "instruction_index": index,
                            "offset": insn.get("offset")})
            if insn.get("issues"):
                raise Blocked("incomplete-export", function, index)
            raw = bytes.fromhex(insn["bytes"])
            refs, nxt = insn["refs"], insn["next"]
            if raw == b"\xc3" and not refs:
                if nxt is not None:
                    raise Blocked("inconsistent-cfg", function, index)
                if value is None:
                    raise Blocked("unknown-return-value", function, index)
                return value
            if len(raw) == 5 and raw[0] == 0xb8 and not refs:
                value = int.from_bytes(raw[1:], "little")
            elif raw == b"\x90" and not refs:
                pass
            elif len(raw) == 5 and raw[0] in (0xe8, 0xe9):
                if len(refs) != 1 or refs[0]["kind"] != "function":
                    raise Blocked("unresolved-call", function, index)
                value = walk(refs[0]["target"], active + (function,))
                if raw[0] == 0xe9:
                    if nxt is not None:
                        raise Blocked("inconsistent-cfg", function, index)
                    return value
            else:
                raise Blocked("unsupported-instruction", function, index)
            index = nxt

    try:
        value = walk(entry, ())
    except Blocked as error:
        return {**result, "status": "unknown", "issues": [error.issue], "visited": visited}
    return {**result, "status": "extracted-fragment", "return_eax": ["bv", 32, "%08x" % value],
            "memory_effects": [], "issues": [], "visited": visited}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("export")
    parser.add_argument("opcode", type=lambda v: int(v, 0))
    args = parser.parse_args()
    try:
        raw = Path(args.export).read_bytes()
        if len(raw) > 256 * 1024 * 1024:
            raise ValueError("export size limit")
        def unique(items):
            obj = {}
            for key, value in items:
                if key in obj:
                    raise ValueError("duplicate JSON key")
                obj[key] = value
            return obj
        graph = json.loads(raw, object_pairs_hook=unique)
        result = extract(graph, args.opcode)
        result["export_sha256"] = hashlib.sha256(raw).hexdigest()
        result["extractor_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    except (ValueError, TypeError, KeyError, IndexError, RecursionError, OSError) as error:
        result = {"status": "invalid-input", "reason": str(error)}
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return {"extracted-fragment": 0, "unknown": 2, "invalid-input": 3}[result["status"]]


if __name__ == "__main__":
    raise SystemExit(main())
