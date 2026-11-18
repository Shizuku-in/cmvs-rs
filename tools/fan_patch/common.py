"""Shared file hashing and command construction for fan patch converters."""
import hashlib
import os
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def cmvs_patch_command(game: str, *args: str) -> list[str]:
    if binary := os.environ.get("CMVS_PATCH_BIN"):
        return [binary, game, *args]
    manifest = Path(__file__).resolve().parents[2] / "Cargo.toml"
    return [
        "cargo", "run", "--offline", "--locked", "--release",
        "--manifest-path", str(manifest), "-p", "cmvs-patch", "--", game, *args,
    ]
