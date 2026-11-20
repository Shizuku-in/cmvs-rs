# cmvs-tooling

Shared offline CLI argument parsing, no-clobber file and directory staging, and script layout / translation-table conventions for the CMVS offline tools.

## Modules

| Module | Contents |
|---|---|
| `lib` | `read_bounded` input reading, `write_new` atomic non-clobbering file creation, and `write_new_dir` staged directory publication |
| `cli` | `Arguments` option/flag parsing, `wants_help`, profile resolution via `cmvs-core`, and script container validation |
| `project` | `cmvs-script-v1` (`FORMAT`, `DATA_ROW`), instruction `Field` classification, `entry_points`, and `callback_target` analysis |
| `script` | `ScriptText` string boundaries across `text` and `data` segments, bytecode references, and string expression accumulators |
| `table` | `cmvs-text-v1` (`FORMAT`), `Encoding` (`shift-jis`, `gbk`, `utf-8`), `escape`/`unescape`, and SHA-256 script `fingerprint` |

## Used by

`cmvs-disassembler` and `cmvs-assembler`.
