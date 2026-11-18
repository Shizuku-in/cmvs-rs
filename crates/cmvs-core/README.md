# cmvs-core

Platform-independent core of the CMVS engine reimplementation. Every host and tool builds on it. On native targets it has no third-party dependencies. It does no windowing, rendering to a device or audio output: hosts read its state and supply those services.

## Domain Modules

| Module | Contents |
|---|---|
| `format` | `cpz` (resource archives, CPZ1–CPZ7), `ps3` (`PS2A`/`PSS0` scripts), `pb3`, `pb2`, `msk`, `jbp1`, `jbpd` (images), `cmv`, `cmv9`, `mgv` (movies), `mv2` (MV2X audio), `cursor`, `exe`, `digest`, `lzss`, `save`, `system_save`, `snapshot` |
| `vm` | `bytecode`, `machine` (bounded `'static + Send` multi-slot stepping), `dispatch` (system-command routing), `evaluate`, `cvns`, plus `ParameterStack`, `StackWord`, `StringRef`, `SystemResult` |
| `profile` | Engine build registry, game identification, and `CommandGate` authorization |
| `render` | `scene`, `camera`, `message` (and `GlyphRasterizer`), `fade`, `capture`, `raster`, `scene_render` (software compositor and GPU draw queue), `viewport` |
| `host` | `filesystem` (`HostFs`, `PathError`, `resolve_under*`), `resources`, `image`, `audio`, `movie`, `platform`, `input`, `gesture`, `random`, `settings`, `text_encoding` |
| `runtime` | `Runtime` (`'static + Send` session and frame loop), `bootstrap`, `script_control`, `save_menu`, and session-owned state types |

Engine builds use static playback tables generated from [`crates/cmvs-profile-research/profiles/`](../../crates/cmvs-profile-research/profiles/). See [engine profiles](../../docs/design/engine-profiles.md). Unknown games, unimplemented commands and unsupported codecs return errors.

## Platforms

- All targets compile `cmvs-core` with zero third-party dependencies.
- The default `std-fs` feature provides `host::filesystem::StdFs`; hosts with custom storage, locking, or `replace` semantics (browser workers, Android, Horizon/Vita) inject a [`host::filesystem::HostFs`] implementation instead (and may disable `std-fs` via `default-features = false`).

## Testing

```sh
cargo test -p cmvs-core
```

Tests use synthetic data only.
