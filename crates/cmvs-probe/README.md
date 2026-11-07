# cmvs-probe

Headless whole-run checks for CMVS games. It drives the engine with virtual mouse or keyboard input and judges the results from engine-level observations. It contains no game names, script names or script addresses, and every result states the verdict level it reached. See [probe design](../../docs/design/probe.md).

## Checks

| Group | Covers |
|---|---|
| S | Static: engine profile, file hashes, command gate status, archives, shipped saves |
| R | Routes: fresh start, choice-tree exploration, unlock rounds, executed against decoded instructions |
| D | Determinism, frame-time independence, damaged saves |
| I | Input: message waits, mouse and keyboard parity, seeded random input |
| P | Page traversal, returning to pages, quit and return-to-title controls |
| C | Settings controls, persistence across restarts, text speed |
| V | Save, restart and load round-trips, system save across restarts |

## Usage

```sh
cargo run --release -p cmvs-probe -- <game-root> --out <new-dir> --checks all
```

`--out` must name a directory that does not exist yet. `--help` lists every check and option, including the step and route budgets.
