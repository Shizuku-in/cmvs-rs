# cmvs-profile-research

Offline tools for maintaining CMVS and CVNS engine profiles and generating static playback tables for `cmvs-core`. Source profiles, game records, and contract evidence live in [`profiles/`](profiles/).

## API

- `ResearchRegistry` loads and validates the source profiles, game records, and contract catalog.
- `exe::PeImage` and `exe::find_dispatch` inspect Windows executables and locate command dispatch tables.

## CLI

| Command | Purpose |
|---|---|
| `identify` | Identify a game build and check its executable dispatch tables against the profile |
| `table` | Print a candidate profile from the game's executables |
| `show` | Inspect command statuses and mappings by profile ID or game alias |
| `generate-playback` | Validate the source data and generate Rust playback tables |
| `owners` | List candidate handlers for CMVS system commands |

Run from the workspace root.

```sh
cargo run --offline --locked -p cmvs-profile-research -- show aki
```

For the profile format and table generation workflow, see [Engine Profiles](../../docs/design/engine-profiles.md).

## Testing

```sh
cargo test -p cmvs-profile-research
```
