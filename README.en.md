# cmvs-rs

[简体中文](README.md) | English

A **Rust** reimplementation of Purple SOFTWARE's CMVS engine.

## Demos

<table>
  <tr>
    <th colspan="2">Amatsutsumi</th>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/amatsutsumi_windows11.webp" alt="Windows Example of Amatsutsumi" width="100%">
    </td>
    <td align="center" width="50%">
      <img src="docs/images/amatsutsumi_ubuntu.webp" alt="Linux Example of Amatsutsumi" width="100%">
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Windows</strong></td>
    <td align="center"><strong>Linux</strong></td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/amatsutsumi_android.webp" alt="Android Example of Amatsutsumi" width="100%">
    </td>
    <td align="center" width="50%">
      <img src="docs/images/amatsutsumi_switch.webp" alt="Switch (Ryujinx) Example of Amatsutsumi" width="100%">
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Android</strong></td>
    <td align="center"><strong>Switch (Ryujinx)</strong></td>
  </tr>
  <tr>
    <th colspan="2">Aoitori</th>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/aoitori_windows11.webp" alt="Windows Example of Aoitori" width="100%">
    </td>
    <td align="center" width="50%">
      <img src="docs/images/aoitori_ubuntu.webp" alt="Linux Example of Aoitori" width="100%">
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Windows</strong></td>
    <td align="center"><strong>Linux</strong></td>
  </tr>
  <tr>
    <td align="center" width="50%">
      <img src="docs/images/aoitori_android.webp" alt="Android Example of Aoitori" width="100%">
    </td>
    <td align="center" width="50%">
      <img src="docs/images/aoitori_switch.webp" alt="Switch (Ryujinx) Example of Aoitori" width="100%">
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Android</strong></td>
    <td align="center"><strong>Switch (Ryujinx)</strong></td>
  </tr>
</table>

## Getting Started

Requires **Rust 1.89+ stable**.

```bash
cargo run --offline -- "<game-directory>/data/pack/start.ps3"
cargo test --offline
```

See the [Running Guide](docs/guides/running.md) for desktop previews, startup snapshot barriers, fonts, audio, and a separate user data directory. See the [CLI Guide](docs/guides/cli.md) for inspection tools.

## Platforms

See [Platforms](platforms/README.md) for targets, support status, and build instructions.

## Documentation and Development

- [Documentation](docs/README.md): usage, APIs, formats, and backends.
- [Game records](docs/games/README.md): support and validation scope.
- [Architecture and checks](docs/architecture.md), [research guidelines](docs/research.md), and [references](docs/references.md).

## License

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)
