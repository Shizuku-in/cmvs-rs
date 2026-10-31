"""Compare same-architecture IDA exports using the Python standard library only.

python -I tools/profile_compare.py BASE.json DERIVED.json --report REPORT.tsv
    [--base-profile BASE.profile --profile DERIVED.profile --output-profile OUT.profile]
python -I tools/profile_compare.py BASE.json DERIVED.json --explain 0x2091
python -I tools/profile_compare.py BASE.json DERIVED.json --group-differences [0x2140 0x215d]
No input is modified. Incomplete, cyclic, indirect or mutable dependencies never inherit.
FLIRT-recognized runtime functions are leaves identified by name and their own body.
A command is presumed when only PRESUMABLE limits block it and both shapes are equal.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

SCHEMA = "cmvs-command-graph-v3"
POLICY = "raw-operands-closed-cfg-readonly-data-libraries-switches-v3"
HEX = re.compile(r"[0-9a-f]{64}\Z")
PLACEHOLDER_NAME = re.compile(r"(?:sub|nullsub|j|unknown_libname)_[0-9A-Fa-f]+\Z")
# Limits that hide content without showing a difference: blocked data, indirect calls,
# recursion and the depth bound. Decoding or CFG gaps never qualify.
PRESUMABLE = frozenset({"unresolved-data", "unresolved-or-mutable-data", "indirect-control-flow",
                        "recursive-cycle", "depth-limit"})
REF_KINDS = ("function", "local", "data", "import", "unknown-data", "opaque",
             "image-base", "switch-table", "switch")


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def validate(graph):
    if graph.get("schema") != SCHEMA or graph.get("policy") != POLICY:
        raise ValueError("unsupported export schema or normalization policy")
    if graph.get("arch") not in ("x64", "x86") or not HEX.fullmatch(graph.get("binary_sha256", "")):
        raise ValueError("invalid architecture or binary hash")
    commands = graph["commands"]
    if not 1 <= len(commands) <= 2048:
        raise ValueError("invalid command count")
    for i, command in enumerate(commands):
        if command["opcode"] != 0x2000+i or command["kind"] not in ("handler", "noop", "absent"):
            raise ValueError("invalid or nonconsecutive command rows")
        if command["kind"] == "handler" and command["entry"] not in graph["functions"]:
            raise ValueError("missing command function")
    for node in graph["functions"].values():
        if not isinstance(node["issues"], list) or not isinstance(node["instructions"], list):
            raise ValueError("invalid function node")
        library = node.get("library")
        if library is not None and not (isinstance(library, str) and library
                                        and not PLACEHOLDER_NAME.match(library)):
            raise ValueError("library leaves need a stable, non-placeholder name")
        def local(target):
            return isinstance(target, int) and (0 <= target < len(node["instructions"]) or (
                target == -1 and "missing-local-target" in node["issues"]))
        for instruction in node["instructions"]:
            if (instruction["next"] is not None and not 0 <= instruction["next"] < len(node["instructions"])
                    and not (instruction["next"] == -1 and "missing-fallthrough" in node["issues"])):
                raise ValueError("invalid fallthrough target")
            if not re.fullmatch(r"(?:[0-9a-f]{2}){1,15}", instruction["bytes"]):
                raise ValueError("invalid instruction encoding")
            for ref in instruction["refs"]:
                kind = ref["kind"]
                if kind == "function" and ref["target"] not in graph["functions"]:
                    raise ValueError("missing dependency function")
                if kind == "local" and not local(ref["target"]):
                    raise ValueError("invalid local branch target")
                if kind == "switch" and not (
                        (ref["default"] is None or local(ref["default"])) and all(
                            len(case) == 2 and local(case[1]) and all(isinstance(v, int) for v in case[0])
                            for case in ref["cases"])):
                    raise ValueError("invalid switch cases")
                if kind == "data" and (not re.fullmatch(r"(?:[0-9a-f]{2})+", ref["bytes"])
                                       or not 0 <= ref["offset"] < len(ref["bytes"])//2):
                    raise ValueError("invalid data dependency")
                if kind == "import" and not ref.get("name"):
                    raise ValueError("unnamed import")
                if kind == "opaque" and library is None:
                    raise ValueError("opaque references belong only to library functions")
                if library is not None and kind not in ("local", "import", "opaque"):
                    raise ValueError("library functions keep only local, import or opaque references")
                if kind not in REF_KINDS:
                    raise ValueError("unknown reference kind")


def fingerprint(graph, entry, max_depth=8):
    """Hash the complete dependency tree down to library leaves. Cycles stay incomplete."""
    memo = {}
    def walk(key, remaining, active):
        cache_key = (key, remaining)
        if key in active:
            return None, ("recursive-cycle",)
        if remaining < 0:
            return None, ("depth-limit",)
        if cache_key in memo:
            return memo[cache_key]
        node = graph["functions"][key]
        issues = set(node["issues"])
        if not node["instructions"]:
            issues.add("empty-function")
        normalized = []
        for instruction in node["instructions"]:
            if "library" in node:
                # A leaf: refs are local branches or opaque placeholders, never walked.
                normalized.append({k: instruction[k] for k in ("bytes", "refs", "next")})
                continue
            refs = []
            for ref in instruction["refs"]:
                if ref["kind"] == "function":
                    child, child_issues = walk(ref["target"], remaining-1, active | {key})
                    issues.update(child_issues)
                    refs.append({"kind": "function", "fingerprint": child})
                elif ref["kind"] == "unknown-data":
                    issues.add("unresolved-data")
                else:
                    refs.append(ref)
            normalized.append({"bytes": instruction["bytes"], "refs": refs, "next": instruction["next"]})
        if not 0 <= node.get("entry_index", -1) < len(node["instructions"]):
            issues.add("missing-entry-index")
        identity = [node["entry_index"], normalized]
        if "library" in node:
            identity = ["library", node["library"]] + identity
        result = (None, tuple(sorted(issues))) if issues else (digest(identity), ())
        memo[cache_key] = result
        return result
    return walk(entry, max_depth, set())


def shape(graph, entry, max_depth=8, budget=200000):
    """Hash everything comparable: blocked data becomes a placeholder, a back edge names
    its ancestor by distance, the depth bound is a token, and node issues are included.
    Raises OverflowError when the walk exceeds `budget` function visits."""
    memo = {}
    left = [budget]

    def walk(key, remaining, path):
        if key in path:
            return {"cycle": len(path)-path.index(key)}, True
        if remaining < 0:
            return {"truncated": True}, False
        if (key, remaining) in memo:
            return memo[(key, remaining)], False
        left[0] -= 1
        if left[0] < 0:
            raise OverflowError("shape walk budget exhausted")
        node = graph["functions"][key]
        cyclic, normalized = False, []
        for instruction in node["instructions"]:
            refs = []
            for ref in instruction["refs"]:
                if ref["kind"] == "function" and "library" not in node:
                    child, back = walk(ref["target"], remaining-1, path+(key,))
                    cyclic |= back
                    refs.append({"kind": "function", "shape": child})
                elif ref["kind"] == "unknown-data":
                    refs.append({"kind": "blocked"})
                else:
                    refs.append(ref)
            normalized.append({"bytes": instruction["bytes"], "refs": refs, "next": instruction["next"]})
        result = digest([node.get("library"), node.get("entry_index"), sorted(node["issues"]), normalized])
        # A result containing back edges depends on the path; recompute it elsewhere.
        if not cyclic:
            memo[(key, remaining)] = result
        return result, cyclic
    return walk(entry, max_depth, ())[0]


def compare(base, derived, max_depth=8):
    validate(base)
    validate(derived)
    if base["arch"] != derived["arch"] or base["policy"] != derived["policy"]:
        raise ValueError("only same-architecture exports with identical policies may be compared")
    if base["profile"] == derived["profile"] or base["binary_sha256"] == derived["binary_sha256"]:
        raise ValueError("expected distinct base and derived builds")
    rows = []
    for i, command in enumerate(derived["commands"]):
        status, proof, reason = command["kind"], "-", command["kind"]
        if command["kind"] == "handler":
            status = "unreviewed"
            reference = base["commands"][i] if i < len(base["commands"]) else None
            if reference is None or reference["kind"] != "handler":
                reason = "no-base-handler"
            else:
                left, li = fingerprint(base, reference["entry"], max_depth)
                right, ri = fingerprint(derived, command["entry"], max_depth)
                if li or ri:
                    reason = ",".join(sorted(set(li+ri)))
                    if set(li+ri) <= PRESUMABLE:
                        try:
                            same = shape(base, reference["entry"], max_depth)
                            if same == shape(derived, command["entry"], max_depth):
                                status, proof = "presumed", same
                            else:
                                reason += ",shape-different"
                        except OverflowError:
                            reason += ",shape-limit"
                elif left != right:
                    reason = "different"
                else:
                    status, proof, reason = "inherited", left, "complete-match"
        if status == "absent":
            reason = "absent"
        rows.append({"opcode": command["opcode"], "status": status, "fingerprint": proof, "reason": reason})
    return rows


def profile_lines(path):
    return [line.split() for line in Path(path).read_text(encoding="utf-8-sig").splitlines()
            if line.strip() and not line.lstrip().startswith("#")]


def bind_profile(lines, graph):
    if next(w[1] for w in lines if w[0] == "profile") != graph["profile"]:
        raise ValueError("profile ID differs from export")
    binaries = [w for w in lines if w[0] == "binary"]
    column = next(i for i, w in enumerate(binaries) if w[1] == graph["arch"])
    fields = dict(w.split("=", 1) for w in binaries[column][2:])
    if (fields["sha256"] != graph["binary_sha256"] or fields["image-base"] != graph["image_base"]
            or fields["table"] != graph["table_rva"]):
        raise ValueError("profile binary identity differs from export")
    rows = [w for w in lines if w[0].startswith("0x")]
    if len(rows) != len(graph["commands"]):
        raise ValueError("profile switch size differs from export")
    for row, command in zip(rows, graph["commands"]):
        kind = "absent" if row[1] == "absent" else "noop" if row[1] == "noop" else "handler"
        entry = row[2+column]
        if int(row[0], 0) != command["opcode"] or kind != command["kind"] or (
                None if entry == "-" else hex(int(entry, 0))) != command["entry"]:
            raise ValueError("profile command differs from export")
    return binaries


def apply_profile(base, derived, rows, base_path, derived_path):
    left, right = profile_lines(base_path), profile_lines(derived_path)
    bind_profile(left, base)
    binaries = bind_profile(right, derived)
    if next(w[1] for w in left if w[0] == "base") != "-" or next(
            w[1] for w in right if w[0] == "base") != base["profile"]:
        raise ValueError("inheritance must name its root profile")
    proof_keys = {"inherited": "inherit", "presumed": "presume"}
    output, proofs = [], False
    for words in right:
        if words[0] == "compare":
            continue
        if words[0].startswith("0x") and words[1] == "reviewed":
            # Manual review records outrank a fresh comparison and are kept verbatim.
            if rows[int(words[0], 0)-0x2000]["status"] in ("noop", "absent"):
                raise ValueError("reviewed command is not a handler in the export")
        elif words[0].startswith("0x"):
            result = rows[int(words[0], 0)-0x2000]
            if words[1] not in ("unreviewed", "inherited", "presumed", "noop", "absent"):
                raise ValueError("will not overwrite variant or unknown status")
            words = words[:2+len(binaries)]
            words[1] = result["status"]
            if result["status"] in proof_keys:
                words.append(f"{proof_keys[result['status']]}={base['arch']}:{result['fingerprint']}")
                proofs = True
        output.append(" ".join(words))
        if words is binaries[-1]:
            header = len(output)
    if proofs:
        # The derived exe is the profile's own binary of this arch; only the root exe is named.
        output.insert(header, f"compare {base['arch']} base-sha256={base['binary_sha256']}")
    return "\n".join(output)+"\n"


def where(instruction, index):
    """Function-relative offset when the export has one, else the instruction index."""
    offset = instruction.get("offset")
    return f"+0x{offset:x}" if offset is not None and offset >= 0 else (
        f"-0x{-offset:x}" if offset is not None else f"#{index}")


def same_instruction(left, right):
    """Semantic equality of one instruction, with function refs paired by position."""
    if (left["bytes"], left["next"], len(left["refs"])) != (right["bytes"], right["next"], len(right["refs"])):
        return False
    for a, b in zip(left["refs"], right["refs"]):
        if a["kind"] != b["kind"]:
            return False
        if a["kind"] not in ("function", "unknown-data") and a != b:
            return False
    return True


def group_differences(base, derived, opcodes=None, max_depth=8):
    """Read-only triage: group commands by their first differing function boundaries.

    Walk callees only while the entire parent body aligns. A differing body is a
    manual-review boundary, not an assertion that its descendants are equivalent.
    Data/indirect/depth limitations remain in the ordinary comparison report.
    """
    if not 0 <= max_depth <= 32:
        raise ValueError("max-depth must be 0..32")
    rows = compare(base, derived, max_depth)
    selected = set(opcodes) if opcodes is not None else {
        r["opcode"] for r in rows if r["status"] == "unreviewed"}
    if any(not 0 <= op-0x2000 < len(derived["commands"]) for op in selected):
        raise ValueError("opcode outside the derived switch")
    groups = {}
    for opcode in sorted(selected):
        index = opcode-0x2000
        left = base["commands"][index] if index < len(base["commands"]) else None
        right = derived["commands"][index]
        if left is None or left["kind"] != "handler" or right["kind"] != "handler":
            continue
        seen = set()

        def visit(lkey, rkey, remaining):
            key = (lkey, rkey, remaining)
            if key in seen:
                return
            seen.add(key)
            a, b = base["functions"][lkey], derived["functions"][rkey]
            same = (a.get("library") == b.get("library")
                    and a.get("entry_index") == b.get("entry_index")
                    and sorted(a["issues"]) == sorted(b["issues"])
                    and len(a["instructions"]) == len(b["instructions"])
                    and all(same_instruction(x, y) for x, y in zip(a["instructions"], b["instructions"])))
            if not same:
                groups.setdefault((lkey, rkey), set()).add(opcode)
                return
            if remaining == 0 or a.get("library") is not None:
                return
            for x, y in zip(a["instructions"], b["instructions"]):
                for u, v in zip(x["refs"], y["refs"]):
                    if u["kind"] == "function":
                        visit(u["target"], v["target"], remaining-1)

        visit(left["entry"], right["entry"], max_depth)
    return [{"base": lkey, "derived": rkey, "opcodes": sorted(ops)}
            for (lkey, rkey), ops in sorted(groups.items(), key=lambda item: (
                -len(item[1]), int(item[0][0], 0), int(item[0][1], 0)))]


def explain(base, derived, opcode, max_depth=8):
    """Diagnostic only: why one command did not inherit. Never changes a status."""
    validate(base)
    validate(derived)
    if base["arch"] != derived["arch"] or base["policy"] != derived["policy"]:
        raise ValueError("only same-architecture exports with identical policies may be compared")
    index = opcode - 0x2000
    if not 0 <= index < len(derived["commands"]):
        raise ValueError("opcode outside the derived switch")
    row = compare(base, derived, max_depth)[index]
    lines = [f"0x{opcode:04x} {row['status']}: {row['reason']}"]
    left = base["commands"][index] if index < len(base["commands"]) else None
    right = derived["commands"][index]
    if right["kind"] != "handler" or left is None or left["kind"] != "handler":
        return lines
    pairs, seen = {}, set()

    def visit(lkey, rkey, depth):
        pad = "  " * depth
        tag = f"{lkey} / {rkey}"
        if (lkey, rkey) in seen:
            lines.append(f"{pad}{tag}: see above")
            return
        seen.add((lkey, rkey))
        lnode, rnode = base["functions"][lkey], derived["functions"][rkey]
        lfp, liss = fingerprint(base, lkey, max_depth-depth)
        rfp, riss = fingerprint(derived, rkey, max_depth-depth)
        names = [n.get("library") for n in (lnode, rnode)]
        label = f" [{names[0]} / {names[1]}]" if any(names) else ""
        if lfp is not None and lfp == rfp:
            lines.append(f"{pad}{tag}{label}: same")
            return
        if lfp is not None and rfp is not None:
            lines.append(f"{pad}{tag}{label}: different")
        else:
            lines.append(f"{pad}{tag}{label}: incomplete base={','.join(liss) or '-'}"
                         f" derived={','.join(riss) or '-'}")
        for side, node in (("base", lnode), ("derived", rnode)):
            own = set(node["issues"]) - {i for ins in node["instructions"] for i in ins.get("issues", [])}
            if own:
                lines.append(f"{pad}  {side} function: {','.join(sorted(own))}")
            for offset in node.get("undecoded", []):
                lines.append(f"{pad}  {side} undecoded at +0x{offset:x}")
            for k, ins in enumerate(node["instructions"]):
                if ins.get("issues"):
                    lines.append(f"{pad}  {side} {where(ins, k)} {ins['bytes']}: {','.join(ins['issues'])}")
        if names[0] or names[1]:
            return
        if lnode.get("entry_index") != rnode.get("entry_index"):
            lines.append(f"{pad}  entry index {lnode.get('entry_index')} / {rnode.get('entry_index')}")
        aligned = 0
        for a, b in zip(lnode["instructions"], rnode["instructions"]):
            if not same_instruction(a, b):
                lines.append(f"{pad}  first difference: base {where(a, aligned)} {a['bytes']}"
                             f" / derived {where(b, aligned)} {b['bytes']}")
                break
            aligned += 1
        else:
            if len(lnode["instructions"]) != len(rnode["instructions"]):
                lines.append(f"{pad}  instruction count {len(lnode['instructions'])}"
                             f" / {len(rnode['instructions'])}")
        if depth >= max_depth:
            lines.append(f"{pad}  depth limit; callees not explained")
            return
        # Callees and data are paired only within the identical instruction prefix.
        for a, b in zip(lnode["instructions"][:aligned], rnode["instructions"][:aligned]):
            for ra, rb in zip(a["refs"], b["refs"]):
                if ra["kind"] == "unknown-data":
                    pairs.setdefault((ra.get("target"), rb.get("target")), ra.get("why"))
                elif ra["kind"] == "function":
                    visit(ra["target"], rb["target"], depth+1)

    visit(left["entry"], right["entry"], 0)
    for (ltarget, rtarget), why in sorted(pairs.items(), key=str):
        lines.append(f"data {ltarget} / {rtarget}: {why or 'unknown'}")
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("derived", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--explain", type=lambda text: int(text, 0), metavar="OPCODE",
                        help="print why one command did not inherit; writes nothing")
    parser.add_argument("--group-differences", nargs="*", type=lambda text: int(text, 0), metavar="OPCODE",
                        help="group differing function boundaries; no opcodes selects automatic unreviewed rows")
    parser.add_argument("--max-depth", type=int, default=8)
    parser.add_argument("--base-profile", type=Path)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--output-profile", type=Path)
    args = parser.parse_args()
    if not 0 <= args.max_depth <= 32:
        parser.error("max-depth must be 0..32")
    if args.group_differences is not None:
        if any((args.report, args.profile, args.base_profile, args.output_profile)) or args.explain is not None:
            parser.error("--group-differences only reads the two exports")
        base, derived = [json.loads(p.read_text(encoding="utf-8-sig")) for p in (args.base, args.derived)]
        groups = group_differences(base, derived, args.group_differences or None, args.max_depth)
        print(f"# {base['arch']} depth={args.max_depth} base={base['binary_sha256']} derived={derived['binary_sha256']}")
        print("# Diagnostic boundaries only; descendants below a differing body are not compared.")
        print("# No status changes; data/indirect/depth limits still require the ordinary report.")
        print("# Only paired handlers are shown; default selection uses automatic comparison, including manually reviewed commands.")
        for group in groups:
            print(f"{group['base']} / {group['derived']}\t" + " ".join(
                f"0x{op:04x}" for op in group["opcodes"]))
        return
    if args.explain is not None:
        if any((args.report, args.profile, args.base_profile, args.output_profile)):
            parser.error("--explain only reads the two exports")
        base, derived = [json.loads(p.read_text(encoding="utf-8-sig")) for p in (args.base, args.derived)]
        print("\n".join(explain(base, derived, args.explain, args.max_depth)))
        return
    if args.report is None:
        parser.error("--report is required unless --explain is given")
    if any((args.profile, args.base_profile, args.output_profile)) and not all((args.profile, args.base_profile, args.output_profile)):
        parser.error("all three profile options are required together")
    outputs = [p.resolve() for p in (args.report, args.output_profile) if p]
    inputs = {p.resolve() for p in (args.base, args.derived, args.base_profile, args.profile) if p}
    if len(set(outputs)) != len(outputs) or any(p in inputs for p in outputs):
        parser.error("output paths must differ from all inputs and each other")
    base, derived = [json.loads(p.read_text(encoding="utf-8-sig")) for p in (args.base, args.derived)]
    rows = compare(base, derived, args.max_depth)
    candidate = apply_profile(base, derived, rows, args.base_profile, args.profile) if args.profile else None
    header = ["# " + json.dumps({"schema": SCHEMA, "policy": POLICY, "max_depth": args.max_depth,
                               "base_export_sha256": hashlib.sha256(args.base.read_bytes()).hexdigest(),
                               "derived_export_sha256": hashlib.sha256(args.derived.read_bytes()).hexdigest(),
                               "base_binary_sha256": base["binary_sha256"],
                               "derived_binary_sha256": derived["binary_sha256"], "arch": base["arch"]}, sort_keys=True),
              "opcode\tstatus\tfingerprint\treason"]
    # newline="\n": repository data is LF-only, also when generated on Windows.
    args.report.write_text("\n".join(header + [f"0x{r['opcode']:04x}\t{r['status']}\t{r['fingerprint']}\t{r['reason']}" for r in rows])+"\n", encoding="utf-8", newline="\n")
    if candidate is not None:
        args.output_profile.write_text(candidate, encoding="utf-8", newline="\n")
    print(json.dumps({status: sum(r["status"] == status for r in rows)
                      for status in ("inherited", "presumed", "unreviewed", "noop", "absent")}, sort_keys=True))


if __name__ == "__main__":
    main()
