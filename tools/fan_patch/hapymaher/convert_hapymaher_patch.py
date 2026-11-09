"""Convert 里想奈骑士团 ハピメア fan patch to standard PS2A/CPZ5/PB3B.

see: https://vndb.org/r56370

Python 3.10+ and Cargo required.
See tools/fan_patch/hapymaher/README.md.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from mai_characters import character_map

LIMIT = 64 * 1024 * 1024
PATCH_SHA256 = "149cbd277d71d4ecb9012e9cbbbabf8cd6de9bd91114921bbacc255498216d52"
BASE_HASHES = {
    "cmvs32.exe": "8048c7615b019d3b2ef867ad2bcc108f27092c7712250794d27d33e6cd7f2930",
    "cmvs64.exe": "637d2928efab89e7b5fc68b2d0801cfdefbfa65558b69378580c145c52b8c05b",
    "data/pack/start.ps3": "62fc84b6231432a0aed498065329b630f76ef4c9e114c1bf6dbdc4e564c0399e",
    "data/pack/script.cpz": "ae25d8e3bd2a2f7539c81dafe0b6cba445c5c346c6c26f7702337e10a86ed103",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


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
    with tempfile.TemporaryDirectory(prefix="cmvs-hapymaher-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            members = {}
            for info in archive.infolist():
                if info.is_dir() or "/hapymaher_CN/" not in info.filename:
                    continue
                name = info.filename.split("/hapymaher_CN/", 1)[1]
                parts = PurePosixPath(name).parts
                if (not parts or any(p in (".", "..") for p in parts)
                        or name.startswith("/") or "\\" in name or name in members
                        or info.file_size > LIMIT):
                    raise ValueError("Unsafe or duplicate patch member")
                members[name] = info
            dll = archive.read(members["mai.dll"])
            # mai.dll 1012AA80 -> 1012A9D0; no DLL code is executed.
            sjis = struct.unpack_from("<65536H", dll, 0x1CEBE0)
            mapping = character_map(archive.read(members["mai/word.mai"]), sjis)
            (stage / "characters.bin").write_bytes(struct.pack("<65536H",
                *(mapping.get(i, 0xFFFF) for i in range(65536))))
            counts = [0, 0]
            for name, info in members.items():
                if not (name.startswith("mai/sc/") or name.startswith("mai/pic/")):
                    continue
                category, basename = name.split("/")[1:]
                raw = archive.read(info)
                destination = stage / category / basename
                destination.parent.mkdir(exist_ok=True)
                destination.write_bytes(raw)
                if category == "sc":
                    counts[0] += 1
                else:
                    counts[1] += 1
            if counts != [151, 111]:
                raise ValueError("Unexpected patch script/image coverage")
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-hapymaher"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        with (stage / "scripts.tsv").open() as stream:
            scripts = list(csv.DictReader(stream, delimiter="\t"))
        with (stage / "images.tsv").open() as stream:
            images = list(csv.DictReader(stream, delimiter="\t"))
        if len(scripts) != 159 or len(images) != 111:
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "hapymaher.ico"):
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
            game="hapymaher", profile="cmvs-20130131", encoding=encoding,
            archive_format="CPZ5", packing_mode=9,
            patch_sha256=PATCH_SHA256, original_sha256=BASE_HASHES,
            text_substitutions={"♪": "☆", "・": "·"} if encoding == "gbk" else {},
            output_sha256=outputs, scripts=scripts, images=images,
            limitations=["Source translation is the 2017-07-11 test patch.",
                         "No patch exe/DLL, settings or saves installed.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted 159 scripts / 111 images to {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("game", "patch", "output"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--encoding", choices=("utf-8", "utf8", "gbk"), default="utf-8", help="Output text encoding (default: utf-8)")
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output, encoding=args.encoding)
    except (OSError, ValueError, KeyError, struct.error, zipfile.BadZipFile, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
