"""M1 diagnostic contract IR validator/comparator; never writes engine profiles.

Only identical complete normal forms establish equivalence. Bounded concrete
execution can establish a difference, but never establishes equivalence.
"""
import argparse
import copy
import hashlib
import itertools
import json
import re
from pathlib import Path

WIDTHS = (8, 16, 32, 64)
CLOSED = ("cfg_closed", "effects_closed", "calls_closed", "memory_closed", "domain_closed")
ISSUES = frozenset(("unsupported-instruction", "cfg-gap", "unresolved-call", "unknown-memory",
                    "unproven-mapping", "pointer-escape", "unsupported-effect", "undefined-behavior",
                    "loop", "recursion", "budget-exceeded", "unsupported-expression", "unmatched-paths"))
COMMUTATIVE = frozenset(("eq", "ne", "and", "or", "xor", "add"))


class Invalid(ValueError):
    """Malformed IR, distinct from a valid but unsupported feature."""


class Unsupported(ValueError):
    """Well-formed input outside the M1 supported subset."""


def require(condition, message):
    if not condition:
        raise Invalid(message)


def keys(value, names):
    require(isinstance(value, dict) and set(value) == set(names.split()),
            "unexpected or missing fields: " + names)


def integer(value):
    return type(value) is int


def name(value):
    require(isinstance(value, str) and 0 < len(value) <= 256, "invalid identifier")


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False)


def digest(value):
    return hashlib.sha256(encode(value).encode("utf-8")).hexdigest()


def expression(value, args, refs, depth=0):
    require(depth <= 64, "expression nesting limit")
    require(isinstance(value, list) and value and isinstance(value[0], str), "invalid expression")
    op = value[0]
    def child(v):
        return expression(v, args, refs, depth + 1)
    if op == "bool":
        require(len(value) == 2 and type(value[1]) is bool, "invalid boolean")
        return "bool"
    if op == "bv":
        require(len(value) == 3 and integer(value[1]) and value[1] in WIDTHS, "invalid bit width")
        require(isinstance(value[2], str) and re.fullmatch("[0-9a-f]{%d}" % (value[1] // 4), value[2]),
                "invalid bit constant")
        return value[1]
    if op in ("arg", "ref"):
        require(len(value) == 2 and isinstance(value[1], str), "invalid value reference")
        env = args if op == "arg" else refs
        require(value[1] in env, "value used before read: " + value[1])
        return env[value[1]]
    if op in ("zext", "sext", "trunc"):
        require(len(value) == 3 and integer(value[1]) and value[1] in WIDTHS, "invalid conversion")
        source = child(value[2])
        require(source in WIDTHS, "boolean conversion")
        require(value[1] < source if op == "trunc" else value[1] > source, "invalid conversion direction")
        return value[1]
    if op == "not":
        require(len(value) == 2, "invalid unary expression")
        width = child(value[1])
        require(width in WIDTHS, "bitwise not needs bit vector")
        return width
    require(op in ("eq", "ne", "ult", "slt", "and", "or", "xor", "add", "sub") and len(value) == 3,
            "unknown expression or arity")
    left, right = child(value[1]), child(value[2])
    require(left == right and left in WIDTHS, "operand type mismatch")
    return "bool" if op in ("eq", "ne", "ult", "slt") else left


def validate(contract):
    keys(contract, "precondition parameters tree coverage")
    require(expression(contract["precondition"], {}, {}) == "bool", "invalid precondition")
    require(contract["precondition"] == ["bool", True], "M1 requires unconditional input domain")
    require(isinstance(contract["parameters"], list) and len(contract["parameters"]) <= 64, "parameter limit")
    params = {}
    for p in contract["parameters"]:
        keys(p, "id stack_offset width interpretation")
        name(p["id"])
        require(p["id"] not in params, "duplicate parameter")
        require(integer(p["width"]) and p["width"] in WIDTHS, "parameter width")
        require(integer(p["stack_offset"]) and -(1 << 31) <= p["stack_offset"] <= -p["width"] // 8,
                "parameter must precede entry SP")
        require(p["interpretation"] in ("bits", "signed", "unsigned"), "parameter interpretation")
        params[p["id"]] = p["width"]
    keys(contract["coverage"], " ".join(CLOSED) + " issues")
    coverage = contract["coverage"]
    require(all(type(coverage[k]) is bool for k in CLOSED), "coverage flags must be boolean")
    require(isinstance(coverage["issues"], list) and all(isinstance(i, str) and i in ISSUES for i in coverage["issues"]),
            "unknown coverage issue")
    counters = {"nodes": 0, "events": 0, "leaves": 0}
    targets = {}
    unsupported = []

    def target(t, width):
        keys(t, "region object field")
        require(t["region"] in ("vm", "script", "object", "save", "config"), "unknown region")
        name(t["object"])
        name(t["field"])
        require(integer(width) and width in WIDTHS, "state width")
        if t["region"] not in ("vm", "object"):
            unsupported.append("unsupported-effect")
        key = encode(t)
        require(key not in targets or targets[key] == width, "overlapping state widths need alias model")
        targets[key] = width

    def tree(node, args, refs, used, depth):
        require(depth <= 64, "tree nesting limit")
        require(isinstance(node, dict), "invalid tree")
        kind = node.get("kind")
        require(kind in ("branch", "leaf"), "unknown tree kind")
        keys(node, "kind events condition then else" if kind == "branch" else "kind events exit")
        counters["nodes"] += 1
        require(counters["nodes"] <= 128, "tree node limit")
        require(isinstance(node["events"], list), "invalid events")
        args, refs, used = dict(args), dict(refs), set(used)
        for event in node["events"]:
            counters["events"] += 1
            require(counters["events"] <= 4096, "event limit")
            require(isinstance(event, dict), "invalid event")
            op = event.get("op")
            fields = {"read_parameter": "id op parameter", "read_state": "id op target width",
                      "write_state": "id op target width value", "release_stack": "id op bytes"}
            require(isinstance(op, str) and op in fields, "unknown event")
            keys(event, fields[op])
            name(event["id"])
            require(event["id"] not in used, "duplicate event on path")
            used.add(event["id"])
            if op == "read_parameter":
                require(isinstance(event["parameter"], str) and event["parameter"] in params, "unknown parameter")
                # Re-reading after a stack change needs an explicit snapshot model.
                require(event["parameter"] not in args, "duplicate parameter read")
                args[event["parameter"]] = params[event["parameter"]]
            elif op in ("read_state", "write_state"):
                target(event["target"], event["width"])
                if op == "read_state":
                    refs[event["id"]] = event["width"]
                else:
                    require(expression(event["value"], args, refs) == event["width"], "write type mismatch")
            else:
                require(integer(event["bytes"]) and 0 <= event["bytes"] <= 0xffffffff, "invalid stack release")
        if kind == "branch":
            require(expression(node["condition"], args, refs) == "bool", "branch condition not boolean")
            tree(node["then"], args, refs, used, depth + 1)
            tree(node["else"], args, refs, used, depth + 1)
        else:
            counters["leaves"] += 1
            require(counters["leaves"] <= 64, "leaf limit")
            keys(node["exit"], "kind code")
            require(node["exit"]["kind"] in ("return", "yield", "stop"), "unknown exit kind")
            if node["exit"]["kind"] != "return":
                unsupported.append("unsupported-effect")
            require(expression(node["exit"]["code"], args, refs) == 32, "return code must be 32 bits")

    tree(contract["tree"], {}, {}, set(), 0)
    if unsupported:
        raise Unsupported(",".join(sorted(set(unsupported))))
    return targets


def canonical(contract):
    result = copy.deepcopy(contract)
    params = {p["id"]: "a%d" % i for i, p in enumerate(result["parameters"])}
    for p in result["parameters"]:
        p["id"] = params[p["id"]]
    counter = itertools.count()

    def expr(v, refs):
        if v[0] == "arg":
            return ["arg", params[v[1]]]
        if v[0] == "ref":
            return ["ref", refs[v[1]]]
        out = [expr(x, refs) if isinstance(x, list) else x for x in v]
        if out[0] in COMMUTATIVE:
            out[1:] = sorted(out[1:], key=encode)
        return out

    def walk(node, refs):
        refs = dict(refs)
        for event in node["events"]:
            if "value" in event:
                event["value"] = expr(event["value"], refs)
            if "parameter" in event:
                event["parameter"] = params[event["parameter"]]
            old = event["id"]
            event["id"] = "e%d" % next(counter)
            refs[old] = event["id"]
        if node["kind"] == "branch":
            node["condition"] = expr(node["condition"], refs)
            walk(node["then"], refs)
            walk(node["else"], refs)
        else:
            node["exit"]["code"] = expr(node["exit"]["code"], refs)
    walk(result["tree"], {})
    return result


def evaluate(expr, args, refs):
    op = expr[0]
    if op == "bool":
        return expr[1], "bool"
    if op == "bv":
        return int(expr[2], 16), expr[1]
    if op in ("arg", "ref"):
        return (args if op == "arg" else refs)[expr[1]]
    if op in ("zext", "sext", "trunc"):
        value, width = evaluate(expr[2], args, refs)
        if op == "sext" and value & (1 << (width - 1)):
            value -= 1 << width
        return value & ((1 << expr[1]) - 1), expr[1]
    left, width = evaluate(expr[1], args, refs)
    if op == "not":
        return (~left) & ((1 << width) - 1), width
    right, _ = evaluate(expr[2], args, refs)
    if op in ("eq", "ne", "ult", "slt"):
        if op == "slt":
            left = left - (1 << width) if left >> (width - 1) else left
            right = right - (1 << width) if right >> (width - 1) else right
        return {"eq": left == right, "ne": left != right, "ult": left < right, "slt": left < right}[op], "bool"
    value = {"and": lambda: left & right, "or": lambda: left | right, "xor": lambda: left ^ right,
             "add": lambda: left + right, "sub": lambda: left - right}[op]()
    return value & ((1 << width) - 1), width


def execute(contract, stack, initial):
    """Concrete shared byte-stack/state input; a witness is never an equivalence proof."""
    params = {p["id"]: p for p in contract["parameters"]}
    state, args, refs, trace = dict(initial), {}, {}, []
    released = 0
    node = contract["tree"]
    while True:
        for event in node["events"]:
            op = event["op"]
            if op == "read_parameter":
                p = params[event["parameter"]]
                offset = p["stack_offset"]
                value = sum(stack.get(offset + i, 0) << (8 * i) for i in range(p["width"] // 8))
                args[p["id"]] = value, p["width"]
                trace.append((op, offset, p["width"], value))
            elif op == "read_state":
                key = encode(event["target"])
                refs[event["id"]] = state[key], event["width"]
                trace.append((op, key, event["width"], state[key]))
            elif op == "write_state":
                key = encode(event["target"])
                value, _ = evaluate(event["value"], args, refs)
                state[key] = value
                trace.append((op, key, event["width"], value))
            else:
                released += event["bytes"]
                trace.append((op, event["bytes"]))
        if node["kind"] == "leaf":
            return {"events": trace, "exit": evaluate(node["exit"]["code"], args, refs)[0],
                    "released": released, "state": state}
        condition, _ = evaluate(node["condition"], args, refs)
        node = node["then"] if condition else node["else"]


def compare(left, right, witness_budget=256):
    require(integer(witness_budget) and 0 <= witness_budget <= 4096, "invalid witness budget")
    unsupported = []
    targets = {}
    # Validate both sides even when the first uses a known unsupported region.
    for contract in (left, right):
        try:
            found = validate(contract)
            for key, width in found.items():
                if key in targets and targets[key] != width:
                    unsupported.append("unknown-memory")
                targets[key] = width
        except Unsupported as error:
            unsupported.append(str(error))
    issues = sorted(set(unsupported + [i for c in (left, right) for i in c["coverage"]["issues"]]))
    if issues or any(not c["coverage"][k] for c in (left, right) for k in CLOSED):
        return {"status": "unknown", "reason": "incomplete", "issues": issues}
    a, b = canonical(left), canonical(right)
    hashes = {"base_contract_sha256": digest(a), "derived_contract_sha256": digest(b)}
    if a == b:
        return {"status": "equivalent", "reason": "identical-complete-normal-form", **hashes}
    offsets = sorted({p["stack_offset"] + i for c in (left, right)
                      for p in c["parameters"] for i in range(p["width"] // 8)})
    # Deterministic bounded witnesses. No counterexample => unknown, never equal.
    seeds = [0, 1, 0xff, 0x80, 0x7f]
    candidates = [({i: seed for i in offsets}, {k: seed & ((1 << w) - 1) for k, w in targets.items()})
                  for seed in seeds]
    for offset in offsets:
        candidates.append(({offset: 1}, {k: 0 for k in targets}))
    for key in sorted(targets):
        candidates.append(({}, {k: int(k == key) for k in targets}))
    for stack, initial in candidates[:witness_budget]:
        x, y = execute(left, stack, initial), execute(right, stack, initial)
        if x != y:
            return {"status": "different", "reason": "concrete-contract-counterexample", **hashes,
                    "witness": {"stack_bytes": {str(k): v for k, v in stack.items()}, "initial_state": initial},
                    "base_observation": x, "derived_observation": y}
    return {"status": "unknown", "reason": "normal-forms-differ-no-witness", **hashes}


class InvalidEvidence(ValueError):
    """Identity/version binding failure; a tool error outside the three-valued result."""


ENVELOPE = "cmvs-command-contract-v1"
EVIDENCE_HASHES = ("export_sha256", "extractor_revision", "rules_sha256", "mapping_sha256", "summaries_sha256")
SHA = re.compile(r"[0-9a-f]{64}\Z")
ADDR = re.compile(r"0x[0-9a-f]+\Z")


def verify_envelope(envelope):
    def need(condition, message):
        if not condition:
            raise InvalidEvidence(message)
    need(isinstance(envelope, dict) and set(envelope) == {"schema", "identity", "evidence", "contract"}
         and envelope["schema"] == ENVELOPE, "envelope fields")
    identity, evidence = envelope["identity"], envelope["evidence"]
    need(isinstance(identity, dict) and set(identity) == {"profile_id", "binary_sha256", "arch", "image_base",
                                                          "opcode", "case_rva"}, "identity fields")
    need(isinstance(identity["profile_id"], str) and identity["profile_id"], "profile id")
    need(SHA.fullmatch(str(identity["binary_sha256"])) is not None, "binary hash")
    need(identity["arch"] == "x64", "M1 accepts x64 only")
    need(all(ADDR.fullmatch(str(identity[k])) for k in ("image_base", "opcode", "case_rva")), "identity address")
    need(isinstance(evidence, dict) and set(evidence) == set(EVIDENCE_HASHES) | {
        "binary_checked", "mappings_used", "summaries_used"}, "evidence fields")
    need(all(SHA.fullmatch(str(evidence[k])) for k in EVIDENCE_HASHES), "evidence hash")
    need(type(evidence["binary_checked"]) is bool, "binary check flag")
    need(all(isinstance(evidence[k], list) and all(isinstance(i, str) for i in evidence[k])
             for k in ("mappings_used", "summaries_used")), "dependency lists")


def compare_envelopes(base, derived, rules=None):
    """Verify both envelopes and their shared rule versions, then compare contracts.
    Envelopes without original-binary binding never yield `equivalent`."""
    for envelope in (base, derived):
        verify_envelope(envelope)
    a, b = base["identity"], derived["identity"]
    if a["arch"] != b["arch"] or a["opcode"] != b["opcode"] or a["binary_sha256"] == b["binary_sha256"]:
        raise InvalidEvidence("expected distinct same-architecture builds of one opcode")
    for key in EVIDENCE_HASHES[1:]:
        if base["evidence"][key] != derived["evidence"][key] or (rules and rules[key] != base["evidence"][key]):
            raise InvalidEvidence("rule or tool version mismatch: " + key)
    result = compare(base["contract"], derived["contract"])
    if result["status"] == "equivalent" and not (base["evidence"]["binary_checked"]
                                                 and derived["evidence"]["binary_checked"]):
        return {"status": "unknown", "reason": "binary-not-checked",
                **{k: v for k, v in result.items() if k.endswith("sha256")}}
    return result


def load(path):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result
    raw = Path(path).read_bytes()
    require(len(raw) <= 8 * 1024 * 1024, "input size limit")
    return json.loads(raw, object_pairs_hook=pairs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", help="JSON semantic body, not an engine profile")
    parser.add_argument("derived")
    args = parser.parse_args()
    try:
        result = compare(load(args.base), load(args.derived))
    except (ValueError, TypeError, KeyError, RecursionError, OSError) as error:
        result = {"status": "invalid-input", "reason": str(error)}
    result["scope"] = "supplied-ir-only; no binary evidence verification or runtime authorization"
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return {"equivalent": 0, "different": 1, "unknown": 2, "invalid-input": 3}[result["status"]]


if __name__ == "__main__":
    raise SystemExit(main())
