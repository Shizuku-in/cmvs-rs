# cmvs-video

MGV1 movie decoding for CMVS hosts. `cmvs-core` parses the MGV1 container and maps nominal frames to Ogg packets (`cmvs_core::mgv`); this crate decodes the Ogg Theora video stream with the pure-Rust `ogg` and `oxideav-theora` crates and converts frames to RGBA. It has no window or device dependencies, so every native host can install it.

## API

| Item | Purpose |
|---|---|
| `PrefetchTheoraFactory` | Decodes on a worker thread that converts the next packet while the host shows the current one. `on_thread_start` runs first on that thread, e.g. to move it to a free CPU core |
| `TheoraFactory` | Decodes on the calling thread |
| `TheoraStream` | The stateful decoder behind both factories, also used directly by the `cmvs-mgv` tool |

Hosts install a factory with `Startup::set_mgv_factory`. Both factories return identical frames; prefetching only moves the work off the host's main loop.

## Decoding

- The container size must equal the Theora picture or coded size. The picture size crops the visible region; the coded size keeps the whole coding surface (e.g. 1920×1088), as the original DLL does. Frame rate, YUV 4:2:0 and the pixel budget are checked, and mismatches are errors.
- Every Ogg packet read is bounded by `MAX_PAYLOAD`.
- Theora frames depend on earlier frames, so seeking backwards replays from the headers. The last two converted packets are kept, so duplicate nominal frames and a prefetched next packet are served without a replay.
- Colour conversion is BT.601 limited range; each chroma sample covers a 2×2 pixel block at absolute picture coordinates, including an odd left edge.

## Used by

| Host | Factory |
|---|---|
| `cmvs-desktop` (re-exported as `cmvs_desktop::video`) | `PrefetchTheoraFactory` |
| `cmvs-host` (Apple) and the Android player | `PrefetchTheoraFactory` |
| Switch and PS Vita players | `PrefetchTheoraFactory` on core 2; core 0 runs the VM and rendering, core 1 the audio thread |
| `cmvs-probe` | `TheoraFactory` |

The Web player does not install MGV decoding yet.

## Performance

Measured on 2026-11-08 on a Ryzen 9 9900X, single thread, first 600 frames of a 1920×1088, 24 fps movie: decoding and conversion take about 11.3 ms per frame, of which `oxideav-theora` takes about 9.9 ms. FFmpeg decodes the same stream in about 1.3 ms per frame, so Theora decoding is the bottleneck. Console CPUs are several times slower, so 1080p movies are expected to drop frames there while audio keeps time. This has not been measured on hardware.

## Future work: own Theora decoder

If consoles still drop frames with prefetching, `oxideav-theora` may be replaced by a decoder written in this crate. MGV only needs Theora decoding of YUV 4:2:0, which is a much smaller task than a general codec. Planned in the style of the `jbpd` / `jbp1` decoders in `cmvs-core`: table-driven Huffman decoding, integer IDCT, motion compensation and loop filter into reused frame buffers, without per-frame allocations.

This crate may then use `unsafe` for SIMD kernels (NEON on Switch and PS Vita, SSE/AVX on desktop), the hot paths being IDCT, motion compensation, the loop filter and colour conversion. It would follow the `cmvs-native` rule: the crate sets `unsafe_code = "deny"` instead of the workspace `forbid` and allows it only on those kernels, each with a `SAFETY` comment and a scalar fallback that produces the same output.

The new decoder must match the current one exactly. The synthetic fixture tests and the per-frame RGBA hashes of real movies from `cmvs-mgv --check-frames` are the reference, together with FFmpeg's decoder.

## Testing

```sh
cargo test -p cmvs-video
```

Tests use the synthetic 32×32 movie in `tests/fixtures/mgv1.hex`: frame hashes checked against FFmpeg/libtheora, damaged packets and geometry, padded and cropped containers (including an odd left edge), and prefetching against inline decoding. For a real movie, `cmvs-mgv <movie.mgv> --check-frames` prints the RGBA hash of every frame, which can be compared before and after a change.
