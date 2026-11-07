# cmvs-host

Window-independent CMVS player for embedding. The host supplies presentation, input coordinates, a writable save path and a font. The player runs the VM, software compositor, text, native audio and saves.

## API

The Rust `Player` and the C interface in [`cmvs_host.h`](cmvs_host.h) expose the same model:

1. Open a game with an external user directory (`cmvs_host_open`), or with saves in `<game>/savedata/` (`cmvs_host_open_game_savedata`).
2. Call `cmvs_host_tick` with the elapsed time. Only completed VM frames advance `cmvs_host_frame_serial` and publish new RGBA pixels.
3. Forward keys and pointer positions, pause when backgrounded, and call `cmvs_host_save_preferences` before closing.

All calls for one player must come from one host thread. Errors are reported through `cmvs_host_error`. On Android, call `cmvs_host_initialize_android` before opening a player.

## Building

The crate builds as a static library. [`platforms/host`](../../platforms/host/README.md) provides the CMake target and the list of supported targets; the iOS app in [`platforms/apple`](../../platforms/apple/README.md) uses it.
