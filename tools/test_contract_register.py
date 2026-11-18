"""Synthetic certificate candidate generation and pinned contract_rules.txt consistency checks."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


def sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cr, fixtures = sibling("contract_register"), sibling("test_contract_extract")
ce = cr.ce

PROFILES_DIR = cr.REPO_ROOT / "research/profiles"
CHRONOCLOCK_DOCS = cr.REPO_ROOT / "docs/games/chronoclock"


class RegistrationTests(unittest.TestCase):
    def fixture(self, directory):
        paths = {}

        def write(name, value):
            path = Path(directory, name)
            path.write_bytes(value if isinstance(value, bytes) else json.dumps(value).encode())
            paths[name] = str(path)
            return path

        graphs = []
        context = {"schema": ce.CONTEXT, "revision": "synthetic", "sides": []}
        loaded, images = [], []
        for name, seed in (("base", 1), ("derived", 2)):
            raw = bytearray(fixtures.BinaryTests().build(bytes.fromhex("b8004000004883c4205bc3")))
            raw[2] = seed  # distinct file identity without modifying executable instructions
            image = ce.Image(bytes(raw))
            g = fixtures.graph(image.sha256, fixtures.function(fixtures.ret(0x4000) + fixtures.EPILOGUE))
            g["profile"] = name
            g["functions"]["0x1800"] = g["functions"].pop("0x100")
            g["commands"][0]["entry"] = "0x1800"
            write(name + ".exe", bytes(raw))
            graph_path = write(name + ".json", g)
            graphs.append(g)
            context["sides"].append(dict(binary_sha256=image.sha256, arch="x64", image_base="0x140000000",
                                        table_rva="0x2000", vm_stack={"sp_offset": "0x14270", "base_offset": "0x14268"}, evidence=["synthetic"]))
            profile = (f"profile {name}\nfamily cmvs\nbase {'-' if name == 'base' else 'base'}\n"
                       f"binary x64 file=cmvs64.exe sha256={image.sha256} timestamp=0x0 linker=10.0 image-base=0x140000000 table=0x2000 default=0x1900 cases=1\n"
                       f"0x2000 {'base' if name == 'base' else 'unreviewed'} 0x1800\n")
            write(name + ".profile", profile.encode())
            images.append(image)
            loaded.append((g, hashlib.sha256(graph_path.read_bytes()).hexdigest()))
        write("context.json", context)
        write("mappings.json", {"schema": ce.MAPPINGS, "revision": "synthetic", "entries": []})
        write("summaries.json", {"schema": ce.SUMMARIES, "revision": "synthetic", "entries": []})
        rules = dict(extractor_revision=ce.revision(), **{field: hashlib.sha256(Path(paths[name + ".json"]).read_bytes()).hexdigest()
                     for name, field in (("context", "rules_sha256"), ("mappings", "mapping_sha256"), ("summaries", "summaries_sha256"))})
        lifts = [(ce.Lifter(g, ce.check_context(context), {}, {}), g, digest) for g, digest in loaded]
        row = ce.compare_pair(*lifts, rules, 0x2000, images)
        self.assertEqual(row["status"], "equivalent")
        write("report.json", dict(status="report", rules=rules, rows=[row]))
        return argparse.Namespace(base_export=paths["base.json"], derived_export=paths["derived.json"],
                                  base_exe=paths["base.exe"], derived_exe=paths["derived.exe"],
                                  base_profile=paths["base.profile"], derived_profile=paths["derived.profile"],
                                  context=paths["context.json"], mappings=paths["mappings.json"], summaries=paths["summaries.json"],
                                  report=paths["report.json"], contract_rules=None, opcodes=[0x2000])

    def test_reextracts_binds_and_emits_only_candidates(self):
        with tempfile.TemporaryDirectory() as directory:
            args = self.fixture(directory)
            before = {p: p.read_bytes() for p in Path(directory).iterdir()}
            result = cr.register(args)
            self.assertEqual({p: p.read_bytes() for p in before}, before)
            line = result["catalog"].strip()
            ident = hashlib.sha256(line.encode()).hexdigest()
            self.assertIn(f"contract={ident}", result["profile"])
            self.assertIn("0x2000\tunreviewed\tcontract-equivalent\tequivalent", result["migration"])
            self.assertIn("binary=checked", line)
            parsed_rules, parsed_ast = cr.parse_contract_rules_with_meta(result["contract_rules"])
            self.assertEqual(parsed_ast, cr.extractor_ast_digest())
            self.assertEqual(cr.parse_catalog(result["catalog"], active_rules=parsed_rules)[ident]["opcode"], "0x2000")
            self.assertEqual(cr.register(args), result)

    def test_rejects_stale_forged_incomplete_report_and_changed_exe(self):
        for variant in ("rules", "acceptance", "contract", "missing-row", "exe", "reviewed", "variant", "unselected"):
            with self.subTest(variant=variant), tempfile.TemporaryDirectory() as directory:
                args = self.fixture(directory)
                report = json.loads(Path(args.report).read_text(encoding="utf-8"))
                if variant == "rules": report["rules"]["extractor_revision"] = "f" * 64
                if variant == "acceptance": report["rows"][0]["status"] = "unknown"
                if variant == "contract": report["rows"][0]["base_contract_sha256"] = "f" * 64
                if variant == "missing-row": report["rows"] = []
                Path(args.report).write_text(json.dumps(report), encoding="utf-8")
                if variant == "exe":
                    raw = bytearray(Path(args.derived_exe).read_bytes()); raw[3] ^= 1
                    Path(args.derived_exe).write_bytes(raw)
                if variant == "reviewed":
                    p = Path(args.derived_profile)
                    p.write_text(p.read_text(encoding="utf-8").replace("unreviewed", "reviewed"), encoding="utf-8")
                if variant == "variant":
                    p = Path(args.derived_profile)
                    p.write_text(p.read_text(encoding="utf-8").rstrip() + " variant=font-cells:x64:" + "2"*64 + " evidence=games/x/vm-semantics.md\n", encoding="utf-8")
                if variant == "unselected": args.opcodes = []
                with self.assertRaises((ValueError, ce.cc.InvalidEvidence)):
                    cr.register(args)

    def test_repo_contract_rules_match_catalog_and_chronoclock_tables(self):
        rules_text = cr.DEFAULT_CONTRACT_RULES.read_text(encoding="utf-8")
        pinned, pinned_ast = cr.parse_contract_rules_with_meta(rules_text)
        self.assertEqual(pinned_ast, cr.extractor_ast_digest())

        catalog_text = (PROFILES_DIR / "contracts.txt").read_text(encoding="utf-8")
        proofs = cr.parse_catalog(catalog_text, active_rules=pinned)
        self.assertGreaterEqual(len(proofs), 3)
        for fields in proofs.values():
            derived_profile = (PROFILES_DIR / f"{fields['derived-profile']}.profile").read_text(encoding="utf-8")
            line = "proof v1 " + " ".join(f"{k}={fields[k]}" for k in cr.PROOF_KEYS)
            ident = hashlib.sha256(line.encode("utf-8")).hexdigest()
            self.assertIn(f"contract={ident}", derived_profile)

        for key, checker in (
            ("context", ce.check_context),
            ("mappings", ce.check_mappings),
            ("summaries", ce.check_summaries),
        ):
            raw, doc = ce.load_json(CHRONOCLOCK_DOCS / f"contract-{key}.json", 8 * 1024 * 1024)
            checker(doc)
            self.assertEqual(pinned[key], hashlib.sha256(raw).hexdigest())

    def test_extractor_ast_digest_ignores_comments_whitespace_and_detects_logic_edits(self):
        tools_dir = Path(__file__).parent
        baseline_ast = cr.extractor_ast_digest(tools_dir)
        with tempfile.TemporaryDirectory() as directory:
            temp_tools = Path(directory)
            for name in ce.SOURCES:
                original = (tools_dir / name).read_text(encoding="utf-8")
                decorated = f"# Extra header comment in {name}\n\n{original}\n    # Trailing comment and blank lines\n\n"
                (temp_tools / name).write_text(decorated, encoding="utf-8")
            self.assertEqual(cr.extractor_ast_digest(temp_tools), baseline_ast)

            # Mutating an executable constant or expression in any extractor file changes the AST digest.
            target = temp_tools / "contract_extract.py"
            mutated = target.read_text(encoding="utf-8").replace("FRAME_RET = 0x28", "FRAME_RET = 0x30", 1)
            self.assertNotEqual(mutated, target.read_text(encoding="utf-8"))
            target.write_text(mutated, encoding="utf-8")
            self.assertNotEqual(cr.extractor_ast_digest(temp_tools), baseline_ast)

    def test_pinned_contract_rules_preserve_extractor_across_comment_edits_and_reject_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            args = self.fixture(directory)
            pinned_sha = "7" * 64
            rules_path = Path(directory, "contract_rules.txt")
            rules_path.write_text(
                cr.format_contract_rules(
                    {
                        "extractor": pinned_sha,
                        "context": hashlib.sha256(Path(args.context).read_bytes()).hexdigest(),
                        "mappings": hashlib.sha256(Path(args.mappings).read_bytes()).hexdigest(),
                        "summaries": hashlib.sha256(Path(args.summaries).read_bytes()).hexdigest(),
                    },
                    extractor_ast=cr.extractor_ast_digest(),
                ),
                encoding="utf-8",
            )
            args.contract_rules = str(rules_path)
            result = cr.register(args)
            self.assertIn(f"extractor={pinned_sha}", result["catalog"])

            # Drifted AST digest with mismatched raw revision is rejected.
            rules_path.write_text(
                rules_path.read_text(encoding="utf-8").replace(cr.extractor_ast_digest(), "0" * 64),
                encoding="utf-8",
            )
            with mock.patch.object(ce, "revision", return_value="9" * 64):
                report = json.loads(Path(args.report).read_text(encoding="utf-8"))
                report["rules"]["extractor_revision"] = "9" * 64
                Path(args.report).write_text(json.dumps(report), encoding="utf-8")
                with self.assertRaises(ValueError):
                    cr.register(args)

    def test_parse_contract_rules_rejects_malformed_inputs(self):
        valid = cr.format_contract_rules({k: str(i) * 64 for i, k in enumerate(cr.RULE_KEYS, 1)}, extractor_ast="a" * 64)
        self.assertEqual(cr.parse_contract_rules(valid)["extractor"], "1" * 64)
        for bad in (
            "",
            valid.replace("extractor ", "wrong "),
            valid.replace("1" * 64, "Z" * 64),
            valid + "extra " + "b" * 64 + "\n",
            valid.replace("# extractor-ast " + "a" * 64, "# extractor-ast bad"),
            "# extractor-ast " + "a" * 64 + "\n" + valid,
        ):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                cr.parse_contract_rules(bad)


if __name__ == "__main__":
    unittest.main()
