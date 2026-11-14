# cmvs-host

Window-independent CMVS player for embedding. The host supplies presentation, input coordinates, a writable save path and a font. The player runs the VM, software compositor, text, native audio and saves.

## API

Use Rust `Player` for embedding a complete player. Custom hosts that implement their own services can use `cmvs_core::runtime::Runtime` and `RuntimeOptions` instead.

`Player::tick` returns `TickOutcome::{BudgetExhausted, FrameReady, Quit, Inactive}`. On `BudgetExhausted`, resume with the same frame delta; only `FrameReady` publishes a new frame. Errors use `PlayerError`, which retains initialization, execution and persistence error sources or the runtime's structured stop event.

The C interface in [`cmvs_host.h`](cmvs_host.h) exposes the same model:

1. Open a game with an external user directory (`cmvs_host_open`), or with saves in `<game>/savedata/` (`cmvs_host_open_game_savedata`).
2. Call `cmvs_host_tick` with the elapsed time. Only completed VM frames advance `cmvs_host_frame_serial` and publish new RGBA pixels.
3. Forward keys and pointer positions, pause when backgrounded, and call `cmvs_host_save_preferences` before closing.

All calls for one player must come from one host thread. Errors are reported through `cmvs_host_error`. On Android, call `cmvs_host_initialize_android` before opening a player.

Rust input methods are `set_key_state`, `set_pointer_position`, and `set_paused`; `frame_serial` identifies completed frames. The old `key`, `pointer`, `pause`, and `serial` methods remain aliases. C function names and API 1 status codes are unchanged.

## Building

The crate builds as a static library. [`platforms/host`](../../platforms/host/README.md) provides the CMake target and the list of supported targets; the iOS app in [`platforms/apple`](../../platforms/apple/README.md) uses it.
