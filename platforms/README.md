# Platforms

| Platform | Target | Status | Build |
|---|---|---|---|
| Windows / Linux / [macOS](apple/README.md) | Host | Supported | `cargo build --release --features desktop,audio-output --bin cmvs-desktop` |
| [Apple launcher](apple/README.md) | macOS universal, iOS device and arm64 simulator | Experimental | `sh platforms/apple/build.sh macos`, `ios` or `ios-simulator` |
| Android | `aarch64-linux-android`, `x86_64-linux-android` | Supported | From `platforms/android`, run `sh gradlew assembleDebug` |
| [Web](web/README.md) | `wasm32-unknown-unknown` | Preview | `sh platforms/web/build.sh` |
| [Switch](switch/README.md) | `aarch64-switch` | Experimental | `sh platforms/switch/scripts/build-player.sh` |
| [PS Vita](psvita/README.md) | `armv7-sony-vita-newlibeabihf` | Experimental | `sh platforms/psvita/scripts/build-player.sh` |

Run commands from the repository root unless a different directory is listed.

- Linux requires `libfreetype-dev libfontconfig-dev libasound2-dev`.
- The Apple launcher imports fonts at runtime. Sign the device IPA before installation.
- Android: run from `platforms/android` with the NDK and `cargo-ndk`. Only `x86_64` is built by default; pass `-PrustAbis=arm64-v8a,x86_64` for devices.
- Switch and PS Vita scripts run inside the platform's Docker image; see their READMEs.

## Scale filters

Desktop accepts `--scale-filter <name>`. Android, Apple, and Web offer the same filters at launch. Choices are `nearest`, `bilinear`, `keys`, `mitchell`, `lanczos3`, `lanczos3-anti-ringing`, and `lanczos4`. The selection applies to screen presentation. Screenshots use the logical frame.

All players also read this setting from the user's `cmvs.cfg` or `cvns.cfg`. Switch and PS Vita read it from the game's `users/<game-id>/` directory and use software for filters beyond nearest and bilinear.

```ini
[CMVS_RS_DISPLAY]
SCALE_FILTER=mitchell
```

Launch choices take priority. `game` follows the original in-game draw mode.
