#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/../.."
cargo build --locked -p cmvs-web --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/cmvs_web.wasm --target web --out-dir platforms/web/pkg
