"""Generate synthetic Theora fixtures and independent FFmpeg plane hashes.

Run from any directory with Python 3 and FFmpeg built with libtheora.
Fixtures contain no game data. Ogg serials and encoder metadata may vary between
FFmpeg releases; regenerate the hex and hashes together.
"""
from pathlib import Path
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
with tempfile.TemporaryDirectory(prefix="cmvs-theora-") as temp:
    for name, source, quality, count, width, height in cases:
        movie = Path(temp) / f"{name}.ogv"
        subprocess.run([
            "ffmpeg", "-v", "error", "-f", "lavfi", "-i", source,
            "-frames:v", str(count), "-c:v", "libtheora", "-q:v", str(quality),
            "-g", "8", "-pix_fmt", "yuv420p", "-an", str(movie),
        ], check=True)
        raw = subprocess.check_output([
            "ffmpeg", "-v", "error", "-threads", "1", "-i", str(movie),
            "-vsync", "0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-",
        ])
        frame_size = width * height * 3 // 2
        assert len(raw) == count * frame_size, (name, len(raw))
        (destination / f"theora-{name}.hex").write_text(movie.read_bytes().hex() + "\n")
        hashes = [hashlib.sha256(raw[i:i + frame_size]).hexdigest()
                  for i in range(0, len(raw), frame_size)]
        (destination / f"theora-{name}.sha256").write_text("\n".join(hashes) + "\n")
        print(name, count, movie.stat().st_size)
