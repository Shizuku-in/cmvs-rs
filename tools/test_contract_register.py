"""Synthetic certificate candidate generation, including original-PE re-extraction."""
import argparse
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


def sibling(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cr, fixtures = sibling("contract_register"), sibling("test_contract_extract")
ce = cr.ce


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
                                  report=paths["report.json"], opcodes=[0x2000])

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
            self.assertEqual(cr.register(args), result)

    def test_rejects_stale_forged_incomplete_report_and_changed_exe(self):
        for variant in ("rules", "acceptance", "contract", "missing-row", "exe", "reviewed", "research", "unselected"):
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
                if variant == "research":
                    p = Path(args.derived_profile)
                    p.write_text(p.read_text(encoding="utf-8").rstrip() + " research=font-cells:x64:" + "2"*64 + " evidence=games/x/vm-semantics.md\n", encoding="utf-8")
                if variant == "unselected": args.opcodes = []
                with self.assertRaises((ValueError, ce.cc.InvalidEvidence)):
                    cr.register(args)


if __name__ == "__main__":
    unittest.main()
