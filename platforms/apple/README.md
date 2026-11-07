# Apple players

The macOS player uses the existing native window and audio backend. The iOS
player uses UIKit for folder selection, touch input and RGBA presentation, with
the shared `cmvs-host` player for the VM, software compositor, OpenType text,
native audio and saves. Xcode, Python 3 and Rust stable are required.

```sh
python3 platforms/apple/build.py macos
target/release/cmvs-desktop /path/to/game --encoding shift-jis \
  --user-data-dir /path/to/writable-user --audio native --font /path/to/font.otf

rustup target add aarch64-apple-ios aarch64-apple-ios-sim
python3 platforms/apple/build.py ios --font /path/to/font.otf
python3 platforms/apple/build.py ios-simulator --font /path/to/font.otf
```

iOS output is `target/apple/<target>/Cmvs.app`. The device app is unsigned:
sign it with your own provisioning profile and identity before installing it.
The simulator app has an ad-hoc signature. The build includes the explicitly
provided font; verify its redistribution terms before sharing the bundle.
Game files are never bundled.

Open a game folder through the iOS document picker. The security-scoped folder
remains accessible until closing the game. Input is mapped through the
letterboxed game rectangle; a short tap survives until the next VM frame.
Saves and settings go to a stable per-folder namespace under the app's
Documents/Saves directory. Selecting another folder saves preferences and
releases the previous player. Backgrounding pauses execution and mutes audio.

For simulator diagnostics, `SIMCTL_CHILD_CMVS_GAME_DIR` can point to an existing
read-only game directory when running `xcrun simctl launch`. Optional
`SIMCTL_CHILD_CMVS_SMOKE_FRAMES` and `SIMCTL_CHILD_CMVS_SMOKE_OUTPUT` capture a
completed frame and close the player after the requested number of VM frames.
These use the same player and presentation path as the document picker.

The host C API is in `crates/cmvs-host/cmvs_host.h`. Calls on a handle are
serialized by its owner. The host must respect cooperative budget slices:
only completed VM frames advance the serial and expose new RGBA pixels. Native
audio owns its device callback. The host does not implement desktop window
management, a font picker or a save-comment editor. Font selection is cancelled;
unsupported platform requests report errors. CMV decoding is shared with core;
MGV/Theora movies decode through `cmvs-video` on a prefetching worker thread.

## Validation, 2026-11-06

Apple Silicon macOS release compilation, a native Amatsutsumi 1.03 title run
with a 48 kHz stereo output device, and iOS arm64 device/simulator linking were
tested. The iOS 18.3 simulator ran the same installation for 600 completed
frames and exported a non-black 1280x720 title frame. Device execution has not
been tested. 1,050 core/CLI/host tests pass (14 external-data tests skipped).
The host also passes Clippy with `--no-deps -D warnings`. Whole-dependency strict
Clippy encounters the pre-existing `manual_range_contains` warning in
`cmvs-core/src/camera/view.rs` with Rust 1.96.

The real-installation probe passes R1, I4 and V1-V4: fresh startup, backlog
round-trip, save/restart/load continuation through 20 message waits, snapshot
round-trip and backup-slot behavior. These are headless input replays, not
physical iOS touch verification. V5, which plays a complete route before
checking system-save persistence, exhausted both a 10-million-step budget and
the default 50-million-step budget; complete-route coverage is not claimed.

The original and exact 1.03 startup hashes belong to the same `amatsutsumi`
record, retaining the existing Android/Switch save-directory identity. Its
script archive and original x86/x64 executables are unchanged. Game records
may list multiple exact `start` hashes; duplicate hashes or archives across
separate games remain invalid, and unknown startup hashes fail identification.
The tested resources use Japanese text. Third-party DLL/PK translation overlays
are not loaded by this implementation.
