# cmvs-bench

Zero-dependency microbenchmark runner for CMVS engine hot paths using synthetic inputs.

## Benchmarks

| Name | Covers |
|---|---|
| `lzss_encode`, `lzss_decode` | `cmvs-core::format::lzss` compression and decompression |
| `pb3_encode`, `pb3_decode` | `cmvs-core::format::pb3` RGBA32 image encoding and decoding |
| `cpz_lookup_o1` | Case-folded `O(1)` lookup on a 1,200-entry `CPZ5` archive |
| `yuv420_to_rgba` | `cmvs-video` planar YUV 4:2:0 to RGBA BT.601 conversion |
| `scene_render` | `cmvs-font` glyph object preparation, `prepare_scene`, and CPU compositing |
| `vm_step` | Tight `cmvs-core::vm::machine::Machine` bytecode loop stepping |

## Usage

From the workspace root:

```sh
cargo run --release -p cmvs-bench -- [--smoke|--quick] [FILTER]
```

- `FILTER`: run benchmarks whose names contain the given substring.
- `--smoke`, `--quick`: run with smaller inputs and fewer iterations.

## Testing

```sh
cargo test -p cmvs-bench
```

Runs the full benchmark suite in smoke mode and checks CLI argument parsing.
