# cmvs-core

Platform-independent core of the CMVS engine reimplementation. Every host and tool builds on it. On native targets it has no third-party dependencies. It does no windowing, rendering to a device or audio output: hosts read its state and supply those services.

## Contents

| Area | Modules |
|---|---|
| Formats | `cpz` (resource archives), `ps3` (PS2A scripts), `pb3`, `jbp1`, `jbpd` (images), `cmv`, `mgv` (movies), `save`, `system_save`, `snapshot` |
| VM | `bytecode`, `vm`, `machine` (bounded multi-slot stepping), `dispatch` (system-command routing), `evaluate` |
| Engine state | `scene`, `message`, `buttons`, `camera`, `effect_objects`, `settings`, `history`, `input`, `gesture` |
| Rendering | `scene_render`: software compositor and the draw queue executed by GPU backends |
| Hosting | `platform` (host requests), `resources`, `filesystem`, `runner` (startup orchestration), `viewport` |
| Builds | `profile`: which analysed engine build supplies the semantics for a game |

Engine builds are described by the files in [`profiles/`](profiles); see [engine profiles](../../docs/design/engine-profiles.md). Unknown games, unimplemented commands and unsupported codecs return errors instead of being guessed.

## Platforms

- On `wasm32`, `filesystem` reads files through a synchronous browser-worker adapter (via `wasm-bindgen`).
- On Android, save-directory locks use `rustix`, because the standard file-lock API is unavailable there.

## Testing

```sh
cargo test -p cmvs-core
```

Tests use synthetic data only.
