#!/bin/sh
# First gate: compile the portable engine and std for Horizon without linking.
set -eu
cd "$(dirname "$0")/../../.."
cargo +nightly-2026-01-20 check -p cmvs-core --target platforms/switch/toolchain/aarch64-switch.json -Zbuild-std=std,panic_abort
