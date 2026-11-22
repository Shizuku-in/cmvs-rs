# cmvs-disassembler

Linear `PS2A`/`PSS0` script disassembler, symbolic project exporter, and translation table exporter for CMVS and CVNS scripts.

## Usage

From the workspace root:

```sh
# Print linear instruction decode and system command counts (not VM execution)
cargo run --offline -p cmvs-disassembler -- <script> [--profile <id|game>]

# Export an editable cmvs-script-v1 project (default --source-encoding: shift-jis)
cargo run --offline -p cmvs-disassembler -- <script> --export <new.project> [--profile <id|game>] [--source-encoding <encoding>]

# Export a cmvs-text-v1 translation TSV
cargo run --offline -p cmvs-disassembler -- <script> --export-text <new.tsv> --source-encoding <shift-jis|gbk|utf-8>
```

Output paths must not already exist. Reassemble projects or import edited translation tables with [`cmvs-assembler`](../cmvs-assembler/README.md); see the [CLI Guide](../../docs/guides/cli.md) for the `cmvs-script-v1` and `cmvs-text-v1` formats.
