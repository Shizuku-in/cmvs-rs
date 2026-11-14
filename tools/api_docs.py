"""Generate public Rust API documentation as HTML and rustdoc JSON."""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def run(command, **kwargs):
    return subprocess.run(command, cwd=ROOT, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-p", "--package", action="append",
                        help="package to document (repeatable; default: cmvs-core, cmvs-rs)")
    parser.add_argument("--toolchain", default="nightly-2026-01-20")
    parser.add_argument("--features", help="Cargo features, including package/feature syntax")
    parser.add_argument("--all-features", action="store_true")
    parser.add_argument("--no-default-features", action="store_true")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    packages = list(dict.fromkeys(args.package or ["cmvs-core", "cmvs-rs"]))
    cargo = ["cargo", f"+{args.toolchain}"]
    metadata = json.loads(run(cargo + ["metadata", "--no-deps", "--format-version", "1"],
                              capture_output=True, text=True).stdout)
    libraries = {}
    for package in metadata["packages"]:
        if package["id"] in metadata["workspace_members"]:
            for target in package["targets"]:
                if any(kind in target["kind"] for kind in ("lib", "rlib", "cdylib", "staticlib")):
                    libraries[package["name"]] = target["name"]
    for package in packages:
        if package not in libraries:
            parser.error(f"no workspace library: {package}")

    output = ROOT / "target" / "api"
    options = ["--locked", "--lib"]
    if args.features:
        options += ["--features", args.features]
    for flag in ("all_features", "no_default_features", "offline"):
        if getattr(args, flag):
            options.append("--" + flag.replace("_", "-"))
    env = os.environ.copy()
    # Keep local source paths out of shared artifacts, including rustdoc spans.
    remap = shlex.join(["-Z", "unstable-options", f"--remap-path-prefix={Path.home()}=/user",
                       f"--remap-path-prefix={ROOT}=."])
    env["RUSTDOCFLAGS"] = (env.get("RUSTDOCFLAGS", "") + " " + remap).strip()
    selection = [item for package in packages for item in ("-p", package)]
    run(cargo + ["doc", "--no-deps", "--target-dir", str(output / "html")]
        + options + selection, env=env)
    entries = []
    for package in packages:
        run(cargo + ["rustdoc", "--target-dir", str(output / "json")]
            + options + ["-p", package, "--",
            "-Z", "unstable-options", "--output-format", "json"], env=env)
        name = libraries[package]
        artifact = output / "json" / "doc" / f"{name}.json"
        data = json.loads(artifact.read_text())
        # rustdoc does not remap external-crate artifact paths.
        for external in data["external_crates"].values():
            if external.get("path"):
                external["path"] = Path(external["path"]).name
        artifact.write_text(json.dumps(data, separators=(",", ":")) + "\n")
        entries.append({"package": package, "crate": name,
                        "html": f"html/doc/{name}/index.html", "json": f"json/doc/{name}.json",
                        "format_version": data["format_version"]})
    manifest = {
        "schema": "cmvs-api-docs-v1",
        "rustc": run(["rustc", f"+{args.toolchain}", "-vV"],
                     capture_output=True, text=True).stdout.strip(),
        "features": args.features,
        "all_features": args.all_features,
        "default_features": not args.no_default_features,
        "packages": entries,
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("API documentation: target/api/manifest.json")


if __name__ == "__main__":
    main()
