# cmvs-desktop

Desktop host for Windows, Linux and macOS, built on `winit` and `softbuffer`. It owns the window, keyboard and mouse input, the application event loop, and fullscreen and cursor handling. MGV (Ogg Theora) movies decode through [`cmvs-video`](../cmvs-video/README.md), re-exported as `cmvs_desktop::video`. Rendering is software by default.

The `cmvs-desktop` binary is built from the root package:

```sh
cargo run --release --features desktop,audio-output --bin cmvs-desktop -- \
  <game-dir> --encoding shift-jis --user-data-dir <dir> --audio native --font <font>
```

See the [running guide](../../docs/guides/running.md) for options.

## Features

| Feature | Adds |
|---|---|
| `audio-output` | Native audio through `cmvs-audio` |
| `wgpu-renderer` | GPU compositing through `cmvs-wgpu` |

## Testing

`tests/wgpu.rs` is an ignored GPU scene test on a real game; it needs the `wgpu-renderer` feature, a GPU, and `CMVS_GAME_ROOT` and `CMVS_FONT_PATH`.
