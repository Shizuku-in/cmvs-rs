# cmvs-video

MGV1 movie decoding for CMVS hosts: a Rust Theora decoder, RGBA conversion and optional worker-thread prefetching. No window or device dependencies.

`cmvs-core::mgv` handles the container and nominal-frame mapping; `ogg` handles packet framing.

## API

| Item | Purpose |
|---|---|
| `PrefetchTheoraFactory` | Keeps bounded lookahead on a worker thread; `on_thread_start` allows thread setup |
| `TheoraFactory` | Decodes on the calling thread |
| `TheoraStream` | Stateful decoder used by both factories and the `cmvs-mgv` tool |

Install a factory with `Runtime::set_mgv_factory`. Both factories produce the same frames.

## Decoding

- Supports Theora 3.2 YUV420. Container dimensions must match the picture or coded surface; picture dimensions crop the visible region, while coded dimensions retain padding.
- Validates frame rate, packet sizes and pixel limits. Invalid streams return errors.
- Reuses frame buffers and caches the current/next RGBA frames. Repeated nominal frames keep the current packet cached; backward seeks replay from the headers.
- Default lookahead retains at most 16 RGBA frames / 64 MiB (minimum two for large frames). `with_lookahead(bytes, frames)` stores YUV instead and converts only displayed frames; cache hits do not wait for the decoding worker.
- Converts YUV to RGBA using BT.601 limited range.

## Testing

```sh
cargo test --offline -p cmvs-video
```

## Implementation and validation

The bounded decoder lives in `src/theora`: headers and geometry, coefficient
packets, reference-frame reconstruction, IDCT and loop filtering. Malformed packets
return errors; the stream rebuilds decoder state before retrying. Unsupported
4:2:2, 4:4:4 and legacy VP3 streams are rejected.

Private DSP kernels allow unsafe locally under `unsafe_code = deny` and
`unsafe_op_in_unsafe_fn = deny`. Fixed arrays or checked spans establish memory
bounds; each block documents its CPU and memory requirements. x86-64 uses SSE2,
AArch64 uses NEON for pixel operations, and other targets use scalar kernels.
SIMD must preserve the scalar overflow, truncation and rounding behavior.

Synthetic fixtures cover reference frames, quantizers, cropping, repeated frames,
malformed input and scalar/SIMD parity. Generation and independent FFmpeg checks
are described in [fixtures](tests/fixtures/README.md). Real-movie hashes and the
scope of completed validation belong to the [game record](../../docs/games/hapymaherfd_re/README.md#mgvtheora-自研后端验证2026-11-08).

External checks use read-only inputs:

```sh
CMVS_THEORA_MGV="<game>/data/video/op.mgv" \
  cargo test --offline --release -p cmvs-video external_mgv_playback_matches_packet_decoder -- --ignored --nocapture
CMVS_THEORA_OGV="<sample>/op.ogv" CMVS_THEORA_FFMPEG=ffmpeg \
  cargo test --offline --release -p cmvs-video external_corpus_matches_ffmpeg -- --ignored --nocapture
```

The playback check compares container mapping and replay against direct packet
decoding. FFmpeg supplies the independent pixel reference; set
`CMVS_THEORA_CODED=1` to compare the full coded surface instead of the visible crop.
`external_decoder_timing` measures packet decoding separately from I/O, conversion
and presentation; desktop timings do not establish console performance.

## Sources

- [Xiph Theora I specification](https://www.theora.org/doc/Theora.pdf), sections 2, 6 and 7;
  [LaTeX source](https://github.com/xiph/theora/blob/master/doc/spec/spec.tex).
- `oxideav-theora` 0.0.12 (MIT) supplied implementation references and fixed
  differential outputs. The dependency was removed; its attribution remains in
  [THIRD_PARTY_NOTICES.txt](../../THIRD_PARTY_NOTICES.txt).
- FFmpeg / libtheora are external fixture and validation tools, not linked runtime dependencies.
