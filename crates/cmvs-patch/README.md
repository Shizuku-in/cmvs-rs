# cmvs-patch

Offline Rust helper library and CLI for converting supported fan-translation patches into standard `cmvs-rs` script, image, and archive containers (`PS2A`, `PB3B`, `CPZ3`/`CPZ5`/`CPZ6`/`CPZ7`).

## Usage

User-facing conversion entry points live in [`tools/fan_patch/`](../../tools/fan_patch/README.md), which validate input SHA-256 digests, unpack patch archives into a temporary stage, and invoke `cmvs-patch`:

```sh
cargo run --offline --locked --release -p cmvs-patch -- <game> <args...>
```

Supported `<game>` subcommands: `amatsutsumi`, `aoitori`, `asukimi`, `chronoclock`, `hapymaher`, `kunado`, `miranosu`, and `realive`. Run `cmvs-patch --help` for subcommand arguments.
