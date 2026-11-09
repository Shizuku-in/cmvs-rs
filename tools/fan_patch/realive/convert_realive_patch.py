"""Convert 天之圣杯汉化组 リアライブ fan patch to standard PS2A/CPZ7/PB3B.

see: https://vndb.org/r76601

Python 3.10+ and Cargo required.
See tools/fan_patch/realive/README.md.
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
import sys
import tempfile
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from mai_characters import character_map

LIMIT = 64 * 1024 * 1024
ROOT = "[天之圣杯汉化组]realive/"
PATCH_SHA256 = "13010b66a5510a094a582d3e148f4585fbd7c1d3abf336e610c58a5ec9d21d82"
BASE_HASHES = {
    "cmvs32.exe": "6705a43d85121b56961515f3dd0a0766a14a91ffa76df8866d8c7d6133a56a44",
    "cmvs64.exe": "02b71aac3d3d1ced1d0312b6c37748cf12df9cffd9585fb07787e6ca43199f00",
    "data/pack/start.ps3": "9a331e2cfa3b031687a6334065198db99ee86f1c70bbfbd45b9545865560a1d5",
    "data/pack/script.cpz": "ef496c6470359b45bdb3838be5c1d6dd4aca896dc976f17f7003a984e376cf24",
}
GBK_SUBSTITUTIONS = {
    "♪": "☆", "・": "·", "≪": "《", "≫": "》", "‧": "·", "∀": "▽",
    "´": "′", "｡": "。", "ﾉ": "ノ", "ﾞ": "゛", "ﾟ": "゜",
}


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def picture_names(names: list[str]) -> tuple[dict[str, str], list[str]]:
    """Restore omitted .pb3 extensions; exact DLL filenames take precedence."""
    result = {}
    ignored = []
    for name in sorted(names):
        if not name.endswith(".mai"):
            raise ValueError("Unexpected picture extension")
        stem = name[:-4]
        target = stem if stem.endswith(".pb3") else stem + ".pb3"
        if not stem.endswith(".pb3") and target + ".mai" in names:
            ignored.append(name)
        elif target in result:
            raise ValueError("Duplicate picture target")
        else:
            result[target] = name
    return result, ignored


def extract_patch(archive: zipfile.ZipFile, stage: Path) -> tuple[dict[str, str], list[str]]:
    members = {}
    for info in archive.infolist():
        if info.is_dir():
            continue
        if not info.filename.startswith(ROOT):
            raise ValueError("Unexpected patch root")
        name = info.filename[len(ROOT):]
        parts = name.split("/")
        if (name in members or not name.isascii() or any(p in ("", ".", "..") for p in parts)
                or "\\" in name or ":" in name or info.file_size > LIMIT):
            raise ValueError("Unsafe, oversized or duplicate patch member")
        members[name] = info
    # mai.dll 10004260 -> 100041B0: Unicode -> custom SJIS -> Unicode.
    # Read only its fixed table; no DLL code is executed.
    dll = archive.read(members["mai.dll"])
    if len(dll) < 0x3E020 + 131072:
        raise ValueError("Truncated character table")
    sjis = struct.unpack_from("<65536H", dll, 0x3E020)
    mapping = character_map(archive.read(members["mai/word.mai"]), sjis)
    (stage / "characters.bin").write_bytes(struct.pack("<65536H",
        *(mapping.get(i, 0xFFFF) for i in range(65536))))
    scripts = {}
    pictures = {}
    for name, info in members.items():
        if not (name.startswith("mai/sc/") or name.startswith("mai/pic/")):
            continue
        parts = name.split("/")
        if len(parts) != 3:
            raise ValueError("Unexpected nested resource")
        category, basename = parts[1:]
        if category == "sc":
            if not basename.endswith(".ps3"):
                raise ValueError("Unexpected script extension")
            scripts[basename] = info
        else:
            pictures[basename] = info
    if len(scripts) != 71 or len(pictures) != 192:
        raise ValueError("Unexpected patch script/image coverage")
    aliases, ignored = picture_names(list(pictures))
    if len(aliases) != 188 or ignored != ["lt101.mai", "lt201.mai", "lt229.mai", "lt301.mai"]:
        raise ValueError("Unexpected picture aliases")
    (stage / "sc").mkdir()
    for name, info in scripts.items():
        (stage / "sc" / name).write_bytes(archive.read(info))
    (stage / "pic").mkdir()
    for target, source in aliases.items():
        (stage / "pic" / (target + ".mai")).write_bytes(archive.read(pictures[source]))
    return aliases, ignored


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
    with tempfile.TemporaryDirectory(prefix="cmvs-realive-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            aliases, ignored = extract_patch(archive, stage)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-realive"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        reports = {}
        for name in ("scripts", "images"):
            with (stage / f"{name}.tsv").open() as stream:
                reports[name] = list(csv.DictReader(stream, delimiter="\t"))
        if len(reports["scripts"]) != 81 or len(reports["images"]) != 188:
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "realive.ico"):
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
            game="realive", profile="cmvs-20190305", encoding=encoding,
            patch_sha256=PATCH_SHA256, original_sha256=BASE_HASHES,
            archive_format="CPZ7", image_format="PB3B type 1",
            text_substitutions=GBK_SUBSTITUTIONS if encoding == "gbk" else {},
            source_adaptations=["Recover single-byte CP932 kaomoji omitted by the DLL inverse table.",
                                "Restore omitted .pb3 picture extensions; prefer explicit .pb3.mai files.",
                                "Retain the pinned patch code and labels, including sn706/sn854 edits.",
                                "Retain CPZ7 directories, keys and encryption mode; keep PB3B dimensions and bit depth."],
            picture_sources=aliases, unused_pictures=ignored,
            output_sha256=outputs, **reports,
            limitations=["No patch exe/DLL, settings or saves installed.",
                         "Untranslated scripts retain their original Japanese text.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted Realive ({encoding}) to {output}")


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
