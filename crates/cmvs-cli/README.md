# cmvs-cli

Read-only format inspectors, linear bytecode disassembler, bounded VM trace tool, engine-profile inspector, and desktop player entry point for CMVS / CVNS games.

## Binaries

| Binary | Features | Purpose |
|---|---|---|
| `cmvs-inspect` | default (`default-run`) | PS2A/PSS0 script header, segment and raw hex string inspection |
| `cmvs-disasm` | default | Linear PS2A/PSS0 bytecode disassembly and system command counts |
| `cmvs-trace` | default | Bounded read-only VM execution trace and profile gate diagnostics |
| `cmvs-cpz` | default | CPZ1/2/3/5/6/7 archive header inspection, directory listing and extraction |
| `cmvs-profile` | default | Engine profile identification, table generation and ownership queries |
| `cmvs-save` | default | CSV2 gameplay save layout, record framing, VM and visual snapshot inspection |
| `cmvs-system-save` | default | CSS1 persistent system save inspection and payload export |
| `cmvs-cursor` | default | Windows `.cur` cursor metadata and RGBA / AND-XOR plane export |
| `cmvs-pb3` | default | PB3B image header inspection and RGBA export |
| `cmvs-cmv` | default | CMV6 movie container inspection and JBPD frame decoding |
| `cmvs-mgv` | default | MGV1 movie container and headless Ogg/Theora frame decoding (`cmvs-video`) |
| `cmvs-audio` | `audio-output` | Vorbis / CMV soundtrack inspection and f32 PCM export without opening a device |
| `cmvs-desktop` | `desktop` (+ optional `wgpu-renderer`, `audio-output`) | Native desktop player entry point |

See the [CLI Guide](../../docs/guides/cli.md) and [Running Guide](../../docs/guides/running.md) for usage details.
