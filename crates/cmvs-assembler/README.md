# cmvs-assembler

Offline CMVS and CVNS resource conversion, script assembly, archive editing, and incremental game builds. Outputs must be new paths; image encoders verify pixel round-trips and archive writers verify written plaintext before publishing.

## Subcommands

| Resource | Actions | Purpose |
|---|---|---|
| `build` | `export`, `extract`, `assemble` | Whole-game `game.tsv` manifest export, single-asset extraction, and SHA-256-verified incremental rebuild from `--source` |
| `script` | `assemble`, `import-text` | Assemble a `cmvs-script-v1` project or import a `cmvs-text-v1` translation TSV into a `PS2A`/`PSS0` script |
| `cpz` | `pack`, `export`, `replace` | Export `archive.tsv` manifests, pack `cpz1`/`cpz2`/`cpz3`/`cpz5`/`cpz6`/`cpz7` archives, or replace entries by zero-based index (`--start-script`, `--keys`, `--verify-all`) |
| `pb2` | `decode`, `encode`, `export`, `pack` | Decode static `PB2A` to PNG. Encode PNG or 32-bit `BI_RGB` BMP as `PB2A` (types 1 and 2, 24 or 32 bpp). Export and pack animations or named parts with `images.tsv`. |
| `pb3` | `decode`, `encode` | Decode `PB3B` to PNG. Encode PNG or 32-bit `BI_RGB` BMP as `PB3B` (types 1, 5, 6, 7 and 8). Overlay types 6 and 8 require `--base` in both directions and `--reference` when encoding. |
| `msk` | `decode`, `encode` | Decode/encode CVNS `MSK0` masks as 8-bit grayscale PNGs |
| `cursor` | `decode`, `encode` | Export/assemble Windows `.cur` files via `cmvs-cur-v1` hex TSV rows |

## Usage

From the workspace root:

```sh
cargo run --offline -p cmvs-assembler -- --help
```

Pairs with [`cmvs-disassembler`](../cmvs-disassembler/README.md) for `--export` projects and `--export-text` translation tables. See the [CLI Guide](../../docs/guides/cli.md) for format details and examples.

## Library & Used by

Exposes `archive`, `build`, `cursor`, `image`, `pb`, and `script` modules, and is used as a library by [`cmvs-patch`](../cmvs-patch/README.md).

## Testing

```sh
cargo test --offline -p cmvs-assembler
```
