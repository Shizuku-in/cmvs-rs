#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
: "${DEVKITPRO:=/opt/devkitpro}"
: "${DEVKITA64:=$DEVKITPRO/devkitA64}"
sh scripts/build-core.sh
mkdir -p build
"$DEVKITA64/bin/aarch64-none-elf-gcc" runtime/smoke.c runtime/abi.c ../../target/aarch64-switch/release/libcmvs_native.a -o build/cmvs-switch-smoke.elf \
    -Wall -Wextra -Werror -O2 -march=armv8-a+crc+crypto -mtune=cortex-a57 \
    -mtp=soft -fPIE -I"$DEVKITPRO/libnx/include" \
    -specs="$DEVKITPRO/libnx/switch.specs" -L"$DEVKITPRO/libnx/lib" -lnx -lm
"$DEVKITPRO/tools/bin/nacptool" --create 'CMVS Switch smoke' 'cmvs-rs' '0.1.0' build/cmvs-switch-smoke.nacp
"$DEVKITPRO/tools/bin/elf2nro" build/cmvs-switch-smoke.elf build/cmvs-switch-smoke.nro --nacp=build/cmvs-switch-smoke.nacp
