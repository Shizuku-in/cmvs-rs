"""Read-only PS2A script decoder. Python 3.10+, standard library only.

Format references: xmoezzz/CMVS-Engine (MIT), RxCMVS notes.
Decode transform and layout checked against Amatsutsumi cmvs64.exe in IDA.
This is analysis tooling, not a game runtime or complete disassembler.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct


class FormatError(ValueError):
    pass


def lzss_decode(data: bytes, expected: int) -> bytes:
    if not 0 <= expected <= 64 * 1024 * 1024:
        raise FormatError("Unreasonable decompressed size")
    ring = bytearray(2048)
    pos, cursor, flags = 0x7DF, 0, 0
    out = bytearray()
    while cursor < len(data):
        flags >>= 1
        if not flags & 0x100:
            flags = data[cursor] | 0xFF00
            cursor += 1
        if flags & 1:
            if cursor == len(data):
                raise FormatError("Truncated LZSS literal")
            values = (data[cursor],)
            cursor += 1
            for value in values:
                out.append(value)
                ring[pos] = value
                pos = (pos + 1) & 0x7FF
        else:
            if cursor + 2 > len(data):
                raise FormatError("Truncated LZSS reference")
            low, high = data[cursor:cursor + 2]
            cursor += 2
            offset, length = low | ((high & 0xE0) << 3), (high & 31) + 2
            if len(out) + length > expected:
                raise FormatError("LZSS reference exceeds declared output size")
            for i in range(length):
                value = ring[(offset + i) & 0x7FF]
                out.append(value)
                ring[pos] = value
                pos = (pos + 1) & 0x7FF
        if len(out) > expected:
            raise FormatError("LZSS output exceeds declared size")
    if len(out) != expected:
        raise FormatError(f"LZSS length mismatch: {len(out)} != {expected}")
    return bytes(out)


def decode_script(raw: bytes) -> tuple[dict, bytes]:
    if len(raw) < 48 or raw[:4] != b"PS2A":
        raise FormatError("Expected PS2A script with 48-byte header")
    fields = struct.unpack_from("<11I", raw, 4)
    header, version, key, labels, code, unknown, text, entry, packed, unpacked, reserved = fields
    if header != 48:
        raise FormatError(f"Unsupported header size: {header}")
    if packed and header + packed > len(raw):
        raise FormatError("Compressed payload is truncated")
    xor = ((key >> 3) + (key >> 24)) & 255
    shift = (key >> 20) % 5 + 1
    payload = bytearray()
    for value in raw[header:header + packed if packed else len(raw)]:
        value = ((value - 124) & 255) ^ xor
        payload.append(((value >> shift) | (value << (8 - shift))) & 255)
    decoded = lzss_decode(bytes(payload), unpacked) if packed else bytes(payload)
    expected = labels * 4 + code + unknown + text
    if len(decoded) != expected:
        raise FormatError(f"Segment lengths disagree: {len(decoded)} != {expected}")
    if code < 2 or entry >= code:
        raise FormatError("Invalid script entry offset")
    info = dict(signature="PS2A", header_size=header, version_raw=version,
                key=f"0x{key:08x}", label_count=labels, code_size=code,
                unknown_size=unknown, text_size=text, entry_pc=entry,
                compressed_size=packed, decoded_size=len(decoded), reserved=reserved,
                trailing_file_bytes=len(raw) - header - packed if packed else 0,
                source_sha256=hashlib.sha256(raw).hexdigest(),
                payload_sha256=hashlib.sha256(decoded).hexdigest())
    return info, decoded


def inspect(raw: bytes, encoding: str = "cp932") -> tuple[dict, bytes]:
    info, payload = decode_script(raw)
    count = info["label_count"]
    labels = list(struct.unpack_from(f"<{count}I", payload)) if count else []
    if any(label >= info["code_size"] for label in labels):
        raise FormatError("Label outside code segment")
    start = count * 4
    code = payload[start:start + info["code_size"]]
    text = payload[start + info["code_size"] + info["unknown_size"]:]
    strings, offset = [], 0
    while offset < len(text):
        end = text.find(b"\0", offset)
        if end < 0:
            raise FormatError("Unterminated text segment string")
        value = text[offset:end]
        if value:
            try:
                strings.append(dict(offset=offset, text=value.decode(encoding), decode_error=False))
            except UnicodeDecodeError:
                strings.append(dict(offset=offset, text=value.decode(encoding, errors="replace"),
                                    decode_error=True, raw_hex=value.hex()))
        offset = end + 1
    # Heuristic matches only; never claim these are instruction boundaries.
    refs = []
    for at in range(max(0, len(code) - 11)):
        if code[at:at+4] == bytes.fromhex("01022001") and code[at+8:at+12] == bytes.fromhex("0f023004"):
            target = struct.unpack_from("<I", code, at + 4)[0]
            refs.append(dict(code_offset=at, text_offset=target, in_bounds=target < len(text)))
    info.update(labels=labels, encoding=encoding, strings=strings,
                code_preview_hex=code[:128].hex(" "), string_reference_candidates=refs,
                status="decoded; no VM execution", disassembly_available=False)
    return info, payload


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("script", type=Path)
    parser.add_argument("--encoding", default="cp932")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--payload", type=Path, help="Optional decompressed payload, WITHOUT original header")
    args = parser.parse_args()
    if args.output.resolve() == args.script.resolve() or (args.payload and args.payload.resolve() in {args.script.resolve(), args.output.resolve()}):
        parser.error("Input, JSON and payload paths must be distinct")
    try:
        info, payload = inspect(args.script.read_bytes(), args.encoding)
    except (OSError, ValueError, LookupError) as exc:
        parser.exit(1, f"Error: {exc}\n")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(info, ensure_ascii=False, indent=2), encoding="utf-8")
    if args.payload:
        args.payload.parent.mkdir(parents=True, exist_ok=True)
        args.payload.write_bytes(payload)
    print(f"Decoded {len(payload)} bytes, {info['label_count']} labels, {len(info['strings'])} strings")


if __name__ == "__main__":
    main()
