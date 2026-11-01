"""Re-extract reviewed opcodes and emit runtime certificate/profile candidates.

Original executables, exports and a matching offline report are mandatory. Nothing is
written to the registry: stdout is JSON containing reviewable catalog/profile text.
The caller must explicitly select the pilot opcodes; inherited/manual reviews stay intact.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("contract_extract", Path(__file__).with_name("contract_extract.py"))
ce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ce)


def certificate(base, derived, row, rules, exports, report_sha):
    """Canonical v1 line; Rust parses and reserializes this format independently."""
    deps = lambda name: ",".join(sorted(set(row["base"][name]) | set(row["derived"][name]))) or "-"
    fields = {
        "arch": "x64", "opcode": row["opcode"],
        "base-profile": base["profile"], "derived-profile": derived["profile"],
        "base-sha256": base["binary_sha256"], "derived-sha256": derived["binary_sha256"],
        "base-image": base["image_base"], "derived-image": derived["image_base"],
        "base-case": row["base"]["case_rva"], "derived-case": row["derived"]["case_rva"],
        "base-export": exports[0], "derived-export": exports[1],
        "extractor": rules["extractor_revision"], "context": rules["rules_sha256"],
        "mappings": rules["mapping_sha256"], "summaries": rules["summaries_sha256"],
        "base-contract": row["base_contract_sha256"], "derived-contract": row["derived_contract_sha256"],
        "report": report_sha, "mappings-used": deps("mappings"), "summaries-used": deps("summaries"),
        "model": "cmvs-command-contract-v1", "export-format": "cmvs-command-graph-v3",
        "result": "equivalent", "coverage": "closed", "binary": "checked",
    }
    if row["status"] != "equivalent" or fields["base-contract"] != fields["derived-contract"]:
        raise ValueError("certificate needs identical complete contracts")
    return "proof v1 " + " ".join(f"{key}={value}" for key, value in fields.items())


def register(args):
    if not args.opcodes or len(set(args.opcodes)) != len(args.opcodes):
        raise ValueError("select unique reviewed pilot opcodes explicitly")
    docs, hashes = {}, {}
    for key in ("context", "mappings", "summaries"):
        raw, docs[key] = ce.load_json(getattr(args, key), 8 * 1024 * 1024)
        hashes[key] = hashlib.sha256(raw).hexdigest()
    context, mappings, summaries = ce.check_context(docs["context"]), ce.check_mappings(docs["mappings"]), ce.check_summaries(docs["summaries"])
    rules = dict(extractor_revision=ce.revision(), rules_sha256=hashes["context"], mapping_sha256=hashes["mappings"], summaries_sha256=hashes["summaries"])
    loaded = []
    for path in (args.base_export, args.derived_export):
        raw, graph = ce.load_json(path)
        loaded.append((ce.Lifter(graph, context, mappings, summaries), graph, hashlib.sha256(raw).hexdigest()))
    left, right = (ce.pc.profile_lines(p) for p in (args.base_profile, args.derived_profile))
    for lines, (_, graph, _) in zip((left, right), loaded):
        ce.pc.bind_profile(lines, graph)
    if next(w[1] for w in left if w[0] == "base") != "-" or next(w[1] for w in right if w[0] == "base") != loaded[0][1]["profile"]:
        raise ValueError("certificate must compare directly with root semantics")
    images = tuple(ce.Image(Path(p).read_bytes()) for p in (args.base_exe, args.derived_exe))
    report_raw, report = ce.load_json(args.report)
    if report.get("status") != "report" or report.get("rules") != rules:
        raise ValueError("offline report rules are stale or absent")
    if any(row["status"] == "invalid-evidence" for row in report["rows"]):
        raise ValueError("resolve evidence errors before registration")
    report_sha = hashlib.sha256(report_raw).hexdigest()
    by_opcode = {int(row["opcode"], 0): row for row in report["rows"]}
    if len(by_opcode) != len(report["rows"]):
        raise ValueError("duplicate report opcode")
    if set(by_opcode) != {c["opcode"] for c in loaded[1][1]["commands"] if c["kind"] == "handler"}:
        raise ValueError("migration requires a report of every derived handler")
    replacements, proofs = {}, []
    root_rows = {int(w[0], 0): w for w in left if w[0].startswith("0x")}
    derived_rows = {int(w[0], 0): w for w in right if w[0].startswith("0x")}
    for op in args.opcodes:
        if op not in derived_rows or root_rows.get(op, [None, None])[1] != "base":
            raise ValueError("pilot command lacks root semantics")
        old = derived_rows[op]
        if any(word.startswith("research=") for word in old):
            raise ValueError("will not replace an explicit research implementation with root semantics")
        if old[1] not in ("presumed", "unreviewed", "contract-equivalent"):
            raise ValueError("will not replace inherited or manually reviewed evidence")
        row = ce.compare_pair(*loaded, rules, op, images)
        if row != by_opcode.get(op) or row["status"] != "equivalent":
            raise ValueError(f"pilot {hex(op)} does not reproduce the complete report acceptance")
        line = certificate(loaded[0][1], loaded[1][1], row, rules, [x[2] for x in loaded], report_sha)
        ident = hashlib.sha256(line.encode()).hexdigest()
        # Keep precisely the binary address columns, replacing only this row's source.
        columns = 2 + len([w for w in right if w[0] == "binary"])
        replacements[op] = " ".join([old[0], "contract-equivalent", *old[2:columns], "contract=" + ident])
        proofs.append(line)
    source = Path(args.derived_profile).read_text(encoding="utf-8")
    candidate = []
    migration = ["opcode\tprevious\tcandidate\tcomparison\treason"]
    for line in source.splitlines():
        words = line.split()
        if words and words[0].startswith("0x"):
            op = int(words[0], 0)
            proposed = replacements.get(op, line)
            row = by_opcode.get(op, {})
            # Complete profile inventory, including no-op and absent rows.
            migration.append("\t".join([words[0], words[1], proposed.split()[1], row.get("status", "not-handler"), row.get("reason", "switch-kind")]))
            line = proposed
        candidate.append(line)
    return {"scope": "review candidates; no registry write", "report_sha256": report_sha,
            "catalog": "\n".join(proofs) + "\n", "profile": "\n".join(candidate) + "\n",
            "migration": "\n".join(migration) + "\n"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("base_export")
    p.add_argument("derived_export")
    for name in ("context", "mappings", "summaries", "base-exe", "derived-exe", "base-profile", "derived-profile", "report"):
        p.add_argument("--" + name, required=True)
    p.add_argument("opcodes", nargs="*", type=lambda v: int(v, 0))
    try:
        print(json.dumps(register(p.parse_intermixed_args()), ensure_ascii=False, indent=1))
        return 0
    except (ValueError, KeyError, TypeError, IndexError, OSError, ce.cc.InvalidEvidence) as error:
        print(json.dumps({"status": "rejected", "reason": str(error)}, ensure_ascii=False))
        return 4


if __name__ == "__main__":
    raise SystemExit(main())
