# cmvs-host

Cross-platform CMVS host session layer, window-independent player, and versioned C ABI.

## Architecture & Cargo Features

- `default = ["std-fs", "native-audio"]` enables the embedded `Player` and C ABI.
- `std-fs` enables filesystem-backed sessions, game-local saves, and Android save-directory locking independently of native audio.
- `--no-default-features` — pure `HostFs` session layer (`HostSession`, `GameBootstrap`, `SessionConfig`, `InputState`, `EncodingPolicy`, `GpuTarget`, `compose_cpu_frame`, `with_gpu_target`), portable to `wasm32-unknown-unknown` (`cmvs-web`).

## Shared Session Layer

Desktop, Apple, Android, Web (`wasm32`), Switch, PS Vita, and C hosts share:

- **Bootstrap & encoding detection (`GameBootstrap`, `HostSession`, `SessionConfig`, `EncodingPolicy`)**: identifies the game via `HostFs`, decodes the startup script, resolves text encoding (`encoding.txt`, fan-patch `conversion.json` `"encoding"`, or bounded script-text detection), binds `FontRenderer` metrics + `DEFAULT_FONT_FACE` fallback, configures `TheoraFactory` / `PrefetchTheoraFactory`, image decode threads, cache budgets, and `KeyBindings`.
- **Input latching (`InputState`)**: 128-code hold + sub-frame pulse latch + `CodeInput` edge state + script pointer coordinates.
- **Dual CPU & GPU frame composition**: `compose_cpu_frame` / `compose_cpu_frame_with_capture` (`RgbaImage`) and `compose_gpu_frame` / `with_gpu_target` (`GpuTarget::Movie` vs `GpuTarget::Scene(RenderFrame)`).

## Embedded `Player` & C ABI (`native-audio` feature)

Use Rust `Player` or [`cmvs_host.h`](cmvs_host.h) when embedding a complete single-threaded software player:

1. Open a game with an external user directory (`cmvs_host_open`), or with saves in `<game>/savedata/` (`cmvs_host_open_game_savedata`).
2. Call `cmvs_host_tick` with the elapsed time. Only completed VM frames advance `cmvs_host_frame_serial` and publish new RGBA pixels.
3. Forward keys and pointer positions, pause when backgrounded, and call `cmvs_host_save_preferences` before closing.

All calls for one player must come from one host thread. Errors are reported through `cmvs_host_error`. On Android, call `cmvs_host_initialize_android` before opening a `Player` with native audio.

## Building

[`platforms/host`](../../platforms/host/README.md) provides the CMake static-library target for embedding via `cmvs_host.h`.

The iOS and macOS SwiftUI apps link `CmvsApple.xcframework`, built from `cmvs-apple`. This bridge uses the Rust session API from `cmvs-host`. See the [Apple build guide](../../platforms/apple/README.md).
