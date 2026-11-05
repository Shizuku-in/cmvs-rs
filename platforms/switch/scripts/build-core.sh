#!/bin/sh
set -eu
cd "$(dirname "$0")/../../.."
sh platforms/switch/scripts/prepare-libc.sh
cargo +nightly-2026-01-20 build --config 'patch.crates-io.libc.path="platforms/switch/build/libc-0.2.178"' -p cmvs-native --lib --release --target platforms/switch/toolchain/aarch64-switch.json -Zbuild-std=std,panic_abort
