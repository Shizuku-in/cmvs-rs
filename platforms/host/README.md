# Embedded host builds

`cmvs-host` exposes the window-independent player through
[`cmvs_host.h`](../../crates/cmvs-host/cmvs_host.h). Its CMake target `cmvs_host`
builds the Rust static library and exports the platform's native link libraries.
Hosts own presentation, input coordinates, writable save paths and font files.

`cmvs_host_open` keeps game resources read-only and uses an explicit external
user directory. Embedding applications that want portable game-local saves can
instead call `cmvs_host_open_game_savedata`: gameplay slots, system progress,
settings and font preferences then live in `<game>/savedata/`. Its external
path is the old per-game profile and the fallback for read-only game media.
Existing external saves are imported once under both profile locks, without
overwriting local files or deleting the originals. Only permission/read-only
errors select the fallback; lock contention and corrupt saves remain errors.
The game-local directory must be a real directory, not a symlink to resources.

Supported targets are macOS arm64/x86_64, iOS device/simulator, Android
arm64-v8a/armeabi-v7a/x86_64/x86 (API 26+), and Linux aarch64/x86_64 with glibc.
Install Rust stable and the corresponding `rustup target` before building.
Build one architecture per directory. Linux additionally requires ALSA headers
and pkg-config (for example `libasound2-dev pkg-config` on Debian/Ubuntu).

```sh
# Native Linux, or macOS with -DCMAKE_OSX_ARCHITECTURES=arm64.
cmake -S platforms/host -B target/host -G Ninja
cmake --build target/host

# Android: the NDK wrappers are used by Cargo as well as CMake.
rustup target add aarch64-linux-android
cmake -S platforms/host -B target/host-android -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
  -DANDROID_ABI=arm64-v8a -DANDROID_PLATFORM=android-26
cmake --build target/host-android
```

For embedding, add this directory with `add_subdirectory` and link `cmvs_host`.
`CMVS_CARGO_TARGET_DIR` may select a shared Cargo cache. Apple toolchains choose
the device or simulator Rust target from the normalized SDK path. The UIKit
application builder remains in [`platforms/apple`](../apple/README.md).

On Android, call `cmvs_host_initialize_android(java_vm, application_context)`
before opening a player with native audio. The context must be a JNI global
reference to the Application, retained until process exit; an Activity may be
destroyed while audio threads are still using it. Initialization is idempotent
and belongs to the embedding host, in place of `ndk-glue`/`android-activity`
initialization. Those runtimes must not also initialize the same `ndk-context`.
Without initialization the player returns an error instead of entering CPAL
with an absent Android context. Other platforms need no JNI initialization.

Android save directories use nonblocking OS file locks through rustix because
Rust's standard file-lock API does not support Android. The lock is released
when the player closes, including failed opens. The `system_profile` integration
tests cover competing sessions, save replacement, backups and reopening. Set
`CMVS_TEST_TMPDIR` to a writable directory when running those tests on a device
whose filesystem differs from the build machine.

An optional Amatsutsumi regression plays into the opening, writes a gameplay
slot, closes the player, reloads it and compares the resaved snapshot bytes:

```sh
CMVS_TEST_GAME=/path/to/game CMVS_TEST_FONT=/path/to/font.otf \
  CMVS_TEST_TMPDIR=/path/to/existing/output-directory \
  cargo test -p cmvs-host --release amatsutsumi_game_savedata_survives_player_restart \
  -- --ignored --nocapture
```

The output directory must be on the game's filesystem: the fixture hard-links
read-only resources and creates its own `savedata`, leaving the original game
and saves unchanged. Generated evidence and game assets must not be committed.
