# cmvs-probe

Headless whole-run checks for CMVS games. It drives the engine with virtual mouse or keyboard input and judges the results from engine-level observations. It contains no game names, script names or script addresses, and every result states the verdict level it reached.

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

## Results

Reports distinguish `pass`, `fail`, `blocked`, `skipped` and `inconclusive`.
L0 means no execution error, gate stop or stall; L1 adds a round-trip or invariant
check; L2 requires reference data from the original executable. Budget exhaustion
is inconclusive. Instruction coverage is not branch coverage or game completion.
The probe does not silently relax the selected command policy.

## Manifest

`--manifest` supplies corrections using engine-level identifiers, without game
logic. One directive per line; `#` starts a comment and evidence after `--` is required:

```text
profile <id>                                      -- <evidence>
entry <group> <button>                            -- <evidence>
start-route                                      -- <evidence>
choice-group <script> <slot>                      -- <evidence>
label page <script> <slot>x<buttons>[+...] <text>  -- <evidence>
label choice <script> <pc-hex> <text>              -- <evidence>
```

Reports record manifest usage and disagreements with automatic recognition.
Keep game-specific results in [game records](../../docs/games/README.md).
