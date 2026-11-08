"""Convert CK-GAL 未来ノスタルジア fan patch into standard PS2A/CPZ3.

see: https://vndb.org/r24913

Python 3.10+ and Cargo are required.
See tools/fan_patch/miranosu/README.md.
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
PATCH_SHA256 = "232277b268e17faa084b8e33460166803cb14dfb9499edf22a264c7b7545a60a"
DECODED_SHA256 = "575d506e0c57d0a9a012cd6b3f9860a31e752e6e35bf3fabcbf7cfad06737ec2"
BASE_HASHES = {
    "cmvs32.exe": "78c2283990f3fec167a8097e5e56304edbad7e94d5367eae9e3f5cfa56b4cc4e",
    "cmvs64.exe": "9870e6d5cbd5242b9c686aae20fe10697e87328a66d322cfdb2c80021cb645c5",
    "data/pack/start.ps2": "2e8c7261ea801f59c9e80b9cdbde89832c0a65f7b622e2fa4551509b1cfe5143",
    "data/pack/script.cpz": "bd70dba1785ec45494e73c451e6d492a91fd2eb0e56787dc60bce96c9b06effe",
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def decode_mirai(raw: bytes) -> bytes:
    """Bounded translation of cm32.dll 0x10002130 (LE32 flags, NRV-style LZ).

    The two leading words are compressed/decoded lengths. Alignment bytes in the
    decoded records are arbitrary; the DLL skips them rather than reading text.
    """
    if len(raw) < 12:
        raise ValueError("Truncated mirai.bin header")
    packed, expected = struct.unpack_from("<II", raw)
    if packed != len(raw) - 8 or not 0 < expected <= LIMIT or packed > LIMIT:
        raise ValueError("Invalid mirai.bin lengths")
    data, at, flags = raw[8:], 0, 0
    out, previous = bytearray(), -1

    def byte() -> int:
        nonlocal at
        if at >= len(data):
            raise ValueError("Truncated compressed byte")
        value = data[at]
        at += 1
        return value

    def bit() -> int:
        nonlocal at, flags
        carry = flags >> 31
        flags = flags << 1 & 0xFFFFFFFF
        if not flags:
            if at + 4 > len(data):
                raise ValueError("Truncated compressed flag word")
            word = struct.unpack_from("<I", data, at)[0]
            at += 4
            carry = word >> 31
            flags = (word << 1 | 1) & 0xFFFFFFFF
        return carry

    while True:
        while bit():
            if len(out) >= expected:
                raise ValueError("Literal exceeds decoded length")
            out.append(byte())
        offset = 1
        for _ in range(32):
            offset = offset * 2 + bit()
            if bit():
                break
            offset = (offset - 1) * 2 + bit()
        else:
            raise ValueError("Unbounded match prefix")
        offset -= 3
        if offset >= 0:
            encoded = offset << 8 | byte()
            if encoded > 0xFFFFFFFF:
                raise ValueError("Match offset exceeds 32 bits")
            if encoded == 0xFFFFFFFF:
                break
            signed = ~encoded
            small = signed & 1
            previous = signed >> 1
        else:
            small = bit()
        if small:
            length = bit()
        else:
            length = 1
            if bit():
                length = length * 2 + bit()
            else:
                for _ in range(32):
                    length = length * 2 + bit()
                    if bit():
                        break
                else:
                    raise ValueError("Unbounded match length")
                length += 2
        length += 2 + (previous < -0x500)
        if not 0 <= len(out) + previous < len(out) or len(out) + length > expected:
            raise ValueError("Match exceeds decoded bounds")
        for _ in range(length):
            out.append(out[len(out) + previous])
    if len(out) != expected or at != len(data):
        raise ValueError("Compressed stream length mismatch")
    return bytes(out)


def convert(game: Path, patch: Path, output: Path) -> None:
    game, patch, output = game.resolve(), patch.resolve(), output.resolve()
    if output.exists() or output.is_relative_to(game):
        raise ValueError("Output must be a new directory outside the game")
    for name, expected in BASE_HASHES.items():
        if sha256(game / name) != expected:
            raise ValueError(f"Unsupported original game: {name}")
    if sha256(patch) != PATCH_SHA256:
        raise ValueError("Unsupported patch ZIP (SHA-256 mismatch)")
    with zipfile.ZipFile(patch) as archive:
        members = [i for i in archive.infolist() if Path(i.filename).name == "mirai.bin"]
        if len(members) != 1 or members[0].file_size > LIMIT:
            raise ValueError("Expected one bounded mirai.bin")
        decoded = decode_mirai(archive.read(members[0]))
    if hashlib.sha256(decoded).hexdigest() != DECODED_SHA256:
        raise ValueError("Decoded translation identity mismatch")
    tool = Path(__file__).resolve().parent
    repository = tool.parents[2]
    # Finish script conversion/strict format checks before copying the game.
    with tempfile.TemporaryDirectory(prefix="cmvs-miranosu-") as temp:
        stage = Path(temp)
        (stage / "mirai.decoded").write_bytes(decoded)
        subprocess.run([
            "cargo", "run", "--offline", "--locked", "--release", "--manifest-path",
            str(tool / "Cargo.toml"), "--target-dir", str(repository / "target/fan-patch-miranosu"),
            "--", str(game), str(stage / "mirai.decoded"), str(stage),
        ], check=True)
        with (stage / "scripts.tsv").open() as stream:
            scripts = list(csv.DictReader(stream, delimiter="\t"))
        if len(scripts) != 115 or sum(int(row["strings"]) for row in scripts) != 95112:
            raise ValueError("Unexpected script/string coverage")
        output.mkdir(parents=True)
        shutil.copytree(game / "data", output / "data")
        for name in ("cmvs32.exe", "cmvs64.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "miranosu.ico"):
            if (game / name).is_file():
                shutil.copy2(game / name, output / name)
        # Use only the original system settings template; no plot or patch unlock saves.
        if (game / "save/system.dat").is_file():
            (output / "save").mkdir()
            shutil.copy2(game / "save/system.dat", output / "save/system.dat")
        for name in ("script.cpz", "start.ps2"):
            shutil.copy2(stage / name, output / "data/pack" / name)
        report = dict(
            game="miranosu", profile="cmvs-20110525", encoding="gbk",
            patch_sha256=PATCH_SHA256, decoded_patch_sha256=DECODED_SHA256,
            original_sha256=BASE_HASHES,
            output_sha256={name: sha256(output / name) for name in
                           ("data/pack/script.cpz", "data/pack/start.ps2")},
            scripts=scripts,
            limitations=["Original image assets: image-based menus remain Japanese.",
                         "Untranslated Japanese strings are retained in GBK.",
                         "No patch DLL/exe/system.dat or full-CG saves installed.",
                         "Full routes and original save interoperability are not verified."],
        )
        (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Converted 115 scripts / 95112 strings to {output}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("game", type=Path)
    parser.add_argument("patch", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output)
    except (OSError, ValueError, zipfile.BadZipFile, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
