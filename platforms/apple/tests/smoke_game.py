"""Write a synthetic two-frame game using the repository's CPZ6 test vector."""
from pathlib import Path
import struct
import sys


def generate(game):
    pack = game / "data/pack"
    pack.mkdir(parents=True)
    fixture = Path(__file__).resolve().parents[3] / "crates/cmvs-core/tests/fixtures/cpz6-script.hex"
    (pack / "script.cpz").write_bytes(bytes.fromhex(fixture.read_text()))
    code = bytearray()

    def command(op, *args):
        for arg in reversed(args):
            code.extend(struct.pack("<HHI", 0x0440, 4, arg))
        code.extend(struct.pack("<H", op))

    command(0x215d, 0, 7, 0, 0, 64, 48, 64, 48)
    command(0x2152, 0, 0xffffffff, 1)
    command(0x2000)
    command(0x2000)
    command(0x2001)
    text = b"\0\\wcApple\0"
    payload = code + text
    header = struct.pack("<4s11I", b"PS2A", 48, 0, 0, 0, len(code), 0, len(text), 0, 0, len(payload), 0)
    (pack / "start.ps3").write_bytes(header + bytes((((b << 1 | b >> 7) + 124) & 255) for b in payload))
    (game / "cmvs.cfg").write_text("[WINDOW_RESOLVING]\nBASE=64,48\n")
    (game / "encoding.txt").write_text("utf-8\n")


if __name__ == "__main__":
    generate(Path(sys.argv[1]))
