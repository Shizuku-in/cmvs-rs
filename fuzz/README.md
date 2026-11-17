# `cmvs-rs` Fuzz Targets

Standalone [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html) workspace covering
all binary container, codec, save, executable, and bytecode decoders in `cmvs-core`.

`fuzz/` defines its own `[workspace]` so offline CI and standard `cargo --workspace` builds
remain 100% dependency-free of `libfuzzer-sys`. Deterministic offline parser fuzzing also runs
on every `cargo test -p cmvs-core` via `crates/cmvs-core/tests/integration/fuzz_parsers.rs`.

## Available Targets

| Target | Covered Decoders |
|---|---|
| `ps3_bytecode` | `ps3::Header`, `ps3::Script::decode`, `Script::from_memory_image`, `strings`, `vm::bytecode::decode_layout`, bounded `Machine::step` |
| `cpz_archive` | `cpz::ArchiveLayout::read`, `cpz::ArchiveIndex::read` (CPZ1/2/3/5/6/7), `lookup`, `find`, `read_entry_verified` |
| `images` | `pb2`, `pb3`, `jbp1`, `jbpd`, `jbpd::decode_alpha`, `msk`, `cursor::CursorBitmap`, `lzss` |
| `media` | `cmv::CmvIndex` (`read` & `read_scene` + `decode_frame`), `cmv9::Cmv9Index`, `mgv::MgvIndex`, `mv2` |
| `saves_and_pe` | `save::Csv2Save::decode_any`, `system_save::Css1Save`, `snapshot::{EngineSnapshot, CvnsSnapshot, VisualSnapshot, SavedEffect, LocalVariables}`, `exe::{PeImage, find_dispatch}` |

## Usage

Requires a nightly Rust toolchain and `cargo-fuzz`:

```bash
cargo +nightly fuzz list
cargo +nightly fuzz run ps3_bytecode -- -max_total_time=60
cargo +nightly fuzz run cpz_archive -- -max_total_time=60
cargo +nightly fuzz run images -- -max_total_time=60
cargo +nightly fuzz run media -- -max_total_time=60
cargo +nightly fuzz run saves_and_pe -- -max_total_time=60
```
