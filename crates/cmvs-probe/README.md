# cmvs-probe

Headless checks for CMVS and CVNS games using virtual mouse or keyboard input. Each result records the engine-level observations used to judge it.

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
| L | Live resource counts over a complete route |

## Usage

```sh
cargo run --release -p cmvs-probe -- game --out report --font font.ttf
```

`--out` must name a new directory. The default checks are S1–S5, R1 and D1. Use
`--checks all` to include full-route checks or `--checks R2,R3` to select specific
checks. `--help` lists all IDs, options and budgets. The output contains
`summary.json` and `events.jsonl`. Compare report directories with
`python3 -I tools/probe_matrix.py report1 report2 --markdown`.

## Results

Reports distinguish `pass`, `fail`, `blocked`, `skipped` and `inconclusive`.
L0 means no execution error, gate stop or stall; L1 adds a round-trip or invariant
check; L2 requires reference data from the original executable. Budget exhaustion
is inconclusive. Instruction coverage is not branch coverage or game completion.
The exit status is 1 for a failed or blocked check and 2 for an argument or
run error. Exit status 0 can include skipped or inconclusive checks.

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
