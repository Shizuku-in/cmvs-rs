"""Convert 天之圣杯汉化组 アオイトリ Package fan patch to standard PS2A/CPZ7/PB3B.

see: https://vndb.org/r57712

Python 3.10+ and Cargo required.
See tools/fan_patch/aoitori/README.md.
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
ROOT = "[天之圣杯汉化组]青鸟汉化补丁/"
PATCH_SHA256 = '5550414766ae827adf1eaaa5d996a2759f1f9ccb5b9a8d97849264b01aebbc2e'
STEAM_PATCH_SHA256 = '3176f72c82c71ad1b83174cde95b9950d22e4fb09ca67b961c51f2ecf52e158f'
BASE_HASHES = {
    "cmvs32.exe": "b1621c5b504a800344316d72dc4477fbf13a00f15f8308470952429f4013003a",
    "cmvs64.exe": "3bccda5d7611412c6938c0c9b06b2195914cfb78763a53f2c235bb6479395ee0",
    "data/pack/start.ps3": "10afc37c05c970a898ea263b2a45e553813be8753302754349d8b9c1b801f5fe",
    "data/pack/script.cpz": "d2a22d23d600a75ecbabef3c28fa9bdcbe69a33aed756487c9c515e021295cc7",
}
GBK_SUBSTITUTIONS = {"♪": "☆", "・": "·"}


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def longinus_entries(raw: bytes) -> dict[int, bytes]:
    if len(raw) < 16 or raw[:12] != b"Longinus\x01\x04\x00\x01":
        raise ValueError("Unsupported Longinus header")
    count = struct.unpack_from("<I", raw, 12)[0]
    end = 16 + count * 12
    if count > 10000 or end > len(raw):
        raise ValueError("Truncated Longinus index")
    result = {}
    for at in range(16, end, 12):
        key, size, offset = struct.unpack_from("<III", raw, at)
        if key in result or offset != end or size > LIMIT or offset + size > len(raw):
            raise ValueError("Invalid Longinus entry range or duplicate hash")
        result[key] = raw[offset:offset + size]
        end = offset + size
    if end != len(raw):
        raise ValueError("Trailing Longinus bytes")
    return result


def normalize_script(data: bytes) -> bytes:
    if len(data) < 48:
        raise ValueError("Truncated script header")
    header = struct.unpack_from("<11I", data, 4)
    actual = len(data) - 48
    if actual != header[9]:
        # 24 pinned scripts omit one/two trailing empty-string terminators.
        delta = header[9] - actual
        if delta not in (1, 2) or data[-1:] != b"\0":
            raise ValueError("Unexpected script size mismatch")
        data += b"\0" * delta
    return data


def extract_patch(archive: zipfile.ZipFile, stage: Path) -> tuple[dict[str, str], list[str]]:
    members = [i for i in archive.infolist() if i.filename == ROOT + "Longinus.dat"]
    if len(members) != 1 or members[0].file_size > 512 * 1024 * 1024:
        raise ValueError("Missing, duplicate or oversized Longinus.dat")
    entries = longinus_entries(archive.read(members[0]))
    for category in ("sc", "pic"):
        (stage / category).mkdir()
    pictures = {}
    scripts = 0
    for key, data in entries.items():
        name = f"{key:08x}"
        category = "sc" if data.startswith(b"PS2A") else "pic"
        if category == "sc":
            data = normalize_script(data)
        (stage / category / name).write_bytes(data)
        if category == "sc":
            scripts += 1
        else:
            pictures[name] = name
    if scripts != 85 or len(pictures) != 52:
        raise ValueError("Unexpected patch coverage")
    return pictures, []


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
    patch_digest = sha256(patch)
    if patch_digest == STEAM_PATCH_SHA256:
        raise ValueError("2024 Steam patch requires a matching Steam game; use the 2018 patch with this original")
    if patch_digest != PATCH_SHA256:
        raise ValueError("Unsupported patch ZIP (SHA-256 mismatch)")
    tool = Path(__file__).resolve().parent
    repository = tool.parents[2]
    with tempfile.TemporaryDirectory(prefix="cmvs-aoitori-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            aliases, ignored = extract_patch(archive, stage)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-aoitori"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        reports = {}
        for name in ("scripts", "images"):
            with (stage / f"{name}.tsv").open() as stream:
                reports[name] = list(csv.DictReader(stream, delimiter="\t"))
        if len(reports["scripts"]) != 87 or len(reports["images"]) != 52:
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "aoitori.ico"):
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
            game="aoitori", profile="cmvs-20171114", encoding=encoding,
            patch_sha256=patch_digest, original_sha256=BASE_HASHES,
            archive_format="CPZ7", image_format="PB3B type 1",
            text_substitutions=GBK_SUBSTITUTIONS if encoding == "gbk" else {},
            source_adaptations=["Resolve Longinus UTF-16LE uppercase filename hashes.",
                                "Restore omitted trailing empty-string terminators in 24 scripts.",
                                "Decode GBK patch text and restore A1E1 music notes; retain original startup and translate its title.",
                                "Convert raw BGRA images to PB3B type 1; retain CPZ7 keys."],
            picture_sources=aliases, unused_pictures=ignored,
            output_sha256=outputs, **reports,
            limitations=["No patch exe/DLL, settings or saves installed.",
                         "Untranslated scripts retain their original Japanese text.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted Aoi Tori ({encoding}) to {output}")


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
