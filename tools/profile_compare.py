"""Compare same-architecture IDA exports using the Python standard library only.

python -I tools/profile_compare.py BASE.json DERIVED.json --report REPORT.tsv
    [--base-profile BASE.profile --profile DERIVED.profile --output-profile OUT.profile]
No input is modified. Incomplete, cyclic, indirect or mutable dependencies never inherit.
FLIRT-recognized runtime functions are leaves identified by name and their own body.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

SCHEMA = "cmvs-command-graph-v2"
POLICY = "raw-operands-closed-cfg-readonly-data-libraries-v2"
HEX = re.compile(r"[0-9a-f]{64}\Z")
PLACEHOLDER_NAME = re.compile(r"(?:sub|nullsub|j|unknown_libname)_[0-9A-Fa-f]+\Z")


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
                if (kind == "local" and not 0 <= ref["target"] < len(node["instructions"])
                        and not (ref["target"] == -1 and "missing-local-target" in node["issues"])):
                    raise ValueError("invalid local branch target")
                if kind == "data" and (not re.fullmatch(r"(?:[0-9a-f]{2})+", ref["bytes"])
                                       or not 0 <= ref["offset"] < len(ref["bytes"])//2):
                    raise ValueError("invalid data dependency")
                if kind == "import" and not ref.get("name"):
                    raise ValueError("unnamed import")
                if kind == "opaque" and library is None:
                    raise ValueError("opaque references belong only to library functions")
                if library is not None and kind not in ("local", "import", "opaque"):
                    raise ValueError("library functions keep only local, import or opaque references")
                if kind not in ("function", "local", "data", "import", "unknown-data", "opaque"):
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
                normalized.append(instruction)
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
    output, inherits = [], False
    for words in right:
        if words[0] == "inherit":
            continue
        if words[0].startswith("0x") and words[1] == "reviewed":
            # Manual review records outrank a fresh comparison and are kept verbatim.
            if rows[int(words[0], 0)-0x2000]["status"] in ("noop", "absent"):
                raise ValueError("reviewed command is not a handler in the export")
        elif words[0].startswith("0x"):
            result = rows[int(words[0], 0)-0x2000]
            if words[1] not in ("unreviewed", "inherited", "noop", "absent"):
                raise ValueError("will not overwrite variant or unknown status")
            words = words[:2+len(binaries)]
            words[1] = result["status"]
            if result["status"] == "inherited":
                words.append(f"inherit={base['arch']}:{result['fingerprint']}")
                inherits = True
        output.append(" ".join(words))
        if words is binaries[-1]:
            header = len(output)
    if inherits:
        # The derived exe is the profile's own binary of this arch; only the root exe is named.
        output.insert(header, f"inherit {base['arch']} base-sha256={base['binary_sha256']}")
    return "\n".join(output)+"\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("derived", type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--max-depth", type=int, default=8)
    parser.add_argument("--base-profile", type=Path)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--output-profile", type=Path)
    args = parser.parse_args()
    if not 0 <= args.max_depth <= 32:
        parser.error("max-depth must be 0..32")
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
                      for status in ("inherited", "unreviewed", "noop", "absent")}, sort_keys=True))


if __name__ == "__main__":
    main()
