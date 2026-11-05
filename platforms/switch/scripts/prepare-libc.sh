#!/bin/sh
# Private build-std dependency copy; never modify Cargo's registry or host libc.
set -eu
cd "$(dirname "$0")/.."
mkdir -p build
if [ ! -f build/libc-0.2.178/.cmvs-patched ]; then
    curl -fsSL https://static.crates.io/crates/libc/libc-0.2.178.crate -o build/libc.crate
    echo '37c93d8daa9d8a012fd8ab92f088405fb202ea0b6ab73ee2482ae66af4f42091  build/libc.crate' | sha256sum -c -
    tar -xzf build/libc.crate -C build
    # This private dependency is used exclusively by the AArch64 Horizon build.
    sed -i 's/pub type blkcnt_t = i32;/pub type blkcnt_t = i64;/; s/pub type blksize_t = i32;/pub type blksize_t = i64;/; s/pub type dev_t = u32;/pub type dev_t = i16;/; s/pub type ino_t = u32;/pub type ino_t = u16;/' build/libc-0.2.178/src/unix/newlib/mod.rs
    touch build/libc-0.2.178/.cmvs-patched
fi
