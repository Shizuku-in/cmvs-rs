#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
: "${DEVKITPRO:=/opt/devkitpro}"
: "${DEVKITA64:=$DEVKITPRO/devkitA64}"
sh platforms/switch/prepare-libc.sh
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/window.c -o platforms/switch/build/window.o \
    -Wall -Wextra -Werror -O2 -march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE -I"$DEVKITPRO/libnx/include"
"$DEVKITA64/bin/aarch64-none-elf-gcc" -c platforms/switch/runtime/abi.c -o platforms/switch/build/abi.o
cargo +nightly-2026-01-20 rustc --config 'patch.crates-io.libc.path="platforms/switch/build/libc-0.2.178"' \
    -p cmvs-switch --release --target platforms/switch/aarch64-switch.json -Zbuild-std=std,panic_abort -- \
    -C "linker=$PWD/platforms/switch/linker.py" \
    -C "link-arg=-specs=$DEVKITPRO/libnx/switch.specs" \
    -C "link-arg=$PWD/platforms/switch/build/window.o" \
    -C "link-arg=-L$DEVKITPRO/libnx/lib" -C link-arg=-lnx -C link-arg=-lm
"$DEVKITPRO/tools/bin/nacptool" --create 'CMVS Switch' 'cmvs-rs' '0.1.0' platforms/switch/build/cmvs-switch.nacp
"$DEVKITPRO/tools/bin/elf2nro" target/aarch64-switch/release/cmvs-switch platforms/switch/build/cmvs-switch.nro --nacp=platforms/switch/build/cmvs-switch.nacp
