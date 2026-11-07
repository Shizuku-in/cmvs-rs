"""Writes the LiveArea images (8-bit indexed PNG, as the Vita shell requires) and the
LiveArea template into the directory given as the only argument."""
import os
import struct
import sys
import zlib

BACKGROUND = (24, 30, 42)
ACCENT = (46, 104, 196)
TEXT = (255, 255, 255)
# 5x7 letters for the "CMVS" wordmark.
GLYPHS = {
    "C": ["01111", "10000", "10000", "10000", "10000", "10000", "01111"],
    "M": ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
    "V": ["10001", "10001", "10001", "10001", "01010", "01010", "00100"],
    "S": ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
}
TEMPLATE = """<?xml version="1.0" encoding="utf-8"?>
<livearea style="a1" format-ver="01.00" content-rev="1">
  <livearea-background>
    <image>bg.png</image>
  </livearea-background>
  <gate>
    <startup-image>startup.png</startup-image>
  </gate>
</livearea>
"""


def image(width, height, cell):
    """Palette indices: 0 background, 1 accent bar, 2 wordmark; centred, `cell` px dots."""
    rows = [bytearray(width) for _ in range(height)]
    text_width = (len(GLYPHS) * 6 - 1) * cell
    left = (width - text_width) // 2
    top = (height - 7 * cell) // 2
    for i, letter in enumerate("CMVS"):
        for gy, line in enumerate(GLYPHS[letter]):
            for gx, bit in enumerate(line):
                if bit == "1":
                    for y in range(cell):
                        for x in range(cell):
                            rows[top + gy * cell + y][left + (i * 6 + gx) * cell + x] = 2
    bar = max(2, cell // 2)
    for y in range(top + 8 * cell, top + 8 * cell + bar):
        for x in range(left, left + text_width):
            rows[y][x] = 1
    return rows


def png(rows):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    width, height = len(rows[0]), len(rows)
    palette = bytes(BACKGROUND + ACCENT + TEXT)
    raw = b"".join(b"\0" + bytes(row) for row in rows)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 3, 0, 0, 0))
        + chunk(b"PLTE", palette)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def main():
    out = sys.argv[1]
    os.makedirs(os.path.join(out, "livearea", "contents"), exist_ok=True)
    for name, size, cell in [
        ("icon0.png", (128, 128), 4),
        ("livearea/contents/bg.png", (840, 500), 12),
        ("livearea/contents/startup.png", (280, 158), 5),
    ]:
        with open(os.path.join(out, name), "wb") as f:
            f.write(png(image(*size, cell)))
    with open(os.path.join(out, "livearea", "contents", "template.xml"), "w") as f:
        f.write(TEMPLATE)


main()
