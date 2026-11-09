"""Convert Dir-A クナド国記 fan patch to standard PS2A/CPZ7/PB3B.

see: https://vndb.org/r106394

Python 3.10+ and Cargo required.
See tools/fan_patch/kunado/README.md.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zipfile

LIMIT = 64 * 1024 * 1024
ROOT = "Release1.2/"
PATCH_SHA256 = 'e81a4e9deb1c903625c715eeee80629a2c04802968947f959f4b6688e134a014'
BASE_HASHES = {
    "cmvs32.exe": "4b42514abba732bd3f4bf71cbc8a7862831074fd8905a7b128fc195b30279999",
    "cmvs64.exe": "58e993a82e741ea73d3d1b5d89363dcaf435e72e6d505324e1de0c0ba5486631",
    "data/pack/start.ps3": "08e371cc3af967a12e4a45d7063d0644a8fc4d589a08348b2d9a4b919efda4ad",
    "data/pack/script.cpz": "d3e2309180801dcff74abf2597e2d07b21c29c1b7afd0ab51b5c9a13fbd81ea9",
}
GBK_SUBSTITUTIONS = {"♪": "☆", "・": "·"}


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def extract_patch(archive: zipfile.ZipFile, stage: Path) -> tuple[dict[str, str], list[str]]:
    members = {}
    for info in archive.infolist():
        if info.is_dir():
            continue
        if not info.filename.startswith(ROOT):
            raise ValueError("Unexpected patch root")
        name = info.filename[len(ROOT):]
        if (name in members or not name.isascii() or any(p in ("", ".", "..") for p in name.split("/"))
                or "\\" in name or ":" in name or info.file_size > LIMIT):
            raise ValueError("Unsafe, oversized or duplicate patch member")
        members[name] = info
    for category in ("sc", "pic"):
        (stage / category).mkdir()
    pictures = {}
    scripts = 0
    for name, info in members.items():
        if not name.startswith("FileHook/"):
            continue
        basename = name[len("FileHook/"):]
        if "/" in basename or not basename.endswith((".ps3", ".pb3")):
            raise ValueError("Unexpected patch resource")
        if basename == "ed_bg.pb3":
            continue  # Unreferenced by every pinned patch script; absent from the original.
        category = "sc" if basename.endswith(".ps3") else "pic"
        (stage / category / basename).write_bytes(archive.read(info))
        if category == "sc":
            scripts += 1
        else:
            pictures[basename] = name
    if scripts != 58 or len(pictures) != 48:
        raise ValueError(f"Unexpected patch coverage: {scripts} scripts, {len(pictures)} images")
    return pictures, ["ed_bg.pb3"]


def convert(game: Path, patch: Path, output: Path, encoding: str = "utf-8") -> None:
    if encoding not in ("utf-8", "utf8", "gbk"):
        raise ValueError("encoding must be utf-8, utf8 or gbk")
    encoding = "utf-8" if encoding == "utf8" else encoding
    game, patch, output = game.resolve(), patch.resolve(), output.resolve()
    if output.exists() or output.is_relative_to(game):
        raise ValueError("Output must be a new directory outside the game")
    for name, expected in BASE_HASHES.items():
        if sha256(game / name) != expected:
            raise ValueError(f"Unsupported original game: {name}")
    if sha256(patch) != PATCH_SHA256:
        raise ValueError("Unsupported patch ZIP (SHA-256 mismatch)")
    tool = Path(__file__).resolve().parent
    repository = tool.parents[2]
    with tempfile.TemporaryDirectory(prefix="cmvs-kunado-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            aliases, ignored = extract_patch(archive, stage)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-kunado"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        reports = {}
        for name in ("scripts", "images"):
            with (stage / f"{name}.tsv").open() as stream:
                reports[name] = list(csv.DictReader(stream, delimiter="\t"))
        if len(reports["scripts"]) != 60 or len(reports["images"]) != 48:
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "kunado.ico"):
            if (game / name).is_file():
                shutil.copy2(game / name, output / name)
        if (game / "save/system.dat").is_file():
            (output / "save").mkdir()
            shutil.copy2(game / "save/system.dat", output / "save/system.dat")
        outputs = {}
        for asset in (stage / "pack").iterdir():
            destination = output / "data/pack" / asset.name
            shutil.copy2(asset, destination)
            outputs[str(destination.relative_to(output))] = sha256(destination)
        report = dict(
            game="kunado", profile="cmvs-20211006", encoding=encoding,
            patch_sha256=PATCH_SHA256, original_sha256=BASE_HASHES,
            archive_format="CPZ7", image_format="PB3B type 1",
            text_substitutions=GBK_SUBSTITUTIONS if encoding == "gbk" else {},
            source_adaptations=["Decode retained Japanese prefix as CP932 and appended translations as GBK.",
                                "Retain the sn1260 Japanese typo correction and patch code/labels.",
                                "Keep original startup (patch mounts absent update.cpz); translate title.",
                                "Convert PNG images to PB3B type 1; preserve CPZ7 keys."],
            picture_sources=aliases, unused_pictures=ignored,
            output_sha256=outputs, **reports,
            limitations=["No patch exe/DLL, settings or saves installed.",
                         "Untranslated scripts retain their original Japanese text.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted Kunado ({encoding}) to {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("game", "patch", "output"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--encoding", choices=("utf-8", "utf8", "gbk"), default="utf-8")
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output, args.encoding)
    except (OSError, ValueError, KeyError, struct.error, zipfile.BadZipFile, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
