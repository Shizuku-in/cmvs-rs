# cmvs-video

MGV1 movie decoding for CMVS hosts: a Rust Theora decoder, RGBA conversion and optional worker-thread prefetching. No window or device dependencies.

`cmvs-core::mgv` handles the container and nominal-frame mapping; `ogg` handles packet framing.

## API

| Item | Purpose |
|---|---|
| `PrefetchTheoraFactory` | Decodes ahead on a worker thread; `on_thread_start` allows thread setup |
| `TheoraFactory` | Decodes on the calling thread |
| `TheoraStream` | Stateful decoder used by both factories and the `cmvs-mgv` tool |

Install a factory with `Startup::set_mgv_factory`. Both factories produce the same frames.

## Decoding

- Supports Theora 3.2 YUV420. Container dimensions must match the picture or coded surface; picture dimensions crop the visible region, while coded dimensions retain padding.
- Validates frame rate, packet sizes and pixel limits. Invalid streams return errors.
- Reuses frame buffers and caches the current/next RGBA frames. Repeated nominal frames keep the current packet cached; backward seeks replay from the headers.
- Converts YUV to RGBA using BT.601 limited range.

## Testing

```sh
cargo test --offline -p cmvs-video
```

Implementation, SIMD/unsafe boundaries, benchmarks and real-movie validation are documented in [the decoder notes](../../docs/design/theora-decoder.md).
