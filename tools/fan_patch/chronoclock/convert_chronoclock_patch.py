"""Convert 葵海计划 クロノクロック 1.0/1.05 fan patches to standard PS2A/CPZ3/PB3B.

see: https://vndb.org/r114391

Python 3.10+ and Cargo required.
See tools/fan_patch/chronoclock/README.md.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tempfile
import zipfile

LIMIT = 64 * 1024 * 1024
PATCH_SHA256 = "24d2872ae77177234b72adcd8b15494cea2dcc8f0c41c4c4eb6467f028ed7550"
BASE_HASHES = {
    "cmvs32.exe": "30edecfde8d69b60102c79c82db7eda3895126510e51c0ec76658a303f5a49b4",
    "cmvs64.exe": "d1c4ade9551b827b8e401fc10f68a73e2525a67a217b63747898f5797894911d",
    "data/pack/start.ps3": "d4263a32830bb385fabfa7e06c2fde2d70e072722e3ed6c317292e7086fb6dce",
    "data/pack/script.cpz": "aa4bd891cc50fe658cacc050c6b216e8ba0389d17d01163e335f3177d6f14ba3",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def extract_patch(archive: zipfile.ZipFile, stage: Path, version: str) -> None:
    if version not in ("1.0", "1.05"):
        raise ValueError("Unsupported patch version")
    marker = f"/柯罗的怀表{version}版补丁/"
    members = {}
    for info in archive.infolist():
        if info.is_dir() or marker not in info.filename:
            continue
        name = info.filename.split(marker, 1)[1]
        parts = PurePosixPath(name).parts
        if (not parts or any(p in (".", "..") for p in parts)
                or name.startswith("/") or "\\" in name or name in members
                or info.file_size > LIMIT):
            raise ValueError("Unsafe or duplicate patch member")
        members[name] = info
    selected = {}
    for name, info in members.items():
        parts = PurePosixPath(name).parts
        if len(parts) == 2 and parts[0] == "Project":
            if not (parts[1] == "trans.txt" or parts[1].endswith(".pb3.bmp")):
                raise ValueError("Unexpected Project asset")
            if not parts[1].isascii() or ":" in parts[1]:
                raise ValueError("Unsafe asset basename")
            selected[name] = info
    if ("Project/trans.txt" not in selected or len(selected) != 93
            or "补丁使用说明.txt" not in members):
        raise ValueError("Unexpected patch text/image coverage")
    for name, info in selected.items():
        destination = stage / name
        destination.parent.mkdir(exist_ok=True)
        destination.write_bytes(archive.read(info))
    (stage / "汉化补丁说明.txt").write_bytes(archive.read(members["补丁使用说明.txt"]))


def convert(game: Path, patch: Path, output: Path, version: str = "1.0", encoding: str = "utf-8") -> None:
    if encoding not in ("utf-8", "utf8", "gbk"):
        raise ValueError("encoding must be utf-8, utf8 or gbk")
    encoding = "utf-8" if encoding == "utf8" else encoding
    game, patch, output = game.resolve(), patch.resolve(), output.resolve()
    if output.exists() or output.is_relative_to(game):
        raise ValueError("Output must be a new directory outside the game")
    if version not in ("1.0", "1.05"):
        raise ValueError("Unsupported patch version")
    for name, expected in BASE_HASHES.items():
        if sha256(game / name) != expected:
            raise ValueError(f"Unsupported original game: {name}")
    if sha256(patch) != PATCH_SHA256:
        raise ValueError("Unsupported patch ZIP (SHA-256 mismatch)")
    tool = Path(__file__).resolve().parent
    repository = tool.parents[2]
    with tempfile.TemporaryDirectory(prefix="cmvs-chronoclock-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            extract_patch(archive, stage, version)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-chronoclock"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        with (stage / "scripts.tsv").open() as stream:
            scripts = list(csv.DictReader(stream, delimiter="\t"))
        with (stage / "images.tsv").open() as stream:
            images = list(csv.DictReader(stream, delimiter="\t"))
        if (len(scripts) != 86 or len(images) != 92
                or sum(int(s["translated_strings"]) for s in scripts) != 52362):
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "chronoclock.ico"):
            if (game / name).is_file():
                shutil.copy2(game / name, output / name)
        if (game / "save/system.dat").is_file():
            (output / "save").mkdir()
            shutil.copy2(game / "save/system.dat", output / "save/system.dat")
        shutil.copy2(stage / "汉化补丁说明.txt", output / "汉化补丁说明.txt")
        outputs = {}
        for asset in (stage / "pack").iterdir():
            destination = output / "data/pack" / asset.name
            shutil.copy2(asset, destination)
            outputs[str(destination.relative_to(output))] = sha256(destination)
        report = dict(
            game="chronoclock", profile="cmvs-20150405", encoding=encoding, patch_version=version,
            patch_sha256=PATCH_SHA256, original_sha256=BASE_HASHES,
            text_substitutions={"♪": "☆", "・": "·", "䇲": "筴"} if encoding == "gbk" else {},
            source_adaptations=["snky21.ps3 text+0xc41a: match the installed source punctuation."],
            output_sha256=outputs, scripts=scripts, images=images,
            limitations=["Patch 1.0 and 1.05 share translations; dialog_chip.pb3 differs.",
                         "Patch executable/DLL behavior is not installed.",
                         "Full routes, native audio and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted patch {version}: 86 scripts / 92 images to {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("game", "patch", "output"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--patch-version", choices=("1.0", "1.05"), default="1.0")
    parser.add_argument("--encoding", choices=("utf-8", "utf8", "gbk"), default="utf-8", help="Output text encoding (default: utf-8)")
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output, args.patch_version, encoding=args.encoding)
    except (OSError, ValueError, KeyError, zipfile.BadZipFile, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
