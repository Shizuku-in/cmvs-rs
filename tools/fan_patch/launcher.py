"""Build the cmvs-patch command used by the Python converters."""
import os
from pathlib import Path


def cmvs_patch_command(game: str, *args: str) -> list[str]:
    if binary := os.environ.get("CMVS_PATCH_BIN"):
        return [binary, game, *args]
    manifest = Path(__file__).resolve().parents[2] / "Cargo.toml"
    return [
        "cargo", "run", "--offline", "--locked", "--release",
        "--manifest-path", str(manifest), "-p", "cmvs-patch", "--", game, *args,
    ]
