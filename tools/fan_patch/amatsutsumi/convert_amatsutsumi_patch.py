"""Convert 绿茶汉化组 アマツツミ fan patch to standard PS2A/CPZ6/PB3B.

see: https://vndb.org/r97972

Python 3.10+ and Cargo required.
See tools/fan_patch/amatsutsumi/README.md.
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
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from ps3_inspect import lzss_decode

LIMIT = 64 * 1024 * 1024
ROOT = "[绿茶汉化组]アマツツミ/"
BASE = ROOT + "Amatsutsumi_CHS/"
PATCH_MEMBERS = {
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/Scripts.pk':
        '46d1c5b62f227e07c56fd63d1ab718f03f314b90e333e2d4c67f6b8f3c855a23',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/Graphic.pk':
        'e681f35ee0b6d311ab7894338462b8f13258d6017c0603ec8a8e7d41d352b9dc',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/c1.cmv':
        '695f70224155e8b98b94f4a95cd7913e1f2d2b6a433e9ca8e12613a5fc18e9af',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/c2.cmv':
        '52e528565c614ef868cdfcf9265c20ebeb418c2abd764b685a512788c91a4c33',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/c3.cmv':
        'c33e378cdbdd5534be3d860ed58da3145a93dfdf9001de86fd15b5ded560abc7',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/c4.cmv':
        '21e97f46e7c41a3758b742f2da101e42e985b3eed9b8a3cb4b45e67ff0cdfae2',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/op.cmv':
        '5730e3e89347c21f9157ca551c6d11e2f2a20ca0f4d2a48132bf5e4759a7a479',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/Pk/op2.cmv':
        '85fbd3ec6abe9cd9bbefa9712479425ce4b95d7e7a3b370120f6005a265a6cf5',
    '[绿茶汉化组]アマツツミ/Amatsutsumi_CHS/请读我.txt':
        '000bb14613f0e7af1fa5324687e6113e5685df6a1fc78e659ac48acbf4bf69c8',
    '[绿茶汉化组]アマツツミ/补全h汉化补丁/Scriptspk.UPD':
        '47fa7ef7c927d7b9c3e6502394fe3322366d45d207cbd2d8be79bad8fe93e839',
    '[绿茶汉化组]アマツツミ/补全h汉化补丁/注意.txt':
        '7595a257b21b875eb36e8339d26ed881753bba25eb15dad8d2cd1773f89c18e4',
}

BASE_HASHES = {
    'cmvs32.exe':
        '8b09e6b23e0eeaecccf6a584cb94720d239c00f177ed02036ab8845120d99197',
    'cmvs64.exe':
        'dc74d2001387ae1aefbdc4204216de5ebb9d3896009280720c4e7a91b42d84f5',
    'data/pack/start.ps3':
        'c338313949a803e834acf0d746c93f09c4d41f558af8fb6c13ab1ee55aff2bd0',
    'data/pack/script.cpz':
        '92b367e8c3d2791f1dd2ca8426581974fbb640d2766f2e82ab12383d2f5359e4',
}



def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def unpack_pkp(data: bytes) -> dict[str, bytes]:
    """PKP: 12-byte header, 176-byte records, raw DEFLATE payloads."""
    if len(data) < 12 or data[:8] != b"PKP\0\0\0\0\0":
        raise ValueError("Invalid PKP signature")
    count = struct.unpack_from("<I", data, 8)[0]
    end = 12 + count * 176
    if not count or end > len(data):
        raise ValueError("PKP index exceeds bounds")
    result = {}
    cursor = end
    for i in range(count):
        at = 12 + i * 176
        offset, size, stored = struct.unpack_from("<III", data, at)
        field = data[at + 48:at + 176].decode("utf-16le")
        name, separator, padding = field.partition("\0")
        if (not separator or padding.strip("\0") or not name or not name.isascii()
                or any(c in name for c in "/\\:") or name in (".", "..")
                or name in result or any(data[at + 12:at + 48])):
            raise ValueError("Unsafe or duplicate PKP member")
        if offset != cursor or not 0 < stored <= LIMIT or not 0 < size <= LIMIT or offset + stored > len(data):
            raise ValueError("PKP resource exceeds bounds")
        raw = data[offset:offset + stored]
        if size != stored:
            decoder = zlib.decompressobj(-15)
            raw = decoder.decompress(raw, size + 1)
            if not decoder.eof or decoder.unused_data or decoder.unconsumed_tail:
                raise ValueError("Invalid bounded PKP DEFLATE stream")
        if len(raw) != size:
            raise ValueError("PKP decoded length differs")
        result[name] = raw
        cursor += stored
    if cursor != len(data):
        raise ValueError("Trailing PKP data")
    return result


def encode_script(raw: bytes) -> bytes:
    """Patch scripts contain plain LZSS rather than the engine's encrypted LZSS."""
    if len(raw) < 48 or raw[:8] != b"PS2A0\0\0\0":
        raise ValueError("Invalid patch script header")
    key = struct.unpack_from("<I", raw, 12)[0]
    labels, code, unknown, text, entry, packed, size = struct.unpack_from("<7I", raw, 16)
    if packed != len(raw) - 48 or labels * 4 + code + unknown + text != size or size > LIMIT or entry >= code:
        raise ValueError("Invalid patch script segments")
    payload = lzss_decode(raw[48:], size)
    header = bytearray(raw[:48])
    struct.pack_into("<I", header, 36, 0)
    xor = ((key >> 3) + (key >> 24)) & 255
    shift = (key >> 20) % 5 + 1
    encoded = bytes((((((b << shift) | (b >> (8 - shift))) & 255) ^ xor) + 124) & 255
                    for b in payload)
    return bytes(header) + encoded


def extract_patch(archive: zipfile.ZipFile, stage: Path) -> dict[str, str]:
    members = {}
    for info in archive.infolist():
        if info.filename in members:
            raise ValueError("Duplicate ZIP member")
        members[info.filename] = info
    digests = {}
    selected = {}
    for name, expected in PATCH_MEMBERS.items():
        info = members.get(name)
        if info is None or info.is_dir() or info.file_size > 650 * 1024 * 1024:
            raise ValueError(f"Missing or oversized patch member: {name}")
        raw = archive.read(info)
        digest = hashlib.sha256(raw).hexdigest()
        if digest != expected:
            raise ValueError(f"Unsupported patch member: {name}")
        selected[name] = raw
        digests[name] = digest
    scripts = unpack_pkp(selected[BASE + "Pk/Scripts.pk"])
    update = unpack_pkp(selected[ROOT + "补全h汉化补丁/Scriptspk.UPD"])
    if len(scripts) != 86 or len(update) != 27 or not update.keys() <= scripts.keys():
        raise ValueError("Unexpected script update coverage")
    scripts.update(update)
    (stage / "sc").mkdir()
    for name, raw in scripts.items():
        if not name.endswith(".ps3"):
            raise ValueError("Unexpected script member")
        (stage / "sc" / name).write_bytes(encode_script(raw))
    pictures = unpack_pkp(selected[BASE + "Pk/Graphic.pk"])
    if len(pictures) != 114:
        raise ValueError("Unexpected image coverage")
    (stage / "pic").mkdir()
    for name, raw in pictures.items():
        if not name.endswith(".png") or not raw.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError("Unexpected graphic member")
        (stage / "pic" / name).write_bytes(raw)
    (stage / "video").mkdir()
    for name in ("c1", "c2", "c3", "c4", "op", "op2"):
        (stage / "video" / (name + ".cmv")).write_bytes(selected[BASE + f"Pk/{name}.cmv"])
    for source, destination in [(BASE + "请读我.txt", "汉化补丁说明.txt"),
                                (ROOT + "补全h汉化补丁/注意.txt", "脚本更新说明.txt")]:
        (stage / destination).write_bytes(selected[source])
    return digests


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
    tool = Path(__file__).resolve().parent
    repository = tool.parents[2]
    with tempfile.TemporaryDirectory(prefix="cmvs-amatsutsumi-") as temp:
        stage = Path(temp)
        with zipfile.ZipFile(patch) as archive:
            digests = extract_patch(archive, stage)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-amatsutsumi"),
            "--", str(game), str(stage), encoding,
        ], check=True)
        reports = {}
        for name in ("scripts", "images", "videos"):
            with (stage / f"{name}.tsv").open() as stream:
                reports[name] = list(csv.DictReader(stream, delimiter="\t"))
        if (len(reports["scripts"]) != 88 or len(reports["images"]) != 114
                or len(reports["videos"]) != 6):
            raise ValueError("Incomplete conversion report")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "amatsutsumi.ico"):
            if (game / name).is_file():
                shutil.copy2(game / name, output / name)
        if (game / "save/system.dat").is_file():
            (output / "save").mkdir()
            shutil.copy2(game / "save/system.dat", output / "save/system.dat")
        for name in ("汉化补丁说明.txt", "脚本更新说明.txt"):
            shutil.copy2(stage / name, output / name)
        outputs = {}
        for asset in (stage / "pack").iterdir():
            destination = output / "data/pack" / asset.name
            shutil.copy2(asset, destination)
            outputs[str(destination.relative_to(output))] = sha256(destination)

        for asset in (stage / "video").iterdir():
            destination = output / "data/video" / asset.name
            shutil.copy2(asset, destination)
            outputs[str(destination.relative_to(output))] = sha256(destination)
        report = dict(
            game="amatsutsumi", profile="cmvs-20160201", encoding=encoding,
            archive_format="CPZ6",
            patch_sha256=sha256(patch), patch_member_sha256=digests, original_sha256=BASE_HASHES,
            script_update=True, optional_mosaic_patch=False,
            source_adaptations=["Keep installed startup archive mounts and initialization; use the updated Chinese title.",
                                "Retain pinned patch code/labels for its earlier source revision.",
                                "Decode sn4001n's existing translated name as GBK.",
                                "Map patch glyph A2E1 to U+266A."],
            text_substitutions={"♪": "☆", "・": "·"} if encoding == "gbk" else {},
            output_sha256=outputs, **reports,
            limitations=["Patch exe/DLL and its embedded font are not installed.",
                         "Optional graphics and alternate-quality videos are not installed.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted Amatsutsumi ({encoding}) to {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("game", "patch", "output"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--encoding", choices=("utf-8", "utf8", "gbk"), default="utf-8")
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output, args.encoding)
    except (OSError, ValueError, KeyError, zipfile.BadZipFile, zlib.error, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
