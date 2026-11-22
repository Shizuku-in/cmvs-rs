# cmvs-cli

Read-only format inspectors, bounded VM trace tool, and desktop player entry point for CMVS / CVNS games.

## Binaries

| Binary | Features | Purpose |
|---|---|---|
| `cmvs-inspect` | default (`default-run`) | PS2A/PSS0 script header, segment and raw hex string inspection |
| `cmvs-trace` | default | Bounded read-only VM execution trace and profile gate diagnostics |
| `cmvs-cpz` | default | CPZ1/2/3/5/6/7 archive header inspection, directory listing and extraction |
| `cmvs-save` | default | CMVS CSV2 gameplay save layout and snapshot inspection |
| `cmvs-system-save` | default | CMVS CSS1 system save inspection and payload export |
| `cmvs-cursor` | default | Windows `.cur` cursor metadata and RGBA / AND-XOR plane export |
| `cmvs-pb3` | default | PB3B image header inspection and RGBA export |
| `cmvs-cmv` | default | CMV6 movie container inspection and JBPD frame decoding |
| `cmvs-mgv` | default | MGV1 movie container and headless Ogg/Theora frame decoding (`cmvs-video`) |
| `cmvs-audio` | `audio-output` | Ogg Vorbis, PCM WAVE, MV2X and CMV6 soundtrack inspection and f32 PCM export |
| `cmvs-desktop` | `desktop` (+ optional `wgpu-renderer`, `audio-output`) | Native desktop player entry point |

From the workspace root, use `cargo run --bin cmvs-inspect -- --help` or substitute another binary. Add the required feature before `--` for `cmvs-desktop` or `cmvs-audio`. See the [CLI Guide](../../docs/guides/cli.md) for more examples and research tools.
