"""Re-extract reviewed opcodes and emit runtime certificate/profile candidates.

Original executables, exports and a matching offline report are mandatory. Nothing is
written to the registry: stdout is JSON containing reviewable catalog/profile/rules text.
The caller must explicitly select the pilot opcodes; inherited/manual reviews stay intact.
"""
import argparse
import ast
import hashlib
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("contract_extract", Path(__file__).with_name("contract_extract.py"))
ce = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ce)

RULE_KEYS = ("extractor", "context", "mappings", "summaries")
RULE_FIELDS = (
    ("extractor", "extractor_revision"),
    ("context", "rules_sha256"),
    ("mappings", "mapping_sha256"),
    ("summaries", "summaries_sha256"),
)


def _strip_docstrings(node):
    for child in ast.walk(node):
        if isinstance(child, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            if (child.body and isinstance(child.body[0], ast.Expr)
                    and isinstance(child.body[0].value, ast.Constant)
                    and type(child.body[0].value.value) is str):
                child.body = child.body[1:] or [ast.Pass()]
    return node


def _encode_ast(value):
    if isinstance(value, ast.Constant):
        fields = {"value": _encode_ast(value.value)}
        if value.kind is not None:
            fields["kind"] = value.kind
        return ["Constant", fields]
    if isinstance(value, ast.AST):
        fields = {}
        for name in value._fields:
            child = getattr(value, name)
            if child is None or child == []:
                continue
            fields[name] = _encode_ast(child)
        return [value.__class__.__name__, fields]
    if isinstance(value, list):
        return [_encode_ast(item) for item in value]
    if value is None or type(value) in (bool, int, str):
        return [type(value).__name__, value]
    if type(value) is float:
        return ["float", value.hex()]
    if type(value) is complex:
        return ["complex", value.real.hex(), value.imag.hex()]
    if type(value) is bytes:
        return ["bytes", value.hex()]
    if value is Ellipsis:
        return ["ellipsis"]
    raise TypeError(f"unsupported AST constant {type(value)!r}")


def extractor_ast_digest(sources_dir=None):
    """Comment-, docstring- and whitespace-insensitive SHA-256 over extractor ASTs."""
    root = Path(sources_dir) if sources_dir is not None else Path(__file__).parent
    h = hashlib.sha256()
    for name in ce.SOURCES:
        source = (root / name).read_text(encoding="utf-8")
        tree = _strip_docstrings(ast.parse(source, filename=name))
        payload = json.dumps(_encode_ast(tree), sort_keys=True, separators=(",", ":")).encode("utf-8")
        h.update(name.encode("utf-8") + b"\0" + payload + b"\0")
    return h.hexdigest()


def parse_contract_rules_with_meta(text):
    """Parse declarative `contract_rules.txt` (matching Rust `ContractRules::parse`) plus optional `# extractor-ast`."""
    entries = []
    extractor_ast = None
    for line_no, raw in enumerate(text.splitlines(), 1):
        words = raw.split()
        if not words:
            continue
        if words[0].startswith("#"):
            stripped = raw.strip()
            if stripped.startswith("# extractor-ast"):
                parts = stripped.split()
                if len(parts) != 3 or parts[1] != "extractor-ast" or not ce.cc.SHA.fullmatch(parts[2]):
                    raise ValueError(f"contract_rules.txt line {line_no}: malformed extractor-ast metadata")
                if extractor_ast is not None:
                    raise ValueError(f"contract_rules.txt line {line_no}: duplicate extractor-ast metadata")
                extractor_ast = parts[2]
            continue
        entries.append((line_no, words))
    if len(entries) > len(RULE_KEYS):
        raise ValueError(f"contract_rules.txt line {entries[len(RULE_KEYS)][0]}: unexpected trailing rule line")
    if len(entries) < len(RULE_KEYS):
        missing = RULE_KEYS[len(entries)]
        raise ValueError(f"contract_rules.txt: missing `{missing}`")
    parsed = {}
    for (line_no, words), expected_key in zip(entries, RULE_KEYS):
        if len(words) != 2 or words[0] != expected_key:
            raise ValueError(f"contract_rules.txt line {line_no}: expected `{expected_key} <sha256>`")
        if not ce.cc.SHA.fullmatch(words[1]):
            raise ValueError(f"contract_rules.txt line {line_no}: expected 64 lowercase hexadecimal digits")
        parsed[expected_key] = words[1]
    return parsed, extractor_ast


def format_contract_rules(rules, extractor_ast=None):
    normalized = {}
    for short_key, report_key in RULE_FIELDS:
        digest = rules.get(short_key, rules.get(report_key))
        if not isinstance(digest, str) or not ce.cc.SHA.fullmatch(digest):
            raise ValueError(f"missing or invalid `{short_key}` rule digest")
        normalized[short_key] = digest
    lines = []
    if extractor_ast is not None:
        if not isinstance(extractor_ast, str) or not ce.cc.SHA.fullmatch(extractor_ast):
            raise ValueError("invalid extractor_ast digest")
        lines.append(f"# extractor-ast {extractor_ast}")
    lines.extend(f"{key} {normalized[key]}" for key in RULE_KEYS)
    return "\n".join(lines) + "\n"


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
    raw_revision = ce.revision()
    ast_digest = extractor_ast_digest()
    report_rules = dict(extractor_revision=raw_revision, rules_sha256=hashes["context"],
                        mapping_sha256=hashes["mappings"], summaries_sha256=hashes["summaries"])
    cert_rules = dict(report_rules)
    contract_rules_path = getattr(args, "contract_rules", None)
    if contract_rules_path is not None:
        pinned, pinned_ast = parse_contract_rules_with_meta(Path(contract_rules_path).read_text(encoding="utf-8"))
        if (pinned["context"], pinned["mappings"], pinned["summaries"]) != (hashes["context"], hashes["mappings"], hashes["summaries"]):
            raise ValueError("pinned contract_rules.txt digests differ from supplied contract tables")
        if pinned["extractor"] != raw_revision and (pinned_ast is None or pinned_ast != ast_digest):
            raise ValueError("extractor AST differs from pinned contract_rules.txt")
        cert_rules["extractor_revision"] = pinned["extractor"]
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
    if report.get("status") != "report" or report.get("rules") != report_rules:
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
        if any(word.startswith(("variant=", "research=")) for word in old):
            raise ValueError("will not replace an explicit variant implementation with root semantics")
        if old[1] not in ("presumed", "unreviewed", "contract-equivalent"):
            raise ValueError("will not replace inherited or manually reviewed evidence")
        row = ce.compare_pair(*loaded, report_rules, op, images)
        if row != by_opcode.get(op) or row["status"] != "equivalent":
            raise ValueError(f"pilot {hex(op)} does not reproduce the complete report acceptance")
        line = certificate(loaded[0][1], loaded[1][1], row, cert_rules, [x[2] for x in loaded], report_sha)
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
            "contract_rules": format_contract_rules(cert_rules, extractor_ast=ast_digest),
            "catalog": "\n".join(proofs) + "\n", "profile": "\n".join(candidate) + "\n",
            "migration": "\n".join(migration) + "\n"}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("base_export")
    p.add_argument("derived_export")
    for name in ("context", "mappings", "summaries", "base-exe", "derived-exe", "base-profile", "derived-profile", "report"):
        p.add_argument("--" + name, required=True)
    p.add_argument("--contract-rules", default=None,
                   help="optional pinned contract_rules.txt to verify and preserve across comment-only extractor edits")
    p.add_argument("opcodes", nargs="*", type=lambda v: int(v, 0))
    try:
        print(json.dumps(register(p.parse_intermixed_args()), ensure_ascii=False, indent=1))
        return 0
    except (ValueError, KeyError, TypeError, IndexError, OSError, ce.cc.InvalidEvidence) as error:
        print(json.dumps({"status": "rejected", "reason": str(error)}, ensure_ascii=False))
        return 4


if __name__ == "__main__":
    raise SystemExit(main())
