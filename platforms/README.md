# Platforms

| Platform | Target | Status | Build |
|---|---|---|---|
| Windows / Linux / [macOS](apple/README.md) | Host | Supported | `cargo build --release --features desktop,audio-output --bin cmvs-desktop` |
| [iOS](apple/README.md) | `aarch64-apple-ios` | Experimental | `python3 platforms/apple/build.py ios` |
| Android | `aarch64-linux-android`, `x86_64-linux-android` | Supported | `platforms/android/gradlew assembleDebug` |
| [Web](web/README.md) | `wasm32-unknown-unknown` | Preview | `platforms/web/build.sh` |
| [Switch](switch/README.md) | `aarch64-switch` | Experimental | `platforms/switch/scripts/build-player.sh` |
| [PS Vita](psvita/README.md) | `armv7-sony-vita-newlibeabihf` | Experimental | `platforms/psvita/scripts/build-player.sh` |

Run commands from the repository root.

- Linux requires `libfreetype-dev libfontconfig-dev libasound2-dev`.
- iOS requires `--font <path>`; the device app is unsigned. Use `ios-simulator` for the simulator.
- Android: run from `platforms/android` with the NDK and `cargo-ndk`. Only `x86_64` is built by default; pass `-PrustAbis=arm64-v8a,x86_64` for devices.
- Switch and PS Vita scripts run inside the platform's Docker image; see their READMEs.
