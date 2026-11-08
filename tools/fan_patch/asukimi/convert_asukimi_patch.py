"""Convert the known CK-GAL Asukimi ZIP to ordinary PS2A/CPZ3/PB3 assets.

Python 3.10+, standard library only. Inputs are read only; output must be new.
No patch executable is run or installed. See tools/fan_patch/asukimi/README.md.
"""
from __future__ import annotations

import argparse
from array import array
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import struct
import sys
import tempfile
import zipfile

# Reuse the generic script decoder in tools/, independent of the current directory.
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from ps3_inspect import FormatError, decode_script

MASK = 0xFFFFFFFF
LIMIT = 64 * 1024 * 1024
PATCH_SHA256 = "9aab18642d9177dfb83a2c792dec62a0c71cc3474166ebf9091229ef64a024fc"
BASE_HASHES = {
    "cmvs.exe": "caff7901a7dc4ec963d7fde358028cdd68dfd87fc7925c9715a20e7a5f36d02b",
    "data/pack/start.ps2": "f92c0997c47900f9ae8fc1f388f04ed2688622274e22bd4bf76920af5fd46c0c",
    "data/pack/script.cpz": "2fe6d38487d252d54d64b1a70c12d29173a789348089b00c6c5413f5cf4f7d32",
}
TABLE = (
    0x4D0D4A5E, 0xB3ABF3E1, 0x3C37336D, 0x86C3F5F3,
    0x7D4F9B89, 0x58D7DE11, 0x6367778D, 0xA5F34629,
    0x067FA4B5, 0xED0AE742, 0xB19450CC, 0xE7204A5A,
    0xD9AF04F5, 0x5D3B687F, 0xC1C7A6FD, 0xFC502289,
)


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
        return digest.hexdigest()


def checksum(data: bytes) -> int:
    end = len(data) // 4 * 4
    return (0x78341256 + sum(v[0] for v in struct.iter_unpack("<I", data[:end]))
            + sum(data[end:])) & MASK


def cipher(data: bytes, key: int, *, encrypt: bool = False) -> bytes:
    table = [(v + key) & MASK for v in TABLE]
    shift = 0xFD
    for n in range(8):
        shift ^= key >> (4 * n)
    shift = (shift & 15) + 8
    end = len(data) // 4 * 4
    words = array("I")
    words.frombytes(data[:end])
    if sys.byteorder != "little":
        words.byteswap()
    flag = 3
    for i, v in enumerate(words):
        if encrypt:
            v = ((v >> shift) | (v << (32 - shift))) & MASK
            words[i] = ((v - 0x6E58A5C2) & MASK) ^ table[flag]
        else:
            v = ((v ^ table[flag]) + 0x6E58A5C2) & MASK
            words[i] = ((v << shift) | (v >> (32 - shift))) & MASK
        flag = (flag + 1) & 15
    if sys.byteorder != "little":
        words.byteswap()
    out = bytearray(words.tobytes())
    tail = data[end:]
    for i, v in enumerate(tail):
        k = (table[flag] >> (4 * (len(tail) - i))) & 255
        out.append((((v - 0x52) & 255) ^ k) if encrypt else ((v ^ k) + 0x52) & 255)
        flag = (flag + 1) & 15
    return bytes(out)


class Cpz3:
    """Flat ASCII indexes for this converter; resource checksums are always checked."""

    def __init__(self, path: Path):
        self.path = path
        with path.open("rb") as stream:
            header = stream.read(20)
            if len(header) != 20 or header[:4] != b"CPZ3":
                raise FormatError("Expected CPZ3")
            count, size, check, key = struct.unpack_from("<4I", header, 4)
            count ^= 0x5E9C4F37
            size ^= 0xF32AED17
            if size > LIMIT or count * 25 > size or size + 20 > path.stat().st_size:
                raise FormatError("CPZ3 index exceeds bounds")
            packed = stream.read(size)
        if checksum(packed) != check:
            raise FormatError("CPZ3 index checksum mismatch")
        index = cipher(packed, key ^ 0xDDDDDDDD ^ 0x7BF4A539)
        self.entries = {}
        at = 0
        for _ in range(count):
            if at + 25 > len(index):
                raise FormatError("Truncated CPZ3 record")
            length, size, offset, _unused, check, key = struct.unpack_from("<6I", index, at)
            if length < 25 or at + length > len(index):
                raise FormatError("Unsupported CPZ3 record")
            field = index[at + 24:at + length]
            if b"\0" not in field:
                raise FormatError("Unterminated CPZ3 name")
            name = field.split(b"\0", 1)[0].decode("ascii")
            if not name or any(c in name for c in "/\\:") or name in self.entries:
                raise FormatError("Unsafe or duplicate CPZ3 name")
            absolute = 20 + len(packed) + offset
            if absolute + size > path.stat().st_size or size > LIMIT:
                raise FormatError("CPZ3 resource exceeds bounds")
            self.entries[name] = (size, absolute, check, key ^ 0xC7F5DA63)
            at += length

    def encrypted(self, name: str) -> bytes:
        size, offset, check, _ = self.entries[name]
        with self.path.open("rb") as stream:
            stream.seek(offset)
            data = stream.read(size)
        if len(data) != size or checksum(data) != check:
            raise FormatError(f"CPZ3 resource checksum mismatch: {name}")
        return data

    def read(self, name: str) -> bytes:
        return cipher(self.encrypted(name), self.entries[name][3])


def write_cpz3(path: Path, source: Cpz3, replacements: dict[str, bytes]) -> None:
    if not replacements.keys() <= source.entries.keys():
        raise FormatError("Replacement names missing from original archive")
    # Stream encrypted resources into a temporary sibling, then write the small index.
    index = bytearray()
    offset = 0
    index_size = sum(25 + len(name.encode("ascii")) for name in source.entries)
    with path.open("xb") as stream:
        stream.write(bytes(20 + index_size))
        for name, (_, _, _, key) in source.entries.items():
            data = (cipher(replacements[name], key, encrypt=True) if name in replacements
                    else source.encrypted(name))
            encoded = name.encode("ascii") + b"\0"
            index.extend(struct.pack("<6I", 24 + len(encoded), len(data), offset, 0,
                                     checksum(data), key ^ 0xC7F5DA63))
            index.extend(encoded)
            stream.write(data)
            offset += len(data)
            if offset > MASK:
                raise FormatError("CPZ3 output exceeds 32-bit offsets")
        packed = cipher(bytes(index), 0, encrypt=True)
        stream.seek(0)
        stream.write(b"CPZ3" + struct.pack("<4I", len(source.entries) ^ 0x5E9C4F37,
                                          len(packed) ^ 0xF32AED17, checksum(packed),
                                          0xDDDDDDDD ^ 0x7BF4A539))
        stream.write(packed)


def bmp_to_pb3(raw: bytes, target_bpp: int = 32) -> tuple[bytes, dict]:
    """Preserve the original resource's alpha semantics, not the BMP storage depth."""
    if target_bpp not in (24, 32):
        raise FormatError("Unsupported target PB3 bit depth")
    if len(raw) < 54 or raw[:2] != b"BM":
        raise FormatError("Expected BMP")
    size, _, _, at = struct.unpack_from("<IHHI", raw, 2)
    dib, width, height, planes, bpp, compression = struct.unpack_from("<IiiHHI", raw, 14)
    if (size != len(raw) or dib != 40 or planes != 1 or bpp != 32 or compression != 0
            or not 0 < width <= 65535 or not 0 < abs(height) <= 65535
            or width * abs(height) * 4 > LIMIT or at < 54
            or not 0 <= len(raw) - at - width * abs(height) * 4 <= 3):
        raise FormatError("Expected bounded 32-bit BI_RGB BMP")
    pixels = raw[at:at + width * abs(height) * 4]
    if height > 0:
        stride = width * 4
        pixels = b"".join(pixels[y:y + stride] for y in range(len(pixels) - stride, -1, -stride))
    height = abs(height)
    count = width * height
    channels = target_bpp // 8
    flags = struct.pack("<3I", 1, 0, count) + b"\0" + bytes((count + 7) // 8)
    data_at = 52 + channels * 4 + len(flags) * channels
    out = bytearray(52)
    out[:4] = b"PB3B"
    struct.pack_into("<I", out, 8, 1)
    struct.pack_into("<I4H", out, 24, max(width, height), 1, width, height, target_bpp)
    struct.pack_into("<2I", out, 44, 52, data_at)
    out.extend(struct.pack(f"<{channels}I", *([len(flags)] * channels)))
    out.extend(flags * channels)
    out.extend(struct.pack(f"<{channels}I", *([count] * channels)))
    for channel in range(channels):
        out.extend(pixels[channel::4])
    # An explicit zero seed/key region makes the header transform the identity.
    out.extend(bytes(47))
    struct.pack_into("<I", out, 4, len(out))
    rgba = bytearray(pixels)
    rgba[0::4], rgba[2::4] = pixels[2::4], pixels[0::4]
    if target_bpp == 24:
        rgba[3::4] = bytes([255]) * count
    return bytes(out), dict(width=width, height=height,
                            bpp=target_bpp,
                            rgba_sha256=hashlib.sha256(rgba).hexdigest())


def normalize_start(raw: bytes) -> bytes:
    # Only called after verifying the entire supported ZIP. These exact three fields
    # are stale; no general truncated-input recovery is enabled in the engine.
    if len(raw) != 956 or struct.unpack_from("<3I", raw, 28) != (60, 0, 915):
        raise FormatError("Unexpected CK-GAL startup layout")
    out = bytearray(raw)
    struct.pack_into("<I", out, 28, 382)
    struct.pack_into("<I", out, 36, 908)
    struct.pack_into("<I", out, 40, 3756)
    decode_script(bytes(out))
    return bytes(out)


def normalize_script_archive(raw: bytes) -> tuple[bytes, int]:
    """Known patch retains old index/resource checksums; rebuild them offline.

    Caller MUST pin the complete input ZIP before using this conversion rule.
    The strict Cpz3 reader then verifies the resulting index and all scripts.
    """
    count, size, _, key = struct.unpack_from("<4I", raw, 4)
    count ^= 0x5E9C4F37
    size ^= 0xF32AED17
    if raw[:4] != b"CPZ3" or size > LIMIT or size + 20 > len(raw):
        raise FormatError("Unexpected patch CPZ3 layout")
    key ^= 0xDDDDDDDD ^ 0x7BF4A539
    index = bytearray(cipher(raw[20:20 + size], key))
    at, repaired = 0, 0
    for _ in range(count):
        if at + 25 > len(index):
            raise FormatError("Truncated patch record")
        length, length_data, offset, _unused, old = struct.unpack_from("<5I", index, at)
        if length < 25 or at + length > size or 20 + size + offset + length_data > len(raw):
            raise FormatError("Patch resource exceeds bounds")
        new = checksum(raw[20 + size + offset:20 + size + offset + length_data])
        repaired += old != new
        struct.pack_into("<I", index, at + 16, new)
        at += length
    packed = cipher(bytes(index), key, encrypt=True)
    out = bytearray(raw)
    out[20:20 + size] = packed
    struct.pack_into("<I", out, 12, checksum(packed))
    return bytes(out), repaired


def convert(game: Path, patch: Path, output: Path) -> None:
    game, patch, output = game.resolve(), patch.resolve(), output.resolve()
    if output.exists() or output.is_relative_to(game):
        raise FormatError("Output must be a new directory outside the game")
    for name, expected in BASE_HASHES.items():
        if sha256(game / name) != expected:
            raise FormatError(f"Unsupported original game: {name}")
    if sha256(patch) != PATCH_SHA256:
        raise FormatError("Unsupported patch ZIP (SHA-256 mismatch)")
    with zipfile.ZipFile(patch) as archive:
        files = {}
        for entry in archive.infolist():
            if entry.is_dir():
                continue
            relative = PurePosixPath(*PurePosixPath(entry.filename).parts[1:])
            if relative.is_absolute() or ".." in relative.parts or relative.as_posix() in files:
                raise FormatError("Unsafe or duplicate patch path")
            if entry.file_size > LIMIT:
                raise FormatError("Patch file too large")
            files[relative.as_posix()] = archive.read(entry)
    start = normalize_start(files["data/pack/start.ps2"])
    script_archive, repaired = normalize_script_archive(files["data/pack/script.cpz"])
    # Validate all scripts strictly before creating the output directory.
    # The ZIP archive is trusted by digest; the temporary CPZ still uses bounded readers.
    with tempfile.TemporaryDirectory(prefix="asukimi-patch-") as temp:
        script_path = Path(temp) / "script.cpz"
        script_path.write_bytes(script_archive)
        scripts = Cpz3(script_path)
        if len(scripts.entries) != 126:
            raise FormatError("Unexpected script count")
        for name in scripts.entries:
            decode_script(scripts.read(name))
    chip = Cpz3(game / "data/pack/chip.cpz")
    images, details = {}, {}
    for name, raw in files.items():
        if name.startswith("image/") and name.endswith(".bmp"):
            target = PurePosixPath(name).stem + ".pb3"
            original = chip.read(target)
            header = bytearray(original[:52])
            for i in range(44):
                header[8 + i] = ((header[8 + i] ^ original[-3 + i % 2])
                                 - original[len(original) - 47 + i]) & 255
            _, width, height, bpp = struct.unpack_from("<4H", header, 28)
            images[target], details[target] = bmp_to_pb3(raw, bpp)
            if (width, height) != (details[target]["width"], details[target]["height"]):
                raise FormatError(f"Replacement dimensions differ from original: {target}")
    if len(images) != 28 or not images.keys() <= chip.entries.keys():
        raise FormatError("Unexpected image replacement set")
    print("Validated 126 scripts, startup repair and 28 images; copying original data", flush=True)
    output.mkdir(parents=True)
    # On failure leave the partial directory for diagnosis, without a success manifest.
    shutil.copytree(game / "data", output / "data")
    for name in ("cmvs.exe", "cmvs.cfg", "initial.cfg", "key.cfg", "asukimi.ico"):
        shutil.copyfile(game / name, output / name)
    # Use the original system template; patch save125.dat is an optional bug workaround,
    # not a clean startup or a guarantee that original saves fit translated scripts.
    if (game / "save/system.dat").is_file():
        (output / "save").mkdir()
        shutil.copyfile(game / "save/system.dat", output / "save/system.dat")
    (output / "data/pack/start.ps2").write_bytes(start)
    (output / "data/pack/script.cpz").write_bytes(script_archive)
    temp_chip = output / "data/pack/chip.converted"
    write_cpz3(temp_chip, chip, images)
    temp_chip.replace(output / "data/pack/chip.cpz")
    (output / "patch-readme.txt").write_bytes(files["readme.txt"])
    report = dict(converter="asukimi-ck-gal-v1", engine_profile="cmvs-20071117",
                  encoding="gbk", patch_sha256=PATCH_SHA256,
                  base_sha256={**BASE_HASHES, "data/pack/chip.cpz": sha256(chip.path)},
                  scripts=len(scripts.entries), images=details,
                  resource_checksums_rebuilt=repaired, index_checksum_rebuilt=True,
                  startup_repair={"text_size": [60, 382], "compressed_size": [915, 908],
                                  "decoded_size": [7130, 3756]},
                  outputs={name: sha256(output / name) for name in
                           (*BASE_HASHES, "data/pack/chip.cpz")},
                  limitations=["Patch's documented Asahi July 2 crash is not repaired",
                               "Patch save125.dat not installed; use separate user data",
                               "Original EXE retained for engine identity; use cmvs-rs",
                               "No guarantee of complete routes or GDI layout equivalence"])
    (output / "conversion.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n",
                                             encoding="utf-8")
    print(f"Converted to {output}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("game", type=Path, help="Supported unmodified original game directory")
    parser.add_argument("patch", type=Path, help="Supported CK-GAL V1.0 ZIP")
    parser.add_argument("output", type=Path, help="New independent output directory")
    args = parser.parse_args()
    try:
        convert(args.game, args.patch, args.output)
    except (OSError, ValueError, KeyError, zipfile.BadZipFile) as exc:
        parser.exit(1, f"Error: {exc}\n")


if __name__ == "__main__":
    main()
