"""Generate synthetic Theora fixtures and independent FFmpeg plane hashes.

Run from any directory with Python 3 and FFmpeg built with libtheora.
Use --verify to check all committed streams, including archived encoder fixtures.
Fixtures contain no game data. Ogg serials and encoder metadata may vary between
FFmpeg releases; regenerate the hex and hashes together.
"""
from pathlib import Path
import argparse
import hashlib
import subprocess
import tempfile

destination = Path(__file__).parent
cases = [
    ("tall", "testsrc2=size=48x80:rate=24", 1, 24, 48, 80),
    ("wide", "testsrc2=size=80x48:rate=24,noise=alls=5:allf=t:all_seed=7", 9, 24, 80, 48),
    ("crop", "testsrc2=size=50x34:rate=24,noise=alls=30:allf=t:all_seed=7", 9, 12, 50, 34),
    ("noise", "testsrc2=size=128x96:rate=24,noise=alls=30:allf=t:all_seed=7", 3, 12, 128, 96),
]


def hashes(movie, count, width, height, coded=False):
    options = ["-apply_cropping", "0"] if coded else []
    raw = subprocess.check_output([
        "ffmpeg", "-v", "error", "-threads", "1", "-c:v", "theora", *options,
        "-i", str(movie), "-vsync", "0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-",
    ])
    frame_size = width * height * 3 // 2
    assert len(raw) == count * frame_size, (movie, len(raw), count * frame_size)
    result = []
    for start in range(0, len(raw), frame_size):
        frame = raw[start:start + frame_size]
        if coded:
            # The Rust decoder stores each complete plane bottom-up.
            flipped = bytearray()
            offset = 0
            for w, h in [(width, height), (width // 2, height // 2), (width // 2, height // 2)]:
                for row in reversed(range(h)):
                    flipped.extend(frame[offset + row * w:offset + (row + 1) * w])
                offset += w * h
            frame = flipped
        result.append(hashlib.sha256(frame).hexdigest())
    return "\n".join(result) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    entries = cases + ([("quant", None, None, 4, 32, 32),
                         ("motion", None, None, 12, 32, 32)] if args.verify else [])
    with tempfile.TemporaryDirectory(prefix="cmvs-theora-") as temp:
        for name, source, quality, count, width, height in entries:
            movie = Path(temp) / f"{name}.ogv"
            if args.verify:
                movie.write_bytes(bytes.fromhex((destination / f"theora-{name}.hex").read_text()))
            else:
                subprocess.run([
                    "ffmpeg", "-v", "error", "-f", "lavfi", "-i", source,
                    "-frames:v", str(count), "-c:v", "libtheora", "-q:v", str(quality),
                    "-g", "8", "-pix_fmt", "yuv420p", "-an", str(movie),
                ], check=True)
                (destination / f"theora-{name}.hex").write_text(movie.read_bytes().hex() + "\n")
            for coded in [False, True]:
                suffix = "coded.sha256" if coded else "sha256"
                expected = destination / f"theora-{name}.{suffix}"
                if args.verify and not expected.exists():
                    continue
                w = (width + 15) // 16 * 16 if coded else width
                h = (height + 15) // 16 * 16 if coded else height
                result = hashes(movie, count, w, h, coded)
                if args.verify:
                    assert expected.read_text() == result, expected
                else:
                    expected.write_text(result)
            print(name, count, movie.stat().st_size)


if __name__ == "__main__":
    main()
